use serde::{Deserialize, Serialize};

use crate::action::{ActionKind, step_cost};
use crate::problem::Problem;
use crate::types::*;

pub const FRACTURE_ORB: &str = "fracturing-orb";
pub const FRACTURE_MIN_MODS: usize = 4; // a Fracturing Orb needs a rare with at least this many mods
pub const FRACTURE_SECONDS: f64 = 3.0;

/// Make your own fractured base: reroll until the mod lands with few enough mods, then fracture.
///
/// A Fracturing Orb fixes one random mod of `k`, so it hits the wanted one
/// 1 time in k; a miss ruins the base and the loop starts over on a new one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelfFracture {
    pub target: usize,        // index into the problem's targets
    pub action: String,       // the reroll to spam
    pub max_mods: usize,      // fracture only when the item has at most this many mods
    pub hit: f64,             // chance one reroll lands the mod with few enough mods
    pub fracture_chance: f64, // chance the orb then picks it
    pub cost: f64,            // expected chaos (currency and clicking) to a base with the mod fractured
    pub hours: f64,
}

impl SelfFracture {
    pub fn rerolls(&self) -> f64 {
        1.0 / (self.hit * self.fracture_chance)
    }

    pub fn orbs(&self) -> f64 {
        1.0 / self.fracture_chance
    }
}

/// The cheapest way to roll and fracture `problem.targets[target]` yourself (gear and jewels: a
/// jewel holds 4 mods at most, so it is fractured only with all 4).
///
/// Each rare reroll (chaos, scour+alch, essences, fossils, Harvest) gives,
/// from its exact roll distribution, the chance P_k of the mod landing at a
/// good tier on an item with k mods. Fracturing only at k <= K costs
/// (reroll / P(k <= K) + orb) / E[1/k | k <= K]; the best reroll and K win.
pub fn self_fracture(problem: &Problem, target: usize, prices: &Prices, settings: &Settings) -> Option<SelfFracture> {
    if !problem.fixed.is_empty() || !prices.contains_key(FRACTURE_ORB) {
        return None;
    }
    let orb = prices[FRACTURE_ORB] + FRACTURE_SECONDS * settings.chaos_per_hour / 3600.0;
    let mut best: Option<SelfFracture> = None;
    for a in &problem.actions {
        if !(a.kind == ActionKind::RerollRare || a.kind == ActionKind::Reforge)
            || !a.from_rarity.contains(&RARE)
            || a.cost.iter().any(|(pid, _)| !prices.contains_key(pid))
        {
            continue;
        }
        let mut by_k: Vec<(usize, f64)> = Vec::new();
        let roll = problem.roll(a)?;
        if !roll.is_exact() {
            continue;
        }
        for &(p, s) in roll.iter() {
            let k = problem.total(&s);
            if s.status[target] == GOOD && k >= FRACTURE_MIN_MODS {
                match by_k.iter_mut().find(|x| x.0 == k) {
                    Some(x) => x.1 += p,
                    None => by_k.push((k, p)),
                }
            }
        }
        let step = step_cost(a, prices, settings);
        let mut ks: Vec<usize> = by_k.iter().map(|x| x.0).collect();
        ks.sort_unstable();
        let mut hit = 0.0;
        for k in ks {
            hit += by_k.iter().find(|x| x.0 == k).unwrap().1;
            let chance = by_k.iter().filter(|x| x.0 <= k).map(|&(j, p)| p / j as f64).sum::<f64>() / hit;
            let cost = (step / hit + orb) / chance;
            if best.as_ref().is_none_or(|b| cost < b.cost) {
                let hours = (a.seconds / hit + FRACTURE_SECONDS) / chance / 3600.0;
                best = Some(SelfFracture { target, action: a.name.clone(), max_mods: k, hit, fracture_chance: chance, cost, hours });
            }
        }
    }
    best
}
