//! Solving: one problem through the plan cache, or all of them in parallel with rayon.

use std::sync::atomic::{AtomicUsize, Ordering};

use ponder_core::{self as craft, Plan, Prices, Problem};
use rayon::prelude::*;

use super::cache::plan_key;
use super::context::{Context, Progress};
use super::finish::Finish;
use super::types::{Estimate, Fracture};

pub fn continuation(
    cx: &Context,
    problem: &Problem,
    initial: std::sync::Arc<craft::Distribution>,
    known: Option<&Plan>,
    table: &Prices,
) -> Result<Plan, craft::SolveError> {
    let raw = serde_json::to_vec(&(initial.to_vec(), initial.bounds())).expect("continuation key serialises");
    let key = format!("{}-{}", plan_key(problem, table, &cx.settings), blake3::hash(&raw).to_hex());
    cx.plans.get_or_solve(&key, || {
        let problem = problem.fresh_transition_cache();
        let followed = known.map(|plan| craft::follow_from(&problem, initial.clone(), &plan.policy, table, &cx.settings));
        match followed {
            Some(Ok(plan)) => Ok(plan),
            _ => craft::solve_from(&problem, initial, table, &cx.settings),
        }
    })
}

pub fn acquired_continuation(
    cx: &Context,
    problem: &Problem,
    initial: std::sync::Arc<craft::Distribution>,
    preserved: craft::Preservation,
    recovery: craft::Recovery,
    known: Option<&Plan>,
    table: &Prices,
) -> Result<Plan, craft::SolveError> {
    let usage: std::collections::BTreeMap<_, _> = recovery.usage.iter().collect();
    let raw = serde_json::to_vec(&(
        initial.to_vec(),
        initial.bounds(),
        preserved.targets,
        preserved.filler,
        recovery.cost,
        recovery.hours,
        usage,
        recovery.initial.to_vec(),
        recovery.initial.bounds(),
        &recovery.sale,
    ))
    .expect("acquisition key serialises");
    let key = format!("{}-acquisition-{}", plan_key(problem, table, &cx.settings), blake3::hash(&raw).to_hex());
    cx.plans.get_or_solve(&key, || {
        let private = problem.fresh_transition_cache();
        let direct = known
            .filter(|plan| initial.iter().all(|(_, state)| plan.to_finish(problem, state).is_some()))
            .and_then(|plan| craft::follow_from(&private, initial.clone(), &plan.policy, table, &cx.settings).ok());
        let staged = craft::solve_after_acquisition(&private, initial, preserved, recovery, table, &cx.settings);
        let plan = match (staged, direct) {
            (Ok(staged), Some(direct)) if direct.cost < staged.cost => direct,
            (Ok(staged), _) => staged,
            (Err(error @ craft::SolveError::InvalidRecovery(_)), _) => return Err(error),
            (Err(_), Some(direct)) => direct,
            (Err(error), None) => return Err(error),
        };
        Ok(plan)
    })
}

/// The three fields solving fills in, borrowed from an `Estimate`, a `Fracture` or a `Finish`.
pub struct SolveSlot<'a> {
    pub problem: &'a mut Option<Problem>,
    pub plan: &'a mut Option<Plan>,
    pub error: &'a mut Option<String>,
}

/// Something with a problem to solve: `Estimate`, `Fracture`, `Finish`, recombinator donors.
pub trait Solvable: Send {
    fn slot(&mut self) -> SolveSlot<'_>;
}

impl Solvable for Estimate {
    fn slot(&mut self) -> SolveSlot<'_> {
        SolveSlot { problem: &mut self.problem, plan: &mut self.plan, error: &mut self.error }
    }
}

impl Solvable for Fracture {
    fn slot(&mut self) -> SolveSlot<'_> {
        SolveSlot { problem: &mut self.problem, plan: &mut self.plan, error: &mut self.error }
    }
}

impl Solvable for Finish {
    fn slot(&mut self) -> SolveSlot<'_> {
        SolveSlot { problem: &mut self.problem, plan: &mut self.plan, error: &mut self.error }
    }
}

/// Every estimate in `which` (indices) and its fractures, as slots.
pub fn slots_of<'a>(estimates: &'a mut [Estimate], which: &[usize]) -> Vec<SolveSlot<'a>> {
    let wanted: std::collections::HashSet<usize> = which.iter().copied().collect();
    let mut out = Vec::new();
    for (i, e) in estimates.iter_mut().enumerate() {
        if wanted.contains(&i) {
            let Estimate { problem, plan, error, fractures, .. } = e;
            out.push(SolveSlot { problem, plan, error });
            out.extend(fractures.iter_mut().map(|f| SolveSlot { problem: &mut f.problem, plan: &mut f.plan, error: &mut f.error }));
        }
    }
    out
}

/// Solve one slot in place (from the plan cache when it has it).
fn solve_slot(cx: &Context, slot: &mut SolveSlot, table: &Prices, key: Option<String>) {
    let Some(problem) = slot.problem.as_ref() else {
        return;
    };
    let key = key.unwrap_or_else(|| plan_key(problem, table, &cx.settings));
    match cx.plans.get_or_solve(&key, || {
        let problem = problem.fresh_transition_cache();
        craft::solve(&problem, table, &cx.settings)
    }) {
        Ok(plan) => *slot.plan = Some(plan),
        Err(error) => *slot.error = Some(error.to_string()),
    }
}

/// Solve an Estimate, Fracture or Finish in place (from the plan cache when it has it).
pub fn solve_one(cx: &Context, item: &mut dyn Solvable, table: &Prices) {
    solve_slot(cx, &mut item.slot(), table, None);
}

pub fn load_estimates(cx: &Context, estimates: &mut [Estimate], table: &Prices) {
    estimates.par_iter_mut().for_each(|estimate| {
        for mut slot in slots_of(std::slice::from_mut(estimate), &[0]) {
            load_slot(cx, &mut slot, table);
        }
    });
}

fn load_slot(cx: &Context, slot: &mut SolveSlot<'_>, table: &Prices) -> Option<String> {
    let problem = slot.problem.as_ref()?;
    if slot.plan.is_some() || slot.error.is_some() {
        return None;
    }
    let key = plan_key(problem, table, &cx.settings);
    if let Some(plan) = cx.plans.get(&key) {
        *slot.plan = Some(plan);
        None
    } else if let Some(error) = cx.plans.failure(&key) {
        *slot.error = Some(error.to_string());
        None
    } else {
        Some(key)
    }
}

fn solve_below(cx: &Context, slot: SolveSlot<'_>, table: &Prices, lower_bound: &mut Option<f64>, ceiling: Option<f64>) -> bool {
    let Some(problem) = slot.problem.as_ref() else { return false };
    if slot.plan.is_some() {
        *lower_bound = None;
        return false;
    }
    if slot.error.is_some() {
        return false;
    }
    let key = plan_key(problem, table, &cx.settings);
    let ceiling = ceiling.filter(|ceiling| ceiling.is_finite());
    if let Some(ceiling) = ceiling {
        let floor = cx
            .plans
            .lower_bound(&key)
            .or_else(|| problem.actions.iter().all(|action| craft::step_cost(action, table, &cx.settings) >= 0.0).then_some(0.0));
        if let Some(floor) = floor
            && floor > ceiling + 1e-9 * ceiling.abs().max(1.0)
        {
            *lower_bound = Some(floor);
            return false;
        }
    }
    let mut searched = false;
    let result = cx.plans.get_or_solve(&key, || {
        searched = true;
        let problem = problem.fresh_transition_cache();
        craft::solve_with(&problem, table, &cx.settings, craft::SolveOptions { cost_ceiling: ceiling, ..Default::default() })
    });
    match result {
        Ok(plan) => {
            *slot.plan = Some(plan);
            *lower_bound = None;
        }
        Err(craft::SolveError::Dominated { lower_bound: bound, .. }) => {
            cx.plans.put_lower_bound(&key, bound);
            *lower_bound = Some(bound);
        }
        Err(error) => *slot.error = Some(error.to_string()),
    }
    searched
}

pub fn solve_estimates(cx: &Context, estimates: &mut [Estimate], table: &Prices, workers: Option<usize>, progress: Option<Progress>) {
    let mut ordered: Vec<_> = estimates
        .iter_mut()
        .filter(|estimate| {
            (estimate.plan.is_none() && estimate.error.is_none())
                || estimate.fractures.iter().any(|fracture| fracture.plan.is_none() && fracture.error.is_none())
        })
        .collect();
    ordered.sort_by_key(|estimate| std::cmp::Reverse((estimate.problem.as_ref().map_or(0, |p| p.targets.len()), estimate.fractures.len())));
    let work = |estimate: &mut Estimate| {
        let started = std::time::Instant::now();
        let mut searched = false;
        let mut order: Vec<_> = (0..estimate.fractures.len()).collect();
        order.sort_by(|&a, &b| {
            estimate.fractures[b]
                .base_cost()
                .unwrap_or(f64::NEG_INFINITY)
                .total_cmp(&estimate.fractures[a].base_cost().unwrap_or(f64::NEG_INFINITY))
        });
        for i in order {
            let upper = estimate.upper_bound();
            let fracture = &mut estimate.fractures[i];
            let ceiling = upper.zip(fracture.base_cost()).map(|(upper, base)| upper - base - fracture.bench_cost);
            searched |= solve_below(
                cx,
                SolveSlot { problem: &mut fracture.problem, plan: &mut fracture.plan, error: &mut fracture.error },
                table,
                &mut fracture.craft_lower_bound,
                ceiling,
            );
        }
        let ceiling = estimate.upper_bound().map(|upper| upper - estimate.bench_cost() - estimate.base_price.map_or(0.0, |b| b.0));
        searched |= solve_below(
            cx,
            SolveSlot { problem: &mut estimate.problem, plan: &mut estimate.plan, error: &mut estimate.error },
            table,
            &mut estimate.craft_lower_bound,
            ceiling,
        );
        if searched && let Some(progress) = progress {
            let dominated = usize::from(estimate.craft_lower_bound.is_some())
                + estimate.fractures.iter().filter(|fracture| fracture.craft_lower_bound.is_some()).count();
            progress(&format!("solved {} ({:.2}s; {dominated} dominated alternatives)", estimate.title, started.elapsed().as_secs_f64()));
        }
    };
    if workers == Some(1) {
        ordered.into_iter().for_each(work);
    } else {
        let mut run = || ordered.par_iter_mut().with_max_len(1).for_each(|estimate| work(estimate));
        match workers.filter(|workers| *workers > 1).and_then(|workers| rayon::ThreadPoolBuilder::new().num_threads(workers).build().ok()) {
            Some(pool) => pool.install(run),
            None => run(),
        }
    }
}

/// Solve every item that has a problem but no plan yet, in parallel.
///
/// Each solve is independent CPU work, so they scale with cores. Biggest problems go first so
/// no core idles at the end. `workers` caps the threads (`Some(1)` solves inline); `None` uses
/// rayon's global pool.
pub fn solve_all(cx: &Context, items: &mut [&mut dyn Solvable], table: &Prices, workers: Option<usize>, progress: Option<Progress>) {
    let slots: Vec<SolveSlot> = items.iter_mut().map(|i| i.slot()).collect();
    solve_slots(cx, slots, table, workers, progress);
}

/// `solve_all` over slots (see `slots_of`).
pub fn solve_slots(cx: &Context, slots: Vec<SolveSlot>, table: &Prices, workers: Option<usize>, progress: Option<Progress>) {
    let say = |m: &str| {
        if let Some(p) = progress {
            p(m)
        }
    };
    let mut candidates: Vec<SolveSlot> =
        slots.into_iter().filter(|s| s.problem.is_some() && s.plan.is_none() && s.error.is_none()).collect();
    // Keys and cached plans first, in parallel: hashing a problem costs about what reading its plan does.
    let keyed: Vec<Option<String>> = candidates.par_iter_mut().map(|s| load_slot(cx, s, table)).collect();
    let mut todo: Vec<(SolveSlot, String)> = candidates.into_iter().zip(keyed).filter_map(|(s, key)| Some((s, key?))).collect();
    todo.sort_by_key(|(s, _)| std::cmp::Reverse(s.problem.as_ref().map_or(0, |p| p.targets.len())));
    let n = todo.len();
    let done = AtomicUsize::new(0);
    let started = AtomicUsize::new(0);
    let work = |(slot, key): &mut (SolveSlot, String)| {
        let label =
            slot.problem.as_ref().map(|p| p.targets.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(" + ")).unwrap_or_default();
        let index = started.fetch_add(1, Ordering::Relaxed) + 1;
        say(&format!("solving {index}/{n}: {label}"));
        let before = std::time::Instant::now();
        solve_slot(cx, slot, table, Some(std::mem::take(key)));
        say(&format!("solved {}/{n}: {label} ({:.2}s)", done.fetch_add(1, Ordering::Relaxed) + 1, before.elapsed().as_secs_f64()));
    };
    if workers == Some(1) || n < 4 {
        for (i, item) in todo.iter_mut().enumerate() {
            say(&format!("solving {}/{n}", i + 1));
            work(item);
        }
        return;
    }
    let mut run = || todo.par_iter_mut().with_max_len(1).for_each(work);
    match workers.filter(|w| *w > 1).and_then(|w| rayon::ThreadPoolBuilder::new().num_threads(w).build().ok()) {
        Some(pool) => pool.install(run),
        None => run(),
    }
}
