//! Recombinator trees for gear estimates.

use ponder_core::Prices;
use ponder_game::mods::ModDB;
use rayon::prelude::*;
use serde_json::Value;

use super::context::{Context, Progress};
use super::solving::{SolveSlot, solve_slots};
use super::types::{Estimate, RecombInfo};
use crate::gear::Pricing;
use crate::recomb::Planner;

/// Cost a recombinator tree for every gear estimate that has 2+ wanted mods.
///
/// All their leaf donors are solved in one parallel batch (and plan-cached).
pub fn add_recombs(cx: &Context, db: &ModDB, estimates: &mut [Estimate], table: &Prices, workers: Option<usize>, progress: Option<Progress>) {
    add_recombs_in_league(cx, db, estimates, table, "Standard", workers, progress);
}

pub fn add_recombs_in_league(cx: &Context, db: &ModDB, estimates: &mut [Estimate], table: &Prices, league: &str, workers: Option<usize>, progress: Option<Progress>) {
    let pricing = Pricing::new(table, &cx.settings);
    let mut planners: Vec<(usize, Planner)> = estimates
        .par_iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let problem = e.problem.as_ref()?;
            if e.kind != "gear" || e.error.is_some() {
                return None;
            }
            let bid = db.find_base(&e.base)?;
            let planner = Planner::new(db, problem, bid, e.item_level, pricing);
            planner.ok.then_some((i, planner))
        })
        .collect();
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
        let info = best_part(planner, &estimates[*i], cx, league);
        estimates[*i].recomb = info;
    }
}

/// The cheapest of recombining some of the wanted mods and crafting the rest: a tree for a set
/// (a side's mods, a pair, or everything) plus what the estimate's own plan says finishing the
/// clean rare it hands over costs. None when no tree has a cost.
pub fn best_part(planner: &Planner, e: &Estimate, cx: &Context, league: &str) -> Option<RecombInfo> {
    let (problem, plan) = (e.problem.as_ref()?, e.plan.as_ref());
    let mut best: Option<(f64, Vec<String>, f64, Value)> = None;
    for (set, step) in planner.best_by_set() {
        let Some(tree) = step.cost else { continue };
        let state = planner.state_after(problem, &set);
        let finish = if problem.done(&state) {
            0.0
        } else {
            match plan.and_then(|p| p.to_finish(problem, &state)) {
                Some(v) => v,
                None => continue, // the plan never meets that item: no price for the rest
            }
        };
        let total = tree + finish;
        if best.as_ref().is_none_or(|b| total < b.0) {
            let mut json = step.to_dict();
            let donors: Vec<_> = step.donors().into_iter().filter_map(|d| {
                let wanted: Option<Vec<_>> = d.carries.iter().map(|name| planner.want.iter().find(|w| &w.name == name)?.target.as_ref().map(|t| t.good.clone())).collect();
                let wanted = wanted?;
                let query = super::donors::donor_query(cx, &d.base, &wanted, Some(e.item_level), wanted.len(), wanted.len())?;
                let trade_url = ponder_net::trade::search_url(league, &query);
                Some(serde_json::json!({"base": d.base, "targets": d.carries, "query": query, "trade_url": trade_url, "required_mods": wanted.len()}))
            }).collect();
            json["donors"] = serde_json::json!(donors);
            json["part"] = serde_json::json!(set.iter().collect::<Vec<_>>());
            json["then_craft"] = serde_json::json!({"from": ponder_core::describe(problem, &state), "chaos": finish});
            json["cost"] = serde_json::json!(total);
            best = Some((total, set.into_iter().collect(), finish, json));
        }
    }
    best.map(|(cost, part, finish, json)| RecombInfo { cost: Some(cost), json, part, finish })
}
