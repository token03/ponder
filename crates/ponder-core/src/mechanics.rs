use rustc_hash::FxHashMap;

use crate::action::{Action, Effect};
use crate::problem::Problem;
use crate::types::*;

#[derive(Clone, Copy)]
pub(crate) struct ItemRules {
    pub rare_limit: usize,
    pub notable_cap: usize,
    rare_counts: &'static [(usize, f64)],
}

impl ItemRules {
    pub fn new(kind: Kind, notable_cap: usize) -> ItemRules {
        let (rare_limit, rare_counts) = match kind {
            Kind::Jewel => (RARE_AFFIX_LIMIT_JEWEL, RARE_MOD_COUNTS_JEWEL.as_slice()),
            Kind::Gear => (RARE_AFFIX_LIMIT_GEAR, RARE_MOD_COUNTS_GEAR.as_slice()),
        };
        ItemRules { rare_limit, rare_counts, notable_cap }
    }

    pub fn limit(self, rarity: u8) -> usize {
        if rarity == MAGIC { 1 } else { self.rare_limit }
    }

    pub fn counts(self, rarity: u8) -> &'static [(usize, f64)] {
        const MAGIC_COUNTS: [(usize, f64); 2] = [(1, 1.0 - MAGIC_TWO_MOD_CHANCE), (2, MAGIC_TWO_MOD_CHANCE)];
        if rarity == MAGIC { &MAGIC_COUNTS } else { self.rare_counts }
    }
}

pub(crate) fn groups_overlap<T: PartialEq>(a: &[T], b: &[T]) -> bool {
    a.iter().any(|g| b.contains(g))
}

pub(crate) fn removable_side(side: Side, requested: Option<Side>, meta: u8) -> bool {
    requested.is_none_or(|s| s == side) && meta_locks(meta) != Some(side)
}

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
    pub(crate) fn choose(&mut self, cum: &[f64]) -> usize {
        let r = self.random() * cum[cum.len() - 1];
        cum.partition_point(|&c| c <= r).min(cum.len() - 1)
    }
}

/// Most mods an item carries while replaying: a rare holds 6, fractured and forced ones included.
const MAX_MODS: usize = 8;

/// (rarity, profile, the item's mods sorted and padded with `u16::MAX`, side + 1 or 0, the
/// metamod on the item)
type DrawKey = (u8, u16, [u16; MAX_MODS], u8, u8);

/// An action with its kind parsed and its profiles and forced mod numbered.
pub(crate) struct PreparedEffect<'p> {
    pub(crate) action: &'p Action,
    kind: Effect,
    profile: u16,
    first: Option<u16>,
    forced: Option<u16>,
}

pub(crate) struct Mechanics<'p> {
    rules: ItemRules,
    profile_weights: &'p std::collections::BTreeMap<String, std::collections::BTreeMap<String, f64>>,
    /// every mod a replay can place, pool first (only pool mods are drawn)
    pub(crate) mods: Vec<&'p Mod>,
    pool: usize,
    side: Vec<Side>,
    notable: Vec<bool>,
    pub(crate) fixed: Vec<bool>,
    groups: Vec<Vec<u32>>,
    /// which of `META_TAGS` each mod carries, as bits
    tags: Vec<u8>,
    /// profile ("base" first) -> pool mod -> weight
    weights: Vec<Vec<f64>>,
    profiles: Vec<String>,
    cache: FxHashMap<DrawKey, (Vec<u16>, Vec<f64>)>,
    rng: Rng,
}

impl<'p> Mechanics<'p> {
    pub(crate) fn new(problem: &'p Problem, seed: u64) -> Mechanics<'p> {
        let mut mods: Vec<&Mod> = problem.pool.iter().collect();
        let pool = mods.len();
        let mut fixed = vec![false; pool];
        for m in &problem.fixed {
            mods.push(m);
            fixed.push(true);
        }
        for m in problem.extra_mods.values() {
            if !mods.iter().any(|x| x.id == m.id) {
                mods.push(m);
                fixed.push(false);
            }
        }
        let mut group_ids: FxHashMap<&str, u32> = FxHashMap::default();
        let groups = mods
            .iter()
            .map(|m| {
                m.groups
                    .iter()
                    .map(|g| {
                        let next = group_ids.len() as u32;
                        *group_ids.entry(g.as_str()).or_insert(next)
                    })
                    .collect()
            })
            .collect();
        let tags = mods
            .iter()
            .map(|m| META_TAGS.iter().enumerate().filter(|(_, t)| m.tags.iter().any(|x| x == *t)).fold(0u8, |b, (i, _)| b | 1 << i))
            .collect();
        let mut sim = Mechanics {
            rules: ItemRules::new(problem.kind, problem.notable_cap),
            profile_weights: &problem.profiles,
            tags,
            side: mods.iter().map(|m| m.side).collect(),
            notable: mods.iter().map(|m| m.notable).collect(),
            groups,
            fixed,
            pool,
            mods,
            weights: Vec::new(),
            profiles: Vec::new(),
            cache: FxHashMap::default(),
            rng: Rng::new(seed),
        };
        sim.profile("base");
        sim
    }

    /// The number of a weight profile, its weights loaded on first use.
    fn profile(&mut self, name: &str) -> u16 {
        if let Some(i) = self.profiles.iter().position(|n| n == name) {
            return i as u16;
        }
        let weights = self.mods[..self.pool]
            .iter()
            .map(|m| if name == "base" { m.weight } else { self.profile_weights[name].get(&m.id).copied().unwrap_or(0.0) })
            .collect();
        self.weights.push(weights);
        self.profiles.push(name.to_string());
        (self.profiles.len() - 1) as u16
    }

    pub(crate) fn act(&mut self, a: &'p Action) -> PreparedEffect<'p> {
        let kind = a.kind.effect();
        let forced = a
            .forced
            .as_deref()
            .filter(|f| !f.is_empty())
            .map(|f| self.mods.iter().position(|x| x.id == f).expect("forced mods are numbered") as u16);
        let profile = self.profile(&a.profile);
        let first = a.first_profile.as_deref().filter(|f| !f.is_empty()).map(|f| self.profile(f));
        PreparedEffect { action: a, kind, profile, first, forced }
    }

    /// Apply `act` to `item` and its metamod; the new rarity.
    pub(crate) fn apply(&mut self, act: &PreparedEffect, rarity: u8, item: &mut Vec<u16>, meta: &mut u8) -> u8 {
        match act.kind {
            Effect::Roll(r) => {
                self.roll(r, act, item, 0);
                *meta = 0;
                r
            }
            Effect::Add(r, side) => {
                if let Some(m) = self.draw(r, item, 0, side, *meta) {
                    item.push(m);
                }
                r
            }
            Effect::Annul(side) => {
                self.remove(item, side, meta, true);
                rarity
            }
            Effect::Reforge => {
                match meta_locks(*meta) {
                    Some(keep) => {
                        item.retain(|&k| self.side[k as usize] == keep || self.fixed[k as usize]);
                        let n = self.count(self.rules.counts(RARE));
                        let mut profile = act.first.unwrap_or(act.profile);
                        while item.len() < n {
                            let Some(m) = self.draw(RARE, item, profile, None, 0) else { break };
                            profile = act.profile;
                            item.push(m);
                        }
                    }
                    None => self.roll(RARE, act, item, *meta),
                }
                *meta = 0;
                RARE
            }
            Effect::Augment => {
                if let Some(m) = self.draw(RARE, item, act.profile, None, *meta) {
                    self.remove(item, None, meta, false);
                    item.push(m);
                }
                rarity
            }
            Effect::Meta(m) => {
                *meta = m;
                rarity
            }
            Effect::Unmeta => {
                *meta = 0;
                rarity
            }
            Effect::Scour => {
                let keep = meta_locks(*meta).expect("scour is offered on a locked item");
                item.retain(|&k| self.side[k as usize] == keep || self.fixed[k as usize]);
                *meta = 0;
                if item.is_empty() { NORMAL } else { RARE }
            }
        }
    }

    /// Annul one random mod: not fractured, not on a locked side, not one "cannot roll" protects;
    /// the metamod itself is a candidate.
    fn remove(&mut self, item: &mut Vec<u16>, side: Option<Side>, meta: &mut u8, protect_tags: bool) {
        let tag = if protect_tags { meta_tag(*meta).map_or(0, |t| 1u8 << t) } else { 0 };
        let ok = |k: u16| {
            let k = k as usize;
            !self.fixed[k] && removable_side(self.side[k], side, *meta) && self.tags[k] & tag == 0
        };
        let n = item.iter().filter(|&&k| ok(k)).count();
        let meta_ok = meta_side(*meta).is_some_and(|ms| side.is_none_or(|s| s == ms));
        let total = n + meta_ok as usize;
        if total == 0 {
            return;
        }
        let pick = ((self.rng.random() * total as f64) as usize).min(total - 1);
        if pick == n {
            *meta = 0;
        } else {
            let at = item.iter().enumerate().filter(|&(_, &k)| ok(k)).nth(pick).expect("pick is under the count").0;
            item.remove(at);
        }
    }

    fn count(&mut self, counts: &[(usize, f64)]) -> usize {
        let total: f64 = counts.iter().map(|c| c.1).sum();
        let mut r = self.rng.random() * total;
        for &(n, p) in counts {
            if r < p {
                return n;
            }
            r -= p;
        }
        counts[counts.len() - 1].0
    }

    fn roll(&mut self, rarity: u8, act: &PreparedEffect, item: &mut Vec<u16>, first_meta: u8) {
        let n = self.count(self.rules.counts(rarity));
        item.retain(|&k| self.fixed[k as usize]);
        if let Some(f) = act.forced {
            item.push(f);
        }
        let mut profile = if act.forced.is_some() { act.profile } else { act.first.unwrap_or(act.profile) };
        if act.first.is_some() && act.forced.is_none() && meta_tag(first_meta).is_some() {
            profile = self.restricted_profile(profile, first_meta);
        }
        while item.len() < n {
            let Some(m) = self.draw(rarity, item, profile, None, 0) else { break };
            profile = act.profile;
            item.push(m);
        }
    }

    fn restricted_profile(&mut self, profile: u16, meta: u8) -> u16 {
        let name = format!("{}:metamod:{meta}", self.profiles[profile as usize]);
        if let Some(i) = self.profiles.iter().position(|p| p == &name) {
            return i as u16;
        }
        let tag = 1 << meta_tag(meta).expect("a modifier restriction");
        let weights =
            self.weights[profile as usize].iter().enumerate().map(|(i, &w)| if self.tags[i] & tag == 0 { w } else { 0.0 }).collect();
        self.weights.push(weights);
        self.profiles.push(name);
        (self.profiles.len() - 1) as u16
    }

    /// One mod added to `item` from `profile` (on `side` only, if given). The metamod holds its
    /// slot and keeps its tag off.
    fn draw(&mut self, rarity: u8, item: &[u16], profile: u16, side: Option<Side>, meta: u8) -> Option<u16> {
        assert!(item.len() <= MAX_MODS, "an item with {} mods", item.len());
        let mut sorted = [u16::MAX; MAX_MODS];
        sorted[..item.len()].copy_from_slice(item);
        sorted.sort_unstable();
        let key = (rarity, profile, sorted, side.map_or(0, |s| s.index() as u8 + 1), meta);
        let (pool, cum) = self.cache.entry(key).or_insert_with(|| {
            let limit = self.rules.limit(rarity);
            let skip = meta_tag(meta).map_or(0, |t| 1u8 << t);
            let on = |s: Side| item.iter().filter(|&&k| self.side[k as usize] == s).count() + (meta_side(meta) == Some(s)) as usize;
            let room = [on(Side::Prefix) < limit, on(Side::Suffix) < limit];
            let taken: Vec<u32> = item.iter().flat_map(|&k| self.groups[k as usize].iter().copied()).collect();
            let cap_hit = item.iter().filter(|&&k| self.notable[k as usize]).count() >= self.rules.notable_cap;
            let weights = &self.weights[profile as usize];
            let mut pool = Vec::new();
            let mut cum = Vec::new();
            let mut acc = 0.0;
            for k in 0..self.pool {
                let w = weights[k];
                if w > 0.0
                    && side.is_none_or(|s| self.side[k] == s)
                    && room[self.side[k].index()]
                    && !groups_overlap(&self.groups[k], &taken)
                    && !(self.notable[k] && cap_hit)
                    && self.tags[k] & skip == 0
                {
                    acc += w;
                    pool.push(k as u16);
                    cum.push(acc);
                }
            }
            (pool, cum)
        });
        if pool.is_empty() { None } else { Some(pool[self.rng.choose(cum)]) }
    }
}
