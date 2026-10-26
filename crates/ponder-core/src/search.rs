use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use faer::Mat;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::linear::{self, Backend, Lu, System};
use crate::transition::{Distribution, ProbabilityRange as Bound, TransitionBuffer, TransitionResult};
use crate::types::{Moments, Outcome, PolicyRisk, State};

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
    fn actions_for(&self, _state: &State) -> &[Decision] {
        self.actions()
    }
    fn dominant_action(&self, _state: &State) -> Option<usize> {
        None
    }
    fn measurements(&self, _state: &State, _action: usize) -> Option<[f64; 4]> {
        None
    }
    fn outcomes(&self, state: &State, action: usize, buffer: &mut TransitionBuffer) -> TransitionResult;
    fn refine(&mut self, _candidates: &[(State, usize)]) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub algorithm: Algorithm,
    pub limits: SolveLimits,
    pub max_rounds: usize,
    pub backend: Backend,
}

impl Default for Options {
    fn default() -> Options {
        Options { algorithm: Algorithm::Eager, limits: SolveLimits::default(), max_rounds: 60, backend: Backend::Auto }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Eager,
    Ilao,
    CgIlao,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Work {
    pub expanded_states: usize,
    pub state_actions: usize,
    pub transitions: usize,
    pub q_values: usize,
    pub policy_evaluations: usize,
}

pub struct Solution {
    pub work: Work,
    pub risk: Option<PolicyRisk>,
    pub cost: f64,
    pub states: Vec<State>,
    pub policy: Vec<usize>,
    pub values: Vec<f64>,
    pub visits: Vec<f64>,
    pub explored: usize,
    /// The policy relies on lumped (estimated) chances somewhere it goes.
    pub approximate: bool,
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

pub fn solve<M: Model>(model: &mut M, options: Options) -> Result<Solution, SolveError> {
    if options.algorithm != Algorithm::Eager && model.actions().iter().all(|decision| decision.cost >= 0.0) {
        focused(model, options)
    } else {
        eager(model, options)
    }
}

fn eager<M: Model>(model: &mut M, options: Options) -> Result<Solution, SolveError> {
    if model.is_goal(&model.origin()) {
        return Ok(Solution {
            work: Work::default(),
            risk: Some(PolicyRisk::default()),
            cost: 0.0,
            states: Vec::new(),
            policy: Vec::new(),
            values: Vec::new(),
            visits: Vec::new(),
            explored: 1,
            approximate: false,
        });
    }
    let mut warm: StateMap<usize> = StateMap::default();
    let build = |model: &mut M| explore(model, options.limits, None)?.without_dead_ends().ok_or(SolveError::Unreachable);
    let mut g = build(model)?;
    let mut refreshed = false;
    loop {
        let start = g.states.iter().position(|s| *s == model.origin()).ok_or(SolveError::Unreachable)?;
        let proper: Vec<usize> = match g.proper_policy().into_iter().collect::<Option<_>>() {
            Some(proper) => proper,
            None if refreshed => {
                g = build(model)?;
                refreshed = false;
                continue;
            }
            None => return Err(SolveError::Unreachable),
        };
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
        // Lumped rolls the plan actually uses are worked out exactly, then the plan is redone.
        let wanted = refinements(&g, &policy, &visits);
        if model.refine(&wanted) {
            let mut actions: Vec<usize> = wanted.iter().map(|&(_, action)| action).collect();
            actions.sort_unstable();
            actions.dedup();
            (g, refreshed) = match g.refresh(model, &actions, options.limits)? {
                Some(graph) => (graph, true),
                None => (build(model)?, false),
            };
            continue;
        }
        return Ok(Solution {
            work: Work { expanded_states: g.n(), state_actions: g.pair_list.len(), transitions: g.e_next.len(), ..Work::default() },
            risk: g.assess(model, &policy, &lu, &visits, start),
            cost: value[start],
            approximate: g.approximate(&policy, &visits),
            policy: policy.iter().map(|&p| g.pair_action[p]).collect(),
            states: g.states,
            values: value,
            visits,
            explored: g.seen,
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

struct Candidate {
    owner: usize,
    decision: Decision,
    list: Option<usize>,
    active: bool,
}

struct PartialGraph {
    builder: Builder,
    rows: Vec<Option<Vec<usize>>>,
    candidates: Vec<Candidate>,
    values: Vec<f64>,
    predecessor_lists: Vec<Vec<usize>>,
    list_users: Vec<Vec<usize>>,
    list_values: Vec<Option<f64>>,
    pending: BTreeSet<usize>,
    shared: FxHashMap<usize, usize>,
    held: Vec<Arc<Distribution>>,
    work: Work,
}

impl PartialGraph {
    fn new(origin: State, limits: SolveLimits) -> Result<Self, SolveError> {
        if limits.states == 0 {
            return Err(SolveError::ResourceLimit { resource: "states".into(), limit: 0 });
        }
        Ok(Self {
            builder: Builder {
                seen: FxHashMap::from_iter([(origin, 0)]),
                by_id: vec![origin],
                frontier: Vec::new(),
                list_ptr: vec![0],
                e_id: Vec::new(),
                e_prob: Vec::new(),
                loose: Vec::new(),
            },
            rows: vec![None],
            candidates: Vec::new(),
            values: vec![0.0],
            predecessor_lists: vec![Vec::new()],
            list_users: Vec::new(),
            list_values: Vec::new(),
            pending: BTreeSet::new(),
            shared: FxHashMap::default(),
            held: Vec::new(),
            work: Work::default(),
        })
    }

    fn list(&mut self, result: TransitionResult, buffer: &TransitionBuffer, limits: SolveLimits) -> Result<Option<usize>, SolveError> {
        let previous = self.builder.list_ptr.len() - 1;
        let list = match result {
            TransitionResult::Empty => return Ok(None),
            TransitionResult::Buffered => self.builder.push_list(buffer.outcomes(), Some(buffer.bounds()), limits)?,
            TransitionResult::Shared(out) => {
                let address = Arc::as_ptr(&out) as usize;
                if let Some(&list) = self.shared.get(&address) {
                    return Ok(Some(list));
                }
                let list = self.builder.push_list(&out, out.bounds(), limits)?;
                self.shared.insert(address, list);
                self.held.push(out);
                list
            }
        };
        let n = self.builder.by_id.len();
        self.rows.resize_with(n, || None);
        self.values.resize(n, 0.0);
        self.predecessor_lists.resize_with(n, Vec::new);
        if list == previous {
            self.list_users.push(Vec::new());
            self.list_values.push(None);
            for e in self.builder.list_ptr[list]..self.builder.list_ptr[list + 1] {
                self.predecessor_lists[self.builder.e_id[e] as usize].push(list);
            }
        }
        self.builder.frontier.clear();
        Ok(Some(list))
    }

    fn q(&mut self, candidate: usize) -> f64 {
        self.work.q_values += 1;
        let candidate = &self.candidates[candidate];
        let Some(list) = candidate.list else { return f64::INFINITY };
        let expected = *self.list_values[list].get_or_insert_with(|| {
            (self.builder.list_ptr[list]..self.builder.list_ptr[list + 1])
                .filter(|&e| self.builder.e_prob[e] > 0.0)
                .map(|e| self.builder.e_prob[e] * self.values[self.builder.e_id[e] as usize])
                .sum::<f64>()
        });
        candidate.decision.cost + expected
    }

    fn expand(&mut self, model: &impl Model, states: &[usize], options: Options) -> Result<(), SolveError> {
        let mut buffer = TransitionBuffer::default();
        for &id in states {
            let state = self.builder.by_id[id];
            if self.rows[id].is_some() || model.is_goal(&state) {
                continue;
            }
            let dominant = model.dominant_action(&state);
            let mut row = Vec::new();
            for &decision in model.actions_for(&state) {
                if dominant.is_some_and(|action| action != decision.id) {
                    continue;
                }
                let result = model.outcomes(&state, decision.id, &mut buffer);
                let Some(list) = self.list(result, &buffer, options.limits)? else { continue };
                if self.candidates.len() >= options.limits.state_actions {
                    return Err(SolveError::ResourceLimit { resource: "state/action pairs".into(), limit: options.limits.state_actions });
                }
                let pair = self.candidates.len();
                self.candidates.push(Candidate { owner: id, decision, list: Some(list), active: options.algorithm == Algorithm::Ilao });
                self.list_users[list].push(pair);
                row.push(pair);
            }
            if options.algorithm == Algorithm::CgIlao {
                let mut best = None;
                let mut value = f64::INFINITY;
                for &pair in &row {
                    let q = self.q(pair);
                    if best.is_none() || q < value {
                        best = Some(pair);
                        value = q;
                    }
                }
                if let Some(best) = best {
                    self.candidates[best].active = true;
                }
            }
            self.rows[id] = Some(row);
            self.work.expanded_states += 1;
        }
        Ok(())
    }

    fn graph(&self, envelope: &[usize]) -> Graph {
        let mut index = vec![-1; self.rows.len()];
        let mut states = Vec::new();
        let (mut state_ptr, mut pair_action, mut pair_cost, mut pair_list) = (vec![0], Vec::new(), Vec::new(), Vec::new());
        let mut lists = Vec::new();
        let mut list_index = FxHashMap::default();
        for &id in envelope {
            let row = &self.rows[id];
            let Some(row) = row else { continue };
            index[id] = states.len() as i32;
            states.push(self.builder.by_id[id]);
            for &pair in row {
                let candidate = &self.candidates[pair];
                if let (true, Some(list)) = (candidate.active, candidate.list) {
                    pair_action.push(candidate.decision.id);
                    pair_cost.push(candidate.decision.cost);
                    let next = lists.len();
                    let mapped = *list_index.entry(list).or_insert_with(|| {
                        lists.push(list);
                        next
                    });
                    pair_list.push(mapped);
                }
            }
            state_ptr.push(pair_list.len());
        }
        let mut list_ptr = vec![0];
        let mut boundaries = Vec::new();
        let (mut e_next, mut e_prob, mut loose) = (Vec::new(), Vec::new(), Vec::new());
        for list in lists {
            let mut boundary = 0.0;
            for e in self.builder.list_ptr[list]..self.builder.list_ptr[list + 1] {
                let id = self.builder.e_id[e] as usize;
                e_next.push(index[id]);
                e_prob.push(self.builder.e_prob[e]);
                if index[id] < 0 && self.builder.e_prob[e] > 0.0 {
                    boundary += self.builder.e_prob[e] * self.values[id];
                }
            }
            list_ptr.push(e_next.len());
            loose.push(self.builder.loose[list]);
            boundaries.push(boundary);
        }
        for (cost, &list) in pair_cost.iter_mut().zip(&pair_list) {
            *cost += boundaries[list];
        }
        let mut retained_ptr = vec![0];
        let (mut retained_actions, mut retained_costs, mut retained_lists) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..states.len() {
            for pair in state_ptr[i]..state_ptr[i + 1] {
                if pair_cost[pair].is_finite() {
                    retained_actions.push(pair_action[pair]);
                    retained_costs.push(pair_cost[pair]);
                    retained_lists.push(pair_list[pair]);
                }
            }
            retained_ptr.push(retained_lists.len());
        }
        Graph {
            factorization: Default::default(),
            states,
            seen: self.builder.by_id.len(),
            state_ptr: retained_ptr,
            pair_action: retained_actions,
            pair_cost: retained_costs,
            pair_list: retained_lists,
            list_ptr,
            e_next,
            e_prob,
            loose,
        }
    }

    fn update_values(&mut self, values: StateMap<f64>, envelope: &[usize]) {
        for &id in envelope {
            if self.rows[id].is_none() {
                continue;
            }
            let previous = self.values[id];
            let value = values.get(&self.builder.by_id[id]).copied().unwrap_or(f64::INFINITY);
            self.values[id] = value;
            if value != previous {
                for &list in &self.predecessor_lists[id] {
                    self.list_values[list] = None;
                }
            }
            if value > previous {
                self.pending.extend(self.rows[id].as_ref().unwrap().iter().copied());
            } else if value < previous {
                for &list in &self.predecessor_lists[id] {
                    self.pending.extend(self.list_users[list].iter().copied());
                }
            }
        }
    }

    fn complete_unresolved_components(&mut self, values: &StateMap<f64>, envelope: &[usize]) -> bool {
        let mut added = false;
        for &id in envelope {
            let row = &self.rows[id];
            if values.contains_key(&self.builder.by_id[id]) {
                continue;
            }
            for &pair in row.iter().flatten() {
                let candidate = &mut self.candidates[pair];
                if candidate.list.is_some() && !candidate.active {
                    candidate.active = true;
                    added = true;
                }
            }
        }
        added
    }

    fn add_improving_actions(&mut self) -> bool {
        let pending = std::mem::take(&mut self.pending);
        let mut added = false;
        for pair in pending {
            if self.candidates[pair].list.is_none() {
                continue;
            }
            let q = self.q(pair);
            let owner = self.candidates[pair].owner;
            let value = self.values[owner];
            if q.is_finite() && (!value.is_finite() || q < value - 1e-9 * value.abs().max(1.0)) {
                self.candidates[pair].active = true;
                self.values[owner] = q;
                for &list in &self.predecessor_lists[owner] {
                    self.list_values[list] = None;
                    self.pending.extend(self.list_users[list].iter().copied());
                }
                added = true;
            }
        }
        added || !self.pending.is_empty()
    }

    fn envelope(&mut self, model: &impl Model, policy: &StateMap<usize>) -> (Vec<usize>, Vec<usize>) {
        let mut reached = vec![false; self.rows.len()];
        let mut frontier = vec![0];
        let mut fringes = Vec::new();
        let mut envelope = Vec::new();
        while let Some(id) = frontier.pop() {
            if std::mem::replace(&mut reached[id], true) || model.is_goal(&self.builder.by_id[id]) {
                continue;
            }
            let Some(row) = self.rows[id].clone() else {
                fringes.push(id);
                continue;
            };
            envelope.push(id);
            let current = policy.get(&self.builder.by_id[id]).copied();
            let mut best = row.iter().copied().find(|&pair| {
                let candidate = &self.candidates[pair];
                candidate.active && candidate.list.is_some() && Some(candidate.decision.id) == current
            });
            let mut value = best.map_or(f64::INFINITY, |pair| self.q(pair));
            for pair in row {
                if !self.candidates[pair].active || self.candidates[pair].list.is_none() {
                    continue;
                }
                let q = self.q(pair);
                if best.is_none() || (!value.is_finite() && q.is_finite()) || q < value - 1e-9 * value.abs().max(1.0) {
                    best = Some(pair);
                    value = q;
                }
            }
            let Some(pair) = best else { continue };
            let candidate = &self.candidates[pair];
            let list = candidate.list.unwrap();
            frontier.extend((self.builder.list_ptr[list]..self.builder.list_ptr[list + 1]).map(|e| self.builder.e_id[e] as usize));
        }
        envelope.sort_unstable();
        (envelope, fringes)
    }

    fn refine(&mut self, model: &impl Model, actions: &[usize], options: Options) -> Result<(), SolveError> {
        let mut buffer = TransitionBuffer::default();
        for pair in 0..self.candidates.len() {
            if !actions.contains(&self.candidates[pair].decision.id) {
                continue;
            }
            let candidate = &self.candidates[pair];
            let result = model.outcomes(&self.builder.by_id[candidate.owner], candidate.decision.id, &mut buffer);
            let list = self.list(result, &buffer, options.limits)?;
            self.candidates[pair].list = list;
            if let Some(list) = list {
                self.list_users[list].push(pair);
            }
        }
        self.values.fill(0.0);
        self.list_values.fill(None);
        self.pending.extend(0..self.candidates.len());
        Ok(())
    }
}

fn focused(model: &mut impl Model, options: Options) -> Result<Solution, SolveError> {
    if model.is_goal(&model.origin()) {
        return evaluate_solution(model, &StateMap::default(), options);
    }
    let mut partial = PartialGraph::new(model.origin(), options.limits)?;
    partial.expand(model, &[0], options)?;
    let mut warm = StateMap::default();
    loop {
        let (_, fringes) = partial.envelope(model, &warm);
        if !fringes.is_empty() {
            partial.expand(model, &fringes, options)?;
            continue;
        }
        let envelope: Vec<_> = partial.rows.iter().enumerate().filter_map(|(id, row)| row.is_some().then_some(id)).collect();
        let graph = partial.graph(&envelope).without_dead_ends();
        let mut policy = StateMap::default();
        let mut values = StateMap::default();
        if let Some(graph) = &graph {
            let proper = graph.proper_policy().into_iter().collect::<Option<Vec<_>>>().ok_or(SolveError::Unreachable)?;
            let seeded = (0..graph.n())
                .map(|i| {
                    warm.get(&graph.states[i])
                        .and_then(|&action| (graph.state_ptr[i]..graph.state_ptr[i + 1]).find(|&p| graph.pair_action[p] == action))
                        .unwrap_or(proper[i])
                })
                .collect();
            partial.work.policy_evaluations += 1;
            let (value, chosen, _) = iterate(graph, seeded, options.max_rounds, options.backend)
                .or_else(|_| iterate(graph, proper, options.max_rounds, options.backend))?;
            policy = graph.states.iter().zip(&chosen).map(|(s, &p)| (*s, graph.pair_action[p])).collect();
            values = graph.states.iter().copied().zip(value).collect();
        }
        if partial.complete_unresolved_components(&values, &envelope) {
            continue;
        }
        partial.update_values(values, &envelope);
        warm.extend(policy);
        if partial.add_improving_actions() {
            continue;
        }
        if !partial.values[0].is_finite() {
            return Err(SolveError::Unreachable);
        }
        let (_, fringes) = partial.envelope(model, &warm);
        if !fringes.is_empty() {
            continue;
        }
        let mut solution = evaluate_solution(model, &warm, options)?;
        if (solution.cost - partial.values[0]).abs() > 1e-7 * solution.cost.abs().max(1.0) {
            return Err(SolveError::Singular("closed policy and partial graph disagree on cost".into()));
        }
        let graph = graph.as_ref().unwrap();
        let loose: StateMap<usize> = graph
            .states
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let action = warm[s];
                (graph.state_ptr[i]..graph.state_ptr[i + 1])
                    .find(|&p| graph.pair_action[p] == action && graph.loose[graph.pair_list[p]])
                    .map(|_| (*s, action))
            })
            .collect();
        let wanted: Vec<_> = solution
            .states
            .iter()
            .zip(&solution.visits)
            .filter(|(_, visits)| **visits > 1e-9)
            .filter_map(|(s, _)| loose.get(s).map(|&action| (*s, action)))
            .collect();
        if model.refine(&wanted) {
            let mut actions: Vec<_> = wanted.iter().map(|&(_, action)| action).collect();
            actions.sort_unstable();
            actions.dedup();
            partial.refine(model, &actions, options)?;
            continue;
        }
        partial.work.state_actions = partial.candidates.len();
        partial.work.transitions = partial.builder.e_id.len();
        solution.work = partial.work;
        solution.explored = partial.builder.by_id.len();
        return Ok(solution);
    }
}

pub fn evaluate(model: &impl Model, policy: &StateMap<usize>, options: Options) -> Result<f64, SolveError> {
    evaluate_solution(model, policy, options).map(|solution| solution.cost)
}

pub fn evaluate_solution(model: &impl Model, policy: &StateMap<usize>, options: Options) -> Result<Solution, SolveError> {
    if model.is_goal(&model.origin()) {
        return Ok(Solution {
            work: Work::default(),
            risk: Some(PolicyRisk::default()),
            cost: 0.0,
            states: Vec::new(),
            policy: Vec::new(),
            values: Vec::new(),
            visits: Vec::new(),
            explored: 1,
            approximate: false,
        });
    }
    let g = explore(model, options.limits, Some(policy))?;
    let chosen: Vec<usize> = (0..g.n()).map(|i| g.state_ptr[i]).collect();
    if !g.is_proper(&chosen) {
        return Err(SolveError::Singular("policy has a closed component without a goal".into()));
    }
    let lu = g.factor(&chosen, options.backend).ok_or_else(|| SolveError::Singular("singular matrix".into()))?;
    let start = g.states.iter().position(|s| *s == model.origin()).expect("origin is open");
    let values = g.evaluate(&lu, &chosen);
    let visits = g.visits(&lu, start);
    if !values.iter().chain(&visits).all(|value| value.is_finite()) {
        return Err(SolveError::Singular("policy values or visit counts are not finite".into()));
    }
    Ok(Solution {
        work: Work { expanded_states: g.n(), state_actions: g.pair_list.len(), transitions: g.e_next.len(), ..Work::default() },
        risk: g.assess(model, &chosen, &lu, &visits, start),
        cost: values[start],
        approximate: g.approximate(&chosen, &visits),
        policy: chosen.iter().map(|&p| g.pair_action[p]).collect(),
        states: g.states,
        values,
        visits,
        explored: g.seen,
    })
}

/// Policy iteration on `g` from `policy`: the values, the policy and its factored system.
fn iterate(g: &Graph, mut policy: Vec<usize>, max_rounds: usize, backend: Backend) -> Result<(Vec<f64>, Vec<usize>, Lu), SolveError> {
    for _ in 0..max_rounds {
        if !g.is_proper(&policy) {
            return Err(SolveError::Singular("policy has a closed component without a goal".into()));
        }
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

/// The explored states as flat arrays.
///
/// Open (unfinished) states are numbered 0..n-1. Every (state, action) pair is
/// a row; pairs of one state are contiguous, in action order. A pair's
/// outcomes are a *list* of entries (next state, probability), with next = -1
/// for a finished item, whose remaining cost is 0. Rerolls give every state
/// the same list, so lists are stored once and pairs point at them: a
/// reroll's expected cost to finish is computed once, not once per state.
///
/// A list is `loose` when its chances are a lumped estimate rather than exact for every real
/// item the state stands for; the model may refine such lists.
pub(super) struct Graph {
    pub(super) factorization: std::cell::RefCell<linear::FactorizationCache>,
    pub states: Vec<State>, // open states, in the order they were first acted on
    pub seen: usize,        // every state found, finished ones too
    pub(super) state_ptr: Vec<usize>,
    pub(super) pair_action: Vec<usize>,
    pub(super) pair_cost: Vec<f64>,
    pub(super) pair_list: Vec<usize>,
    pub(super) list_ptr: Vec<usize>, // list k's entries: list_ptr[k]..list_ptr[k+1]
    pub(super) e_next: Vec<i32>,
    pub(super) e_prob: Vec<f64>,
    /// lists whose chances are lumped estimates
    pub(super) loose: Vec<bool>,
}

/// Grows the entry arrays while states are discovered.
struct Builder {
    pub(super) seen: FxHashMap<State, u32>,
    pub(super) by_id: Vec<State>,
    pub(super) frontier: Vec<u32>,
    pub(super) list_ptr: Vec<usize>,
    pub(super) e_id: Vec<u32>,
    pub(super) e_prob: Vec<f64>,
    pub(super) loose: Vec<bool>,
}

impl Builder {
    pub(super) fn push_list(&mut self, out: &[Outcome], bounds: Option<&[Bound]>, limits: SolveLimits) -> Result<usize, SolveError> {
        if self.e_id.len().saturating_add(out.len()) > limits.transitions {
            return Err(SolveError::ResourceLimit { resource: "transitions".into(), limit: limits.transitions });
        }
        let loose = bounds.is_some_and(|b| b.iter().zip(out).any(|(&(lo, hi), &(p, _))| lo != p || hi != p));
        for &(p, n) in out {
            let next_id = self.by_id.len() as u32;
            if self.by_id.len() >= limits.states && !self.seen.contains_key(&n) {
                return Err(SolveError::ResourceLimit { resource: "states".into(), limit: limits.states });
            }
            let id = *self.seen.entry(n).or_insert(next_id);
            if id == next_id {
                self.by_id.push(n);
                self.frontier.push(id);
            }
            self.e_id.push(id);
            self.e_prob.push(p);
        }
        self.list_ptr.push(self.e_id.len());
        self.loose.push(loose);
        Ok(self.list_ptr.len() - 2)
    }
}

/// Every state reachable from the origin, with each action's outcomes; with `only`, just the
/// action it names in each state (an error if it names none there).
pub(super) fn explore(model: &impl Model, limits: SolveLimits, only: Option<&StateMap<usize>>) -> Result<Graph, SolveError> {
    if limits.states == 0 {
        return Err(SolveError::ResourceLimit { resource: "states".into(), limit: 0 });
    }
    let origin = model.origin();
    let mut b = Builder {
        seen: FxHashMap::default(),
        by_id: vec![origin],
        frontier: vec![0],
        list_ptr: vec![0],
        e_id: Vec::new(),
        e_prob: Vec::new(),
        loose: Vec::new(),
    };
    b.seen.insert(origin, 0);
    let mut open: Vec<u32> = Vec::new();
    let (mut state_ptr, mut pair_action, mut pair_cost, mut pair_list) = (vec![0], Vec::new(), Vec::new(), Vec::new());
    // Rerolls share one outcome list across many states: scan each once.
    let mut shared: FxHashMap<usize, usize> = FxHashMap::default();
    let mut held = Vec::new();
    let mut buffer = TransitionBuffer::default();
    while let Some(id) = b.frontier.pop() {
        let s = b.by_id[id as usize];
        if model.is_goal(&s) {
            continue;
        }
        open.push(id);
        let chosen = match only {
            Some(policy) => Some(*policy.get(&s).ok_or_else(|| SolveError::MissingPolicy(format!("policy has no action for {s:?}")))?),
            None => model.dominant_action(&s),
        };
        for &Decision { id: ai, cost } in model.actions_for(&s) {
            if chosen.is_some_and(|c| c != ai) {
                continue;
            }
            let list = match model.outcomes(&s, ai, &mut buffer) {
                TransitionResult::Empty => continue,
                TransitionResult::Buffered => b.push_list(buffer.outcomes(), Some(buffer.bounds()), limits)?,
                TransitionResult::Shared(arc) => match shared.get(&(std::sync::Arc::as_ptr(&arc) as usize)) {
                    Some(&l) => l,
                    None => {
                        let l = b.push_list(&arc, arc.bounds(), limits)?;
                        shared.insert(std::sync::Arc::as_ptr(&arc) as usize, l);
                        held.push(arc); // keeps the address from being reused
                        l
                    }
                },
            };
            if pair_action.len() >= limits.state_actions {
                return Err(SolveError::ResourceLimit { resource: "state/action pairs".into(), limit: limits.state_actions });
            }
            pair_action.push(ai);
            pair_cost.push(cost);
            pair_list.push(list);
        }
        if let Some(c) = chosen
            && state_ptr.last() == Some(&pair_list.len())
        {
            return Err(SolveError::MissingPolicy(format!("action {c} is not available in {s:?}")));
        }
        state_ptr.push(pair_list.len());
    }
    let mut index = vec![-1i32; b.by_id.len()];
    for (k, &id) in open.iter().enumerate() {
        index[id as usize] = k as i32;
    }
    Ok(Graph {
        factorization: Default::default(),
        states: open.iter().map(|&id| b.by_id[id as usize]).collect(),
        seen: b.by_id.len(),
        state_ptr,
        pair_action,
        pair_cost,
        pair_list,
        list_ptr: b.list_ptr,
        e_next: b.e_id.iter().map(|&id| index[id as usize]).collect(),
        e_prob: b.e_prob,
        loose: b.loose,
    })
}

impl Graph {
    pub(super) fn n(&self) -> usize {
        self.states.len()
    }

    fn is_proper(&self, policy: &[usize]) -> bool {
        let mut users = vec![Vec::new(); self.lists()];
        for (state, &pair) in policy.iter().enumerate() {
            users[self.pair_list[pair]].push(state);
        }
        let mut predecessors = vec![Vec::new(); self.n()];
        let mut reaches = vec![false; self.lists()];
        let mut frontier = Vec::new();
        let mut reached = vec![false; self.n()];
        for (list, users) in users.iter().enumerate().filter(|(_, users)| !users.is_empty()) {
            for e in self.list_ptr[list]..self.list_ptr[list + 1] {
                if self.e_prob[e] <= 0.0 {
                    continue;
                }
                if self.e_next[e] < 0 {
                    reaches[list] = true;
                } else {
                    predecessors[self.e_next[e] as usize].push(list);
                }
            }
            if reaches[list] {
                for &state in users {
                    reached[state] = true;
                    frontier.push(state);
                }
            }
        }
        while let Some(state) = frontier.pop() {
            for &list in &predecessors[state] {
                if std::mem::replace(&mut reaches[list], true) {
                    continue;
                }
                for &owner in &users[list] {
                    if !std::mem::replace(&mut reached[owner], true) {
                        frontier.push(owner);
                    }
                }
            }
        }
        reached.into_iter().all(|reached| reached)
    }

    /// The graph again after the model refined `actions`: only their outcome lists are redone.
    /// None if that reaches a state the graph doesn't have (or loses one), so it must be explored afresh.
    pub(super) fn refresh(mut self, model: &impl Model, actions: &[usize], limits: SolveLimits) -> Result<Option<Graph>, SolveError> {
        let index: FxHashMap<State, i32> = self.states.iter().enumerate().map(|(i, s)| (*s, i as i32)).collect();
        let mut shared: FxHashMap<usize, usize> = FxHashMap::default();
        let mut held = Vec::new();
        let mut buffer = TransitionBuffer::default();
        for i in 0..self.n() {
            for p in self.state_ptr[i]..self.state_ptr[i + 1] {
                if !actions.contains(&self.pair_action[p]) {
                    continue;
                }
                let list = match model.outcomes(&self.states[i], self.pair_action[p], &mut buffer) {
                    TransitionResult::Empty => return Ok(None),
                    TransitionResult::Buffered => match self.push_list(model, &index, buffer.outcomes(), Some(buffer.bounds()), limits)? {
                        Some(l) => l,
                        None => return Ok(None),
                    },
                    TransitionResult::Shared(arc) => match shared.get(&(std::sync::Arc::as_ptr(&arc) as usize)) {
                        Some(&l) => l,
                        None => {
                            let Some(l) = self.push_list(model, &index, &arc, arc.bounds(), limits)? else { return Ok(None) };
                            shared.insert(std::sync::Arc::as_ptr(&arc) as usize, l);
                            held.push(arc);
                            l
                        }
                    },
                };
                self.pair_list[p] = list;
            }
        }
        Ok(Some(self))
    }

    /// Appends an outcome list over known states; None if it reaches one the graph lacks.
    fn push_list(
        &mut self,
        model: &impl Model,
        index: &FxHashMap<State, i32>,
        out: &[Outcome],
        bounds: Option<&[Bound]>,
        limits: SolveLimits,
    ) -> Result<Option<usize>, SolveError> {
        if self.e_next.len().saturating_add(out.len()) > limits.transitions {
            return Err(SolveError::ResourceLimit { resource: "transitions".into(), limit: limits.transitions });
        }
        let start = self.e_next.len();
        for &(p, next) in out {
            match index.get(&next) {
                Some(&i) => self.e_next.push(i),
                None if model.is_goal(&next) => self.e_next.push(-1),
                None => {
                    self.e_next.truncate(start);
                    self.e_prob.truncate(start);
                    return Ok(None);
                }
            }
            self.e_prob.push(p);
        }
        self.list_ptr.push(self.e_next.len());
        self.loose.push(bounds.is_some_and(|b| b.iter().zip(out).any(|(&(lo, hi), &(p, _))| lo != p || hi != p)));
        Ok(Some(self.list_ptr.len() - 2))
    }

    pub(super) fn lists(&self) -> usize {
        self.list_ptr.len() - 1
    }

    /// Whether the policy acts with lumped chances in some state it reaches.
    pub(super) fn approximate(&self, policy: &[usize], visits: &[f64]) -> bool {
        policy.iter().zip(visits).any(|(&p, &n)| n > 1e-12 && self.loose[self.pair_list[p]])
    }

    /// Expected cost to finish after taking each pair's action.
    pub(super) fn q(&self, value: &[f64]) -> Vec<f64> {
        let prob = &self.e_prob;
        let expected: Vec<f64> = (0..self.lists())
            .map(|l| {
                (self.list_ptr[l]..self.list_ptr[l + 1])
                    .map(|e| match self.e_next[e] {
                        n if n >= 0 => prob[e] * value[n as usize],
                        _ => 0.0,
                    })
                    .sum()
            })
            .collect();
        self.pair_cost.iter().zip(&self.pair_list).map(|(c, &l)| c + expected[l]).collect()
    }

    /// Each state's cheapest pair (first in action order on ties), kept unless clearly better.
    pub(super) fn improve(&self, q: &[f64], policy: &[usize]) -> Vec<usize> {
        (0..self.n())
            .map(|i| {
                let range = self.state_ptr[i]..self.state_ptr[i + 1];
                let (mut best, mut candidate) = (f64::INFINITY, policy[i]);
                for p in range {
                    if q[p] < best {
                        best = q[p];
                        candidate = p;
                    }
                }
                let current = q[policy[i]];
                if best < current - 1e-9 * current.abs().max(1.0) { candidate } else { policy[i] }
            })
            .collect()
    }

    /// A starting policy that surely finishes: always step toward fewer moves-to-done.
    ///
    /// Policy iteration needs one; "always reroll" isn't enough when no single
    /// reroll can hit every target (e.g. three notables need alt, regal, exalt).
    /// Returns None for states that can't finish.
    pub(super) fn proper_policy(&self) -> Vec<Option<usize>> {
        let n = self.n();
        let mut dist = vec![f64::INFINITY; n];
        let mut policy = vec![None; n];
        let mut list_min = vec![0.0; self.lists()];
        loop {
            for (l, m) in list_min.iter_mut().enumerate() {
                *m = (self.list_ptr[l]..self.list_ptr[l + 1])
                    .map(|e| {
                        if self.e_prob[e] <= 0.0 {
                            f64::INFINITY
                        } else if self.e_next[e] >= 0 {
                            dist[self.e_next[e] as usize]
                        } else {
                            0.0
                        }
                    })
                    .fold(f64::INFINITY, f64::min);
            }
            let mut improved = false;
            for i in 0..n {
                let (mut best, mut first) = (f64::INFINITY, None);
                for p in self.state_ptr[i]..self.state_ptr[i + 1] {
                    let step = list_min[self.pair_list[p]] + 1.0;
                    if step < best {
                        best = step;
                        first = Some(p);
                    }
                }
                if best < dist[i] {
                    dist[i] = best;
                    policy[i] = first;
                    improved = true;
                }
            }
            if !improved {
                return policy;
            }
        }
    }

    /// The states from which some policy finishes for sure, and only the actions that keep there.
    ///
    /// An action that may land where nothing can finish (an alchemy when only alterations are
    /// left) is a dead end to avoid, not a reason the target is out of reach (Kolobov, Mausam
    /// and Weld, "A theory of goal-oriented MDPs with dead ends", UAI 2012). Any outcome that
    /// may happen counts, even one the point estimate gives no chance. None if nothing is left.
    pub(super) fn without_dead_ends(self) -> Option<Graph> {
        let n = self.n();
        let pairs = self.pair_list.len();
        let mut valid = vec![true; pairs];
        let mut owner = vec![0usize; pairs];
        for i in 0..n {
            owner[self.state_ptr[i]..self.state_ptr[i + 1]].fill(i);
        }
        let mut alive;
        loop {
            // States that finish with some chance using the valid actions.
            alive = vec![false; n];
            loop {
                let reaches: Vec<bool> = (0..self.lists())
                    .map(|l| {
                        (self.list_ptr[l]..self.list_ptr[l + 1])
                            .any(|e| self.e_prob[e] > 0.0 && (self.e_next[e] < 0 || alive[self.e_next[e] as usize]))
                    })
                    .collect();
                let mut grew = false;
                for p in 0..pairs {
                    let i = owner[p];
                    if alive[i] || !valid[p] {
                        continue;
                    }
                    if reaches[self.pair_list[p]] {
                        alive[i] = true;
                        grew = true;
                    }
                }
                if !grew {
                    break;
                }
            }
            let dead: Vec<bool> = (0..self.lists())
                .map(|l| (self.list_ptr[l]..self.list_ptr[l + 1]).any(|e| self.e_next[e] >= 0 && !alive[self.e_next[e] as usize]))
                .collect();
            let mut cut = false;
            for (ok, &l) in valid.iter_mut().zip(&self.pair_list) {
                if *ok && dead[l] {
                    *ok = false;
                    cut = true;
                }
            }
            if !cut {
                break;
            }
        }
        if alive.iter().all(|&a| a) {
            return Some(self);
        }
        let mut index = vec![-1i32; n];
        let mut states = Vec::new();
        for i in 0..n {
            if alive[i] {
                index[i] = states.len() as i32;
                states.push(self.states[i]);
            }
        }
        if states.is_empty() {
            return None;
        }
        let (mut state_ptr, mut pair_action, mut pair_cost, mut pair_list) = (vec![0], Vec::new(), Vec::new(), Vec::new());
        for i in (0..n).filter(|&i| alive[i]) {
            for p in (self.state_ptr[i]..self.state_ptr[i + 1]).filter(|&p| valid[p]) {
                pair_action.push(self.pair_action[p]);
                pair_cost.push(self.pair_cost[p]);
                pair_list.push(self.pair_list[p]);
            }
            state_ptr.push(pair_list.len());
        }
        // Lists only cut actions used may still point at dead states; nothing reads them.
        let e_next = self.e_next.iter().map(|&x| if x >= 0 { index[x as usize].max(-1) } else { x }).collect();
        Some(Graph {
            factorization: Default::default(),
            states,
            seen: self.seen,
            state_ptr,
            pair_action,
            pair_cost,
            pair_list,
            list_ptr: self.list_ptr,
            e_next,
            e_prob: self.e_prob,
            loose: self.loose,
        })
    }

    /// I - P for the policy, factored. None if the matrix is singular.
    pub(super) fn factor(&self, policy: &[usize], backend: Backend) -> Option<Lu> {
        let rows = policy.iter().map(|&p| self.pair_list[p]).collect();
        linear::factor(
            &System { rows, list_ptr: &self.list_ptr, e_next: &self.e_next, e_prob: &self.e_prob },
            backend,
            &mut self.factorization.borrow_mut(),
        )
    }

    pub(super) fn evaluate(&self, lu: &Lu, policy: &[usize]) -> Vec<f64> {
        let mut rhs = Mat::<f64>::from_fn(self.n(), 1, |i, _| self.pair_cost[policy[i]]);
        lu.solve(&mut rhs);
        (0..self.n()).map(|i| rhs[(i, 0)]).collect()
    }

    fn assess(&self, model: &impl Model, policy: &[usize], lu: &Lu, visits: &[f64], start: usize) -> Option<PolicyRisk> {
        let measurements: Vec<_> = self
            .states
            .iter()
            .enumerate()
            .map(|(i, state)| if visits[i] > 0.0 { model.measurements(state, self.pair_action[policy[i]]) } else { Some([0.0; 4]) })
            .collect::<Option<_>>()?;
        let mut means = Mat::from_fn(self.n(), 4, |i, metric| measurements[i][metric]);
        lu.solve(&mut means);
        let mut variances = Mat::from_fn(self.n(), 4, |i, metric| {
            let list = self.pair_list[policy[i]];
            (self.list_ptr[list]..self.list_ptr[list + 1])
                .map(|e| {
                    let next = if self.e_next[e] >= 0 { means[(self.e_next[e] as usize, metric)] } else { 0.0 };
                    self.e_prob[e] * (measurements[i][metric] + next - means[(i, metric)]).powi(2)
                })
                .sum::<f64>()
        });
        lu.solve(&mut variances);
        if (0..4).any(|metric| {
            !means[(start, metric)].is_finite() || !variances[(start, metric)].is_finite() || variances[(start, metric)] < -1e-9
        }) {
            return None;
        }
        let metric = |column| Moments { mean: means[(start, column)], variance: variances[(start, column)].max(0.0) };
        Some(PolicyRisk {
            objective: metric(0),
            currency: metric(1),
            operations: metric(2),
            hours: metric(3),
            approximate: self.approximate(policy, visits),
        })
    }

    /// Expected times each state is acted on, starting from state `start`.
    pub(super) fn visits(&self, lu: &Lu, start: usize) -> Vec<f64> {
        let mut rhs = Mat::<f64>::from_fn(self.n(), 1, |i, _| if i == start { 1.0 } else { 0.0 });
        lu.solve_transpose(&mut rhs);
        (0..self.n()).map(|i| rhs[(i, 0)]).collect()
    }
}
