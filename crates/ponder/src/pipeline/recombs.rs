//! Recombinator trees for gear estimates.

use ponder_core::Prices;
use ponder_game::mods::ModDB;
use rayon::prelude::*;

use super::context::{Context, Progress};
use super::solving::{SolveSlot, continuation, solve_slots};
use super::types::{Estimate, RecombInfo};
use crate::gear::Pricing;
use crate::recomb::Planner;

/// Cost a recombinator tree for every gear estimate that has 2+ wanted mods.
///
/// All their leaf donors are solved in one parallel batch (and plan-cached).
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
    let pricing = Pricing::new(table, &cx.settings);
    if let Some(p) = progress {
        p("pricing protected finishing recipes");
    }
    super::protected::add_protected_routes(cx, estimates, table, workers);
    if let Some(p) = progress {
        p("preparing crafting donors");
    }
    let mut planners: Vec<(usize, Planner)> = estimates
        .par_iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let problem = e.problem.as_ref()?;
            if e.kind != "gear" {
                return None;
            }
            let bid = db.find_base(&e.base)?;
            let planner = Planner::new(db, problem, bid, e.item_level, pricing);
            planner.ok.then_some((i, planner))
        })
        .collect();
    let mut awakeners: Vec<_> = estimates
        .par_iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let problem = e.problem.as_ref()?;
            if e.kind != "gear" || problem.influences.len() != 2 {
                return None;
            }
            let bid = db.find_base(&e.base)?;
            let planner = crate::awakener::Planner::new(db, problem, bid, e.item_level, pricing);
            (!planner.pairs.is_empty()).then_some((i, planner))
        })
        .collect();
    let items = planners.len();
    let mut donors: Vec<_> = planners.iter_mut().flat_map(|(_, p)| p.leaves_mut()).collect();
    donors.extend(awakeners.iter_mut().flat_map(|(_, p)| p.leaves_mut()));
    if let Some(p) = progress {
        p(&format!("crafting donors: {} for {} items", donors.len(), items));
    }
    solve_donor_plans(cx, donors, table, workers, progress);
    if let Some(progress) = progress {
        progress("pricing donor finishing recipes");
    }
    let recombine = |(i, planner): &(usize, Planner)| (*i, best_part(planner, &estimates[*i], cx, league, table));
    let recipes: Vec<_> =
        if workers == Some(1) { planners.iter().map(recombine).collect() } else { planners.par_iter().map(recombine).collect() };
    for (i, recipe) in recipes {
        estimates[i].recomb = recipe;
    }
    let awaken = |(i, planner): &(usize, crate::awakener::Planner)| {
        let e = &estimates[*i];
        let mut recipe = None;
        if let Some(problem) = &e.problem {
            recipe = planner.best(problem, pricing, |pair, mut recovery| {
                let preserved =
                    ponder_core::Preservation { targets: pair.targets.iter().fold(0, |mask, &i| mask | 1 << i), filler: [0; 2] };
                let baseline = super::solving::acquired_continuation(
                    cx,
                    problem,
                    pair.outcomes.clone(),
                    preserved,
                    recovery.clone(),
                    e.plan.as_ref(),
                    table,
                )
                .ok()?;
                let thresholds = super::resale::thresholds(problem, &baseline, &recovery, preserved, recovery.cost);
                let Some(quote) = cx.resale_quote(problem, preserved) else { return Some((baseline, thresholds, None)) };
                let (cost, hours, usage) = (recovery.cost, recovery.hours, recovery.usage.clone());
                quote.apply(&mut recovery, cost, hours, usage, cx.settings.chaos_per_hour);
                match super::solving::acquired_continuation(cx, problem, pair.outcomes.clone(), preserved, recovery, e.plan.as_ref(), table)
                {
                    Ok(plan) => Some((plan, thresholds, None)),
                    Err(error) => Some((baseline, thresholds, Some(error.to_string()))),
                }
            });
            if let Some(recipe) = &mut recipe {
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
            }
        }
        (*i, recipe)
    };
    let recipes: Vec<_> =
        if workers == Some(1) { awakeners.iter().map(awaken).collect() } else { awakeners.par_iter().map(awaken).collect() };
    for (i, recipe) in recipes {
        estimates[i].awakener = recipe;
    }
}

fn solve_donor_plans(
    cx: &Context,
    donors: Vec<&mut crate::recomb::Donor>,
    table: &Prices,
    workers: Option<usize>,
    progress: Option<Progress>,
) {
    type Work = (Option<ponder_core::Problem>, Option<ponder_core::Plan>, Option<String>);
    let mut unique: indexmap::IndexMap<blake3::Hash, Work> = indexmap::IndexMap::new();
    let keys: Vec<_> = donors
        .iter()
        .map(|d| {
            d.problem.as_ref().map(|problem| {
                let key = problem.fingerprint();
                unique.entry(key).or_insert_with(|| (Some(problem.clone()), None, None));
                key
            })
        })
        .collect();
    let slots = unique.values_mut().map(|(problem, plan, error)| SolveSlot { problem, plan, error }).collect();
    solve_slots(cx, slots, table, workers, progress);
    let results: std::collections::HashMap<_, _> =
        unique.into_iter().map(|(key, (_, plan, error))| (key, (plan.map(std::sync::Arc::new), error))).collect();
    for (donor, key) in donors.into_iter().zip(keys) {
        if let Some(result) = key.and_then(|key| results.get(&key)) {
            donor.plan = result.0.clone();
            donor.error = result.1.clone();
        }
    }
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
