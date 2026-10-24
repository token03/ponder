//! Replaying a policy on real mods (exact mod groups, no lumping) to check the lumped numbers.

use std::collections::{BTreeSet, HashMap};

use crate::problem::Problem;
use crate::solve::{Plan, step_cost};
use crate::types::*;

/// A small seedable generator (SplitMix64): the stream is not Python's, only the statistics matter.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn random(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Index chosen by cumulative weights.
    fn choose(&mut self, cum: &[f64]) -> usize {
        let r = self.random() * cum[cum.len() - 1];
        cum.partition_point(|&c| c <= r).min(cum.len() - 1)
    }
}

type DrawKey<'a> = (u8, &'a str, Vec<&'a str>, Option<Side>, usize);

struct Sim<'a> {
    problem: &'a Problem,
    weights: HashMap<&'a str, HashMap<String, f64>>,
    cache: HashMap<DrawKey<'a>, (Vec<&'a Mod>, Vec<f64>)>,
    rng: Rng,
}

/// Average chaos spent replaying `plan` with real mods and exact group exclusion.
///
/// Panics if the policy has no action for a state the replay reaches.
pub fn simulate(problem: &Problem, plan: &Plan, prices: &Prices, settings: &Settings, runs: usize, seed: u64, max_steps: usize) -> f64 {
    let by_name: HashMap<&str, (&Action, f64)> = problem.actions.iter().map(|a| (a.name.as_str(), (a, step_cost(a, prices, settings)))).collect();
    let mut sim = Sim { problem, weights: HashMap::new(), cache: HashMap::new(), rng: Rng::new(seed) };
    let mut total = 0.0;
    for _ in 0..runs {
        let (mut rarity, mut mods): (u8, Vec<&Mod>) = (NORMAL, problem.fixed.iter().collect());
        for _ in 0..max_steps {
            let state = problem.state_of(rarity, mods.iter().copied());
            if problem.done(&state) {
                break;
            }
            let name = plan.policy.get(&state).unwrap_or_else(|| panic!("the policy has no action for {state:?}"));
            let (a, cost) = by_name[name.as_str()];
            total += cost;
            (rarity, mods) = sim.apply(a, rarity, mods);
        }
    }
    total / runs as f64
}

impl<'a> Sim<'a> {
    fn apply(&mut self, a: &'a Action, rarity: u8, mut mods: Vec<&'a Mod>) -> (u8, Vec<&'a Mod>) {
        let problem = self.problem;
        let fixed = |m: &Mod| problem.fixed.contains(m);
        match a.kind.as_str() {
            "sell" => (NORMAL, problem.fixed.iter().collect()),
            "transmute" | "alt" | "reroll_magic" => (MAGIC, self.roll(MAGIC, a)),
            "reroll_rare" => (RARE, self.roll(RARE, a)),
            "aug" | "regal" | "exalt" => {
                let rarity = if a.kind == "aug" { MAGIC } else { RARE };
                if let Some(m) = self.draw(rarity, &mods, "base", None, 0) {
                    mods.push(m);
                }
                (rarity, mods)
            }
            "exalt_prefix" | "exalt_suffix" => {
                let side = if a.kind == "exalt_prefix" { Side::Prefix } else { Side::Suffix };
                if let Some(m) = self.draw(RARE, &mods, "base", Some(side), 0) {
                    mods.push(m);
                }
                (rarity, mods)
            }
            "annul" | "annul_prefix" | "annul_suffix" => {
                let side = a.kind.strip_prefix("annul_").map(|s| if s == "prefix" { Side::Prefix } else { Side::Suffix });
                let removable: Vec<usize> = (0..mods.len()).filter(|&i| !fixed(mods[i]) && side.is_none_or(|s| mods[i].side == s)).collect();
                if !removable.is_empty() {
                    mods.remove(removable[(self.rng.random() * removable.len() as f64) as usize]);
                }
                (rarity, mods)
            }
            "keep_prefixes" | "keep_suffixes" => {
                let side = if a.kind == "keep_prefixes" { Side::Prefix } else { Side::Suffix };
                let kept: Vec<&Mod> = mods.into_iter().filter(|m| m.side == side || fixed(m)).collect();
                (if kept.is_empty() { NORMAL } else { RARE }, kept)
            }
            "lockreroll_prefixes" | "lockreroll_suffixes" => {
                let keep = if a.kind == "lockreroll_prefixes" { Side::Prefix } else { Side::Suffix };
                let mut kept: Vec<&Mod> = mods.into_iter().filter(|m| m.side == keep || fixed(m)).collect();
                let counts = problem.rare_counts();
                let n = self.count(counts);
                let mut first = a.first_profile.as_deref().filter(|f| !f.is_empty());
                while kept.len() + 1 < n {
                    // + the metamod
                    let profile = first.unwrap_or(&a.profile);
                    let Some(m) = self.draw(RARE, &kept, profile, Some(keep.other()), 1) else { break };
                    first = None;
                    kept.push(m);
                }
                (RARE, kept)
            }
            other => panic!("unknown action kind {other:?}"),
        }
    }

    fn count(&mut self, counts: &[(usize, f64)]) -> usize {
        let cum: Vec<f64> = counts.iter().scan(0.0, |acc, c| { *acc += c.1; Some(*acc) }).collect();
        counts[self.rng.choose(&cum)].0
    }

    fn roll(&mut self, rarity: u8, a: &'a Action) -> Vec<&'a Mod> {
        let problem = self.problem;
        let n = if rarity == MAGIC {
            self.count(&[(1, 1.0 - MAGIC_TWO_MOD_CHANCE), (2, MAGIC_TWO_MOD_CHANCE)])
        } else {
            self.count(problem.rare_counts())
        };
        let mut mods: Vec<&Mod> = problem.fixed.iter().collect();
        let forced = a.forced.as_deref().filter(|f| !f.is_empty());
        if let Some(f) = forced {
            mods.push(&problem.mods()[f]);
        }
        let mut first = if forced.is_some() { None } else { a.first_profile.as_deref().filter(|f| !f.is_empty()) };
        while mods.len() < n {
            let Some(m) = self.draw(rarity, &mods, first.unwrap_or(&a.profile), None, 0) else { break };
            first = None;
            mods.push(m);
        }
        mods
    }

    fn draw(&mut self, rarity: u8, mods: &[&'a Mod], profile: &'a str, side: Option<Side>, occupied: usize) -> Option<&'a Mod> {
        let problem = self.problem;
        let mut ids: Vec<&str> = mods.iter().map(|m| m.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        let key = (rarity, profile, ids, side, occupied);
        if !self.cache.contains_key(&key) {
            let limit = if rarity == MAGIC { 1 } else { problem.limit() };
            let taken: BTreeSet<&str> = mods.iter().flat_map(|m| m.groups.iter().map(String::as_str)).collect();
            let notables = mods.iter().filter(|m| m.notable).count();
            let weights = self.weights.entry(profile).or_insert_with(|| problem.weights(profile));
            let pool: Vec<&Mod> = problem
                .pool
                .iter()
                .filter(|m| {
                    weights.get(&m.id).copied().unwrap_or(0.0) > 0.0
                        && side.is_none_or(|s| m.side == s)
                        && mods.iter().filter(|x| x.side == m.side).count() + occupied < limit
                        && !m.groups.iter().any(|g| taken.contains(g.as_str()))
                        && !(m.notable && notables >= problem.notable_cap)
                })
                .collect();
            let cum = pool.iter().scan(0.0, |acc, m| { *acc += weights[&m.id]; Some(*acc) }).collect();
            self.cache.insert(key.clone(), (pool, cum));
        }
        let (pool, cum) = &self.cache[&key];
        if pool.is_empty() { None } else { Some(pool[self.rng.choose(cum)]) }
    }
}
