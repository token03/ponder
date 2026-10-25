use std::collections::HashMap;
use std::sync::Arc;

use ponder_core::{Action, ActionKind, Distribution, Effect, GOOD, Plan, Prices, Problem, RARE, Recovery, Side, step_cost};
use rayon::prelude::*;
use serde_json::{Value, json};

use super::cache::plan_key;
use super::context::Context;
use super::solving::{SolveSlot, solve_slots};
use super::types::Estimate;

#[derive(Clone, Debug)]
pub struct ProtectedRecipe {
    pub side: Side,
    pub preparation: Problem,
    pub plan: Arc<Plan>,
    pub cleanup: Action,
    pub finishing: Arc<Plan>,
    pub cost: f64,
    pub hours: f64,
    pub usage: HashMap<String, f64>,
    pub recovery: Recovery,
    pub sale_thresholds: Vec<super::resale::SaleThreshold>,
    pub sale_error: Option<String>,
}

impl ProtectedRecipe {
    pub fn upper_bound(&self) -> Option<f64> {
        if self.plan.bounds.upper != self.plan.cost || self.finishing.bounds.upper < self.finishing.cost {
            return None;
        }
        let upper = self.cost - self.finishing.cost + self.finishing.bounds.upper;
        upper.is_finite().then_some(upper)
    }

    pub fn to_dict(&self, problem: &Problem) -> Value {
        json!({
            "side": self.side,
            "cost": self.cost,
            "preparation": ponder_core::steps(&self.preparation, &self.plan, 0.02),
            "cleanup": self.cleanup.name,
            "rebuilds": self.finishing.restarts - self.finishing.sales,
            "recoveries": self.finishing.restarts,
            "sales": self.finishing.sales,
            "sale_proceeds": self.finishing.sale_proceeds,
            "sale_thresholds": self.sale_thresholds,
            "sale_error": self.sale_error,
            "rebuild_cost": self.recovery.cost,
            "finishing": ponder_core::steps(problem, &self.finishing, 0.02),
        })
    }

    pub fn continue_from(&self, cx: &Context, problem: &Problem, initial: Arc<Distribution>, prices: &Prices) -> Option<Plan> {
        if initial.iter().any(|(_, s)| {
            problem.count(s, self.side) != problem.side_limit(RARE, self.side)
                || problem.targets.iter().enumerate().any(|(i, t)| t.side == self.side && s.status[i] != GOOD)
        }) {
            return None;
        }
        let raw = serde_json::to_vec(&(initial.to_vec(), initial.bounds(), &self.recovery.sale)).ok()?;
        let key = format!("{}-recovery-{}-{}", plan_key(problem, prices, &cx.settings), self.side.index(), blake3::hash(&raw).to_hex());
        cx.plans
            .get_or_solve(&key, || {
                let private = problem.fresh_transition_cache();
                ponder_core::follow_with_recovery(
                    &private,
                    initial.clone(),
                    &self.finishing,
                    self.side,
                    self.recovery.clone(),
                    prices,
                    &cx.settings,
                )
                .or_else(|_| ponder_core::solve_with_recovery(&private, initial, self.side, self.recovery.clone(), prices, &cx.settings))
            })
            .ok()
    }
}

struct Work {
    estimate: usize,
    side: Side,
    problem: Option<Problem>,
    plan: Option<Plan>,
    error: Option<String>,
    cleanup: Action,
}

pub fn add_protected_routes(cx: &Context, estimates: &mut [Estimate], prices: &Prices, workers: Option<usize>) {
    let mut work = Vec::new();
    for (i, estimate) in estimates.iter().enumerate().filter(|(_, e)| e.kind == "gear") {
        let Some(problem) = &estimate.problem else { continue };
        if !problem.fixed.is_empty() || problem.begin.is_some() {
            continue;
        }
        for side in [Side::Prefix, Side::Suffix] {
            let filler = problem.filler_sides.map_or(0, |(p, s)| if side == Side::Prefix { p } else { s });
            let wanted = problem.targets.iter().filter(|t| t.side == side).count() + filler;
            if wanted == 0 || wanted != problem.side_limit(RARE, side) || problem.reserve.get(&side).copied().unwrap_or(0) != 0 {
                continue;
            }
            let cleanup = problem
                .actions
                .iter()
                .filter_map(|action| {
                    let kind = match (side, action.kind) {
                        (Side::Prefix, ActionKind::MetaLockPrefixes) | (Side::Suffix, ActionKind::MetaLockSuffixes) => {
                            ActionKind::ProtectedScour(side)
                        }
                        (Side::Suffix, ActionKind::BristleMatron) => ActionKind::MatronScour,
                        _ => return None,
                    };
                    let mut cleanup = action.clone();
                    cleanup.kind = kind;
                    cleanup.name = format!("{}; scour {}es", action.name, side.other().name());
                    cleanup.cost.push(("scour".into(), 1.0));
                    cleanup.seconds += 1.0;
                    cleanup.cost.iter().all(|(id, _)| prices.contains_key(id)).then_some(cleanup)
                })
                .min_by(|a, b| step_cost(a, prices, &cx.settings).total_cmp(&step_cost(b, prices, &cx.settings)));
            let Some(cleanup) = cleanup else { continue };
            let mut spec = problem.to_spec();
            spec.targets.retain(|t| t.side == side);
            spec.need = spec.targets.len();
            spec.filler_need = filler;
            if filler == 0 {
                spec.filler.clear();
                spec.filler_label.clear();
            }
            spec.filler_sides = (filler > 0).then_some(if side == Side::Prefix { (filler, 0) } else { (0, filler) });
            spec.reserve.clear();
            spec.reserve.insert(side.other(), 1);
            spec.bench.clear();
            spec.blockers.clear();
            spec.allow_terminal_bench = false;
            spec.junk_cap = Some(if side == Side::Prefix {
                (0, problem.side_limit(RARE, Side::Suffix))
            } else {
                (problem.side_limit(RARE, Side::Prefix), 0)
            });
            spec.actions.retain(|a| {
                !matches!(a.kind.effect(), Effect::Veil(_) | Effect::Unveil(_) | Effect::RevealUnveil(_) | Effect::FinishBench)
            });
            let veils: std::collections::HashSet<_> = spec.unveils.iter().map(|m| m.id.clone()).collect();
            spec.extra_mods.retain(|id, _| !veils.contains(id));
            spec.unveils.clear();
            spec.min_mods = 0;
            work.push(Work { estimate: i, side, problem: Some(spec.compile()), plan: None, error: None, cleanup });
        }
    }
    let slots = work.iter_mut().map(|w| SolveSlot { problem: &mut w.problem, plan: &mut w.plan, error: &mut w.error }).collect();
    solve_slots(cx, slots, prices, workers, None);
    let solve = |work: &Work| -> Option<(usize, ProtectedRecipe)> {
        let preparation = work.problem.as_ref()?;
        let plan = work.plan.as_ref()?;
        let problem = estimates[work.estimate].problem.as_ref()?;
        let mut seed = problem.start();
        seed.rarity = RARE;
        for (i, _) in problem.targets.iter().enumerate().filter(|(_, t)| t.side == work.side) {
            seed.status[i] = GOOD;
        }
        seed.junk[4 + work.side.index()] = preparation.filler_need as u8;
        let initial = Arc::new(Distribution::Exact(vec![(1.0, seed)]));
        let cleanup_cost = step_cost(&work.cleanup, prices, &cx.settings);
        let mut recovery_usage = plan.usage.clone();
        *recovery_usage.entry(work.cleanup.name.clone()).or_default() += 1.0;
        *recovery_usage.entry("scour to restart".into()).or_default() += 1.0;
        let mut recovery = Recovery {
            initial: initial.clone(),
            cost: plan.cost + cleanup_cost + prices.get("scour")? + cx.settings.chaos_per_hour / 3600.0,
            hours: plan.hours + (work.cleanup.seconds + 1.0) / 3600.0,
            usage: recovery_usage,
            sale: None,
        };
        let key = format!("{}-protected-recovery-{}", plan_key(problem, prices, &cx.settings), work.side.index());
        let baseline = cx
            .plans
            .get_or_solve(&key, || {
                let private = problem.fresh_transition_cache();
                ponder_core::solve_with_recovery(&private, initial.clone(), work.side, recovery.clone(), prices, &cx.settings)
            })
            .ok()?;
        let replacement_cost = plan.cost + cleanup_cost;
        let replacement_hours = plan.hours + work.cleanup.seconds / 3600.0;
        let sale_thresholds =
            super::resale::thresholds(problem, &baseline, &recovery, ponder_core::Preservation::side(problem, work.side), replacement_cost);
        let mut sale_error = None;
        let finishing = match cx.resale_quote(problem, ponder_core::Preservation::side(problem, work.side)) {
            Some(quote) => {
                let mut sale_usage = plan.usage.clone();
                *sale_usage.entry(work.cleanup.name.clone()).or_default() += 1.0;
                quote.apply(&mut recovery, replacement_cost, replacement_hours, sale_usage, cx.settings.chaos_per_hour);
                let raw = serde_json::to_vec(&recovery.sale).ok()?;
                let sale_key = format!("{key}-{}", blake3::hash(&raw).to_hex());
                match cx.plans.get_or_solve(&sale_key, || {
                    let private = problem.fresh_transition_cache();
                    ponder_core::solve_with_recovery(&private, initial, work.side, recovery.clone(), prices, &cx.settings)
                }) {
                    Ok(plan) => plan,
                    Err(error) => {
                        sale_error = Some(error.to_string());
                        recovery.sale = None;
                        baseline
                    }
                }
            }
            None => baseline,
        };
        let mut usage = plan.usage.clone();
        for (action, count) in &finishing.usage {
            *usage.entry(action.clone()).or_default() += count;
        }
        *usage.entry(work.cleanup.name.clone()).or_default() += 1.0;
        Some((
            work.estimate,
            ProtectedRecipe {
                side: work.side,
                preparation: preparation.clone(),
                plan: Arc::new(plan.clone()),
                cleanup: work.cleanup.clone(),
                cost: plan.cost + cleanup_cost + finishing.cost,
                hours: plan.hours + work.cleanup.seconds / 3600.0 + finishing.hours,
                finishing: Arc::new(finishing),
                usage,
                recovery,
                sale_thresholds,
                sale_error,
            },
        ))
    };
    let recipes: Vec<_> =
        if workers == Some(1) { work.iter().filter_map(solve).collect() } else { work.par_iter().filter_map(solve).collect() };
    for (i, recipe) in recipes {
        if estimates[i].protected.as_ref().is_none_or(|best| recipe.cost < best.cost) {
            estimates[i].protected = Some(recipe);
        }
    }
}
