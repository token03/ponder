use std::collections::HashMap;
use std::sync::Arc;

use crate::action::{ActionKind, step_cost};
use crate::linear::Backend;
use crate::problem::{PreparedAction, Problem};
use crate::search::{self, Decision, Model, Options};
pub use crate::search::{SolveError, SolveLimits, StateMap};
use crate::transition::{Distribution, TransitionBuffer, TransitionResult};
use crate::types::*;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    pub search_work: search::Work,
    pub risk: Option<PolicyRisk>,
    pub initial: Vec<Outcome>,
    pub recovery: Vec<Outcome>,
    pub restarts: f64,
    pub sales: f64,
    pub sale_proceeds: f64,
    pub sell_states: StateMap<f64>,
    pub restart_values: StateMap<f64>,
    /// Expected chaos from the problem's origin (a fresh base unless `begin`), excluding the base itself, clicking time included.
    pub cost: f64,
    pub policy: StateMap<Arc<str>>,
    pub usage: HashMap<String, f64>, // expected uses of each action
    pub visits: StateMap<f64>,       // expected times each state is acted on
    pub states: usize,
    pub unpriced: Vec<String>, // actions left out for lack of a price
    pub hours: f64,            // expected time spent clicking
    pub values: StateMap<f64>, // expected chaos to finish from every state the plan reaches
    /// The open states in the order the solver numbered them (Python's dict order): ties in `steps` follow it.
    pub order: Vec<State>,
    /// The plan relies on lumped (estimated) chances somewhere it goes.
    pub approximate: bool,
}

#[derive(Clone, Debug)]
pub struct Recovery {
    pub initial: Arc<Distribution>,
    pub cost: f64,
    pub hours: f64,
    pub usage: HashMap<String, f64>,
    pub sale: Option<SaleRecovery>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SaleRecovery {
    pub proceeds: f64,
    pub cost: f64,
    pub hours: f64,
    pub usage: std::collections::BTreeMap<String, f64>,
}

#[derive(Clone, Copy, Debug)]
pub struct Preservation {
    pub targets: u8,
    pub filler: [u8; 2],
}

impl Preservation {
    pub fn side(problem: &Problem, side: Side) -> Self {
        let targets = problem.targets.iter().enumerate().filter(|(_, t)| t.side == side).fold(0, |mask, (i, _)| mask | 1 << i);
        let mut filler = [0; 2];
        filler[side.index()] = problem.filler_sides.map_or(0, |(p, s)| if side == Side::Prefix { p } else { s }) as u8;
        Self { targets, filler }
    }

    fn locked_side(self, problem: &Problem) -> Option<Side> {
        [Side::Prefix, Side::Suffix].into_iter().find(|&side| {
            self.filler[side.other().index()] == 0
                && problem.targets.iter().enumerate().all(|(i, t)| self.targets & (1 << i) == 0 || t.side == side)
        })
    }

    pub fn kept(self, state: &State) -> bool {
        state.kind == StateKind::Item
            && (0..MAX_TARGETS).all(|i| self.targets & (1 << i) == 0 || state.status[i] == GOOD)
            && (0..2).all(|side| state.junk[4 + side] >= self.filler[side])
    }
}

impl Plan {
    /// Expected chaos to finish an item in state `s` under this plan; None if the plan never meets it.
    pub fn to_finish(&self, problem: &Problem, s: &State) -> Option<f64> {
        if problem.done(s) { Some(0.0) } else { self.values.get(s).or_else(|| self.restart_values.get(s)).copied() }
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

fn producible_goal(problem: &Problem, active: &[usize], initial: &[Outcome], recovery: Option<&Recovery>) -> bool {
    let mut available: std::collections::HashSet<&str> = problem.pool.iter().filter(|m| m.weight > 0.0).map(|m| m.id.as_str()).collect();
    available.extend(problem.profiles.values().flat_map(|p| p.iter()).filter(|(_, w)| **w > 0.0).map(|(id, _)| id.as_str()));
    available.extend(active.iter().filter_map(|&i| problem.actions[i].forced.as_deref()));
    if active.iter().any(|&i| matches!(problem.actions[i].kind.effect(), crate::Effect::Veil(_))) {
        available.extend(problem.unveils.iter().filter(|m| m.weight > 0.0).map(|m| m.id.as_str()));
    }
    initial.iter().filter(|(p, _)| *p > 0.0).all(|(_, state)| {
        problem
            .targets
            .iter()
            .enumerate()
            .filter(|(i, target)| {
                state.status[*i] == GOOD
                    || target.good.iter().any(|id| available.contains(id.as_str()))
                    || recovery.is_some_and(|r| r.initial.iter().any(|(p, s)| *p > 0.0 && s.status[*i] == GOOD))
            })
            .count()
            >= problem.need
    })
}

pub fn solve(problem: &Problem, prices: &Prices, settings: &Settings) -> Result<Plan, SolveError> {
    solve_with(problem, prices, settings, Options::default())
}

pub fn solve_with(problem: &Problem, prices: &Prices, settings: &Settings, options: Options) -> Result<Plan, SolveError> {
    let unpriced =
        problem.actions.iter().filter(|a| a.cost.iter().any(|(pid, _)| !prices.contains_key(pid))).map(|a| a.name.clone()).collect();
    if problem.done(&problem.origin()) {
        return Ok(Plan { states: 1, unpriced, ..Plan::default() });
    }
    let active = cheapest_of_equals(problem, prices, settings);
    if !producible_goal(problem, &active, &[(1.0, problem.origin())], None) {
        return Err(SolveError::Unreachable);
    }
    let mut model = CraftModel::new(problem, prices, settings, active);
    let solution = search::solve(&mut model, options)?;
    Ok(make_plan(problem, solution, unpriced, Vec::new(), None))
}

pub fn solve_from(problem: &Problem, initial: Arc<Distribution>, prices: &Prices, settings: &Settings) -> Result<Plan, SolveError> {
    solve_from_stage(problem, initial, None, None, prices, settings)
}

pub fn solve_preserving_side(
    problem: &Problem,
    initial: Arc<Distribution>,
    side: Side,
    prices: &Prices,
    settings: &Settings,
) -> Result<Plan, SolveError> {
    if initial.iter().any(|(_, s)| {
        s.rarity != RARE
            || problem.count(s, side) != problem.side_limit(RARE, side)
            || problem.targets.iter().enumerate().any(|(i, t)| t.side == side && s.status[i] != GOOD)
    }) {
        return Err(SolveError::Unreachable);
    }
    solve_from_stage(problem, initial, Some(Preservation::side(problem, side)), None, prices, settings)
}

pub fn solve_with_recovery(
    problem: &Problem,
    initial: Arc<Distribution>,
    side: Side,
    recovery: Recovery,
    prices: &Prices,
    settings: &Settings,
) -> Result<Plan, SolveError> {
    solve_after_acquisition(problem, initial, Preservation::side(problem, side), recovery, prices, settings)
}

pub fn solve_after_acquisition(
    problem: &Problem,
    initial: Arc<Distribution>,
    preserved: Preservation,
    recovery: Recovery,
    prices: &Prices,
    settings: &Settings,
) -> Result<Plan, SolveError> {
    solve_from_stage(problem, initial, Some(preserved), Some(recovery), prices, settings)
}

fn solve_from_stage(
    problem: &Problem,
    initial: Arc<Distribution>,
    preserved: Option<Preservation>,
    recovery: Option<Recovery>,
    prices: &Prices,
    settings: &Settings,
) -> Result<Plan, SolveError> {
    let active = cheapest_of_equals(problem, prices, settings);
    if !producible_goal(problem, &active, &initial, recovery.as_ref()) {
        return Err(SolveError::Unreachable);
    }
    let active = active
        .into_iter()
        .filter(|&i| preserved.is_none() || !matches!(problem.actions[i].kind.effect(), crate::Effect::Roll(_)))
        .collect();
    let mut model = CraftModel::with_preserved(problem, prices, settings, active, preserved);
    model.recovery = recovery;
    model.set_initial(initial)?;
    let solution = search::solve(&mut model, Options::default())?;
    let unpriced =
        problem.actions.iter().filter(|a| a.cost.iter().any(|(id, _)| !prices.contains_key(id))).map(|a| a.name.clone()).collect();
    Ok(make_plan(problem, solution, unpriced, model.initial.as_ref().expect("initial distribution").to_vec(), model.recovery.as_ref()))
}

pub fn follow_from(
    problem: &Problem,
    initial: Arc<Distribution>,
    policy: &StateMap<Arc<str>>,
    prices: &Prices,
    settings: &Settings,
) -> Result<Plan, SolveError> {
    follow_stage(
        problem,
        initial,
        FollowPolicy { actions: policy, restarts: &StateMap::default(), sales: &StateMap::default() },
        None,
        None,
        prices,
        settings,
    )
}

pub fn follow_with_recovery(
    problem: &Problem,
    initial: Arc<Distribution>,
    plan: &Plan,
    side: Side,
    recovery: Recovery,
    prices: &Prices,
    settings: &Settings,
) -> Result<Plan, SolveError> {
    follow_after_acquisition(problem, initial, plan, Preservation::side(problem, side), recovery, prices, settings)
}

pub fn follow_after_acquisition(
    problem: &Problem,
    initial: Arc<Distribution>,
    plan: &Plan,
    preserved: Preservation,
    recovery: Recovery,
    prices: &Prices,
    settings: &Settings,
) -> Result<Plan, SolveError> {
    follow_stage(
        problem,
        initial,
        FollowPolicy { actions: &plan.policy, restarts: &plan.restart_values, sales: &plan.sell_states },
        Some(preserved),
        Some(recovery),
        prices,
        settings,
    )
}

struct FollowPolicy<'a> {
    actions: &'a StateMap<Arc<str>>,
    restarts: &'a StateMap<f64>,
    sales: &'a StateMap<f64>,
}

fn follow_stage(
    problem: &Problem,
    initial: Arc<Distribution>,
    policy: FollowPolicy<'_>,
    preserved: Option<Preservation>,
    recovery: Option<Recovery>,
    prices: &Prices,
    settings: &Settings,
) -> Result<Plan, SolveError> {
    let mut indexed: StateMap<usize> = policy
        .actions
        .iter()
        .map(|(s, name)| {
            problem
                .actions
                .iter()
                .position(|a| a.name.as_str() == name.as_ref())
                .map(|a| (*s, a))
                .ok_or_else(|| SolveError::MissingPolicy(name.to_string()))
        })
        .collect::<Result<_, _>>()?;
    let mut active: Vec<usize> = indexed.values().copied().collect();
    active.sort_unstable();
    active.dedup();
    let mut model = CraftModel::with_preserved(problem, prices, settings, active, preserved);
    model.recovery = recovery;
    model.set_initial(initial)?;
    indexed.insert(model.origin(), problem.actions.len());
    if model.recovery.is_some() {
        indexed.insert(State::recovery(), problem.actions.len() + 1);
        indexed.extend(policy.restarts.keys().map(|state| (*state, problem.actions.len() + 1)));
        indexed.extend(policy.sales.keys().map(|state| (*state, problem.actions.len() + 2)));
    }
    let solution = search::evaluate_solution(&model, &indexed, Options::default())?;
    Ok(make_plan(problem, solution, Vec::new(), model.initial.as_ref().expect("initial distribution").to_vec(), model.recovery.as_ref()))
}

fn make_plan(
    problem: &Problem,
    mut solution: search::Solution,
    unpriced: Vec<String>,
    initial: Vec<Outcome>,
    recovery: Option<&Recovery>,
) -> Plan {
    let sales: f64 = solution.policy.iter().zip(&solution.visits).filter(|(a, _)| **a == problem.actions.len() + 2).map(|(_, n)| n).sum();
    let restarts: f64 = solution.policy.iter().zip(&solution.visits).filter(|(a, _)| **a > problem.actions.len()).map(|(_, n)| n).sum();
    let restart_values = solution
        .states
        .iter()
        .zip(&solution.policy)
        .zip(&solution.values)
        .filter(|((state, action), _)| state.kind == StateKind::Item && **action > problem.actions.len())
        .map(|((&state, _), &value)| (state, value))
        .collect();
    let sell_states = solution
        .states
        .iter()
        .zip(&solution.policy)
        .zip(&solution.values)
        .filter(|((_, action), _)| **action == problem.actions.len() + 2)
        .map(|((&state, _), &value)| (state, value))
        .collect();
    let retained: Vec<usize> =
        solution.policy.iter().enumerate().filter_map(|(i, &action)| (action < problem.actions.len()).then_some(i)).collect();
    solution.states = retained.iter().map(|&i| solution.states[i]).collect();
    solution.policy = retained.iter().map(|&i| solution.policy[i]).collect();
    solution.visits = retained.iter().map(|&i| solution.visits[i]).collect();
    solution.values = retained.iter().map(|&i| solution.values[i]).collect();
    let mut usage: HashMap<String, f64> = HashMap::new();
    for (i, &action) in solution.policy.iter().enumerate() {
        *usage.entry(problem.actions[action].name.clone()).or_default() += solution.visits[i];
    }
    let mut hours = problem.actions.iter().filter_map(|a| usage.get(&a.name).map(|n| n * a.seconds)).sum::<f64>() / 3600.0;
    if let Some(recovery) = recovery {
        hours += (restarts - sales) * recovery.hours;
        for (action, count) in &recovery.usage {
            *usage.entry(action.clone()).or_default() += (restarts - sales) * count;
        }
        if let Some(sale) = &recovery.sale {
            hours += sales * sale.hours;
            for (action, count) in &sale.usage {
                *usage.entry(action.clone()).or_default() += sales * count;
            }
        }
    }
    let names: Vec<Arc<str>> = problem.actions.iter().map(|a| Arc::from(a.name.as_str())).collect();
    Plan {
        search_work: solution.work,
        risk: solution.risk,
        initial,
        recovery: recovery.map(|r| r.initial.to_vec()).unwrap_or_default(),
        restarts,
        sales,
        sale_proceeds: recovery.and_then(|r| r.sale.as_ref()).map_or(0.0, |s| sales * s.proceeds),
        sell_states,
        restart_values,
        cost: solution.cost,
        policy: solution.states.iter().zip(&solution.policy).map(|(s, &a)| (*s, names[a].clone())).collect(),
        usage,
        visits: solution.states.iter().zip(&solution.visits).map(|(s, &v)| (*s, v)).collect(),
        states: solution.explored,
        unpriced,
        hours,
        values: solution.states.iter().zip(&solution.values).map(|(s, &v)| (*s, v)).collect(),
        order: solution.states,
        approximate: solution.approximate,
    }
}

pub fn evaluate(problem: &Problem, policy: &StateMap<Arc<str>>, prices: &Prices, settings: &Settings) -> Result<f64, SolveError> {
    evaluate_with(problem, policy, prices, settings, Backend::Auto)
}

pub fn evaluate_with(
    problem: &Problem,
    policy: &StateMap<Arc<str>>,
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
                .position(|a| a.name.as_str() == name.as_ref())
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
    measurements: Vec<[f64; 4]>,
    decisions: Vec<Decision>,
    classes: Vec<Vec<Decision>>,
    prepared: Vec<Option<PreparedAction>>,
    initial: Option<Arc<Distribution>>,
    preserved: Option<Preservation>,
    recovery: Option<Recovery>,
}

impl<'p> CraftModel<'p> {
    fn new(problem: &'p Problem, prices: &Prices, settings: &Settings, active: Vec<usize>) -> CraftModel<'p> {
        Self::with_preserved(problem, prices, settings, active, None)
    }

    fn with_preserved(
        problem: &'p Problem,
        prices: &Prices,
        settings: &Settings,
        active: Vec<usize>,
        preserved: Option<Preservation>,
    ) -> CraftModel<'p> {
        let decisions = active.into_iter().map(|id| Decision { id, cost: step_cost(&problem.actions[id], prices, settings) }).collect();
        let mut model = CraftModel {
            problem,
            measurements: problem
                .actions
                .iter()
                .map(|action| {
                    [step_cost(action, prices, settings), crate::action::action_cost(action, prices), 1.0, action.seconds / 3600.0]
                })
                .collect(),
            decisions,
            classes: Vec::new(),
            prepared: (0..problem.actions.len()).map(|_| None).collect(),
            initial: None,
            preserved,
            recovery: None,
        };
        model.prepare();
        model.classify();
        model
    }

    fn class(state: &State) -> Option<usize> {
        (state.kind == StateKind::Item && state.rarity <= RARE).then(|| {
            state.rarity as usize * 8
                + (state.unveil != 0) as usize
                + 2 * Problem::pending_side(state).is_some() as usize
                + 4 * (state.meta != 0) as usize
        })
    }

    fn classify(&mut self) {
        let actions = &self.problem.actions;
        self.classes = (0..64)
            .map(|class| {
                let (rarity, offers, pending, meta) = (class / 8, class & 1 != 0, class & 2 != 0, class & 4 != 0);
                self.decisions
                    .iter()
                    .filter(|d| {
                        let Some(a) = actions.get(d.id) else { return true };
                        let effect = a.kind.effect();
                        !(!effect.available_observation(pending, offers)
                            || !a.from_rarity.contains(&(rarity as u8))
                            || meta && matches!(effect, crate::Effect::Roll(_)))
                    })
                    .copied()
                    .collect()
            })
            .collect();
    }

    fn prepare(&mut self) {
        let prepared: Vec<_> = self
            .decisions
            .iter()
            .filter(|d| d.id < self.problem.actions.len())
            .map(|d| (d.id, self.problem.prepare_with_roll(&self.problem.actions[d.id], self.preserved.is_none())))
            .collect();
        for (id, action) in prepared {
            self.prepared[id] = Some(action);
        }
    }

    fn set_initial(&mut self, initial: Arc<Distribution>) -> Result<(), SolveError> {
        if initial.is_empty()
            || initial.iter().any(|(p, _)| !p.is_finite() || *p < 0.0)
            || (initial.iter().map(|(p, _)| p).sum::<f64>() - 1.0).abs() > 1e-8
        {
            return Err(SolveError::Unreachable);
        }
        self.initial = Some(initial);
        self.decisions.push(Decision { id: self.problem.actions.len(), cost: 0.0 });
        if let Some(recovery) = &self.recovery {
            self.decisions.push(Decision { id: self.problem.actions.len() + 1, cost: recovery.cost });
            if let Some(sale) = &recovery.sale {
                if !sale.cost.is_finite() || !sale.proceeds.is_finite() || sale.proceeds < 0.0 || sale.cost < 0.0 {
                    return Err(SolveError::InvalidRecovery(
                        "sale proceeds and gross replacement costs must be finite and nonnegative".into(),
                    ));
                }
                self.decisions.push(Decision { id: self.problem.actions.len() + 2, cost: sale.cost - sale.proceeds });
            }
        }
        self.classify();
        Ok(())
    }
}

impl Model for CraftModel<'_> {
    fn origin(&self) -> State {
        if self.initial.is_some() { State::acquisition() } else { self.problem.origin() }
    }
    fn is_goal(&self, state: &State) -> bool {
        state.kind == StateKind::Item && self.problem.done(state)
    }
    fn actions(&self) -> &[Decision] {
        &self.decisions
    }
    fn actions_for(&self, state: &State) -> &[Decision] {
        Self::class(state).map_or(&self.decisions, |c| &self.classes[c])
    }
    fn measurements(&self, _state: &State, action: usize) -> Option<[f64; 4]> {
        if action == self.problem.actions.len() { Some([0.0; 4]) } else { self.measurements.get(action).copied() }
    }
    fn dominant_action(&self, state: &State) -> Option<usize> {
        if state.unveil == 0 || !self.decisions.iter().all(|decision| decision.cost >= 0.0) {
            return None;
        }
        self.actions_for(state).iter().filter(|decision| decision.cost == 0.0).find_map(|decision| {
            let ActionKind::Unveil(class) = self.problem.actions.get(decision.id)?.kind else { return None };
            self.problem.choose_unveil(state, class).filter(|next| self.problem.done(next)).map(|_| decision.id)
        })
    }

    fn outcomes(&self, state: &State, action: usize, buffer: &mut TransitionBuffer) -> TransitionResult {
        if action == self.problem.actions.len() + 2 {
            return if state.rarity == RARE
                && state.meta == 0
                && self.preserved.is_some_and(|p| p.kept(state))
                && [Side::Prefix, Side::Suffix]
                    .into_iter()
                    .all(|side| self.problem.count(state, side) == self.problem.side_limit(RARE, side))
            {
                TransitionResult::Shared(self.recovery.as_ref().expect("sale recovery").initial.clone())
            } else {
                TransitionResult::Empty
            };
        }
        if state.kind == StateKind::Recovery {
            return if action == self.problem.actions.len() + 1 {
                TransitionResult::Shared(self.recovery.as_ref().expect("recovery action").initial.clone())
            } else {
                TransitionResult::Empty
            };
        }
        if action == self.problem.actions.len() + 1 {
            return if state.kind == StateKind::Item && state.meta == 0 {
                TransitionResult::Shared(self.recovery.as_ref().expect("recovery action").initial.clone())
            } else {
                TransitionResult::Empty
            };
        }
        if action == self.problem.actions.len() {
            return if state.kind == StateKind::Acquisition {
                TransitionResult::Shared(self.initial.as_ref().expect("initial distribution").clone())
            } else {
                TransitionResult::Empty
            };
        }
        if state.kind != StateKind::Item {
            return TransitionResult::Empty;
        }
        let prepared = self.prepared[action].as_ref().expect("active action is prepared");
        if self.preserved.is_some_and(|preserved| {
            matches!(prepared.kind, crate::Effect::Reforge | crate::Effect::Veil(true))
                && (preserved.locked_side(self.problem).is_none() || meta_locks(state.meta) != preserved.locked_side(self.problem))
        }) {
            return TransitionResult::Empty;
        }
        let (outcomes, bounds) = buffer.parts_mut();
        let result = self.problem.outcomes_into(state, prepared, outcomes, bounds);
        let Some(preserved) = self.preserved else { return result };
        let kept = |next: &State| preserved.kept(next);
        let keep = |out: &[Outcome], ranges: Option<&[(f64, f64)]>| {
            out.iter().enumerate().all(|(i, &(p, next))| (p == 0.0 && ranges.is_none_or(|r| r[i].1 == 0.0)) || kept(&next))
        };
        let valid = match &result {
            TransitionResult::Empty => false,
            TransitionResult::Buffered => keep(outcomes, Some(bounds)),
            TransitionResult::Shared(out) => keep(out, out.bounds()),
        };
        if valid || self.recovery.is_none() {
            return if valid { result } else { TransitionResult::Empty };
        }
        let (out, ranges) = match result {
            TransitionResult::Empty => return TransitionResult::Empty,
            TransitionResult::Buffered => (std::mem::take(outcomes), std::mem::take(bounds)),
            TransitionResult::Shared(out) => {
                (out.to_vec(), out.iter().enumerate().map(|(i, &(p, _))| out.bounds().map_or((p, p), |b| b[i])).collect())
            }
        };
        for ((p, next), range) in out.into_iter().zip(ranges) {
            outcomes.push((p, if kept(&next) { next } else { State::recovery() }));
            bounds.push(range);
        }
        TransitionResult::Buffered
    }

    fn refine(&mut self, candidates: &[(State, usize)]) -> bool {
        let mut wanted: Vec<usize> =
            candidates.iter().filter(|(s, a)| *a < self.problem.actions.len() && meta_locks(s.meta).is_none()).map(|(_, a)| *a).collect();
        wanted.sort_unstable();
        wanted.dedup();
        let changed = wanted.iter().map(|&id| self.problem.refine(&self.problem.actions[id])).collect::<Vec<_>>().contains(&true);
        if changed {
            self.prepare();
        }
        changed
    }
}
