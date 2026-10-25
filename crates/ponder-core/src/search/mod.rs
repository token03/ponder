mod bounds;
mod graph;

use std::fmt;

use faer::Mat;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::linear::{self, Backend, Lu, System};
use crate::transition::{ProbabilityRange as Bound, TransitionBuffer, TransitionResult};
use crate::types::{Outcome, State};
use graph::{Graph, explore};

pub type StateMap<V> = FxHashMap<State, V>;

#[derive(Clone, Copy, Debug)]
pub struct Decision {
    pub id: usize,
    pub cost: f64,
}

pub trait Model {
    fn origin(&self) -> State;
    fn is_goal(&self, state: &State) -> bool;
    fn actions(&self) -> &[Decision];
    fn outcomes(&self, state: &State, action: usize, buffer: &mut TransitionBuffer) -> TransitionResult;
    fn refine(&mut self, _candidates: &[(State, usize)]) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub limits: SolveLimits,
    pub max_rounds: usize,
    pub backend: Backend,
    pub cost_ceiling: Option<f64>,
}

impl Default for Options {
    fn default() -> Options {
        Options { limits: SolveLimits::default(), max_rounds: 60, backend: Backend::Auto, cost_ceiling: None }
    }
}

pub struct Solution {
    pub cost: f64,
    pub states: Vec<State>,
    pub policy: Vec<usize>,
    pub values: Vec<f64>,
    pub visits: Vec<f64>,
    pub explored: usize,
    pub bounds: Bounds,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveError {
    Unreachable,
    InvalidRecovery(String),
    /// The policy's linear system is singular or its values are nonsense.
    Singular(String),
    /// `evaluate` was given a policy with no action for a state.
    MissingPolicy(String),
    IterationLimit(usize),
    Dominated {
        lower_bound: f64,
        ceiling: f64,
    },
    ResourceLimit {
        resource: String,
        limit: usize,
    },
}

impl fmt::Display for SolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolveError::Unreachable => f.write_str("no policy reaches the goal with probability 1"),
            SolveError::Singular(m) | SolveError::MissingPolicy(m) | SolveError::InvalidRecovery(m) => f.write_str(m),
            SolveError::IterationLimit(rounds) => write!(f, "solver did not converge within {rounds} policy iterations"),
            SolveError::Dominated { lower_bound, ceiling } => {
                write!(f, "certified cost of at least {lower_bound:.2} exceeds the complete recipe cost of {ceiling:.2}")
            }
            SolveError::ResourceLimit { resource, limit } => write!(f, "search incomplete: exceeded {limit} {resource}"),
        }
    }
}

impl std::error::Error for SolveError {}

#[derive(Clone, Copy, Debug)]
pub struct SolveLimits {
    pub states: usize,
    pub transitions: usize,
    pub state_actions: usize,
}

impl Default for SolveLimits {
    fn default() -> Self {
        Self { states: 100_000, transitions: 5_000_000, state_actions: 1_000_000 }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub lower: f64,
    pub upper: f64,
}

impl Bounds {
    /// The bounds of a plan whose chances are all exact.
    pub fn exact(cost: f64) -> Bounds {
        Bounds { lower: cost, upper: cost }
    }

    /// How far apart the bounds are, as a share of `cost`.
    pub fn spread(&self, cost: f64) -> f64 {
        if cost <= 0.0 { 0.0 } else { (self.upper - self.lower) / cost }
    }
}

struct OptimisticPolicy {
    actions: Vec<usize>,
    visits: Vec<f64>,
    values: Vec<f64>,
}

pub fn solve(model: &mut impl Model, options: Options) -> Result<Solution, SolveError> {
    if model.is_goal(&model.origin()) {
        return Ok(Solution {
            cost: 0.0,
            states: Vec::new(),
            policy: Vec::new(),
            values: Vec::new(),
            visits: Vec::new(),
            explored: 1,
            bounds: Bounds::exact(0.0),
        });
    }
    let mut warm: StateMap<usize> = StateMap::default();
    loop {
        let g = explore(model, options.limits, None)?.without_dead_ends().ok_or(SolveError::Unreachable)?;
        let start = g.states.iter().position(|s| *s == model.origin()).ok_or(SolveError::Unreachable)?;
        let proper: Vec<usize> = g.proper_policy().into_iter().collect::<Option<_>>().ok_or(SolveError::Unreachable)?;
        let seeded: Vec<usize> = (0..g.n())
            .map(|i| {
                warm.get(&g.states[i])
                    .and_then(|&ai| (g.state_ptr[i]..g.state_ptr[i + 1]).find(|&p| g.pair_action[p] == ai))
                    .unwrap_or(proper[i])
            })
            .collect();
        let (value, policy, lu) = if warm.is_empty() {
            iterate(&g, proper, options.max_rounds, options.backend)?
        } else {
            iterate(&g, seeded, options.max_rounds, options.backend)
                .or_else(|_| iterate(&g, proper, options.max_rounds, options.backend))?
        };
        warm = g.states.iter().zip(&policy).map(|(s, &p)| (*s, g.pair_action[p])).collect();
        let visits = g.visits(&lu, start);
        if model.refine(&refinements(&g, &policy, &visits)) {
            continue;
        }
        let (lower, kind) =
            if g.is_loose() { g.optimistic(&policy, &value, start, options.backend, options.max_rounds) } else { (value[start], None) };
        if let Some(ceiling) = options.cost_ceiling.filter(|ceiling| ceiling.is_finite())
            && lower > ceiling + 1e-9 * ceiling.abs().max(1.0)
        {
            return Err(SolveError::Dominated { lower_bound: lower, ceiling });
        }
        let upper = if g.is_loose() { g.upper_bound(&policy, &value, start, options.backend, options.max_rounds) } else { value[start] };
        let bounds = Bounds { lower, upper };
        if kind.is_some_and(|optimistic| model.refine(&potential_refinements(&g, &optimistic, &value))) {
            continue;
        }
        return Ok(Solution {
            cost: value[start],
            policy: policy.iter().map(|&p| g.pair_action[p]).collect(),
            states: g.states,
            values: value,
            visits,
            explored: g.seen,
            bounds,
        });
    }
}

fn refinements(g: &Graph, policy: &[usize], visits: &[f64]) -> Vec<(State, usize)> {
    g.states
        .iter()
        .enumerate()
        .filter(|&(i, _)| visits[i] > 1e-9 && g.loose[g.pair_list[policy[i]]])
        .map(|(i, s)| (*s, g.pair_action[policy[i]]))
        .collect()
}

fn potential_refinements(g: &Graph, optimistic: &OptimisticPolicy, incumbent: &[f64]) -> Vec<(State, usize)> {
    let costs = g.q_with(&optimistic.values, &g.extreme(&optimistic.values, false));
    g.states
        .iter()
        .enumerate()
        .filter(|(i, _)| optimistic.visits[*i] > 1e-9)
        .flat_map(|(i, &state)| {
            let costs = &costs;
            (g.state_ptr[i]..g.state_ptr[i + 1])
                .filter(move |&pair| {
                    g.loose[g.pair_list[pair]]
                        && (pair == optimistic.actions[i] || costs[pair] <= incumbent[i] + 1e-9 * incumbent[i].abs().max(1.0))
                })
                .map(move |pair| (state, g.pair_action[pair]))
        })
        .collect()
}

pub fn evaluate(model: &impl Model, policy: &StateMap<usize>, options: Options) -> Result<f64, SolveError> {
    evaluate_solution(model, policy, options).map(|solution| solution.cost)
}

pub fn evaluate_solution(model: &impl Model, policy: &StateMap<usize>, options: Options) -> Result<Solution, SolveError> {
    if model.is_goal(&model.origin()) {
        return Ok(Solution {
            cost: 0.0,
            states: Vec::new(),
            policy: Vec::new(),
            values: Vec::new(),
            visits: Vec::new(),
            explored: 1,
            bounds: Bounds::exact(0.0),
        });
    }
    let g = explore(model, options.limits, Some(policy))?;
    let chosen: Vec<usize> = (0..g.n()).map(|i| g.state_ptr[i]).collect();
    let lu = g.factor(&chosen, options.backend).ok_or_else(|| SolveError::Singular("singular matrix".into()))?;
    let start = g.states.iter().position(|s| *s == model.origin()).expect("origin is open");
    let values = g.evaluate(&lu, &chosen);
    let visits = g.visits(&lu, start);
    if !values.iter().chain(&visits).all(|value| value.is_finite()) {
        return Err(SolveError::Singular("policy values or visit counts are not finite".into()));
    }
    let bounds =
        if g.is_loose() { g.bounds(&chosen, &values, start, options.backend, options.max_rounds).0 } else { Bounds::exact(values[start]) };
    Ok(Solution {
        cost: values[start],
        policy: chosen.iter().map(|&p| g.pair_action[p]).collect(),
        states: g.states,
        values,
        visits,
        explored: g.seen,
        bounds,
    })
}

/// Policy iteration on `g` from `policy`: the values, the policy and its factored system.
fn iterate(g: &Graph, mut policy: Vec<usize>, max_rounds: usize, backend: Backend) -> Result<(Vec<f64>, Vec<usize>, Lu), SolveError> {
    for _ in 0..max_rounds {
        let singular = || SolveError::Singular("singular matrix".into());
        let f = g.factor(&policy, backend).ok_or_else(singular)?;
        let value = g.evaluate(&f, &policy);
        if !value.iter().all(|v| v.is_finite()) {
            return Err(SolveError::Singular("solver values are not finite".into()));
        }
        let better = g.improve(&g.q(&value), &policy);
        if better == policy {
            return Ok((value, policy, f));
        }
        policy = better;
    }
    Err(SolveError::IterationLimit(max_rounds))
}
