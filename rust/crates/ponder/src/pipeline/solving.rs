//! Solving: one problem through the plan cache, or all of them in parallel with rayon.

use std::sync::atomic::{AtomicUsize, Ordering};

use ponder_core::{self as craft, Plan, Prices, Problem};
use rayon::prelude::*;

use super::cache::plan_key;
use super::context::{Context, Progress};
use super::finish::Finish;
use super::types::{Estimate, Fracture};

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

/// A worker solved a copy; drop the same unpriced actions here so recipe and simulate agree.
fn attach(slot: &mut SolveSlot, plan: Plan) {
    if let Some(p) = slot.problem.as_mut() {
        p.actions.retain(|a| !plan.unpriced.contains(&a.name));
    }
    *slot.plan = Some(plan);
}

/// Solve one slot in place (from the plan cache when it has it).
fn solve_slot(cx: &Context, slot: &mut SolveSlot, table: &Prices, key: Option<String>) {
    let Some(problem) = slot.problem.as_mut() else { return };
    let key = key.unwrap_or_else(|| plan_key(problem, table, &cx.settings));
    if let Some(plan) = cx.plans.get(&key) {
        attach(slot, plan);
        return;
    }
    match craft::solve(problem, table, &cx.settings) {
        Ok(plan) => {
            cx.plans.put(&key, &plan);
            attach(slot, plan);
        }
        Err(e) => *slot.error = Some(e.to_string()),
    }
}

/// Solve an Estimate, Fracture or Finish in place (from the plan cache when it has it).
pub fn solve_one(cx: &Context, item: &mut dyn Solvable, table: &Prices) {
    solve_slot(cx, &mut item.slot(), table, None);
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
    let mut candidates: Vec<SolveSlot> = slots.into_iter().filter(|s| s.problem.is_some() && s.plan.is_none() && s.error.is_none()).collect();
    // Keys and cached plans first, in parallel: hashing a problem costs about what reading its plan does.
    let hits = AtomicUsize::new(0);
    let keyed: Vec<(Option<String>, bool)> = candidates
        .par_iter_mut()
        .map(|s| {
            let key = plan_key(s.problem.as_ref().expect("filtered"), table, &cx.settings);
            match cx.plans.get(&key) {
                Some(plan) => {
                    attach(s, plan);
                    hits.fetch_add(1, Ordering::Relaxed);
                    (None, true)
                }
                None => (Some(key), false),
            }
        })
        .collect();
    let mut todo: Vec<(SolveSlot, String)> = candidates.into_iter().zip(keyed).filter_map(|(s, (key, _))| Some((s, key?))).collect();
    todo.sort_by_key(|(s, _)| std::cmp::Reverse(s.problem.as_ref().map_or(0, |p| p.targets.len())));
    let n = todo.len();
    let done = AtomicUsize::new(0);
    let work = |(slot, key): &mut (SolveSlot, String)| {
        solve_slot(cx, slot, table, Some(std::mem::take(key)));
        say(&format!("solved {}/{n}", done.fetch_add(1, Ordering::Relaxed) + 1));
    };
    if workers == Some(1) || n < 4 {
        for (i, item) in todo.iter_mut().enumerate() {
            say(&format!("solving {}/{n}", i + 1));
            solve_slot(cx, &mut item.0, table, Some(std::mem::take(&mut item.1)));
        }
        return;
    }
    let mut run = || todo.par_iter_mut().with_max_len(1).for_each(work);
    match workers.filter(|w| *w > 1).and_then(|w| rayon::ThreadPoolBuilder::new().num_threads(w).build().ok()) {
        Some(pool) => pool.install(run),
        None => run(),
    }
}
