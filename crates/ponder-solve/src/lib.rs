//! Cheapest expected cost to reach a goal: stochastic shortest paths over enumerated states
//! (SPEC §6). Knows nothing about Path of Exile.
//!
//! [`solve`] enumerates the states reachable from the start, drops actions that can end where
//! the goal is unreachable, then solves strongly connected components from the goal backwards
//! (topological value iteration). Inside a component, policy iteration evaluates each policy
//! exactly, so long retry loops converge in a few rounds instead of thousands of sweeps.
//!
//! Starting over (buying a fresh item) would join every state into one component. Instead a
//! model marks restart states, worth whatever the start is worth: `w`. For a fixed policy the
//! start's cost is affine in `w`, `a + b w` with `b` the chance of restarting, so Newton steps
//! `w = a / (1 - b)` find the fixed point in a few solves of the same acyclic structure.
//!

use rustc_hash::FxHashMap as HashMap;
use std::hash::Hash;

use rayon::prelude::*;

mod ilu;
pub mod inspect;

pub type IndexedOutcomes = Box<[(f64, usize)]>;

pub trait Model: Sync {
    type State: Clone + Eq + Hash + Send + Sync;
    fn is_goal(&self, s: &Self::State) -> bool;
    /// A state that starts over: it is worth what the start is worth.
    fn is_restart(&self, _: &Self::State) -> bool {
        false
    }
    /// Does a state that can no longer reach the goal start over (rather than make the goal
    /// unreachable)? Needs restart states to mean anything.
    fn gives_up(&self) -> bool {
        false
    }
    /// Can every state start over at no cost? Then no state is worth more than the start, and
    /// an action that costs that much is never best: the solver leaves those unexplored.
    fn restarts_anywhere(&self) -> bool {
        false
    }
    /// A lower bound on a state's cost to the goal, if the model knows one. The solver leaves
    /// such a state unexpanded, worth its bound, until the best policy reaches it.
    fn bound(&self, _: &Self::State) -> Option<f64> {
        None
    }
    /// Usable actions with their costs; ids are the model's own.
    fn actions(&self, s: &Self::State) -> Vec<(usize, f64)>;
    fn defers(&self, _: &Self::State, _action: usize) -> bool {
        false
    }
    fn outcomes(&self, s: &Self::State, action: usize) -> Vec<(f64, Self::State)>;
    fn for_each_outcome(&self, s: &Self::State, action: usize, emit: &mut dyn FnMut(f64, Self::State)) {
        for (p, t) in self.outcomes(s, action) {
            emit(p, t);
        }
    }
    fn prepare_expectations<'a>(
        &'a self,
        _states: &[Self::State],
        _value: &'a (dyn Fn(&Self::State) -> f64 + Sync),
    ) -> impl Fn(&Self::State, usize) -> Option<f64> + Sync + 'a {
        |_, _| None
    }

    fn prepare_transitions<'a>(
        &'a self,
        _states: &[Self::State],
        _index: &'a (dyn Fn(&Self::State) -> Option<usize> + Sync),
    ) -> impl Fn(&Self::State, usize) -> Option<IndexedOutcomes> + Sync + 'a {
        |_, _| None
    }

    fn discover_successors(
        &self,
        _states: &[&Self::State],
        _action: usize,
        _missing: &(dyn Fn(&Self::State) -> bool + Sync),
    ) -> Vec<Self::State> {
        Vec::new()
    }
}

#[derive(Debug)]
pub struct Solution<S> {
    /// Expected cost from the start distribution.
    pub cost: f64,
    pub states: Vec<S>,
    pub values: Vec<f64>,
    /// Chosen action per state; `None` for goals, restarts and dead ends.
    pub policy: Vec<Option<usize>>,
    pub index: HashMap<S, usize>,
    /// States that gave up (see [`Model::gives_up`]).
    pub given_up: Vec<S>,
    pub outcomes: usize,
    pub explored: usize,
    pub actions: usize,
    pub stats: Stats,
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub attempts: usize,
    pub outcomes: usize,
    /// Seconds exploring states (the model's work), pruning, and solving values.
    pub explore: f64,
    pub prune: f64,
    pub values: f64,
    /// Newton steps on the worth of starting over.
    pub newton: usize,
    pub solves: usize,
    pub checked: usize,
    pub bounded: usize,
    pub prepared: usize,
    pub streamed_outcomes: usize,
    pub violated: usize,
    /// States whose deferred actions were not checked again: no value fell far enough since.
    pub settled: usize,
    pub separation: f64,
    pub discovered: usize,
    /// Rounds of expansion, and states expanded on reaching them (see [`Model::bound`]).
    pub rounds: usize,
    pub expanded: usize,
    /// States given their deferred actions (see [`Model::defers`]).
    pub completed: usize,
    /// Per action id, how many states it was first to reach: where the state space comes from.
    pub found: std::collections::BTreeMap<usize, usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// The goal cannot be reached with certainty from the start.
    Unreachable,
    TooManyStates(usize),
}

struct Edge {
    action: usize,
    cost: f64,
    to: Transitions<usize>,
}

impl Edge {
    fn outcomes(&self) -> &[(f64, usize)] {
        self.to.outcomes()
    }

    fn constant(&self) -> f64 {
        if matches!(self.to, Transitions::Deferred) { f64::INFINITY } else { self.cost }
    }
}

enum Transitions<S> {
    Known(Box<[(f64, S)]>),
    Optimistic,
    Deferred,
}

impl<S> Transitions<S> {
    fn outcomes(&self) -> &[(f64, S)] {
        match self {
            Self::Known(to) => to,
            _ => &[],
        }
    }

    fn pending(&self) -> bool {
        !matches!(self, Self::Known(_))
    }

    fn optimistic(&self) -> bool {
        matches!(self, Self::Optimistic)
    }
}

/// Where a state's value comes from: `a + b w`, with `w` the value of restarting.
#[derive(Clone, Copy)]
struct Affine {
    a: f64,
    b: f64,
}

impl Affine {
    const INFINITE: Affine = Affine { a: f64::INFINITY, b: 0.0 };

    fn at(self, w: f64) -> f64 {
        self.a + self.b * w
    }
}

pub fn solve<M: Model>(model: &M, start: &[(f64, M::State)], limit: usize) -> Result<Solution<M::State>, Error> {
    solve_keeping(model, start, limit, false)
}

/// [`solve`], keeping every explored state with its value rather than only those the policy
/// reaches.
pub fn solve_everywhere<M: Model>(model: &M, start: &[(f64, M::State)], limit: usize) -> Result<Solution<M::State>, Error> {
    solve_keeping(model, start, limit, true)
}

fn solve_keeping<M: Model>(model: &M, start: &[(f64, M::State)], limit: usize, keep_all: bool) -> Result<Solution<M::State>, Error> {
    let mut stats = Stats::default();
    let mut graph = Graph::new();
    if !model.restarts_anywhere() {
        return solve_within(model, start, limit, f64::INFINITY, keep_all, &mut graph, &mut stats).map(|s| Solution { stats, ..s.0 });
    }
    // Explore only actions cheaper than a ceiling. A start worth less than the ceiling proves
    // every action left out dearer than starting over, so the solution is exact; otherwise the
    // ceiling rises to that worth, and the actions the solution used keep it from rising again.
    // The graph explored so far stays: a higher ceiling only adds actions.
    // A bound on the start's worth is a better first guess than its cheapest action.
    let first = start.iter().flat_map(|(_, s)| model.actions(s)).map(|a| a.1).fold(f64::INFINITY, f64::min);
    let bound: f64 = start.iter().map(|(p, s)| p * model.bound(s).unwrap_or(0.0)).sum();
    let mut ceiling = (2.0 * first.clamp(0.5, 1e9)).max(bound.clamp(0.0, 1e9) * (1.0 + 1e-9));
    loop {
        match solve_within(model, start, limit, ceiling, keep_all, &mut graph, &mut stats) {
            Ok((solution, w)) if w < ceiling => return Ok(Solution { stats, ..solution }),
            Ok((_, w)) => ceiling = w * (1.0 + 1e-9),
            Err(Error::Unreachable) if ceiling.is_finite() => ceiling = if ceiling > 1e9 { f64::INFINITY } else { ceiling * 4.0 },
            Err(e) => return Err(e),
        }
    }
}

/// A state's deferred actions with their prepared expected worth, if any.
type Futures = Vec<(usize, Option<f64>)>;

/// The actions of a state with their costs and outcomes.
type Expansion<S> = Vec<(usize, f64, Transitions<S>)>;

const EXPANSION_BATCH: usize = 256;

/// How a state enters the graph.
enum Entry<S> {
    /// Fixed worth: a goal or a restart.
    Fixed(Affine),
    /// Left unexpanded, worth its bound.
    Tip(f64),
    New(Expansion<S>),
}

enum Update<S> {
    Tip(usize, Expansion<S>),
    Deferred(usize, Expansion<S>),
    Discover(usize, usize, Vec<S>),
    Linked(usize, Vec<Edge>),
}

impl<S> Update<S> {
    fn state(&self) -> usize {
        match self {
            Self::Tip(s, _) | Self::Deferred(s, _) | Self::Discover(s, _, _) | Self::Linked(s, _) => *s,
        }
    }
}

#[derive(Default)]
struct Checks {
    actions: usize,
    bounded: usize,
    prepared: usize,
    outcomes: usize,
    violated: usize,
    settled: usize,
}

fn expand_deferred<M: Model>(model: &M, state: &M::State, edges: &[Edge]) -> Expansion<M::State> {
    edges
        .iter()
        .filter(|e| e.to.pending())
        .map(|e| (e.action, e.cost, Transitions::Known(model.outcomes(state, e.action).into_boxed_slice())))
        .collect()
}

/// The states explored so far and their actions under `ceiling`.
struct Graph<S> {
    states: Vec<S>,
    index: HashMap<S, usize>,
    edges: Vec<Vec<Edge>>,
    /// Worth a fixed value rather than its actions: goals, restarts, unexpanded states, states
    /// given up.
    end: Vec<bool>,
    tip: Vec<bool>,
    fixed: Vec<Option<Affine>>,
    /// Has its deferred actions too.
    complete: Vec<bool>,
    ceiling: f64,
    /// The worth of starting over under the last ceiling: at least what it is under a higher one.
    worth: Option<f64>,
    /// Pruned under the last ceiling, back in play under a higher one.
    dropped: Vec<(usize, Edge)>,
    given_up: Vec<usize>,
    found: std::collections::BTreeMap<usize, usize>,
}

impl<S> Graph<S> {
    fn new() -> Graph<S> {
        Graph {
            states: Vec::new(),
            index: HashMap::default(),
            edges: Vec::new(),
            end: Vec::new(),
            tip: Vec::new(),
            fixed: Vec::new(),
            complete: Vec::new(),
            ceiling: 0.0,
            worth: None,
            dropped: Vec::new(),
            given_up: Vec::new(),
            found: Default::default(),
        }
    }
}

/// The graph's number for a state, numbering it if new.
fn intern<S: Clone + Eq + Hash>(s: &S, states: &mut Vec<S>, index: &mut HashMap<S, usize>, limit: usize) -> Result<usize, Error> {
    if let Some(&i) = index.get(s) {
        return Ok(i);
    }
    if states.len() >= limit {
        return Err(Error::TooManyStates(limit));
    }
    index.insert(s.clone(), states.len());
    states.push(s.clone());
    Ok(states.len() - 1)
}

/// The solution and the worth of starting over, using only actions cheaper than `ceiling`.
///
/// States the model bounds stay unexpanded, worth their bound, until the best policy reaches
/// them; then they are expanded and the graph solved again. With admissible bounds the policy
/// that reaches no unexpanded state is optimal (LAO*), and its value is exact either way.
fn solve_within<M: Model>(
    model: &M,
    start: &[(f64, M::State)],
    limit: usize,
    ceiling: f64,
    keep_all: bool,
    graph: &mut Graph<M::State>,
    stats: &mut Stats,
) -> Result<(Solution<M::State>, f64), Error> {
    stats.attempts += 1;
    let separating = model.restarts_anywhere() && !model.gives_up();
    // Actions in [`from`, `ceiling`): a state's own, or those a higher ceiling adds; deferred ones
    // only, all of them, or all but those.
    let actions_from = |s: &M::State, from: f64, deferred: Option<bool>| -> Expansion<M::State> {
        let mut actions = model.actions(s);
        actions.retain(|a| a.1 >= from && a.1 < ceiling && (deferred != Some(true) || model.defers(s, a.0)));
        actions
            .into_iter()
            .map(|(a, cost)| {
                let lazy = deferred == Some(false) && model.defers(s, a);
                let to = if !lazy {
                    Transitions::Known(model.outcomes(s, a).into_boxed_slice())
                } else if separating {
                    Transitions::Deferred
                } else {
                    Transitions::Optimistic
                };
                (a, cost, to)
            })
            .collect()
    };
    let expand = |s: &M::State| actions_from(s, f64::NEG_INFINITY, Some(false));
    let Graph { states, index, edges, end, tip, fixed, complete, ceiling: explored_under, worth, dropped, given_up, found } = graph;
    for s in given_up.drain(..) {
        (end[s], fixed[s]) = (false, None);
    }
    for (s, e) in dropped.drain(..) {
        edges[s].push(e);
    }
    let raise = (*explored_under < ceiling).then_some(*explored_under);
    *explored_under = ceiling;
    let starts: Vec<(f64, usize)> =
        start.iter().map(|(p, s)| intern(s, states, index, limit).map(|i| (*p, i))).collect::<Result<_, _>>()?;
    // The edges of an expansion, numbering the states it finds.
    let link = |out: Expansion<M::State>,
                states: &mut Vec<M::State>,
                index: &mut HashMap<M::State, usize>,
                found: &mut std::collections::BTreeMap<usize, usize>|
     -> Result<Vec<Edge>, Error> {
        let mut successors = Vec::new();
        for (action, cost, to) in out {
            let known = states.len();
            let to = match to {
                Transitions::Known(to) => Transitions::Known(
                    to.iter()
                        .filter(|o| o.0 > 0.0)
                        .map(|(p, t)| intern(t, states, index, limit).map(|i| (*p, i)))
                        .collect::<Result<_, _>>()?,
                ),
                Transitions::Optimistic => Transitions::Optimistic,
                Transitions::Deferred => Transitions::Deferred,
            };
            if states.len() > known {
                *found.entry(action).or_default() += states.len() - known;
            }
            successors.push(Edge { action, cost, to });
        }
        Ok(successors)
    };
    // Under a higher ceiling, the actions it adds to the states already expanded.
    if let Some(from) = raise {
        let open: Vec<usize> = (0..edges.len()).filter(|&s| !end[s]).collect();
        let more: Vec<(usize, Expansion<M::State>)> =
            open.par_iter().map(|&s| (s, actions_from(&states[s], from, (separating || !complete[s]).then_some(false)))).collect();
        for (s, out) in more {
            let new = link(out, states, index, found)?;
            edges[s].extend(new);
        }
    }
    let mut reached: Vec<Update<M::State>> = Vec::new();
    // The last round's policy (as action ids) and worth of starting over, to start from.
    let mut acted: Vec<usize> = Vec::new();
    let mut warm = None;
    // Values at the last solve, extended over states found since.
    let mut known: Vec<f64> = Vec::new();
    let mut seed: Vec<Affine> = Vec::new();
    // Values at the last check of deferred actions, and per state how far its deferred actions'
    // worth stayed above its value then (see `settled` below).
    let mut slack: Option<(Vec<f64>, Vec<f64>)> = None;
    'search: loop {
        stats.rounds += 1;
        let clock = std::time::Instant::now();
        let mut fresh: Vec<usize> = reached.iter().map(Update::state).collect();
        let entered = edges.len();
        if separating && !reached.is_empty() {
            for (s, e) in dropped.drain(..) {
                edges[s].push(e);
            }
        }
        for update in reached.drain(..) {
            match update {
                Update::Linked(s, new) => {
                    edges[s].retain(|e| !e.to.pending() || !new.iter().any(|a| a.action == e.action));
                    edges[s].extend(new);
                    complete[s] = edges[s].iter().all(|e| !e.to.pending());
                }
                Update::Discover(_, action, new) => {
                    let known = states.len();
                    for state in new {
                        intern(&state, states, index, limit)?;
                    }
                    let count = states.len() - known;
                    *found.entry(action).or_default() += count;
                    stats.discovered += count;
                }
                Update::Tip(s, out) => {
                    edges[s] = link(out, states, index, found)?;
                    (end[s], tip[s], fixed[s]) = (false, false, None);
                }
                Update::Deferred(s, out) => {
                    edges[s].retain(|e| !e.to.pending() || !out.iter().any(|a| a.0 == e.action));
                    edges[s].extend(link(out, states, index, found)?);
                    complete[s] = edges[s].iter().all(|e| !e.to.pending());
                    if !separating {
                        acted[s] = usize::MAX;
                    }
                }
            }
        }
        // Enter new states in batches, the model's work in parallel, numbering in order.
        while edges.len() < states.len() {
            let batch = edges.len()..states.len().min(edges.len() + EXPANSION_BATCH);
            let entries: Vec<Entry<M::State>> = states[batch.clone()]
                .par_iter()
                .map(|s| {
                    if model.is_goal(s) {
                        Entry::Fixed(Affine { a: 0.0, b: 0.0 })
                    } else if model.is_restart(s) {
                        Entry::Fixed(Affine { a: 0.0, b: 1.0 })
                    } else if let Some(bound) = model.bound(s).filter(|_| !start.iter().any(|(_, t)| t == s)) {
                        Entry::Tip(bound)
                    } else {
                        Entry::New(expand(s))
                    }
                })
                .collect();
            for entry in entries {
                let (value, unexpanded, out) = match entry {
                    Entry::Fixed(value) => (Some(value), false, Vec::new()),
                    Entry::Tip(bound) => (Some(Affine { a: bound, b: 0.0 }), true, Vec::new()),
                    Entry::New(out) => (None, false, link(out, states, index, found)?),
                };
                end.push(value.is_some());
                complete.push(false);
                tip.push(unexpanded);
                fixed.push(value);
                edges.push(out);
            }
        }
        let n = states.len();
        fresh.extend((entered..n).filter(|&s| !end[s]));
        if let Some(w) = warm.filter(|_| !fresh.is_empty()) {
            known.extend((known.len()..n).map(|s| fixed[s].map_or(0.0, |f| f.at(w))));
            for &s in fresh.iter().rev() {
                known[s] = best(&edges[s], &known).0;
            }
            let (tips, pending) = discover(edges, fixed, tip, &starts, &mut known);
            if !tips.is_empty() || !pending.is_empty() {
                acted.resize(n, usize::MAX);
                reached.extend(tips.par_iter().map(|&s| Update::Tip(s, expand(&states[s]))).collect::<Vec<_>>());
                reached.extend(
                    pending.par_iter().map(|&s| Update::Deferred(s, expand_deferred(model, &states[s], &edges[s]))).collect::<Vec<_>>(),
                );
                stats.expanded += tips.len();
                stats.completed += pending.len();
                stats.explore += clock.elapsed().as_secs_f64();
                continue;
            }
        }
        let outcomes = edges.iter().flatten().map(|e| e.outcomes().len()).sum();
        let actions = edges.iter().map(Vec::len).sum();
        stats.explore += clock.elapsed().as_secs_f64();
        stats.solves += 1;
        let clock = std::time::Instant::now();
        // Keep only actions whose every outcome can still reach the goal (or a restart) and that
        // leave the state; a state is alive while some kept action path gets there.
        let mut alive = vec![true; n];
        let all_reach = all_reach_terminal(edges, end);
        loop {
            for s in 0..n {
                if !end[s] {
                    let (keep, drop) = std::mem::take(&mut edges[s]).into_iter().partition(|e: &Edge| {
                        e.to.pending() || e.outcomes().iter().all(|&(_, t)| alive[t]) && e.outcomes().iter().any(|&(_, t)| t != s)
                    });
                    edges[s] = keep;
                    dropped.extend(drop.into_iter().map(|e| (s, e)));
                }
            }
            if all_reach {
                break;
            }
            let mut into: Vec<Vec<usize>> = vec![Vec::new(); n];
            let mut seen = vec![usize::MAX; n];
            for (s, out) in edges.iter().enumerate() {
                for t in successors(out, &mut seen, s) {
                    into[t].push(s);
                }
            }
            let mut reach: Vec<bool> = (0..n).map(|s| end[s] || edges[s].iter().any(|e| e.to.optimistic())).collect();
            let mut stack: Vec<usize> = (0..n).filter(|&s| reach[s]).collect();
            while let Some(t) = stack.pop() {
                for &s in &into[t] {
                    if !reach[s] {
                        reach[s] = true;
                        stack.push(s);
                    }
                }
            }
            if model.gives_up() {
                for s in 0..n {
                    if reach[s] {
                        continue;
                    }
                    (end[s], fixed[s], reach[s]) = (true, Some(Affine { a: 0.0, b: 1.0 }), true);
                    dropped.extend(std::mem::take(&mut edges[s]).into_iter().map(|e| (s, e)));
                    given_up.push(s);
                }
            }
            if reach == alive {
                break;
            }
            alive = reach;
        }

        let mut seen = vec![usize::MAX; n];
        let adjacency: Vec<Vec<usize>> = edges.iter().enumerate().map(|(s, es)| successors(es, &mut seen, s).collect()).collect();
        let order: Vec<Vec<usize>> = components(n, |s| adjacency[s].iter().copied())
            .into_iter()
            .map(|c| c.into_iter().filter(|&s| alive[s] && !end[s]).collect())
            .filter(|c: &Vec<usize>| !c.is_empty())
            .collect();
        stats.prune += clock.elapsed().as_secs_f64();
        let clock = std::time::Instant::now();
        let restarts = fixed.iter().any(|f| f.is_some_and(|f| f.b > 0.0));
        let rounds = if edges.iter().flatten().any(|e| matches!(e.to, Transitions::Deferred)) { ADMISSION_ROUNDS } else { ROUNDS };
        let mut local = vec![usize::MAX; n];
        for component in &order {
            for (i, &s) in component.iter().enumerate() {
                local[s] = i;
            }
        }
        acted.resize(n, usize::MAX);
        let mut choice: Vec<usize> =
            (0..n).map(|s| edges[s].iter().position(|e| !end[s] && e.action == acted[s]).unwrap_or(usize::MAX)).collect();
        let mut affine: Vec<Affine> = fixed.iter().map(|f| f.unwrap_or(Affine::INFINITE)).collect();
        // Start high so restarting looks dear; Newton steps then fall monotonically to the fixed
        // point. Or from the last round's worth, below it: any proper policy's worth is above, so
        // the first step lands above it too. Or from the last ceiling's, above it.
        let initial = if separating { warm.or(*worth) } else { (*worth).or(warm) };
        let mut w = if restarts { initial.unwrap_or(1e12) } else { 0.0 };
        let mut first = true;
        let mut solved = None;
        for _ in 0..200 {
            stats.newton += 1;
            let mut values: Vec<f64> = affine.iter().map(|f| f.at(w)).collect();
            let mut stable = true;
            for component in &order {
                stable &= solve_component(component, &local, edges, &mut values, &mut choice, &mut affine, &seed, w, rounds);
            }
            let at_start = starts
                .iter()
                .fold(Affine { a: 0.0, b: 0.0 }, |acc, &(p, s)| Affine { a: acc.a + p * affine[s].a, b: acc.b + p * affine[s].b });
            // A policy still improving may not reach the goal yet: keep improving at this `w`.
            let hopeless = !at_start.a.is_finite() || (restarts && at_start.b >= 1.0 - 1e-12);
            if hopeless && !stable {
                continue;
            }
            // Below the start's worth everything may start over: begin again from high up.
            if hopeless && restarts && w < 1e12 {
                (w, first) = (1e12, false);
                continue;
            }
            if hopeless {
                if separating {
                    let pending: Vec<_> = (0..n).filter(|&s| edges[s].iter().any(|e| e.to.pending())).collect();
                    if !pending.is_empty() {
                        stats.values += clock.elapsed().as_secs_f64();
                        let clock = std::time::Instant::now();
                        reached = pending.par_iter().map(|&s| Update::Deferred(s, expand_deferred(model, &states[s], &edges[s]))).collect();
                        stats.completed += pending.len();
                        stats.explore += clock.elapsed().as_secs_f64();
                        continue 'search;
                    }
                }
                return Err(Error::Unreachable);
            }
            let next = if !restarts {
                w
            } else if first {
                at_start.a / (1.0 - at_start.b)
            } else {
                (at_start.a / (1.0 - at_start.b)).min(w)
            };
            first = false;
            if stable && (next - w).abs() <= 1e-10 * next.abs().max(1.0) {
                solved = Some((at_start, next));
                break;
            }
            w = next;
        }
        stats.values += clock.elapsed().as_secs_f64();
        let Some((at_start, w)) = solved else { return Err(Error::Unreachable) };
        seed.clone_from(&affine);
        warm = Some(w);
        known = affine.iter().map(|f| f.at(w)).collect();
        acted = (0..n).map(|s| if choice[s] == usize::MAX { usize::MAX } else { edges[s][choice[s]].action }).collect();
        let mut reachable = policy_reach(edges, &choice, &starts);
        let tips: Vec<usize> = (0..n).filter(|&s| reachable[s] && tip[s]).collect();
        if !tips.is_empty() {
            let clock = std::time::Instant::now();
            reached = tips.par_iter().map(|&s| Update::Tip(s, expand(&states[s]))).collect();
            stats.expanded += reached.len();
            stats.explore += clock.elapsed().as_secs_f64();
            continue;
        }
        let pending: Vec<usize> =
            (0..n).filter(|&s| reachable[s] && choice[s] != usize::MAX && edges[s][choice[s]].to.optimistic()).collect();
        let clock = std::time::Instant::now();
        reached = pending.par_iter().map(|&s| Update::Deferred(s, expand_deferred(model, &states[s], &edges[s]))).collect();
        stats.completed += pending.len();
        stats.explore += clock.elapsed().as_secs_f64();
        if !pending.is_empty() {
            continue;
        }
        if separating && edges.iter().flatten().any(|e| matches!(e.to, Transitions::Deferred)) {
            let clock = std::time::Instant::now();
            let inspect = |s: usize| w < ceiling || reachable[s];
            let finite = known.iter().all(|v| v.is_finite() && *v >= 0.0);
            let lookup = |t: &M::State| index.get(t).map_or_else(|| model.bound(t).filter(|v| v.is_finite()).unwrap_or(0.0), |&t| known[t]);
            let tolerance = |v: f64| 1e-10 * v.abs().max(1.0);
            // A deferred action's worth falls at most as far as any value has since the last
            // check. A state whose deferred actions all stayed further above its value than
            // that cannot have a violated one now, and is not checked again.
            let fallen = slack.as_ref().map(|(then, _)| then.iter().zip(&known).map(|(a, b)| a - b).fold(0.0, f64::max));
            let room_now = |s: usize| match (&slack, fallen) {
                (Some((then, room)), Some(fallen)) if s < room.len() => room[s] - fallen - (known[s] - then[s]),
                _ => f64::NEG_INFINITY,
            };
            let settled = |s: usize| room_now(s) > tolerance(known[s]);
            let open: Vec<usize> = (0..n).into_par_iter().filter(|&s| !end[s] && inspect(s) && !settled(s)).collect();
            let unsettled: Vec<M::State> = open.iter().map(|&s| states[s].clone()).collect();
            let evaluate = finite.then(|| model.prepare_expectations(&unsettled, &lookup));
            // The prepared worth of each unsettled state's deferred actions, at these values.
            let futures: Vec<(usize, Futures)> = match &evaluate {
                Some(evaluate) => open
                    .par_iter()
                    .map(|&s| {
                        let deferred = edges[s].iter().filter(|e| matches!(e.to, Transitions::Deferred) && e.cost < known[s]);
                        (s, deferred.map(|e| (e.action, evaluate(&states[s], e.action))).collect())
                    })
                    .collect(),
                None => Vec::new(),
            };
            let mut link_states = Vec::new();
            if evaluate.is_some() {
                let mut candidates = std::collections::BTreeMap::<usize, Vec<usize>>::new();
                for (s, worth) in &futures {
                    let mut materialize = false;
                    for (action, future) in worth {
                        let cost = edges[*s].iter().find(|e| e.action == *action).map_or(f64::INFINITY, |e| e.cost);
                        match future {
                            Some(future) if cost + future < known[*s] - tolerance(known[*s]) => {
                                candidates.entry(*action).or_default().push(*s);
                                materialize = true;
                            }
                            None => materialize = true,
                            _ => {}
                        }
                    }
                    if materialize {
                        link_states.push(*s);
                    }
                }
                let mut updates = Vec::new();
                let missing = |t: &M::State| !index.contains_key(t) && !model.is_goal(t);
                for (action, sources) in candidates {
                    let inputs: Vec<_> = sources.iter().map(|&s| &states[s]).collect();
                    let new = model.discover_successors(&inputs, action, &missing);
                    if !new.is_empty() {
                        updates.push(Update::Discover(sources[0], action, new));
                    }
                }
                if !updates.is_empty() {
                    stats.separation += clock.elapsed().as_secs_f64();
                    reached = updates;
                    continue;
                }
            }
            let mut at = vec![usize::MAX; n];
            for (k, (s, _)) in futures.iter().enumerate() {
                at[*s] = k;
            }
            let lookup_index = |t: &M::State| index.get(t).copied();
            let link_states: Vec<_> = link_states.into_iter().map(|s| states[s].clone()).collect();
            let transitions = (!link_states.is_empty()).then(|| model.prepare_transitions(&link_states, &lookup_index));
            let (mut updates, checks, rooms) = (0..n)
                .into_par_iter()
                .fold(
                    || (Vec::new(), Checks::default(), Vec::new()),
                    |(mut updates, mut checks, mut rooms), s| {
                        if end[s] || !inspect(s) {
                            return (updates, checks, rooms);
                        }
                        if settled(s) {
                            checks.settled += 1;
                            rooms.push((s, room_now(s)));
                            return (updates, checks, rooms);
                        }
                        let actions = edges[s].iter().filter(|e| matches!(e.to, Transitions::Deferred)).map(|e| (e.action, e.cost));
                        let mut selected = None;
                        let mut room = f64::INFINITY;
                        for (action, cost) in actions {
                            checks.actions += 1;
                            if finite && cost >= known[s] {
                                checks.bounded += 1;
                                room = room.min(cost - known[s]);
                                continue;
                            }
                            let mut value = cost;
                            let mut terms = 0;
                            let cached = futures.get(at[s]).and_then(|(_, worth)| worth.iter().find(|w| w.0 == action)).map(|w| w.1);
                            let future = cached.unwrap_or_else(|| evaluate.as_ref().and_then(|evaluate| evaluate(&states[s], action)));
                            let nonempty = if let Some(future) = future {
                                value += future;
                                checks.prepared += 1;
                                true
                            } else {
                                model.for_each_outcome(&states[s], action, &mut |p, t| {
                                    terms += 1;
                                    if p > 0.0 {
                                        value += p * lookup(&t);
                                    }
                                });
                                terms > 0
                            };
                            checks.outcomes += terms;
                            room = room.min(if finite && nonempty { value - known[s] } else { f64::NEG_INFINITY });
                            if !finite || (nonempty && value < known[s] - tolerance(known[s])) {
                                checks.violated += 1;
                                if selected.is_none_or(|(_, _, best)| value < best) {
                                    selected = Some((action, cost, value));
                                }
                            }
                        }
                        rooms.push((s, room));
                        if let Some((action, cost, _)) = selected {
                            if let Some(to) = transitions.as_ref().and_then(|link| link(&states[s], action)) {
                                updates.push(Update::Linked(s, vec![Edge { action, cost, to: Transitions::Known(to) }]));
                                return (updates, checks, rooms);
                            }
                            let out = model.outcomes(&states[s], action);
                            let linked: Option<Box<[_]>> =
                                out.iter().filter(|o| o.0 > 0.0).map(|(p, t)| index.get(t).map(|&t| (*p, t))).collect();
                            if let Some(to) = linked {
                                updates.push(Update::Linked(s, vec![Edge { action, cost, to: Transitions::Known(to) }]));
                            } else {
                                updates.push(Update::Deferred(s, vec![(action, cost, Transitions::Known(out.into_boxed_slice()))]));
                            }
                        }
                        (updates, checks, rooms)
                    },
                )
                .reduce(
                    || (Vec::new(), Checks::default(), Vec::new()),
                    |(mut left, mut a, mut rooms), (mut right, b, mut more)| {
                        left.append(&mut right);
                        rooms.append(&mut more);
                        a.actions += b.actions;
                        a.bounded += b.bounded;
                        a.prepared += b.prepared;
                        a.outcomes += b.outcomes;
                        a.violated += b.violated;
                        a.settled += b.settled;
                        (left, a, rooms)
                    },
                );
            let mut room = vec![f64::NEG_INFINITY; n];
            for (s, r) in rooms {
                room[s] = r;
            }
            slack = Some((known.clone(), room));
            stats.checked += checks.actions;
            stats.bounded += checks.bounded;
            stats.prepared += checks.prepared;
            stats.streamed_outcomes += checks.outcomes;
            stats.violated += checks.violated;
            stats.settled += checks.settled;
            stats.separation += clock.elapsed().as_secs_f64();
            updates.sort_by_key(Update::state);
            stats.completed += updates.iter().filter(|u| matches!(u, Update::Deferred(_, _) | Update::Linked(_, _))).count();
            stats.expanded += updates.iter().filter(|u| matches!(u, Update::Tip(_, _))).count();
            if !updates.is_empty() {
                reached = updates;
                continue;
            }
        }
        *worth = Some(w);
        stats.outcomes += outcomes;
        stats.found = found.clone();
        let given_up = given_up.iter().map(|&s| states[s].clone()).collect();
        let explored = n;
        if keep_all {
            reachable.iter_mut().for_each(|r| *r = true);
        }
        let retained = reachable.iter().filter(|r| **r).count();
        let mut kept = Vec::with_capacity(retained);
        let mut values = Vec::with_capacity(retained);
        let mut policy = Vec::with_capacity(retained);
        let mut index = HashMap::with_capacity_and_hasher(retained, Default::default());
        for (i, state) in states.iter().enumerate().filter(|(i, _)| reachable[*i]) {
            index.insert(state.clone(), kept.len());
            kept.push(state.clone());
            values.push(affine[i].at(w));
            policy.push((choice[i] != usize::MAX && !end[i] && alive[i]).then(|| edges[i][choice[i]].action));
        }
        let solution = Solution {
            cost: at_start.at(w),
            states: kept,
            values,
            policy,
            index,
            given_up,
            outcomes,
            explored,
            actions,
            stats: stats.clone(),
        };
        return Ok((solution, w));
    }
}

fn all_reach_terminal(edges: &[Vec<Edge>], end: &[bool]) -> bool {
    let mut reached: Vec<_> = edges
        .iter()
        .enumerate()
        .map(|(s, out)| end[s] || out.iter().rev().any(|e| e.to.optimistic() || matches!(e.outcomes(), [(_, t)] if end[*t])))
        .collect();
    for s in (0..edges.len()).rev() {
        if !reached[s]
            && edges[s].iter().any(|e| e.to.optimistic() || !e.outcomes().is_empty() && e.outcomes().iter().all(|&(_, t)| reached[t]))
        {
            reached[s] = true;
        }
    }
    reached.into_iter().all(|r| r)
}

fn policy_reach(edges: &[Vec<Edge>], choice: &[usize], starts: &[(f64, usize)]) -> Vec<bool> {
    let mut seen = vec![false; edges.len()];
    let mut stack = Vec::new();
    for &(_, s) in starts {
        if !seen[s] {
            seen[s] = true;
            stack.push(s);
        }
    }
    while let Some(s) = stack.pop() {
        if let Some(edge) = edges[s].get(choice[s]) {
            for &(_, t) in edge.outcomes() {
                if !seen[t] {
                    seen[t] = true;
                    stack.push(t);
                }
            }
        }
    }
    seen
}

fn discover(
    edges: &[Vec<Edge>],
    fixed: &[Option<Affine>],
    tip: &[bool],
    starts: &[(f64, usize)],
    values: &mut [f64],
) -> (Vec<usize>, Vec<usize>) {
    let walk = |values: &mut [f64], backup: bool| {
        let mut tips = Vec::new();
        let mut pending = Vec::new();
        let mut seen = vec![false; edges.len()];
        let mut stack = Vec::new();
        for &(_, s) in starts {
            if !seen[s] {
                seen[s] = true;
                stack.push((s, false));
            }
        }
        while let Some((s, leave)) = stack.pop() {
            if leave {
                values[s] = best(&edges[s], values).0;
                continue;
            }
            if tip[s] {
                tips.push(s);
            }
            if fixed[s].is_some() {
                continue;
            }
            let Some(edge) = edges[s].get(best(&edges[s], values).1) else { continue };
            if edge.to.optimistic() {
                pending.push(s);
            }
            if backup {
                stack.push((s, true));
            }
            for &(_, t) in edge.outcomes() {
                if !seen[t] {
                    seen[t] = true;
                    stack.push((t, false));
                }
            }
        }
        (tips, pending)
    };
    for _ in 0..16 {
        let (mut tips, mut pending) = walk(values, true);
        let (next_tips, next_pending) = walk(values, false);
        tips.extend(next_tips);
        pending.extend(next_pending);
        tips.sort_unstable();
        tips.dedup();
        pending.sort_unstable();
        pending.dedup();
        if !tips.is_empty() || !pending.is_empty() {
            return (tips, pending);
        }
        let w: f64 = starts.iter().map(|&(p, s)| p * values[s]).sum();
        let mut changed = false;
        for (s, value) in fixed.iter().enumerate() {
            if let Some(value) = value.filter(|f| f.b > 0.0) {
                changed |= (values[s] - value.at(w)).abs() > 1e-10 * w.abs().max(1.0);
                values[s] = value.at(w);
            }
        }
        if !changed {
            break;
        }
    }
    (Vec::new(), Vec::new())
}

fn successors<'a>(edges: &'a [Edge], seen: &'a mut [usize], s: usize) -> impl Iterator<Item = usize> + 'a {
    edges.iter().flat_map(|e| e.outcomes().iter().map(|o| o.1)).filter(move |&t| {
        let new = seen[t] != s;
        seen[t] = s;
        new
    })
}

fn q(e: &Edge, values: &[f64]) -> f64 {
    e.constant() + e.outcomes().iter().map(|&(p, t)| p * values[t]).sum::<f64>()
}

/// The best action by current values; the lowest id wins ties.
fn best(edges: &[Edge], values: &[f64]) -> (f64, usize) {
    let mut out = (f64::INFINITY, usize::MAX);
    for (i, e) in edges.iter().enumerate() {
        let v = q(e, values);
        if v < out.0 - 1e-12 * v.abs().max(1.0) {
            out = (v, i);
        }
    }
    out
}

/// Values of one component, all of whose successors outside it are already solved.
///
/// Policy iteration from a proper policy: one that leaves the component with certainty. The
/// previous solve's choice is one (exits do not depend on `w`); the first time, each state's
/// first action is one that can step closer to the exit, so every state can reach the exit with
/// positive probability. With positive costs, improvement keeps policies proper.
///
/// At most `ROUNDS` improvements per call, true if the policy came out stable: any proper policy's
/// cost bounds the optimum from above, so Newton on `w` stays valid while policy iteration
/// finishes over later calls, and at a huge first `w` the rounds are mostly spent on noise.
#[allow(clippy::too_many_arguments)]
fn solve_component(
    states: &[usize],
    local: &[usize],
    edges: &[Vec<Edge>],
    values: &mut [f64],
    choice: &mut [usize],
    affine: &mut [Affine],
    seed: &[Affine],
    w: f64,
    rounds: usize,
) -> bool {
    let position = |t: usize| states.get(local[t]).filter(|&&s| s == t).map(|_| local[t]);
    let mut chosen: Vec<usize> = states.iter().map(|&s| choice[s]).collect();
    if chosen.contains(&usize::MAX) {
        let mut ranked = vec![false; states.len()];
        let mut progress = true;
        while progress {
            progress = false;
            for (i, &s) in states.iter().enumerate() {
                if ranked[i] {
                    continue;
                }
                let exits = |e: &Edge| e.to.optimistic() || e.outcomes().iter().any(|&(_, t)| position(t).is_none_or(|j| ranked[j]));
                let action = if chosen[i] == usize::MAX {
                    edges[s].iter().position(exits)
                } else {
                    edges[s].get(chosen[i]).filter(|e| exits(e)).map(|_| chosen[i])
                };
                if let Some(a) = action {
                    chosen[i] = a;
                    ranked[i] = true;
                    progress = true;
                }
            }
            if !progress {
                let repair = states.iter().enumerate().filter(|(i, _)| !ranked[*i]).find_map(|(i, &s)| {
                    edges[s]
                        .iter()
                        .position(|e| e.to.optimistic() || e.outcomes().iter().any(|&(_, t)| position(t).is_none_or(|j| ranked[j])))
                        .map(|a| (i, a))
                });
                if let Some((i, a)) = repair {
                    chosen[i] = a;
                    ranked[i] = true;
                    progress = true;
                }
            }
        }
        debug_assert!(ranked.iter().all(|r| *r), "pruning leaves every live state a way out");
    }
    // The last policy evaluated, so values and choices agree even if an improvement fails to
    // evaluate (at a huge first `w`, rounding can pick an improper one).
    let mut kept = chosen.clone();
    let mut stable = true;
    for _ in 0..rounds {
        stable = false;
        let Some(v) = evaluate(states, local, edges, &chosen, affine, seed) else {
            stable = true;
            break;
        };
        kept = chosen.clone();
        for (i, &s) in states.iter().enumerate() {
            affine[s] = v[i];
            values[s] = v[i].at(w);
        }
        let improved: Vec<usize> = states
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                if edges[s].len() == 1 {
                    return chosen[i];
                }
                let (v, a) = best(&edges[s], values);
                let current = q(&edges[s][chosen[i]], values);
                if v < current - 1e-10 * current.abs().max(1.0) { a } else { chosen[i] }
            })
            .collect();
        if improved == chosen {
            stable = true;
            break;
        }
        chosen = improved;
    }
    for (i, &s) in states.iter().enumerate() {
        choice[s] = kept[i];
    }
    stable
}

/// Policy improvements per component and Newton step.
const ROUNDS: usize = 10;
const ADMISSION_ROUNDS: usize = 3;

/// Exact values of a fixed policy inside a component, as `a + b w`. The policy's own graph
/// usually splits into much smaller components; each solves (I - P) x = r for the cost part
/// (r = c + P_out a_out) and the restart part (r = P_out b_out) together, successors first.
fn evaluate(
    states: &[usize],
    local: &[usize],
    edges: &[Vec<Edge>],
    choice: &[usize],
    affine: &[Affine],
    seed: &[Affine],
) -> Option<Vec<Affine>> {
    let chosen = |i: usize| &edges[states[i]][choice[i]];
    let position = |t: usize| states.get(local[t]).filter(|&&s| s == t).map(|_| local[t]);
    let mut out = vec![Affine::INFINITE; states.len()];
    let mut place = vec![usize::MAX; states.len()];
    for part in components(states.len(), |i| chosen(i).outcomes().iter().filter_map(|&(_, t)| position(t))) {
        let n = part.len();
        if n == 1 {
            let i = part[0];
            let edge = chosen(i);
            let mut value = Affine { a: edge.constant(), b: 0.0 };
            let mut diagonal: f64 = 1.0;
            for &(p, t) in edge.outcomes() {
                if t == states[i] {
                    diagonal -= p;
                    continue;
                }
                let known = position(t).map_or(affine[t], |j| out[j]);
                value.a += p * known.a;
                value.b += p * known.b;
            }
            if diagonal.abs() < 1e-14 {
                return None;
            }
            out[i] = Affine { a: value.a / diagonal, b: value.b / diagonal };
            continue;
        }
        for (k, &i) in part.iter().enumerate() {
            place[i] = k;
        }
        // Row k: transitions inside the part, and the known value of the rest.
        let mut rows: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
        let mut ra = vec![0.0; n];
        let mut rb = vec![0.0; n];
        for (k, &i) in part.iter().enumerate() {
            let e = chosen(i);
            ra[k] = e.constant();
            for &(p, t) in e.outcomes() {
                let known = match position(t) {
                    Some(j) if place[j] != usize::MAX && part.get(place[j]) == Some(&j) => {
                        rows[k].push((place[j], p));
                        continue;
                    }
                    Some(j) => out[j],
                    None => affine[t],
                };
                ra[k] += p * known.a;
                rb[k] += p * known.b;
            }
        }
        let x = if n <= DENSE {
            let mut m = vec![0.0; n * n];
            for (k, row) in rows.iter().enumerate() {
                m[k * n + k] += 1.0;
                for &(j, p) in row {
                    m[k * n + j] -= p;
                }
            }
            lu_solve(n, &mut m, &mut [&mut ra, &mut rb])?
        } else {
            let start = |f: fn(&Affine) -> f64| {
                part.iter()
                    .map(|&i| {
                        Some(f(&affine[states[i]]))
                            .filter(|v| v.is_finite())
                            .or_else(|| seed.get(states[i]).map(f).filter(|v| v.is_finite()))
                            .unwrap_or(0.0)
                    })
                    .collect()
            };
            let factor = std::cell::OnceCell::new();
            let solve = |r: &[f64], x: Vec<f64>| gauss_seidel(&rows, r, bicgstab(&rows, r, x, &factor));
            vec![solve(&ra, start(|f| f.a))?, solve(&rb, start(|f| f.b))?]
        };
        for (k, &i) in part.iter().enumerate() {
            out[i] = Affine { a: x[0][k], b: x[1][k] };
        }
    }
    out.iter().all(|f| f.a.is_finite() && f.a >= 0.0 && f.b.is_finite() && f.b >= -1e-9).then_some(out)
}

/// Largest policy component solved by dense elimination; larger ones iterate.
const DENSE: usize = 64;

/// Solves x = r + P x by Gauss-Seidel sweeps from `x`, each state's self-loop eliminated (rerolls
/// mostly return to themselves), until no value moves by more than a relative 1e-12. `None` when
/// it does not converge.
fn gauss_seidel(rows: &[Vec<(usize, f64)>], r: &[f64], mut x: Vec<f64>) -> Option<Vec<f64>> {
    let stay: Vec<f64> = rows.iter().enumerate().map(|(k, row)| row.iter().filter(|e| e.0 == k).map(|e| e.1).sum()).collect();
    if stay.iter().any(|p| 1.0 - p < 1e-14) {
        return None;
    }
    for _ in 0..1_000_000 {
        let mut moved = 0.0f64;
        for (k, row) in rows.iter().enumerate() {
            let inflow: f64 = row.iter().filter(|e| e.0 != k).map(|&(j, p)| p * x[j]).sum();
            let v = (r[k] + inflow) / (1.0 - stay[k]);
            moved = moved.max((v - x[k]).abs() / v.abs().max(1.0));
            x[k] = v;
        }
        if moved < 1e-12 {
            return Some(x);
        }
    }
    None
}

fn bicgstab(rows: &[Vec<(usize, f64)>], b: &[f64], x: Vec<f64>, factor: &std::cell::OnceCell<Option<ilu::Ilu>>) -> Vec<f64> {
    let n = rows.len();
    let d: Vec<f64> = rows.iter().enumerate().map(|(k, row)| 1.0 - row.iter().filter(|e| e.0 == k).map(|e| e.1).sum::<f64>()).collect();
    let apply = |x: &[f64]| -> Vec<f64> {
        rows.iter().enumerate().map(|(k, row)| x[k] - row.iter().map(|&(j, p)| p * x[j]).sum::<f64>()).collect()
    };
    let precondition = |r: &[f64]| {
        if let Some(factor) = factor.get_or_init(|| ilu::Ilu::new(rows)) {
            return factor.apply(r);
        }
        let mut x = vec![0.0; n];
        for (k, row) in rows.iter().enumerate() {
            let inflow: f64 = row.iter().filter(|e| e.0 < k).map(|&(j, p)| p * x[j]).sum();
            x[k] = (r[k] + inflow) / d[k];
        }
        x
    };
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    let norm = |a: &[f64]| a.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let ax = apply(&x);
    let mut r: Vec<f64> = (0..n).map(|k| b[k] - ax[k]).collect();
    let rhat = r.clone();
    let (mut rho, mut alpha, mut omega) = (1.0, 1.0, 1.0);
    let (mut v, mut p) = (vec![0.0; n], vec![0.0; n]);
    let (mut x, mut best) = (x.clone(), x);
    let mut least = norm(&r);
    for _ in 0..10_000 {
        let scale = norm(&x).max(norm(b)).max(1.0);
        if least <= 1e-13 * scale {
            break;
        }
        let rho_next = dot(&rhat, &r);
        let beta = rho_next / rho * (alpha / omega);
        rho = rho_next;
        p = (0..n).map(|k| r[k] + beta * (p[k] - omega * v[k])).collect();
        let y = precondition(&p);
        v = apply(&y);
        alpha = rho / dot(&rhat, &v);
        let s: Vec<f64> = (0..n).map(|k| r[k] - alpha * v[k]).collect();
        if norm(&s) <= 1e-13 * scale && s.iter().all(|v| v.is_finite()) {
            let next: Vec<_> = (0..n).map(|k| x[k] + alpha * y[k]).collect();
            if next.iter().all(|v| v.is_finite()) {
                best = next;
            }
            break;
        }
        let z = precondition(&s);
        let t = apply(&z);
        omega = dot(&t, &s) / dot(&t, &t);
        x = (0..n).map(|k| x[k] + alpha * y[k] + omega * z[k]).collect();
        r = (0..n).map(|k| s[k] - omega * t[k]).collect();
        let size = norm(&r);
        if !size.is_finite() || !omega.is_finite() || omega == 0.0 || rho == 0.0 {
            break;
        }
        if size < least {
            (least, best) = (size, x.clone());
        }
    }
    best
}

/// Gaussian elimination with partial pivoting, for several right-hand sides; `None` when
/// singular.
fn lu_solve(n: usize, a: &mut [f64], rhs: &mut [&mut Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    for k in 0..n {
        let p = (k..n).max_by(|&i, &j| a[i * n + k].abs().total_cmp(&a[j * n + k].abs()))?;
        if a[p * n + k].abs() < 1e-14 {
            return None;
        }
        if p != k {
            for c in 0..n {
                a.swap(k * n + c, p * n + c);
            }
            for b in rhs.iter_mut() {
                b.swap(k, p);
            }
        }
        for i in k + 1..n {
            let f = a[i * n + k] / a[k * n + k];
            if f != 0.0 {
                for c in k..n {
                    a[i * n + c] -= f * a[k * n + c];
                }
                for b in rhs.iter_mut() {
                    b[i] -= f * b[k];
                }
            }
        }
    }
    let mut out = Vec::with_capacity(rhs.len());
    for b in rhs.iter() {
        let mut x = vec![0.0; n];
        for k in (0..n).rev() {
            let s: f64 = (k + 1..n).map(|c| a[k * n + c] * x[c]).sum();
            x[k] = (b[k] - s) / a[k * n + k];
        }
        out.push(x);
    }
    Some(out)
}

/// Strongly connected components of a successor graph, successors before predecessors
/// (Tarjan, iterative).
fn components<I: Iterator<Item = usize>>(n: usize, successors: impl Fn(usize) -> I) -> Vec<Vec<usize>> {
    let (mut index, mut low, mut on) = (vec![usize::MAX; n], vec![0; n], vec![false; n]);
    let (mut stack, mut out, mut counter) = (Vec::new(), Vec::new(), 0);
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut call = vec![(root, successors(root))];
        index[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on[root] = true;
        while let Some((v, next)) = call.last_mut() {
            let v = *v;
            if let Some(w) = next.next() {
                if index[w] == usize::MAX {
                    index[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    on[w] = true;
                    call.push((w, successors(w)));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                call.pop();
                if let Some((parent, _)) = call.last() {
                    low[*parent] = low[*parent].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut comp = Vec::new();
                    while let Some(w) = stack.pop() {
                        on[w] = false;
                        comp.push(w);
                        if w == v {
                            break;
                        }
                    }
                    out.push(comp);
                }
            }
        }
    }
    out
}

/// Where following `solution` from `start` ends: each goal state's chance, with restarts (and
/// states that gave up) beginning again from `start`, so the chances sum to one.
pub fn ends<M: Model>(model: &M, solution: &Solution<M::State>, start: &[(f64, M::State)]) -> Vec<(f64, M::State)> {
    // The states the policy visits, in visiting order, and the flow into each.
    let mut order: Vec<usize> = Vec::new();
    let mut place: HashMap<usize, usize> = HashMap::default();
    let mut seed: Vec<f64> = Vec::new();
    let mut visit = |s: usize, order: &mut Vec<usize>, seed: &mut Vec<f64>| {
        *place.entry(s).or_insert_with(|| {
            order.push(s);
            seed.push(0.0);
            order.len() - 1
        })
    };
    for (p, s) in start {
        let k = visit(solution.index[s], &mut order, &mut seed);
        seed[k] += p;
    }
    let mut into: Vec<Vec<(usize, f64)>> = Vec::new();
    let mut stay: Vec<f64> = Vec::new();
    let mut k = 0;
    while k < order.len() {
        into.resize(order.len(), Vec::new());
        stay.resize(order.len(), 0.0);
        let s = order[k];
        if let Some(a) = solution.policy[s] {
            for (p, t) in model.outcomes(&solution.states[s], a) {
                let Some(&t) = solution.index.get(&t) else { continue };
                let j = visit(t, &mut order, &mut seed);
                into.resize(order.len(), Vec::new());
                stay.resize(order.len(), 0.0);
                if j == k { stay[k] += p } else { into[j].push((k, p)) }
            }
        }
        k += 1;
    }
    // Expected visits: x = seed + flow in.
    let rows: Vec<Vec<(usize, f64)>> = into
        .into_iter()
        .zip(&stay)
        .enumerate()
        .map(|(t, (mut row, &p))| {
            row.push((t, p));
            row
        })
        .collect();
    let guess = bicgstab(&rows, &seed, vec![0.0; rows.len()], &std::cell::OnceCell::new());
    let x = gauss_seidel(&rows, &seed, guess.clone()).unwrap_or(guess);
    let goals: Vec<(f64, usize)> =
        (0..order.len()).filter(|&k| model.is_goal(&solution.states[order[k]])).map(|k| (x[k], order[k])).collect();
    let total: f64 = goals.iter().map(|g| g.0).sum();
    goals.into_iter().map(|(p, s)| (p / total, solution.states[s].clone())).collect()
}
