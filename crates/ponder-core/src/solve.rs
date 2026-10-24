//! Cheapest policy by policy iteration, evaluating each policy exactly.

use std::collections::HashMap;
use std::fmt;

use faer::Mat;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::linear::{self, Backend, Lu, System};
use crate::problem::{ActionC, Problem, Res};
use crate::types::*;

pub type StateMap<V> = FxHashMap<State, V>;

/// Seconds per use, including reading the result.
pub const SECONDS: [(&str, f64); 7] =
    [("transmute", 1.0), ("alt", 1.0), ("aug", 1.0), ("regal", 2.0), ("exalt", 2.0), ("annul", 2.0), ("reroll_magic", 3.0)];
pub const HARVEST_SECONDS: f64 = 2.0; // one click, like a chaos orb
pub const FOSSIL_SECONDS: f64 = 2.0; // socketing each fossil into the resonator
pub const LOCK_SECONDS: f64 = 5.0; // a bench metamod, on top of the reroll after it
pub const SELL_SECONDS: f64 = 90.0; // listing a bricked item and trading it

pub const FRACTURE_ORB: &str = "fracturing-orb";
pub const FRACTURE_MIN_MODS: usize = 4; // a Fracturing Orb needs a rare with at least this many mods
pub const FRACTURE_SECONDS: f64 = 3.0;

#[derive(Debug, Clone, PartialEq)]
pub enum SolveError {
    /// No sequence of the available currencies reaches the target.
    Unreachable,
    /// The policy's linear system is singular or its values are nonsense (a sale worth more than making the item again).
    Singular(String),
    /// `evaluate` was given a policy with no action for a state.
    MissingPolicy(String),
}

impl fmt::Display for SolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolveError::Unreachable => f.write_str("no sequence of the available currencies can reach this target"),
            SolveError::Singular(m) | SolveError::MissingPolicy(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for SolveError {}

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

pub fn seconds(a: &Action) -> f64 {
    if a.kind == "sell" {
        return SELL_SECONDS;
    }
    if let Some((_, t)) = SECONDS.iter().find(|(k, _)| *k == a.kind) {
        return *t;
    }
    if a.kind.starts_with("keep_") || a.kind.starts_with("lockreroll_") {
        let reroll = a.name.split_once('+').map_or("", |x| x.1);
        return LOCK_SECONDS + if reroll == "scour" { 1.0 } else { reroll_seconds(reroll, &a.cost) };
    }
    if a.kind == "reroll_rare" {
        return reroll_seconds(&a.name, &a.cost);
    }
    2.0 // eldritch currency
}

fn reroll_seconds(name: &str, cost: &[(String, f64)]) -> f64 {
    if name.starts_with("harvest") {
        return HARVEST_SECONDS;
    }
    let mut t = 2.0 + if name.starts_with("scour+") { 1.0 } else { 0.0 };
    if name.contains("fossil") {
        t += FOSSIL_SECONDS * cost.iter().filter(|(pid, _)| pid.ends_with("-fossil")).count() as f64;
    }
    t
}

/// Chaos of currency one use takes (less what a sale brings back).
pub fn action_cost(a: &Action, prices: &Prices) -> f64 {
    a.cost.iter().map(|(pid, qty)| prices.get(pid).copied().unwrap_or(0.0) * qty).sum::<f64>() - a.credit
}

/// Currency plus the clicking time, in chaos: what the solver minimises.
pub fn step_cost(a: &Action, prices: &Prices, settings: &Settings) -> f64 {
    action_cost(a, prices) + seconds(a) * settings.chaos_per_hour / 3600.0
}

/// Rerolls with the same outcomes from the same rarities: only the cheapest can be optimal.
fn cheapest_of_equals(acts: Vec<Action>, problem: &Problem, prices: &Prices, settings: &Settings) -> Vec<Action> {
    let keys: Vec<_> = acts.iter().map(|a| problem.roll_key(a).map(|k| (k, a.from_rarity.clone()))).collect();
    let mut best: HashMap<_, usize> = HashMap::new();
    for (i, key) in keys.iter().enumerate() {
        let Some(key) = key else { continue };
        match best.get(key) {
            Some(&b) if step_cost(&acts[i], prices, settings) >= step_cost(&acts[b], prices, settings) => {}
            _ => {
                best.insert(key.clone(), i);
            }
        }
    }
    let keep: Vec<bool> = keys.iter().enumerate().map(|(i, k)| k.as_ref().is_none_or(|k| best[k] == i)).collect();
    // Plain actions first, then the chosen rerolls, as Python orders them.
    let (plain, rolls): (Vec<_>, Vec<_>) =
        acts.into_iter().zip(keys).zip(keep).filter(|(_, k)| *k).map(|(x, _)| x).partition(|(_, k)| k.is_none());
    plain.into_iter().chain(rolls).map(|(a, _)| a).collect()
}

// -- the explored graph -------------------------------------------------------

/// The explored states as flat arrays.
///
/// Open (unfinished) states are numbered 0..n-1. Every (state, action) pair is
/// a row; pairs of one state are contiguous, in action order. A pair's
/// outcomes are a *list* of entries (next state, probability), with next = -1
/// for a finished item, whose remaining cost is 0. Rerolls give every state
/// the same list, so lists are stored once and pairs point at them: a
/// reroll's expected cost to finish is computed once, not once per state.
pub(crate) struct Graph {
    pub states: Vec<State>, // open states, in the order they were first acted on
    pub seen: usize,        // every state found, finished ones too
    state_ptr: Vec<usize>,
    pair_action: Vec<usize>,
    pair_cost: Vec<f64>,
    pair_list: Vec<usize>,
    list_ptr: Vec<usize>, // list k's entries: list_ptr[k]..list_ptr[k+1]
    e_next: Vec<i32>,
    e_prob: Vec<f64>,
}

/// Grows the entry arrays while states are discovered.
struct Builder {
    seen: FxHashMap<State, u32>,
    by_id: Vec<State>,
    frontier: Vec<u32>,
    list_ptr: Vec<usize>,
    e_id: Vec<u32>,
    e_prob: Vec<f64>,
}

impl Builder {
    fn push_list(&mut self, out: &[Outcome]) -> usize {
        for &(p, n) in out {
            let next_id = self.by_id.len() as u32;
            let id = *self.seen.entry(n).or_insert(next_id);
            if id == next_id {
                self.by_id.push(n);
                self.frontier.push(id);
            }
            self.e_id.push(id);
            self.e_prob.push(p);
        }
        self.list_ptr.push(self.e_id.len());
        self.list_ptr.len() - 2
    }
}

/// Every state reachable from the origin, with each action's outcomes.
pub(crate) fn explore(problem: &Problem, costs: &[f64]) -> Graph {
    let acts: Vec<ActionC> = problem.actions.iter().map(|a| problem.prepare(a)).collect();
    let origin = problem.origin();
    let mut b = Builder {
        seen: FxHashMap::default(),
        by_id: vec![origin],
        frontier: vec![0],
        list_ptr: vec![0],
        e_id: Vec::new(),
        e_prob: Vec::new(),
    };
    b.seen.insert(origin, 0);
    let mut open: Vec<u32> = Vec::new();
    let (mut state_ptr, mut pair_action, mut pair_cost, mut pair_list) = (vec![0], Vec::new(), Vec::new(), Vec::new());
    // Rerolls share one outcome list across many states: scan each once.
    let mut shared: FxHashMap<usize, usize> = FxHashMap::default();
    let mut held = Vec::new();
    let mut scratch = Vec::new();
    while let Some(id) = b.frontier.pop() {
        let s = b.by_id[id as usize];
        if problem.done(&s) {
            continue;
        }
        open.push(id);
        for (ai, a) in acts.iter().enumerate() {
            let list = match problem.outcomes_into(&s, a, &mut scratch) {
                Res::Empty => continue,
                Res::Fresh => b.push_list(&scratch),
                Res::Shared(arc) => match shared.get(&(std::sync::Arc::as_ptr(&arc) as usize)) {
                    Some(&l) => l,
                    None => {
                        let l = b.push_list(&arc);
                        shared.insert(std::sync::Arc::as_ptr(&arc) as usize, l);
                        held.push(arc); // keeps the address from being reused
                        l
                    }
                },
            };
            pair_action.push(ai);
            pair_cost.push(costs[ai]);
            pair_list.push(list);
        }
        state_ptr.push(pair_list.len());
    }
    let mut index = vec![-1i32; b.by_id.len()];
    for (k, &id) in open.iter().enumerate() {
        index[id as usize] = k as i32;
    }
    Graph {
        states: open.iter().map(|&id| b.by_id[id as usize]).collect(),
        seen: b.by_id.len(),
        state_ptr,
        pair_action,
        pair_cost,
        pair_list,
        list_ptr: b.list_ptr,
        e_next: b.e_id.iter().map(|&id| index[id as usize]).collect(),
        e_prob: b.e_prob,
    }
}

impl Graph {
    pub fn n(&self) -> usize {
        self.states.len()
    }

    fn lists(&self) -> usize {
        self.list_ptr.len() - 1
    }

    pub fn pair_of(&self, i: usize, actions: &[Action], name: &str) -> Option<usize> {
        (self.state_ptr[i]..self.state_ptr[i + 1]).find(|&p| actions[self.pair_action[p]].name == name)
    }

    /// Expected cost to finish after taking each pair's action.
    fn q(&self, value: &[f64]) -> Vec<f64> {
        let expected: Vec<f64> = (0..self.lists())
            .map(|l| {
                (self.list_ptr[l]..self.list_ptr[l + 1])
                    .map(|e| match self.e_next[e] {
                        n if n >= 0 => self.e_prob[e] * value[n as usize],
                        _ => 0.0,
                    })
                    .sum()
            })
            .collect();
        self.pair_cost.iter().zip(&self.pair_list).map(|(c, &l)| c + expected[l]).collect()
    }

    /// Each state's cheapest pair (first in action order on ties), kept unless clearly better.
    fn improve(&self, q: &[f64], policy: &[usize]) -> Vec<usize> {
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
                if best < current - 1e-9 * current.max(1.0) { candidate } else { policy[i] }
            })
            .collect()
    }

    /// A starting policy that surely finishes: always step toward fewer moves-to-done.
    ///
    /// Policy iteration needs one; "always reroll" isn't enough when no single
    /// reroll can hit every target (e.g. three notables need alt, regal, exalt).
    /// Returns None for states that can't finish.
    fn proper_policy(&self) -> Vec<Option<usize>> {
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

    /// I - P for the policy, factored. None if the matrix is singular.
    fn factor(&self, policy: &[usize], backend: Backend) -> Option<Lu> {
        let rows = policy.iter().map(|&p| self.pair_list[p]).collect();
        linear::factor(&System { rows, list_ptr: &self.list_ptr, e_next: &self.e_next, e_prob: &self.e_prob }, backend)
    }

    fn evaluate(&self, lu: &Lu, policy: &[usize]) -> Vec<f64> {
        let mut rhs = Mat::<f64>::from_fn(self.n(), 1, |i, _| self.pair_cost[policy[i]]);
        lu.solve(&mut rhs);
        (0..self.n()).map(|i| rhs[(i, 0)]).collect()
    }

    /// Expected times each state is acted on, starting from state `start`.
    fn visits(&self, lu: &Lu, start: usize) -> Vec<f64> {
        let mut rhs = Mat::<f64>::from_fn(self.n(), 1, |i, _| if i == start { 1.0 } else { 0.0 });
        lu.solve_transpose(&mut rhs);
        (0..self.n()).map(|i| rhs[(i, 0)]).collect()
    }
}

fn index_of(g: &Graph, s: &State) -> usize {
    g.states.iter().position(|x| x == s).expect("origin is an open state")
}

/// Final values, the policy they belong to, and its factorisation if still current.
type Improved = (Vec<f64>, Vec<usize>, Option<Lu>);

/// Optimal policy by policy iteration, evaluating each policy exactly.
///
/// Actions with any unpriced input are left out rather than treated as free. Like Python,
/// this rewrites `problem.actions` (unpriced and dominated rerolls dropped, and sales if they
/// would pay for themselves), and `Plan` and `steps` refer to the remaining ones.
pub fn solve(problem: &mut Problem, prices: &Prices, settings: &Settings) -> Result<Plan, SolveError> {
    solve_rounds(problem, prices, settings, 60)
}

pub fn solve_rounds(problem: &mut Problem, prices: &Prices, settings: &Settings, max_rounds: usize) -> Result<Plan, SolveError> {
    solve_with(problem, prices, settings, max_rounds, Backend::Auto)
}

/// `solve` with a choice of linear solver (the answers agree; the dense one is the reference).
pub fn solve_with(
    problem: &mut Problem,
    prices: &Prices,
    settings: &Settings,
    max_rounds: usize,
    backend: Backend,
) -> Result<Plan, SolveError> {
    let unpriced: Vec<String> =
        problem.actions.iter().filter(|a| a.cost.iter().any(|(pid, _)| !prices.contains_key(pid))).map(|a| a.name.clone()).collect();
    let kept: Vec<Action> = std::mem::take(&mut problem.actions).into_iter().filter(|a| !unpriced.contains(&a.name)).collect();
    problem.actions = cheapest_of_equals(kept, problem, prices, settings);
    problem.clear_lock_cache();
    let costs: Vec<f64> = problem.actions.iter().map(|a| step_cost(a, prices, settings)).collect();
    if problem.done(&problem.origin()) {
        return Ok(Plan { states: 1, unpriced, ..Plan::default() });
    }
    let g = explore(problem, &costs);
    let mut policy: Vec<usize> = g.proper_policy().into_iter().collect::<Option<_>>().ok_or(SolveError::Unreachable)?;
    let start = index_of(&g, &problem.origin());

    let run = || -> Result<Improved, SolveError> {
        let mut policy = policy.clone();
        let (mut value, mut lu) = (Vec::new(), None);
        for _ in 0..max_rounds {
            let singular = || SolveError::Singular("singular matrix: a sale is worth more than making it again".into());
            let f = g.factor(&policy, backend).ok_or_else(singular)?;
            value = g.evaluate(&f, &policy);
            if !value.iter().all(|v| v.is_finite()) {
                return Err(SolveError::Singular("singular matrix: a sale is worth more than making it again".into()));
            }
            let better = g.improve(&g.q(&value), &policy);
            if better == policy {
                lu = Some(f);
                break;
            }
            policy = better;
        }
        if value.is_empty() || value[start] < 0.0 {
            return Err(SolveError::Singular("a sale is worth more than making it again".into()));
        }
        Ok((value, policy, lu))
    };
    let (value, final_policy, lu) = match run() {
        Ok(x) => x,
        Err(e) => {
            if !problem.actions.iter().any(|a| a.kind == "sell") {
                return Err(e);
            }
            // Selling sub-products would pay for itself (their values are rough): plan without sales.
            problem.actions.retain(|a| a.kind != "sell");
            return solve_with(problem, prices, settings, max_rounds, backend);
        }
    };
    policy = final_policy;
    let lu = match lu {
        Some(lu) => lu,
        None => g.factor(&policy, backend).ok_or_else(|| SolveError::Singular("singular matrix".into()))?,
    };
    let visits = g.visits(&lu, start);
    let mut usage: HashMap<String, f64> = problem.actions.iter().map(|a| (a.name.clone(), 0.0)).collect();
    for (i, &p) in policy.iter().enumerate() {
        *usage.get_mut(&problem.actions[g.pair_action[p]].name).unwrap() += visits[i];
    }
    let hours =
        problem.actions.iter().filter_map(|a| Some(usage[&a.name]).filter(|n| *n != 0.0).map(|n| n * seconds(a))).sum::<f64>() / 3600.0;
    let name = |p: usize| problem.actions[g.pair_action[p]].name.clone();
    Ok(Plan {
        cost: value[start],
        policy: g.states.iter().enumerate().map(|(i, s)| (*s, name(policy[i]))).collect(),
        usage,
        visits: g.states.iter().enumerate().map(|(i, s)| (*s, visits[i])).collect(),
        states: g.seen,
        unpriced,
        hours,
        values: g.states.iter().enumerate().map(|(i, s)| (*s, value[i])).collect(),
        order: g.states,
    })
}

/// Exact expected cost of following `policy` (state -> action name) from the origin.
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
    if problem.done(&problem.origin()) {
        return Ok(0.0);
    }
    let costs: Vec<f64> = problem.actions.iter().map(|a| step_cost(a, prices, settings)).collect();
    let g = explore(problem, &costs);
    let chosen = g
        .states
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let name = policy.get(s).ok_or_else(|| SolveError::MissingPolicy(format!("policy has no action for {s:?}")))?;
            g.pair_of(i, &problem.actions, name).ok_or_else(|| SolveError::MissingPolicy(format!("{name:?} is not available in {s:?}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let lu = g.factor(&chosen, backend).ok_or_else(|| SolveError::Singular("singular matrix".into()))?;
    let value = g.evaluate(&lu, &chosen);
    Ok(value[index_of(&g, &problem.origin())])
}

// -- fracturing your own base ---------------------------------------------------

/// Make your own fractured base: reroll until the mod lands with few enough mods, then fracture.
///
/// A Fracturing Orb fixes one random mod of `k`, so it hits the wanted one
/// 1 time in k; a miss ruins the base and the loop starts over on a new one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelfFracture {
    pub target: usize,        // index into the problem's targets
    pub action: String,       // the reroll to spam
    pub max_mods: usize,      // fracture only when the item has at most this many mods
    pub hit: f64,             // chance one reroll lands the mod with few enough mods
    pub fracture_chance: f64, // chance the orb then picks it
    pub cost: f64,            // expected chaos (currency and clicking) to a base with the mod fractured
    pub hours: f64,
}

impl SelfFracture {
    pub fn rerolls(&self) -> f64 {
        1.0 / (self.hit * self.fracture_chance)
    }

    pub fn orbs(&self) -> f64 {
        1.0 / self.fracture_chance
    }
}

/// The cheapest way to roll and fracture `problem.targets[target]` yourself (gear only).
///
/// Each rare reroll (chaos, scour+alch, essences, fossils, Harvest) gives,
/// from its exact roll distribution, the chance P_k of the mod landing at a
/// good tier on an item with k mods. Fracturing only at k <= K costs
/// (reroll / P(k <= K) + orb) / E[1/k | k <= K]; the best reroll and K win.
pub fn self_fracture(problem: &Problem, target: usize, prices: &Prices, settings: &Settings) -> Option<SelfFracture> {
    if problem.kind != Kind::Gear || !problem.fixed.is_empty() || !prices.contains_key(FRACTURE_ORB) {
        return None;
    }
    let orb = prices[FRACTURE_ORB] + FRACTURE_SECONDS * settings.chaos_per_hour / 3600.0;
    let mut best: Option<SelfFracture> = None;
    for a in &problem.actions {
        if a.kind != "reroll_rare" || !a.from_rarity.contains(&RARE) || a.cost.iter().any(|(pid, _)| !prices.contains_key(pid)) {
            continue;
        }
        let mut by_k: Vec<(usize, f64)> = Vec::new();
        for &(p, s) in problem.rolled(a)?.iter() {
            let k = problem.total(&s);
            if s.status[target] == GOOD && k >= FRACTURE_MIN_MODS {
                match by_k.iter_mut().find(|x| x.0 == k) {
                    Some(x) => x.1 += p,
                    None => by_k.push((k, p)),
                }
            }
        }
        let step = step_cost(a, prices, settings);
        let mut ks: Vec<usize> = by_k.iter().map(|x| x.0).collect();
        ks.sort_unstable();
        let mut hit = 0.0;
        for k in ks {
            hit += by_k.iter().find(|x| x.0 == k).unwrap().1;
            let chance = by_k.iter().filter(|x| x.0 <= k).map(|&(j, p)| p / j as f64).sum::<f64>() / hit;
            let cost = (step / hit + orb) / chance;
            if best.as_ref().is_none_or(|b| cost < b.cost) {
                let hours = (seconds(a) / hit + FRACTURE_SECONDS) / chance / 3600.0;
                best = Some(SelfFracture { target, action: a.name.clone(), max_mods: k, hit, fracture_chance: chance, cost, hours });
            }
        }
    }
    best
}
