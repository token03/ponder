use rustc_hash::FxHashSet;

use crate::{IndexedOutcomes, Model, Solution, components};

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub runs: usize,
    pub max_steps: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self { runs: 2_000, max_steps: 100_000 }
    }
}

#[derive(Debug)]
pub struct State {
    pub action: Option<usize>,
    pub cost: f64,
    pub outcomes: IndexedOutcomes,
    pub goal: bool,
    pub restart: bool,
    pub reachable: bool,
    pub visits: usize,
    pub runs_reaching: usize,
    pub residual: f64,
}

#[derive(Debug)]
pub struct Inspection {
    pub states: Vec<State>,
    pub loops: Vec<Vec<usize>>,
    pub runs: usize,
    pub finished: usize,
    pub mean_cost: f64,
    pub cost_stddev: f64,
    pub mean_steps: f64,
}

fn distribution(row: &[(f64, usize)]) -> Result<(), String> {
    let total: f64 = row.iter().map(|r| r.0).sum();
    if row.iter().any(|r| !r.0.is_finite() || r.0 < 0.0) || (total - 1.0).abs() > 1e-8 {
        return Err(format!("invalid probability distribution (total {total})"));
    }
    Ok(())
}

fn pick(row: &[(f64, usize)], unit: f64) -> usize {
    let mut remaining = unit * row.iter().map(|r| r.0).sum::<f64>();
    for &(p, state) in row {
        if remaining < p {
            return state;
        }
        remaining -= p;
    }
    row.iter().rfind(|r| r.0 > 0.0).unwrap().1
}

pub fn inspect<M: Model>(
    model: &M,
    solution: &Solution<M::State>,
    start: &[(f64, M::State)],
    options: Options,
    mut unit: impl FnMut() -> f64,
) -> Result<Inspection, String> {
    if options.runs == 0 || options.max_steps == 0 {
        return Err("inspection runs and step limit must be positive".into());
    }
    if solution.states.len() != solution.values.len() || solution.states.len() != solution.policy.len() {
        return Err("inconsistent solution arrays".into());
    }
    if !solution.cost.is_finite() {
        return Err("non-finite solution cost".into());
    }
    let lookup = |state: &M::State| {
        solution.index.get(state).copied().ok_or_else(|| "positive-probability successor missing from policy".to_string())
    };
    let starts: Vec<_> = start.iter().filter(|r| r.0 != 0.0).map(|(p, s)| Ok((*p, lookup(s)?))).collect::<Result<_, String>>()?;
    distribution(&starts)?;
    let given_up: FxHashSet<_> = solution.given_up.iter().collect();
    let mut states = Vec::with_capacity(solution.states.len());
    for (i, s) in solution.states.iter().enumerate() {
        let goal = model.is_goal(s);
        let restart = !goal && (model.is_restart(s) || given_up.contains(s));
        let mut state = State {
            action: solution.policy[i],
            cost: 0.0,
            outcomes: Box::new([]),
            goal,
            restart,
            reachable: false,
            visits: 0,
            runs_reaching: 0,
            residual: 0.0,
        };
        if let Some(action) = state.action.filter(|_| !goal && !restart) {
            state.cost =
                model.actions(s).into_iter().find(|a| a.0 == action).ok_or_else(|| format!("state {i}: selected action unavailable"))?.1;
            if !state.cost.is_finite() {
                return Err(format!("state {i}: non-finite action cost"));
            }
            let mut error = None;
            let mut outcomes = Vec::new();
            model.for_each_outcome(s, action, &mut |p, t| {
                if p == 0.0 {
                    return;
                }
                match lookup(&t) {
                    Ok(t) => outcomes.push((p, t)),
                    Err(e) => error = Some(e),
                }
            });
            if let Some(e) = error {
                return Err(format!("state {i}: {e}"));
            }
            distribution(&outcomes).map_err(|e| format!("state {i}: {e}"))?;
            state.residual = state.cost + outcomes.iter().map(|(p, t)| p * solution.values[*t]).sum::<f64>() - solution.values[i];
            state.outcomes = outcomes.into_boxed_slice();
        }
        states.push(state);
    }
    let mut todo: Vec<_> = starts.iter().map(|r| r.1).collect();
    while let Some(s) = todo.pop() {
        if std::mem::replace(&mut states[s].reachable, true) {
            continue;
        }
        if !states[s].goal && !states[s].restart && states[s].action.is_none() {
            return Err(format!("reachable state {s} has no policy action and is not terminal"));
        }
        if !solution.values[s].is_finite() {
            return Err(format!("reachable state {s} has non-finite value"));
        }
        todo.extend(states[s].outcomes.iter().map(|r| r.1));
        if states[s].restart {
            todo.extend(starts.iter().map(|r| r.1));
        }
    }
    let parts = components(states.len(), |s| states[s].outcomes.iter().map(|r| r.1));
    let mut loops: Vec<_> = parts
        .iter()
        .filter(|c| states[c[0]].reachable && (c.len() > 1 || states[c[0]].outcomes.iter().any(|r| r.1 == c[0])))
        .map(<[usize]>::to_vec)
        .collect();
    loops.sort_by_key(|c| std::cmp::Reverse(c.len()));
    let mut report = Inspection { states, loops, runs: options.runs, finished: 0, mean_cost: 0.0, cost_stddev: 0.0, mean_steps: 0.0 };
    let mut last_run = vec![usize::MAX; report.states.len()];
    let mut m2 = 0.0;
    let mut draw = || {
        let value = unit();
        if (0.0..1.0).contains(&value) { Ok(value) } else { Err("uniform source must return a value in [0, 1)".to_string()) }
    };
    for run in 0..options.runs {
        let mut s = pick(&starts, draw()?);
        let mut cost = 0.0;
        let mut steps = 0;
        loop {
            let state = &mut report.states[s];
            state.visits += 1;
            if last_run[s] != run {
                last_run[s] = run;
                state.runs_reaching += 1;
            }
            if state.goal {
                report.finished += 1;
                let delta = cost - report.mean_cost;
                report.mean_cost += delta / report.finished as f64;
                m2 += delta * (cost - report.mean_cost);
                report.mean_steps += (steps as f64 - report.mean_steps) / report.finished as f64;
                break;
            }
            if steps == options.max_steps {
                break;
            }
            steps += 1;
            cost += state.cost;
            s = pick(if state.restart { &starts } else { &state.outcomes }, draw()?);
        }
    }
    report.cost_stddev = if report.finished > 1 { (m2 / (report.finished - 1) as f64).sqrt() } else { 0.0 };
    Ok(report)
}
