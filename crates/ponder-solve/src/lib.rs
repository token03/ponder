//! Cheapest expected cost to reach a goal: stochastic shortest paths over enumerated states
//! (SPEC §6). Knows nothing about Path of Exile.
//!
//! [`solve`] enumerates the states reachable from the start, drops actions that can end where
//! the goal is unreachable, then solves strongly connected components from the goal backwards
//! (topological value iteration). Inside a component, policy iteration evaluates each policy
//! exactly, so long retry loops converge in a few rounds instead of thousands of sweeps.

use std::collections::HashMap;
use std::hash::Hash;

pub trait Model {
    type State: Clone + Eq + Hash;
    fn is_goal(&self, s: &Self::State) -> bool;
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
    /// Chosen action per state; `None` for goals and dead ends.
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
    let mut goal = Vec::new();
    let mut next = 0;
    while next < states.len() {
        if states.len() > limit {
            return Err(Error::TooManyStates(limit));
        }
        let s = states[next].clone();
        let is_goal = model.is_goal(&s);
        let mut out = Vec::new();
        if !is_goal {
            for (action, cost) in model.actions(&s) {
                let to = model.outcomes(&s, action).into_iter().filter(|o| o.0 > 0.0).map(|(p, t)| (p, intern(t, &mut states))).collect();
                out.push(Edge { action, cost, to });
            }
        }
        goal.push(is_goal);
        edges.push(out);
        next += 1;
    }
    let n = states.len();

    // Keep only actions whose every outcome can still reach the goal and that leave the state.
    let mut alive = vec![true; n];
    loop {
        let mut changed = false;
        for s in 0..n {
            if goal[s] || !alive[s] {
                continue;
            }
            edges[s].retain(|e| e.to.iter().all(|&(_, t)| alive[t]) && e.to.iter().any(|&(_, t)| t != s));
            if edges[s].is_empty() {
                alive[s] = false;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut values = vec![f64::INFINITY; n];
    let mut policy = vec![None; n];
    for s in 0..n {
        if goal[s] {
            values[s] = 0.0;
        }
    }
    for component in components(&edges) {
        let live: Vec<usize> = component.into_iter().filter(|&s| alive[s] && !goal[s]).collect();
        if !live.is_empty() {
            solve_component(&live, &edges, &mut values, &mut policy);
        }
    }
    let mut cost = 0.0;
    for &(p, s) in &starts {
        cost += p * values[s];
    }
    if !cost.is_finite() {
        return Err(Error::Unreachable);
    }
    Ok(Solution { cost, states, values, policy, index })
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
/// Policy iteration from a proper policy: one that leaves the component with certainty. Each
/// state's first action is one that can step closer to the exit, so every state can reach the
/// exit with positive probability. With positive costs, improvement keeps policies proper.
fn solve_component(states: &[usize], edges: &[Vec<Edge>], values: &mut [f64], policy: &mut [Option<usize>]) {
    let local: HashMap<usize, usize> = states.iter().enumerate().map(|(i, &s)| (s, i)).collect();
    let mut choice = vec![usize::MAX; states.len()];
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
                choice[i] = a;
                ranked[i] = true;
                progress = true;
            }
        }
    }
    debug_assert!(ranked.iter().all(|r| *r), "pruning leaves every live state a way out");
    for _ in 0..100 {
        let Some(v) = evaluate(states, &local, edges, &choice, values) else { break };
        for (i, &s) in states.iter().enumerate() {
            values[s] = v[i];
        }
        let improved: Vec<usize> = states
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let (v, a) = best(&edges[s], values);
                let current = q(&edges[s][choice[i]], values);
                if v < current - 1e-10 * current.abs().max(1.0) { a } else { choice[i] }
            })
            .collect();
        if improved == choice {
            break;
        }
        choice = improved;
    }
    for (i, &s) in states.iter().enumerate() {
        policy[s] = Some(edges[s][choice[i]].action);
    }
}

/// Exact values of a fixed policy inside a component: solve (I - P) v = c + P_out v_out.
fn evaluate(states: &[usize], local: &HashMap<usize, usize>, edges: &[Vec<Edge>], choice: &[usize], values: &[f64]) -> Option<Vec<f64>> {
    let n = states.len();
    let mut a = vec![0.0; n * n];
    let mut b = vec![0.0; n];
    for (i, &s) in states.iter().enumerate() {
        let e = &edges[s][choice[i]];
        a[i * n + i] += 1.0;
        b[i] = e.cost;
        for &(p, t) in &e.to {
            match local.get(&t) {
                Some(&j) => a[i * n + j] -= p,
                None => b[i] += p * values[t],
            }
        }
    }
    let v = lu_solve(n, &mut a, &mut b)?;
    v.iter().all(|x| x.is_finite() && *x >= 0.0).then_some(v)
}

/// Gaussian elimination with partial pivoting; `None` when singular.
fn lu_solve(n: usize, a: &mut [f64], b: &mut [f64]) -> Option<Vec<f64>> {
    for k in 0..n {
        let p = (k..n).max_by(|&i, &j| a[i * n + k].abs().total_cmp(&a[j * n + k].abs()))?;
        if a[p * n + k].abs() < 1e-14 {
            return None;
        }
        if p != k {
            for c in 0..n {
                a.swap(k * n + c, p * n + c);
            }
            b.swap(k, p);
        }
        for i in k + 1..n {
            let f = a[i * n + k] / a[k * n + k];
            if f != 0.0 {
                for c in k..n {
                    a[i * n + c] -= f * a[k * n + c];
                }
                b[i] -= f * b[k];
            }
        }
    }
    let mut x = vec![0.0; n];
    for k in (0..n).rev() {
        let s: f64 = (k + 1..n).map(|c| a[k * n + c] * x[c]).sum();
        x[k] = (b[k] - s) / a[k * n + k];
    }
    Some(x)
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
