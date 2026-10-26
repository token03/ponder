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

use std::collections::HashMap;
use std::hash::Hash;

pub trait Model {
    type State: Clone + Eq + Hash;
    fn is_goal(&self, s: &Self::State) -> bool;
    /// A state that starts over: it is worth what the start is worth.
    fn is_restart(&self, _: &Self::State) -> bool {
        false
    }
    /// Usable actions with their costs; ids are the model's own.
    fn actions(&self, s: &Self::State) -> Vec<(usize, f64)>;
    fn outcomes(&self, s: &Self::State, action: usize) -> Vec<(f64, Self::State)>;
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
    to: Vec<(f64, usize)>,
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
    let mut states: Vec<M::State> = Vec::new();
    let mut index: HashMap<M::State, usize> = HashMap::new();
    let mut intern = |s: M::State, states: &mut Vec<M::State>| -> usize {
        *index.entry(s.clone()).or_insert_with(|| {
            states.push(s);
            states.len() - 1
        })
    };
    let starts: Vec<(f64, usize)> = start.iter().map(|(p, s)| (*p, intern(s.clone(), &mut states))).collect();
    let mut edges: Vec<Vec<Edge>> = Vec::new();
    let mut end = Vec::new();
    let mut fixed: Vec<Option<Affine>> = Vec::new();
    let mut next = 0;
    while next < states.len() {
        if states.len() > limit {
            return Err(Error::TooManyStates(limit));
        }
        let s = states[next].clone();
        let value = if model.is_goal(&s) {
            Some(Affine { a: 0.0, b: 0.0 })
        } else if model.is_restart(&s) {
            Some(Affine { a: 0.0, b: 1.0 })
        } else {
            None
        };
        let mut out = Vec::new();
        if value.is_none() {
            for (action, cost) in model.actions(&s) {
                let to = model.outcomes(&s, action).into_iter().filter(|o| o.0 > 0.0).map(|(p, t)| (p, intern(t, &mut states))).collect();
                out.push(Edge { action, cost, to });
            }
        }
        end.push(value.is_some());
        fixed.push(value);
        edges.push(out);
        next += 1;
    }
    let n = states.len();

    // Keep only actions whose every outcome can still reach the goal (or a restart) and that
    // leave the state; a state is alive while some kept action path gets there.
    let mut alive = vec![true; n];
    loop {
        for s in 0..n {
            if !end[s] {
                edges[s].retain(|e| e.to.iter().all(|&(_, t)| alive[t]) && e.to.iter().any(|&(_, t)| t != s));
            }
        }
        let mut into: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (s, out) in edges.iter().enumerate() {
            for e in out {
                for &(_, t) in &e.to {
                    into[t].push(s);
                }
            }
        }
        let mut reach = end.clone();
        let mut stack: Vec<usize> = (0..n).filter(|&s| end[s]).collect();
        while let Some(t) = stack.pop() {
            for &s in &into[t] {
                if !reach[s] {
                    reach[s] = true;
                    stack.push(s);
                }
            }
        }
        if reach == alive {
            break;
        }
        alive = reach;
    }

    let order: Vec<Vec<usize>> =
        components(&edges).into_iter().map(|c| c.into_iter().filter(|&s| alive[s] && !end[s]).collect()).filter(|c: &Vec<usize>| !c.is_empty()).collect();
    let restarts = fixed.iter().any(|f| f.is_some_and(|f| f.b > 0.0));
    let mut choice = vec![usize::MAX; n];
    let mut affine: Vec<Affine> = fixed.iter().map(|f| f.unwrap_or(Affine::INFINITE)).collect();
    // Start high so restarting looks dear; Newton steps then fall monotonically to the fixed point.
    let mut w = if restarts { 1e12 } else { 0.0 };
    for _ in 0..60 {
        let mut values: Vec<f64> = affine.iter().map(|f| f.at(w)).collect();
        for component in &order {
            solve_component(component, &edges, &mut values, &mut choice, &mut affine, w);
        }
        let at_start = starts.iter().fold(Affine { a: 0.0, b: 0.0 }, |acc, &(p, s)| Affine { a: acc.a + p * affine[s].a, b: acc.b + p * affine[s].b });
        if !at_start.a.is_finite() {
            return Err(Error::Unreachable);
        }
        let next = if restarts {
            if at_start.b >= 1.0 - 1e-12 {
                return Err(Error::Unreachable);
            }
            at_start.a / (1.0 - at_start.b)
        } else {
            w
        };
        if (next - w).abs() <= 1e-10 * next.abs().max(1.0) {
            let values: Vec<f64> = affine.iter().map(|f| f.at(next)).collect();
            let policy = (0..n).map(|s| (choice[s] != usize::MAX && !end[s] && alive[s]).then(|| edges[s][choice[s]].action)).collect();
            return Ok(Solution { cost: at_start.at(next), states, values, policy, index });
        }
        w = next;
    }
    Err(Error::Unreachable)
}

fn q(e: &Edge, values: &[f64]) -> f64 {
    e.cost + e.to.iter().map(|&(p, t)| p * values[t]).sum::<f64>()
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
fn solve_component(states: &[usize], edges: &[Vec<Edge>], values: &mut [f64], choice: &mut [usize], affine: &mut [Affine], w: f64) {
    let local: HashMap<usize, usize> = states.iter().enumerate().map(|(i, &s)| (s, i)).collect();
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
                let exit = edges[s].iter().position(|e| e.to.iter().any(|&(_, t)| local.get(&t).is_none_or(|&j| ranked[j])));
                if let Some(a) = exit {
                    chosen[i] = a;
                    ranked[i] = true;
                    progress = true;
                }
            }
        }
        debug_assert!(ranked.iter().all(|r| *r), "pruning leaves every live state a way out");
    }
    for _ in 0..100 {
        let Some(v) = evaluate(states, &local, edges, &chosen, affine) else { break };
        for (i, &s) in states.iter().enumerate() {
            affine[s] = v[i];
            values[s] = v[i].at(w);
        }
        let improved: Vec<usize> = states
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let (v, a) = best(&edges[s], values);
                let current = q(&edges[s][chosen[i]], values);
                if v < current - 1e-10 * current.abs().max(1.0) { a } else { chosen[i] }
            })
            .collect();
        if improved == chosen {
            break;
        }
        chosen = improved;
    }
    for (i, &s) in states.iter().enumerate() {
        choice[s] = chosen[i];
    }
}

/// Exact values of a fixed policy inside a component, as `a + b w`: solve (I - P) x = r for the
/// cost part (r = c + P_out a_out) and the restart part (r = P_out b_out) together.
fn evaluate(states: &[usize], local: &HashMap<usize, usize>, edges: &[Vec<Edge>], choice: &[usize], affine: &[Affine]) -> Option<Vec<Affine>> {
    let n = states.len();
    let mut m = vec![0.0; n * n];
    let mut ra = vec![0.0; n];
    let mut rb = vec![0.0; n];
    for (i, &s) in states.iter().enumerate() {
        let e = &edges[s][choice[i]];
        m[i * n + i] += 1.0;
        ra[i] = e.cost;
        for &(p, t) in &e.to {
            match local.get(&t) {
                Some(&j) => m[i * n + j] -= p,
                None => {
                    ra[i] += p * affine[t].a;
                    rb[i] += p * affine[t].b;
                }
            }
        }
    }
    let x = lu_solve(n, &mut m, &mut [&mut ra, &mut rb])?;
    let out: Vec<Affine> = (0..n).map(|i| Affine { a: x[0][i], b: x[1][i] }).collect();
    out.iter().all(|f| f.a.is_finite() && f.a >= 0.0 && f.b.is_finite() && f.b >= -1e-9).then_some(out)
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

/// Strongly connected components, successors before predecessors (Tarjan, iterative).
fn components(edges: &[Vec<Edge>]) -> Vec<Vec<usize>> {
    let n = edges.len();
    let succ: Vec<Vec<usize>> = edges.iter().map(|es| es.iter().flat_map(|e| e.to.iter().map(|o| o.1)).collect()).collect();
    let (mut index, mut low, mut on) = (vec![usize::MAX; n], vec![0; n], vec![false; n]);
    let (mut stack, mut out, mut counter) = (Vec::new(), Vec::new(), 0);
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut call = vec![(root, 0usize)];
        index[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on[root] = true;
        while let Some(&mut (v, ref mut i)) = call.last_mut() {
            if let Some(&w) = succ[v].get(*i) {
                *i += 1;
                if index[w] == usize::MAX {
                    index[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    on[w] = true;
                    call.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                call.pop();
                if let Some(&(parent, _)) = call.last() {
                    low[parent] = low[parent].min(low[v]);
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
