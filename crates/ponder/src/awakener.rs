use std::collections::BTreeSet;

use ponder_core::{Distribution, GOOD, Plan, Problem, RARE, State, Target};
use ponder_game::mods::ModDB;
use serde::Serialize;

use crate::gear::{self, Pricing};
use crate::recomb::Donor;

const INFLUENCES: [&str; 6] = ["shaper", "elder", "crusader", "redeemer", "hunter", "warlord"];
pub const AWAKENER_SECONDS: f64 = 15.0;

pub struct Pair {
    pub targets: [usize; 2],
    pub influences: [String; 2],
    pub donors: [Donor; 2],
    pub outcomes: std::sync::Arc<Distribution>,
}

pub struct Planner {
    pub pairs: Vec<Pair>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Recipe {
    pub cost: f64,
    pub orb_cost: f64,
    pub finish: f64,
    pub influences: [String; 2],
    pub targets: [String; 2],
    pub donors: [DonorRecipe; 2],
    pub reforge: Vec<(f64, State)>,
    pub exact_reforge: bool,
    pub finish_steps: Vec<ponder_core::Step>,
    pub usage: std::collections::HashMap<String, f64>,
    pub hours: f64,
    #[serde(skip)]
    pub finishing: Option<std::sync::Arc<Plan>>,
    pub restarts: f64,
    pub sales: f64,
    pub sale_proceeds: f64,
    pub restart_conditions: Vec<String>,
    pub sale_thresholds: Vec<crate::pipeline::SaleThreshold>,
    pub sale_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DonorRecipe {
    pub base: String,
    pub influence: String,
    pub target: String,
    pub cost: f64,
    pub steps: Vec<ponder_core::Step>,
    pub trade_url: Option<String>,
    pub query: Option<serde_json::Value>,
    pub modifiers: BTreeSet<String>,
}

impl Planner {
    pub fn new(db: &ModDB, problem: &Problem, base_id: &str, item_level: i64, pricing: Pricing) -> Planner {
        let mut pairs = Vec::new();
        let influences: Vec<&str> = problem.influences.iter().map(String::as_str).filter(|i| INFLUENCES.contains(i)).collect();
        if influences.len() != 2 || !problem.fixed.is_empty() {
            return Planner { pairs };
        }
        let donor = |target: &Target, influence: &str| -> Option<Donor> {
            let influences = BTreeSet::from([influence.to_string()]);
            let tags = db.item_tags(base_id, &influences);
            let natural_tags = db.item_tags(base_id, &BTreeSet::new());
            let good: BTreeSet<String> = target
                .good
                .iter()
                .filter(|id| {
                    let m = &db.mods[id.as_str()];
                    ModDB::generation_weight(m, &tags) > 0.0 && ModDB::generation_weight(m, &natural_tags) == 0.0
                })
                .cloned()
                .collect();
            if good.is_empty() {
                return None;
            }
            let target = Target { good, ..target.clone() };
            let p = gear::component_problem_on(
                db,
                base_id,
                gear::ComponentGoal { targets: vec![target.clone()], ..Default::default() },
                item_level,
                &influences,
                pricing,
            );
            let mut donor = Donor::new(&db.bases[base_id].name, base_id, &[&target.name]);
            donor.problem = Some(p);
            Some(donor)
        };
        for i in 0..problem.targets.len() {
            for j in i + 1..problem.targets.len() {
                for assignment in [[influences[0], influences[1]], [influences[1], influences[0]]] {
                    let targets = [&problem.targets[i], &problem.targets[j]];
                    if targets[0].groups.intersection(&targets[1].groups).next().is_some() {
                        continue;
                    }
                    let Some(a) = donor(targets[0], assignment[0]) else {
                        continue;
                    };
                    let Some(b) = donor(targets[1], assignment[1]) else {
                        continue;
                    };
                    let mut state = problem.start();
                    state.rarity = RARE;
                    state.status[i] = GOOD;
                    state.status[j] = GOOD;
                    if [ponder_core::Side::Prefix, ponder_core::Side::Suffix]
                        .iter()
                        .any(|&side| problem.count(&state, side) > problem.side_limit(RARE, side))
                    {
                        continue;
                    }
                    let outcomes = problem.reforge_from(state);
                    pairs.push(Pair { targets: [i, j], influences: assignment.map(str::to_string), donors: [a, b], outcomes });
                }
            }
        }
        Planner { pairs }
    }

    pub fn leaves_mut(&mut self) -> impl Iterator<Item = &mut Donor> {
        self.pairs.iter_mut().flat_map(|p| &mut p.donors)
    }

    pub fn best(
        &self,
        problem: &Problem,
        pricing: Pricing,
        mut continuation: impl FnMut(&Pair, ponder_core::Recovery) -> Option<(Plan, Vec<crate::pipeline::SaleThreshold>, Option<String>)>,
    ) -> Option<Recipe> {
        let orb_cost = *pricing.prices.get("awakeners-orb")? + AWAKENER_SECONDS * pricing.settings.chaos_per_hour / 3600.0;
        self.pairs
            .iter()
            .filter_map(|pair| {
                let donors: Vec<_> = pair
                    .donors
                    .iter()
                    .zip(&pair.influences)
                    .map(|(d, influence)| {
                        Some(DonorRecipe {
                            base: d.base.clone(),
                            influence: influence.clone(),
                            target: d.carries[0].clone(),
                            cost: d.plan.as_ref()?.cost,
                            steps: ponder_core::steps(d.problem.as_ref()?, d.plan.as_ref()?, 0.02),
                            trade_url: None,
                            query: None,
                            modifiers: d.problem.as_ref()?.targets[0].good.clone(),
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;
                let donors: [DonorRecipe; 2] = donors.try_into().ok()?;
                let mut usage = std::collections::HashMap::new();
                *usage.entry("awakener's orb".into()).or_default() += 1.0;
                let mut hours = AWAKENER_SECONDS / 3600.0;
                for donor in &pair.donors {
                    let plan = donor.plan.as_ref()?;
                    hours += plan.hours;
                    for (action, count) in &plan.usage {
                        *usage.entry(action.clone()).or_default() += count;
                    }
                }
                let acquisition = donors.iter().map(|d| d.cost).sum::<f64>() + orb_cost;
                let recovery =
                    ponder_core::Recovery { initial: pair.outcomes.clone(), cost: acquisition, hours, sale: None, usage: usage.clone() };
                let (finishing, sale_thresholds, sale_error) = continuation(pair, recovery)?;
                let finish = finishing.cost;
                hours += finishing.hours;
                for (action, count) in &finishing.usage {
                    *usage.entry(action.clone()).or_default() += count;
                }
                let restart_conditions = pair
                    .outcomes
                    .iter()
                    .filter(|(p, state)| *p > 0.0 && !problem.done(state) && !finishing.policy.contains_key(state))
                    .map(|(_, state)| ponder_core::describe(problem, state))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                Some(Recipe {
                    cost: acquisition + finish,
                    orb_cost,
                    finish,
                    influences: pair.influences.clone(),
                    targets: pair.targets.map(|i| problem.targets[i].name.clone()),
                    donors,
                    reforge: pair.outcomes.to_vec(),
                    exact_reforge: pair.outcomes.is_exact(),
                    finish_steps: ponder_core::steps(problem, &finishing, 0.02),
                    usage,
                    hours,
                    restarts: finishing.restarts,
                    sales: finishing.sales,
                    sale_proceeds: finishing.sale_proceeds,
                    restart_conditions,
                    sale_thresholds,
                    sale_error,
                    finishing: Some(std::sync::Arc::new(finishing)),
                })
            })
            .min_by(|a, b| a.cost.total_cmp(&b.cost))
    }
}
