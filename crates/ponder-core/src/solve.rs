use std::collections::HashMap;

use rayon::prelude::*;

use crate::action::{ActionKind, step_cost};
use crate::linear::Backend;
use crate::problem::{PreparedAction, Problem};
use crate::search::{self, Decision, Model, Options};
pub use crate::search::{Bounds, SolveError, SolveLimits, StateMap};
use crate::transition::{TransitionBuffer, TransitionResult};
use crate::types::*;

#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// Expected chaos from the problem's origin (a fresh base unless `begin`), excluding the base itself, clicking time included.
    pub cost: f64,
    pub policy: StateMap<String>,
    pub usage: HashMap<String, f64>, // expected uses of each action
    pub visits: StateMap<f64>,       // expected times each state is acted on
    pub states: usize,
    pub unpriced: Vec<String>, // actions left out for lack of a price
    pub hours: f64,            // expected time spent clicking
    pub values: StateMap<f64>, // expected chaos to finish from every state the plan reaches
    /// The open states in the order the solver numbered them (Python's dict order): ties in `steps` follow it.
    pub order: Vec<State>,
    /// How far `cost` can be from the truth, given what the lumped states don't know; None for
    /// a plan from before bounds (an old cache entry).
    pub bounds: Option<Bounds>,
}

impl Plan {
    /// Expected chaos to finish an item in state `s` under this plan; None if the plan never meets it.
    pub fn to_finish(&self, problem: &Problem, s: &State) -> Option<f64> {
        if problem.done(s) { Some(0.0) } else { self.values.get(s).copied() }
    }

    /// Chaos spent on currency alone (`cost` less the clicking time).
    pub fn currency(&self, settings: &Settings) -> f64 {
        self.cost - self.hours * settings.chaos_per_hour
    }
}

fn cheapest_of_equals(problem: &Problem, prices: &Prices, settings: &Settings) -> Vec<usize> {
    let keys: Vec<_> =
        problem.actions.iter().map(|a| problem.roll_key(a).map(|k| (k, a.from_rarity.clone(), a.kind == ActionKind::Reforge))).collect();
    let mut best: HashMap<_, usize> = HashMap::new();
    for (i, (a, key)) in problem.actions.iter().zip(&keys).enumerate() {
        if a.cost.iter().any(|(pid, _)| !prices.contains_key(pid)) {
            continue;
        }
        if let Some(key) = key {
            match best.get(key) {
                Some(&b) if step_cost(a, prices, settings) >= step_cost(&problem.actions[b], prices, settings) => {}
                _ => {
                    best.insert(key.clone(), i);
                }
            }
        }
    }
    let (plain, rolls): (Vec<_>, Vec<_>) = problem
        .actions
        .iter()
        .enumerate()
        .filter(|(i, a)| a.cost.iter().all(|(pid, _)| prices.contains_key(pid)) && keys[*i].as_ref().is_none_or(|k| best[k] == *i))
        .map(|(i, _)| i)
        .partition(|&i| keys[i].is_none());
    plain.into_iter().chain(rolls).collect()
}

pub fn solve(problem: &Problem, prices: &Prices, settings: &Settings) -> Result<Plan, SolveError> {
    solve_with(problem, prices, settings, Options::default())
}

pub fn solve_with(problem: &Problem, prices: &Prices, settings: &Settings, options: Options) -> Result<Plan, SolveError> {
    let unpriced =
        problem.actions.iter().filter(|a| a.cost.iter().any(|(pid, _)| !prices.contains_key(pid))).map(|a| a.name.clone()).collect();
    if problem.done(&problem.origin()) {
        return Ok(Plan { states: 1, unpriced, bounds: Some(Bounds::exact(0.0)), ..Plan::default() });
    }
    let mut model = CraftModel::new(problem, prices, settings, cheapest_of_equals(problem, prices, settings));
    let solution = search::solve(&mut model, options)?;
    let mut usage: HashMap<String, f64> = model.decisions.iter().map(|d| (problem.actions[d.id].name.clone(), 0.0)).collect();
    for (i, &action) in solution.policy.iter().enumerate() {
        *usage.get_mut(&problem.actions[action].name).unwrap() += solution.visits[i];
    }
    let hours = problem.actions.iter().filter_map(|a| usage.get(&a.name).map(|n| n * a.seconds)).sum::<f64>() / 3600.0;
    Ok(Plan {
        cost: solution.cost,
        policy: solution.states.iter().zip(&solution.policy).map(|(s, &a)| (*s, problem.actions[a].name.clone())).collect(),
        usage,
        visits: solution.states.iter().zip(&solution.visits).map(|(s, &v)| (*s, v)).collect(),
        states: solution.explored,
        unpriced,
        hours,
        values: solution.states.iter().zip(&solution.values).map(|(s, &v)| (*s, v)).collect(),
        order: solution.states,
        bounds: Some(solution.bounds),
    })
}

pub fn evaluate(problem: &Problem, policy: &StateMap<String>, prices: &Prices, settings: &Settings) -> Result<f64, SolveError> {
    evaluate_with(problem, policy, prices, settings, Backend::Auto)
}

pub fn evaluate_with(
    problem: &Problem,
    policy: &StateMap<String>,
    prices: &Prices,
    settings: &Settings,
    backend: Backend,
) -> Result<f64, SolveError> {
    let indexed: Result<StateMap<usize>, _> = policy
        .iter()
        .map(|(s, name)| {
            problem
                .actions
                .iter()
                .position(|a| &a.name == name)
                .map(|a| (*s, a))
                .ok_or_else(|| SolveError::MissingPolicy(format!("{name:?} is not an action")))
        })
        .collect();
    let indexed = indexed?;
    let mut active: Vec<usize> = indexed.values().copied().collect();
    active.sort_unstable();
    active.dedup();
    let model = CraftModel::new(problem, prices, settings, active);
    search::evaluate(&model, &indexed, Options { backend, ..Default::default() })
}

struct CraftModel<'p> {
    problem: &'p Problem,
    decisions: Vec<Decision>,
    prepared: Vec<Option<PreparedAction>>,
}

impl<'p> CraftModel<'p> {
    fn new(problem: &'p Problem, prices: &Prices, settings: &Settings, active: Vec<usize>) -> CraftModel<'p> {
        let decisions = active.into_iter().map(|id| Decision { id, cost: step_cost(&problem.actions[id], prices, settings) }).collect();
        let mut model = CraftModel { problem, decisions, prepared: (0..problem.actions.len()).map(|_| None).collect() };
        model.prepare();
        model
    }

    fn prepare(&mut self) {
        let prepared: Vec<_> = self.decisions.par_iter().map(|d| (d.id, self.problem.prepare(&self.problem.actions[d.id]))).collect();
        for (id, action) in prepared {
            self.prepared[id] = Some(action);
        }
    }
}

impl Model for CraftModel<'_> {
    fn origin(&self) -> State {
        self.problem.origin()
    }
    fn is_goal(&self, state: &State) -> bool {
        self.problem.done(state)
    }
    fn actions(&self) -> &[Decision] {
        &self.decisions
    }

    fn outcomes(&self, state: &State, action: usize, buffer: &mut TransitionBuffer) -> TransitionResult {
        let (outcomes, bounds) = buffer.parts_mut();
        self.problem.outcomes_into(state, self.prepared[action].as_ref().expect("active action is prepared"), outcomes, bounds)
    }

    fn refine(&mut self, candidates: &[(State, usize)]) -> bool {
        let mut wanted: Vec<usize> = candidates.iter().filter(|(s, _)| meta_locks(s.meta).is_none()).map(|(_, a)| *a).collect();
        wanted.sort_unstable();
        wanted.dedup();
        let changed = wanted.par_iter().map(|&id| self.problem.refine(&self.problem.actions[id])).collect::<Vec<_>>().contains(&true);
        if changed {
            self.prepare();
        }
        changed
    }
}
