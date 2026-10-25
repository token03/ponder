//! Recombinator trees for gear estimates.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ponder_core::Prices;
use ponder_game::mods::ModDB;
use rayon::prelude::*;

use super::context::{Context, Progress};
use super::solving::continuation;
use super::types::{Estimate, RecombInfo};
use crate::gear::Pricing;
use crate::recomb::{Planner, Step};

/// Price the donor routes (protected, recombinator, Awakener) of every gear estimate.
pub fn add_recombs(
    cx: &Context,
    db: &ModDB,
    estimates: &mut [Estimate],
    table: &Prices,
    workers: Option<usize>,
    progress: Option<Progress>,
) {
    add_recombs_in_league(cx, db, estimates, table, "Standard", workers, progress);
}

pub fn add_recombs_in_league(
    cx: &Context,
    db: &ModDB,
    estimates: &mut [Estimate],
    table: &Prices,
    league: &str,
    workers: Option<usize>,
    progress: Option<Progress>,
) {
    if let Some(p) = progress {
        p("pricing protected, recombinator and Awakener recipes");
    }
    let run = |e: &mut Estimate| price_donor_routes(cx, db, e, table, league);
    if workers == Some(1) {
        estimates.iter_mut().for_each(run);
    } else {
        estimates.par_iter_mut().for_each(run);
    }
}

/// The routes that start from prepared components, for one estimate: a protected side, a
/// recombinator tree and an Awakener pair. Each is priced once; reset a field to price it again.
pub fn price_donor_routes(cx: &Context, db: &ModDB, e: &mut Estimate, table: &Prices, league: &str) {
    if e.kind != "gear" || e.problem.is_none() {
        return;
    }
    if e.protected.is_none() {
        e.protected = super::protected::protected_recipe(cx, e, table);
    }
    if e.recomb.is_none() {
        e.recomb = recomb_route(cx, db, e, table, league);
    }
    if e.awakener.is_none() {
        e.awakener = awakener_route(cx, db, e, table, league);
    }
}

/// Solve donors through the shared solves (products that share a donor solve it once).
fn solve_donors<'a>(cx: &Context, donors: impl Iterator<Item = &'a mut crate::recomb::Donor>, table: &Prices) {
    let mut todo: Vec<_> = donors.filter(|d| d.problem.is_some() && d.plan.is_none() && d.error.is_none()).collect();
    todo.par_iter_mut().for_each(|d| match cx.solves.solve(cx, d.problem.as_ref().expect("filtered"), table) {
        Ok(plan) => d.plan = Some(plan),
        Err(error) => d.error = Some(error),
    });
}

/// The cheapest recombinator recipe. Unpadded donors are solved first; the NNN-padded ones
/// (most of the donor work) only when a tree using them at their lower bounds could beat
/// both the unpadded trees and the item's other routes.
fn recomb_route(cx: &Context, db: &ModDB, e: &Estimate, table: &Prices, league: &str) -> Option<RecombInfo> {
    let problem = e.problem.as_ref()?;
    let bid = db.find_base(&e.base)?;
    let pricing = Pricing::new(table, &cx.settings);
    let mut planner = Planner::new(db, problem, bid, e.item_level, pricing);
    if !planner.ok {
        return None;
    }
    solve_donors(cx, planner.leaves_mut().into_iter(), table);
    if planner.has_pending_padded() {
        planner.bound_padded();
        let finishing = Finishing::new(e);
        let solved = finishing.cheapest(&planner, &planner.best_by_set());
        let optimistic = finishing.cheapest(&planner, &planner.optimistic_by_set());
        let incumbent = [e.cheapest().map(|c| c - e.bench_cost()), solved].into_iter().flatten().reduce(f64::min);
        let worth_it = optimistic.is_some_and(|lower| incumbent.is_none_or(|best| lower < best));
        if worth_it {
            planner.prepare_padded(db, pricing);
            solve_donors(cx, planner.leaves_mut().into_iter(), table);
        }
    }
    best_part(&planner, e, cx, league, table)
}

/// What finishing costs from each item a tree can hand over, as far as the estimate's own
/// plans know it (for comparing trees before choosing one; `best_part` prices the choice).
struct Finishing<'a> {
    problem: &'a ponder_core::Problem,
    plans: Vec<&'a ponder_core::Plan>,
}

impl<'a> Finishing<'a> {
    fn new(e: &'a Estimate) -> Finishing<'a> {
        let plans = e.plan.iter().chain(e.protected.as_ref().map(|p| p.finishing.as_ref())).collect();
        Finishing { problem: e.problem.as_ref().expect("a gear problem"), plans }
    }

    /// The cheapest tree plus finish over every set; a set no plan finishes counts as free to
    /// finish, so with optimistic trees this stays a lower bound.
    fn cheapest(&self, planner: &Planner, by_set: &BTreeMap<BTreeSet<String>, Arc<Step>>) -> Option<f64> {
        by_set
            .iter()
            .filter_map(|(set, step)| {
                let state = planner.state_after(self.problem, set);
                let finish = if self.problem.done(&state) {
                    0.0
                } else {
                    self.plans.iter().filter_map(|p| p.to_finish(self.problem, &state)).reduce(f64::min).unwrap_or(0.0)
                };
                Some(step.cost? + finish)
            })
            .reduce(f64::min)
    }
}

/// The cheapest Awakener recipe; a pair whose two donors and orb already cost more than the
/// item's best route skips its finishing solve.
fn awakener_route(cx: &Context, db: &ModDB, e: &Estimate, table: &Prices, league: &str) -> Option<crate::awakener::Recipe> {
    let problem = e.problem.as_ref()?;
    if problem.influences.len() != 2 {
        return None;
    }
    let bid = db.find_base(&e.base)?;
    let pricing = Pricing::new(table, &cx.settings);
    let mut planner = crate::awakener::Planner::new(db, problem, bid, e.item_level, pricing);
    if planner.pairs.is_empty() {
        return None;
    }
    solve_donors(cx, planner.leaves_mut(), table);
    let incumbent = e.cheapest().map(|c| c - e.bench_cost());
    let mut recipe = planner.best(problem, pricing, |pair, mut recovery| {
        let preserved = ponder_core::Preservation { targets: pair.targets.iter().fold(0, |mask, &i| mask | 1 << i), filler: [0; 2] };
        let quote = cx.resale_quote(problem, preserved);
        if quote.is_none() && incumbent.is_some_and(|best| recovery.cost >= best) {
            return None; // finishing costs at least nothing without a sale credit
        }
        let baseline =
            super::solving::acquired_continuation(cx, problem, pair.outcomes.clone(), preserved, recovery.clone(), e.plan.as_ref(), table)
                .ok()?;
        let thresholds = super::resale::thresholds(problem, &baseline, &recovery, preserved, recovery.cost);
        let Some(quote) = quote else { return Some((baseline, thresholds, None)) };
        let (cost, hours, usage) = (recovery.cost, recovery.hours, recovery.usage.clone());
        quote.apply(&mut recovery, cost, hours, usage, cx.settings.chaos_per_hour);
        match super::solving::acquired_continuation(cx, problem, pair.outcomes.clone(), preserved, recovery, e.plan.as_ref(), table) {
            Ok(plan) => Some((plan, thresholds, None)),
            Err(error) => Some((baseline, thresholds, Some(error.to_string()))),
        }
    })?;
    for donor in &mut recipe.donors {
        donor.query = super::donors::donor_query_influenced(
            cx,
            &donor.base,
            std::slice::from_ref(&donor.modifiers),
            Some(e.item_level),
            1,
            1,
            &std::collections::BTreeSet::from([donor.influence.clone()]),
        );
        donor.trade_url = donor.query.as_ref().map(|q| ponder_net::trade::search_url(league, q));
    }
    Some(recipe)
}

/// The cheapest of recombining some of the wanted mods and crafting the rest: a tree for a set
/// (a side's mods, a pair, or everything) plus what the estimate's own plan says finishing the
/// clean rare it hands over costs. None when no tree has a cost.
pub fn best_part(planner: &Planner, e: &Estimate, cx: &Context, league: &str, table: &Prices) -> Option<RecombInfo> {
    let (problem, plan) = (e.problem.as_ref()?, e.plan.as_ref());
    let staged = e.protected.as_ref();
    let candidates: Vec<_> = planner.best_by_set().into_iter().filter(|(_, step)| step.cost.is_some()).collect();
    let missing: indexmap::IndexSet<_> = candidates
        .iter()
        .map(|(set, _)| planner.state_after(problem, set))
        .filter(|state| {
            !problem.done(state)
                && plan.and_then(|p| p.to_finish(problem, state)).is_none()
                && staged.and_then(|p| p.finishing.to_finish(problem, state)).is_none()
        })
        .collect();
    let expanded = if missing.is_empty() || e.protected.is_some() {
        None
    } else {
        let probability = 1.0 / missing.len() as f64;
        let initial = std::sync::Arc::new(ponder_core::Distribution::Exact(missing.into_iter().map(|s| (probability, s)).collect()));
        continuation(cx, problem, initial, plan, table).ok()
    };
    let mut best = None;
    for (set, step) in candidates {
        let Some(tree) = step.cost else { continue };
        let state = planner.state_after(problem, &set);
        let direct = expanded
            .as_ref()
            .and_then(|p| p.to_finish(problem, &state))
            .into_iter()
            .chain(plan.and_then(|p| p.to_finish(problem, &state)))
            .min_by(f64::total_cmp);
        let protected = staged.and_then(|r| r.finishing.to_finish(problem, &state));
        let (finish, recover) = if problem.done(&state) {
            (0.0, false)
        } else {
            match (direct, protected) {
                (Some(direct), Some(protected)) if protected < direct => (protected, true),
                (Some(direct), _) => (direct, false),
                (None, Some(protected)) => (protected, true),
                (None, None) => continue,
            }
        };
        let total = tree + finish;
        if best.as_ref().is_none_or(|b: &(f64, _, _, _, _)| total < b.0) {
            best = Some((total, set, step, state, recover));
        }
    }
    let (_, set, step, state, recover) = best?;
    let known = expanded
        .as_ref()
        .into_iter()
        .chain(plan)
        .filter_map(|p| p.to_finish(problem, &state).map(|cost| (p, cost)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(p, _)| p);
    let initial = std::sync::Arc::new(ponder_core::Distribution::Exact(vec![(1.0, state)]));
    let finishing = if recover {
        staged?.continue_from(cx, problem, initial, table)?
    } else {
        continuation(cx, problem, initial, known, table).ok()?
    };
    let finish = finishing.cost;
    let total = step.cost? + finish;
    let mut json = step.to_dict();
    let donors: Vec<_> = step.donor_copies().into_iter().map(|(d, copies)| {
                let wanted: Option<Vec<_>> = d.carries.iter().map(|name| planner.want.iter().find(|w| &w.name == name)?.target.as_ref().map(|t| t.good.clone())).collect();
                let query = wanted.as_ref().and_then(|wanted| super::donors::donor_query(cx, &d.base, wanted, Some(e.item_level), wanted.len(), wanted.len()));
                let trade_url = query.as_ref().map(|q| ponder_net::trade::search_url(league, q));
                serde_json::json!({"base": d.base, "targets": d.carries, "query": query, "trade_url": trade_url, "required_mods": wanted.as_ref().map(Vec::len), "copies": copies})
            }).collect();
    json["donors"] = serde_json::json!(donors);
    json["part"] = serde_json::json!(set.iter().collect::<Vec<_>>());
    json["then_craft"] = serde_json::json!({"from": ponder_core::describe(problem, &state), "chaos": finish});
    json["cost"] = serde_json::json!(total);
    let (mut usage, mut hours) = step.usage();
    hours += finishing.hours;
    for (action, count) in &finishing.usage {
        *usage.entry(action.clone()).or_default() += count;
    }
    Some(RecombInfo {
        cost: Some(total),
        json,
        part: set.into_iter().collect(),
        finish,
        finishing: Some(std::sync::Arc::new(finishing)),
        usage,
        hours,
        donors: step.donor_copies().into_iter().map(|(donor, copies)| (donor.clone(), copies)).collect(),
        recovery: recover.then(|| std::sync::Arc::new(staged.expect("selected recovery recipe").clone())),
    })
}
