use rustc_hash::FxHashMap;

use crate::action::{Action, Effect};
use crate::problem::Problem;
use crate::types::*;

#[derive(Clone, Copy)]
pub(crate) struct ItemRules {
    pub rare_limits: [usize; 2],
    magic_base: usize,
    pub notable_cap: usize,
    rare_counts: &'static [(usize, f64)],
}

impl ItemRules {
    pub fn new(kind: Kind, notable_cap: usize, limits: Option<[usize; 2]>) -> ItemRules {
        let (rare_limit, rare_counts) = match kind {
            Kind::Jewel => (RARE_AFFIX_LIMIT_JEWEL, RARE_MOD_COUNTS_JEWEL.as_slice()),
            Kind::Gear => (RARE_AFFIX_LIMIT_GEAR, RARE_MOD_COUNTS_GEAR.as_slice()),
        };
        ItemRules { magic_base: rare_limit - 1, rare_limits: limits.unwrap_or([rare_limit; 2]), rare_counts, notable_cap }
    }

    pub fn limit(self, rarity: u8, side: Side) -> usize {
        let limit = self.rare_limits[side.index()];
        if rarity == MAGIC { (limit as isize - self.magic_base as isize).max(0) as usize } else { limit }
    }

    pub fn counts(self, rarity: u8) -> Vec<(usize, f64)> {
        const MAGIC_COUNTS: [(usize, f64); 2] = [(1, 1.0 - MAGIC_TWO_MOD_CHANCE), (2, MAGIC_TWO_MOD_CHANCE)];
        let counts = if rarity == MAGIC { &MAGIC_COUNTS[..] } else { self.rare_counts };
        let cap = self.limit(rarity, Side::Prefix) + self.limit(rarity, Side::Suffix);
        let mut out: Vec<(usize, f64)> = Vec::new();
        for &(n, p) in counts {
            let n = n.min(cap);
            if let Some(entry) = out.iter_mut().find(|e| e.0 == n) {
                entry.1 += p;
            } else {
                out.push((n, p));
            }
        }
        out
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

/// (rarity, profile, the item's draw classes sorted and padded with `u16::MAX`, side + 1 or 0, the
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
    problem: &'p Problem,
    pub(crate) offers: Vec<u16>,
    pub(crate) veiled: Option<u16>,
    pub(crate) pending_side: Option<Side>,
    profile_weights: &'p std::collections::BTreeMap<String, WeightProfile>,
    /// every mod a replay can place, pool first (only pool mods are drawn)
    pub(crate) mods: Vec<&'p Mod>,
    pool: usize,
    side: Vec<Side>,
    notable: Vec<bool>,
    pub(crate) fixed: Vec<bool>,
    groups: Vec<Vec<u32>>,
    draw_classes: Vec<u16>,
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
        let groups: Vec<Vec<u32>> = mods
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
        let mut classes = FxHashMap::default();
        let draw_classes = mods
            .iter()
            .zip(&groups)
            .map(|(m, groups)| {
                let mut groups = groups.clone();
                groups.sort_unstable();
                groups.dedup();
                let next = classes.len() as u16;
                *classes.entry((m.side, m.notable, groups)).or_insert(next)
            })
            .collect();
        let tags = mods
            .iter()
            .map(|m| META_TAGS.iter().enumerate().filter(|(_, t)| m.tags.iter().any(|x| x == *t)).fold(0u8, |b, (i, _)| b | 1 << i))
            .collect();
        let mut sim = Mechanics {
            problem,
            offers: Vec::new(),
            veiled: None,
            pending_side: None,
            rules: ItemRules::new(problem.kind, problem.notable_cap, problem.affix_limits),
            profile_weights: &problem.profiles,
            tags,
            side: mods.iter().map(|m| m.side).collect(),
            notable: mods.iter().map(|m| m.notable).collect(),
            groups,
            draw_classes,
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
                        let n = self.count(&self.rules.counts(RARE));
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
            Effect::RerollSide(side) => {
                item.retain(|&k| self.side[k as usize] != side || self.fixed[k as usize]);
                if meta_side(*meta) == Some(side) {
                    *meta = 0;
                }
                let n = self.count(&self.rules.counts(RARE));
                while item.len() + ((*meta != 0) as usize) < n {
                    let Some(m) = self.draw(RARE, item, 0, Some(side), *meta) else { break };
                    item.push(m);
                }
                RARE
            }
            Effect::RemoveAdd(side) => {
                self.remove(item, Some(side), meta, false);
                if let Some(m) = self.draw(RARE, item, 0, Some(side.other()), *meta) {
                    item.push(m);
                }
                rarity
            }
            Effect::RandomMeta => {
                let mut state = State::new(rarity, &[], &[]).with_meta(*meta);
                let mut candidates = Vec::new();
                for m in [META_LOCK_PREFIXES, META_LOCK_SUFFIXES, META_NO_ATTACK, META_NO_CASTER, META_MULTIMOD] {
                    let side = meta_side(m).unwrap();
                    let on = item.iter().filter(|&&k| self.side[k as usize] == side).count();
                    if state.meta == 0 && on < self.rules.limit(rarity, side) {
                        candidates.push(m);
                    }
                }
                if !candidates.is_empty() {
                    state.meta = candidates[(self.rng.random() * candidates.len() as f64) as usize];
                    *meta = state.meta;
                }
                rarity
            }
            Effect::Veil(chaos) => {
                if chaos {
                    let keep = meta_locks(*meta);
                    item.retain(|&k| self.fixed[k as usize] || keep == Some(self.side[k as usize]));
                    *meta = 0;
                } else {
                    self.remove(item, None, meta, false);
                }
                let sides: Vec<Side> = [Side::Prefix, Side::Suffix]
                    .into_iter()
                    .filter(|&side| {
                        self.problem.unveils.iter().any(|m| m.side == side && m.weight > 0.0)
                            && item.iter().filter(|&&k| self.side[k as usize] == side).count() + ((meta_side(*meta) == Some(side)) as usize)
                                < self.rules.limit(RARE, side)
                    })
                    .collect();
                let side = sides[(self.rng.random() * sides.len() as f64) as usize];
                let placeholder = if side == Side::Prefix { "VeiledPrefix" } else { "VeiledSuffix" };
                let index = self.mods.iter().position(|m| m.id == placeholder).expect("a veil placeholder") as u16;
                item.push(index);
                if chaos {
                    let n = self.count(&self.rules.counts(RARE));
                    while item.len() < n {
                        let Some(m) = self.draw(RARE, item, 0, None, *meta) else { break };
                        item.push(m);
                    }
                }
                item.retain(|&k| k != index);
                self.veiled = None;
                self.pending_side = Some(side);
                self.offers.clear();
                if self.problem.blockers.is_empty() {
                    self.draw_offers(item, side, None);
                }
                RARE
            }
            Effect::RevealUnveil(blocker) => {
                let side = self.pending_side.expect("an unrevealed modifier");
                self.draw_offers(item, side, blocker.checked_sub(1).map(usize::from));
                rarity
            }
            Effect::Unveil(class) => {
                let k = *self
                    .offers
                    .iter()
                    .find(|&&k| {
                        let (i, v) = self.problem.classify_mod(self.mods[k as usize]);
                        let code = if i >= 0 { i as u8 * 2 + v - 1 } else { UNVEIL_TARGET_CLASSES + v };
                        code == class
                    })
                    .expect("the selected modifier was offered");
                item.push(k);
                self.veiled = Some(k);
                self.pending_side = None;
                self.offers.clear();
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
            Effect::ProtectedScour { side: keep, .. } => {
                item.retain(|&k| self.side[k as usize] == keep || self.fixed[k as usize]);
                *meta = 0;
                if item.is_empty() { NORMAL } else { RARE }
            }
            Effect::FinishBench => {
                item.push(act.forced.expect("a terminal bench modifier"));
                rarity
            }
        }
    }

    fn draw_offers(&mut self, item: &[u16], side: Side, blocker: Option<usize>) {
        self.offers.clear();
        for _ in 0..3 {
            let candidates: Vec<(u16, f64)> = self
                .problem
                .unveils
                .iter()
                .filter(|m| m.side == side && m.weight > 0.0)
                .filter(|m| blocker.is_none_or(|i| !groups_overlap(&m.groups, &self.problem.blockers[i].groups)))
                .filter_map(|m| {
                    let k = self.mods.iter().position(|x| x.id == m.id)? as u16;
                    item.iter()
                        .chain(&self.offers)
                        .all(|&j| !groups_overlap(&self.groups[k as usize], &self.groups[j as usize]))
                        .then_some((k, m.weight))
                })
                .collect();
            if candidates.is_empty() {
                break;
            }
            let mut total = 0.0;
            let cumulative: Vec<f64> = candidates
                .iter()
                .map(|(_, weight)| {
                    total += weight;
                    total
                })
                .collect();
            self.offers.push(candidates[self.rng.choose(&cumulative)].0);
        }
    }

    pub(crate) fn project_veil(&self, state: &mut State, item: &[u16]) {
        let class = |k: u16| {
            let (i, v) = self.problem.classify_mod(self.mods[k as usize]);
            if i >= 0 { i as u8 * 2 + v - 1 } else { UNVEIL_TARGET_CLASSES + v }
        };
        if let Some(side) = self.pending_side {
            state.veiled = VEILED_PREFIX + side.index() as u8;
            state.unveil = self.offers.iter().fold(0, |mask, &k| mask | 1 << class(k));
        } else if let Some(k) = self.veiled.filter(|k| item.contains(k)) {
            state.veiled = class(k) + 1;
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
        let n = self.count(&self.rules.counts(rarity));
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
        for (slot, &mod_index) in sorted.iter_mut().zip(item) {
            *slot = self.draw_classes[mod_index as usize];
        }
        sorted.sort_unstable();
        let key = (rarity, profile, sorted, side.map_or(0, |s| s.index() as u8 + 1), meta);
        let (pool, cum) = self.cache.entry(key).or_insert_with(|| {
            let limit = [self.rules.limit(rarity, Side::Prefix), self.rules.limit(rarity, Side::Suffix)];
            let skip = meta_tag(meta).map_or(0, |t| 1u8 << t);
            let on = |s: Side| item.iter().filter(|&&k| self.side[k as usize] == s).count() + (meta_side(meta) == Some(s)) as usize;
            let room = [on(Side::Prefix) < limit[0], on(Side::Suffix) < limit[1]];
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProblemSpec;

    #[test]
    fn tiers_with_identical_draw_constraints_share_the_same_sampling_table() {
        let pool = vec![
            Mod::new("low", Side::Prefix, 1.0, &["life"], false, "", &[]),
            Mod::new("high", Side::Prefix, 4.0, &["life"], false, "", &["attack"]),
            Mod::new("damage", Side::Prefix, 2.0, &["damage"], false, "", &[]),
            Mod::new("resistance", Side::Suffix, 3.0, &["resistance"], false, "", &[]),
        ];
        let problem = ProblemSpec::new(pool, vec![], 0, vec![], Kind::Gear).compile();
        let mut sim = Mechanics::new(&problem, 7);
        let low: Vec<_> = (0..1000).map(|_| sim.draw(RARE, &[0], 0, None, 0)).collect();
        sim.rng = Rng::new(7);
        let high: Vec<_> = (0..1000).map(|_| sim.draw(RARE, &[1], 0, None, 0)).collect();
        assert_eq!(low, high);
        assert_eq!(sim.cache.len(), 1);
        assert!(low.iter().all(|m| matches!(m, Some(2 | 3))));
    }
}
