//! Recombinator trees for gear estimates.

use ponder_core::Prices;
use ponder_game::mods::ModDB;

use super::context::{Context, Progress};
use super::solving::{SolveSlot, solve_slots};
use super::types::{Estimate, RecombInfo};
use crate::gear::Pricing;
use crate::recomb::Planner;

/// Cost a recombinator tree for every gear estimate that has 2+ wanted mods.
///
/// All their leaf donors are solved in one parallel batch (and plan-cached).
pub fn add_recombs(cx: &Context, db: &ModDB, estimates: &mut [Estimate], table: &Prices, workers: Option<usize>, progress: Option<Progress>) {
    let pricing = Pricing::new(table, &cx.settings);
    let mut planners: Vec<(usize, Planner)> = Vec::new();
    for (i, e) in estimates.iter().enumerate() {
        let Some(problem) = e.problem.as_ref() else { continue };
        if e.kind != "gear" || e.error.is_some() {
            continue;
        }
        let Some(bid) = db.find_base(&e.base) else { continue };
        let planner = Planner::new(db, problem, bid, e.item_level, pricing);
        if planner.ok {
            planners.push((i, planner));
        }
    }
    let items = planners.len();
    let slots: Vec<SolveSlot> = planners
        .iter_mut()
        .flat_map(|(_, p)| p.leaves_mut())
        .map(|d| SolveSlot { problem: &mut d.problem, plan: &mut d.plan, error: &mut d.error })
        .collect();
    if let Some(p) = progress {
        p(&format!("recombinator donors: {} for {} items", slots.len(), items));
    }
    solve_slots(cx, slots, table, workers, progress);
    for (i, planner) in &planners {
        estimates[*i].recomb = planner.best().map(|step| RecombInfo { cost: step.cost, json: step.to_dict() });
    }
}
