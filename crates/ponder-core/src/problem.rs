//! The crafting problem and its mechanics: what each currency does to a lumped state.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::types::*;

fn default_kind() -> Kind {
    Kind::Gear
}

fn default_notable_cap() -> usize {
    99
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Problem {
    pub pool: Vec<Mod>,
    pub targets: Vec<Target>,
    pub need: usize,
    pub actions: Vec<Action>,
    #[serde(default = "default_kind")]
    pub kind: Kind,
    #[serde(default = "default_notable_cap")]
    pub notable_cap: usize,
    #[serde(default)]
    pub fixed: Vec<Mod>, // fractured mods, never removed
    #[serde(default)]
    pub profiles: BTreeMap<String, BTreeMap<String, f64>>, // name -> mod id -> weight
    #[serde(default)]
    pub extra_mods: BTreeMap<String, Mod>, // forced mods outside the pool
    #[serde(default)]
    pub reserve: BTreeMap<Side, usize>, // side -> slots left open for bench crafts at the end
    #[serde(default)]
    pub filler: BTreeSet<String>, // mods that count as filler (e.g. any resistance at T3+)
    #[serde(default)]
    pub filler_need: usize, // how many filler mods the finished item needs, on top of the targets
    #[serde(default)]
    pub filler_sides: Option<(usize, usize)>, // (prefix, suffix) filler each side needs; None: any mix of filler_need
    #[serde(default)]
    pub junk_cap: Option<(usize, usize)>, // most other mods allowed (prefix, suffix): recombinator donors want clean items
    #[serde(default)]
    pub begin: Option<State>, // where crafting starts when not a fresh base: a bought or dropped item (`state_of`)
    #[serde(skip)]
    caches: Caches,
}

/// What a reroll's outcome depends on. A forced (essence) mod matters only through what it
/// counts as, so every essence forcing an unwanted prefix rolls the same distribution.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RollKey {
    Magic(String),
    Rare(String, Option<String>, Option<(i32, u8)>),
}

type LockKey = (u8, u32, u32, State);

/// Derived data, built on first use and never invalidated (as Python's `cached_property`s):
/// edit `pool`, `targets` or `fixed` only before the problem is first used, or `reset_caches`.
#[derive(Default)]
struct Caches {
    compiled: OnceLock<Compiled>,
    rolls: Mutex<HashMap<RollKey, Arc<Vec<Outcome>>>>,
    locks: Mutex<FxHashMap<LockKey, Arc<Vec<Outcome>>>>,
}

impl Clone for Caches {
    fn clone(&self) -> Caches {
        Caches::default()
    }
}

impl std::fmt::Debug for Caches {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Caches")
    }
}

/// `{(target, GOOD|BLOCKED): weight}` and `{junk slot: (weight, removed per draw)}` of one profile.
pub(crate) struct Lumped {
    tw: Vec<[f64; 2]>,
    junk: [(f64, f64); JUNK_SLOTS],
}

pub(crate) struct Compiled {
    mods: HashMap<String, Mod>,
    base: State,
    t_side: Vec<Side>,
    t_notable: Vec<bool>,
    profile_names: Vec<String>, // "base" first
    lumped: Vec<Lumped>,
}

/// How an action's outcomes are computed, parsed once from `Action.kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActionKind {
    Roll,
    Aug,
    Regal,
    Exalt,
    Annul,
    ExaltSide(Side),
    AnnulSide(Side),
    Keep(Side),
    LockReroll(Side),
    Sell,
    Other,
}

impl ActionKind {
    fn parse(kind: &str) -> ActionKind {
        use ActionKind::*;
        let side = |s: &str| if s.starts_with("prefix") { Side::Prefix } else { Side::Suffix };
        match kind {
            "transmute" | "alt" | "reroll_magic" | "reroll_rare" => Roll,
            "aug" => Aug,
            "regal" => Regal,
            "exalt" => Exalt,
            "annul" => Annul,
            "exalt_prefix" | "exalt_suffix" => ExaltSide(side(&kind[6..])),
            "annul_prefix" | "annul_suffix" => AnnulSide(side(&kind[6..])),
            "keep_prefixes" | "keep_suffixes" => Keep(side(&kind[5..])),
            "lockreroll_prefixes" | "lockreroll_suffixes" => LockReroll(side(&kind[11..])),
            "sell" => Sell,
            _ => Other,
        }
    }
}

/// An action with everything the hot loop needs resolved.
pub(crate) struct ActionC<'a> {
    pub kind: ActionKind,
    mask: u8,
    profile: usize,
    first: Option<usize>,
    roll: Option<Arc<Vec<Outcome>>>,
    sale: Option<&'a Sale>,
}

pub(crate) enum Res {
    Empty,
    /// The outcomes are in the caller's scratch vector.
    Fresh,
    Shared(Arc<Vec<Outcome>>),
}

/// An insertion-ordered distribution over states (Python dicts keep order; so do the sums).
struct Dist {
    items: Vec<(State, f64)>,
    index: FxHashMap<State, usize>,
}

impl Dist {
    fn new() -> Dist {
        Dist { items: Vec::new(), index: FxHashMap::default() }
    }

    fn one(s: State) -> Dist {
        let mut d = Dist::new();
        d.add(s, 1.0);
        d
    }

    fn add(&mut self, s: State, p: f64) {
        match self.index.get(&s) {
            Some(&i) => self.items[i].1 += p,
            None => {
                self.index.insert(s, self.items.len());
                self.items.push((s, p));
            }
        }
    }
}

fn classify_in(targets: &[Target], filler: &BTreeSet<String>, m: &Mod) -> (i32, u8) {
    for (i, t) in targets.iter().enumerate() {
        if t.good.contains(&m.id) {
            return (i as i32, GOOD);
        }
        if m.groups.iter().any(|g| t.groups.contains(g)) {
            return (i as i32, BLOCKED);
        }
    }
    if filler.contains(&m.id) {
        return (-1, if m.side == Side::Prefix { 4 } else { 5 });
    }
    (-1, (if m.side == Side::Prefix { 0 } else { 2 }) + if m.notable { 0 } else { 1 })
}

impl Problem {
    pub fn new(pool: Vec<Mod>, targets: Vec<Target>, need: usize, actions: Vec<Action>, kind: Kind) -> Problem {
        Problem {
            pool,
            targets,
            need,
            actions,
            kind,
            notable_cap: default_notable_cap(),
            fixed: Vec::new(),
            profiles: BTreeMap::new(),
            extra_mods: BTreeMap::new(),
            reserve: BTreeMap::new(),
            filler: BTreeSet::new(),
            filler_need: 0,
            filler_sides: None,
            junk_cap: None,
            begin: None,
            caches: Caches::default(),
        }
    }

    /// Forget everything derived from `pool`, `targets`, `fixed`, `profiles` and `filler`.
    pub fn reset_caches(&mut self) {
        self.caches = Caches::default();
    }

    pub(crate) fn c(&self) -> &Compiled {
        self.caches.compiled.get_or_init(|| self.compile())
    }

    fn compile(&self) -> Compiled {
        assert!(self.targets.len() <= MAX_TARGETS, "at most {MAX_TARGETS} targets");
        let mut mods = HashMap::new();
        for (k, m) in &self.extra_mods {
            mods.insert(k.clone(), m.clone());
        }
        for m in self.pool.iter().chain(&self.fixed) {
            mods.insert(m.id.clone(), m.clone());
        }
        let mut base = State::new(NORMAL, &[], &[]);
        for m in &self.fixed {
            let (i, v) = classify_in(&self.targets, &self.filler, m);
            if i >= 0 {
                base.status[i as usize] = v;
            } else {
                base.junk[v as usize] += 1;
            }
        }
        let mut profile_names = vec!["base".to_string()];
        profile_names.extend(self.profiles.keys().filter(|k| *k != "base").cloned());
        let mut c = Compiled {
            mods,
            base,
            t_side: self.targets.iter().map(|t| t.side).collect(),
            t_notable: self.targets.iter().map(|t| t.notable).collect(),
            profile_names,
            lumped: Vec::new(),
        };
        c.lumped = c.profile_names.iter().map(|name| self.lump(&c, name)).collect();
        c
    }

    /// profile -> its lumped weights: targets by status, junk by slot.
    fn lump(&self, c: &Compiled, name: &str) -> Lumped {
        let mut tw = vec![[0.0; 2]; self.targets.len()];
        let mut groups: Vec<Vec<(&[String], f64)>> = vec![Vec::new(); JUNK_SLOTS];
        let mut at: Vec<HashMap<&[String], usize>> = vec![HashMap::new(); JUNK_SLOTS];
        for (mod_id, w) in self.weights_ordered(name) {
            if w <= 0.0 {
                continue;
            }
            let m = c.mods.get(mod_id).unwrap_or_else(|| panic!("weight profile {name:?} names unknown mod {mod_id:?}"));
            let (i, v) = classify_in(&self.targets, &self.filler, m);
            if i >= 0 {
                tw[i as usize][v as usize - 1] += w;
            } else {
                let slot = v as usize;
                let g = m.groups.as_slice();
                match at[slot].get(g) {
                    Some(&k) => groups[slot][k].1 += w,
                    None => {
                        at[slot].insert(g, groups[slot].len());
                        groups[slot].push((g, w));
                    }
                }
            }
        }
        let mut junk = [(0.0, 0.0); JUNK_SLOTS];
        for (slot, g) in groups.iter().enumerate() {
            let total: f64 = g.iter().map(|x| x.1).sum();
            // Drawing a junk mod removes its whole group; use the expected group weight.
            junk[slot] = (total, if total != 0.0 { g.iter().map(|x| x.1 * x.1).sum::<f64>() / total } else { 0.0 });
        }
        Lumped { tw, junk }
    }

    fn weights_ordered(&self, profile: &str) -> Vec<(&str, f64)> {
        if profile == "base" {
            let mut out: Vec<(&str, f64)> = Vec::new();
            let mut at: HashMap<&str, usize> = HashMap::new();
            for m in &self.pool {
                match at.get(m.id.as_str()) {
                    Some(&i) => out[i].1 = m.weight,
                    None => {
                        at.insert(&m.id, out.len());
                        out.push((&m.id, m.weight));
                    }
                }
            }
            out
        } else {
            let p = self.profiles.get(profile).unwrap_or_else(|| panic!("unknown weight profile {profile:?}"));
            p.iter().map(|(k, v)| (k.as_str(), *v)).collect()
        }
    }

    /// Mod id -> weight under a profile.
    pub fn weights(&self, profile: &str) -> HashMap<String, f64> {
        self.weights_ordered(profile).into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    pub(crate) fn profile_index(&self, name: &str) -> usize {
        self.c().profile_names.iter().position(|n| n == name).unwrap_or_else(|| panic!("unknown weight profile {name:?}"))
    }

    pub fn mods(&self) -> &HashMap<String, Mod> {
        &self.c().mods
    }

    pub fn limit(&self) -> usize {
        match self.kind {
            Kind::Jewel => RARE_AFFIX_LIMIT_JEWEL,
            Kind::Gear => RARE_AFFIX_LIMIT_GEAR,
        }
    }

    pub fn rare_counts(&self) -> &'static [(usize, f64)] {
        match self.kind {
            Kind::Jewel => &RARE_MOD_COUNTS_JEWEL,
            Kind::Gear => &RARE_MOD_COUNTS_GEAR,
        }
    }

    /// The fractured mods alone.
    pub fn base(&self) -> State {
        self.c().base
    }

    /// A fresh base (with its fractured mods): where a sale starts over.
    pub fn start(&self) -> State {
        let b = self.c().base;
        State { rarity: NORMAL, ..b }
    }

    /// Where this craft starts: `begin`, else a fresh base.
    pub fn origin(&self) -> State {
        self.begin.unwrap_or_else(|| self.start())
    }

    /// (target index, GOOD or BLOCKED), or (-1, junk slot).
    pub fn classify(&self, mod_id: &str) -> (i32, u8) {
        let m = self.mods().get(mod_id).unwrap_or_else(|| panic!("unknown mod {mod_id:?}"));
        self.classify_mod(m)
    }

    /// `classify` for any mod, also one outside the pool (a crafted or influence mod on a bought item).
    pub fn classify_mod(&self, m: &Mod) -> (i32, u8) {
        classify_in(&self.targets, &self.filler, m)
    }

    /// The state of a real item with these mods (fractured ones included, as in `fixed`).
    pub fn state_of<'a>(&self, rarity: u8, mods: impl IntoIterator<Item = &'a Mod>) -> State {
        let mut s = State::new(rarity, &[], &[]);
        for m in mods {
            let (i, v) = self.classify_mod(m);
            if i >= 0 {
                let st = &mut s.status[i as usize];
                *st = if *st != MISSING { *st } else { v };
            } else {
                s.junk[v as usize] += 1;
            }
        }
        s
    }

    fn counts(&self, c: &Compiled, s: &State) -> [usize; 2] {
        let mut n = [0usize; 2];
        for (i, side) in c.t_side.iter().enumerate() {
            if s.status[i] != MISSING {
                n[side.index()] += 1;
            }
        }
        for (side, slots) in SIDE_SLOTS.iter().enumerate() {
            n[side] += slots.iter().map(|&k| s.junk[k] as usize).sum::<usize>();
        }
        n
    }

    pub fn count(&self, s: &State, side: Side) -> usize {
        self.counts(self.c(), s)[side.index()]
    }

    fn notables_in(&self, c: &Compiled, s: &State) -> usize {
        let on = c.t_notable.iter().enumerate().filter(|(i, n)| **n && s.status[*i] != MISSING).count();
        on + s.junk[0] as usize + s.junk[2] as usize
    }

    pub fn notables(&self, s: &State) -> usize {
        self.notables_in(self.c(), s)
    }

    pub fn total(&self, s: &State) -> usize {
        let n = self.counts(self.c(), s);
        n[0] + n[1]
    }

    pub fn done(&self, s: &State) -> bool {
        let c = self.c();
        let good = c.t_side.iter().enumerate().filter(|(i, _)| s.status[*i] == GOOD).count();
        if good < self.need || s.junk[4] as usize + (s.junk[5] as usize) < self.filler_need {
            return false;
        }
        if let Some((p, q)) = self.filler_sides
            && ((s.junk[4] as usize) < p || (s.junk[5] as usize) < q)
        {
            return false;
        }
        if let Some((p, q)) = self.junk_cap
            && (s.junk[0] as usize + s.junk[1] as usize > p || s.junk[2] as usize + s.junk[3] as usize > q)
        {
            return false;
        }
        let limit = if s.rarity == MAGIC { 1 } else { self.limit() };
        let counts = self.counts(c, s);
        self.reserve.iter().all(|(side, n)| counts[side.index()] + n <= limit)
    }

    /// Whether the item passes as a smaller product: its targets on, and its filler, from the
    /// filler slots (4 and 5) and the targets `spare` that count as its filler.
    pub fn sells_as(&self, s: &State, sale: &Sale) -> bool {
        let c = self.c();
        if !sale.need.iter().all(|&i| s.status[i] == GOOD) {
            return false;
        }
        let spare = |side: Side| sale.spare.iter().filter(|&&i| s.status[i] == GOOD && c.t_side[i] == side).count();
        let pre = s.junk[4] as usize + spare(Side::Prefix);
        let suf = s.junk[5] as usize + spare(Side::Suffix);
        pre >= sale.prefix && suf >= sale.suffix && pre + suf >= sale.any
    }

    /// Bench "<other side> Cannot Be Changed", then scour: only `side` (and fractured mods) stay.
    ///
    /// The metamod takes a slot on the other side, so that side needs room.
    pub fn keep(&self, s: &State, side: Side) -> Vec<Outcome> {
        let c = self.c();
        if self.counts(c, s)[side.other().index()] >= self.limit() {
            return Vec::new();
        }
        let mut out = *s;
        out.rarity = RARE;
        for i in 0..self.targets.len() {
            if c.t_side[i] != side {
                out.status[i] = c.base.status[i];
            }
        }
        for slot in 0..JUNK_SLOTS {
            if !side.slots().contains(&slot) {
                out.junk[slot] = c.base.junk[slot];
            }
        }
        if self.total(&out) == 0 {
            out.rarity = NORMAL;
        }
        vec![(1.0, out)]
    }

    // -- mechanics ----------------------------------------------------------

    /// Add one random mod: augment, regal, exalt, or one draw of a reroll (eldritch exalts: one side only).
    ///
    /// `taken` slots on each allowed side are already used by something the state doesn't track (a metamod).
    pub fn add_mod(&self, s: &State, profile: &str, sides: &[Side], taken: usize) -> Vec<Outcome> {
        let mut out = Vec::new();
        self.add_mod_into(self.c(), s, self.profile_index(profile), sides, taken, &mut out);
        out
    }

    pub(crate) fn add_mod_into(&self, c: &Compiled, s: &State, profile: usize, sides: &[Side], taken: usize, out: &mut Vec<Outcome>) {
        let lump = &c.lumped[profile];
        let limit = if s.rarity == MAGIC { 1 } else { self.limit() };
        let cap_hit = self.notables_in(c, s) >= self.notable_cap;
        let counts = self.counts(c, s);
        out.clear();
        for &side in sides {
            if counts[side.index()] + taken >= limit {
                continue;
            }
            for i in 0..self.targets.len() {
                if c.t_side[i] != side || s.status[i] != MISSING || (c.t_notable[i] && cap_hit) {
                    continue;
                }
                for v in [GOOD, BLOCKED] {
                    let w = lump.tw[i][v as usize - 1];
                    if w > 0.0 {
                        let mut n = *s;
                        n.status[i] = v;
                        out.push((w, n));
                    }
                }
            }
            for &slot in side.slots() {
                if (slot == 0 || slot == 2) && cap_hit {
                    continue;
                }
                let (total, removed) = lump.junk[slot];
                let w = (total - removed * s.junk[slot] as f64).max(0.0);
                if w > 0.0 {
                    let mut n = *s;
                    n.junk[slot] += 1;
                    out.push((w, n));
                }
            }
        }
        let total: f64 = out.iter().map(|o| o.0).sum();
        if total != 0.0 {
            for o in out.iter_mut() {
                o.0 /= total;
            }
        } else {
            out.clear();
        }
    }

    pub fn force(&self, s: &State, mod_id: &str) -> State {
        let (i, v) = self.classify(mod_id);
        let mut n = *s;
        if i >= 0 {
            let st = &mut n.status[i as usize];
            *st = if *st != MISSING { *st } else { v };
        } else {
            n.junk[v as usize] += 1;
        }
        n
    }

    /// A fresh roll at `rarity`; fractured mods stay.
    pub fn roll(&self, rarity: u8, profile: &str, first_profile: Option<&str>, forced: Option<&str>) -> Vec<Outcome> {
        let first = first_profile.filter(|f| !f.is_empty()).map(|f| self.profile_index(f));
        self.roll_idx(rarity, self.profile_index(profile), first, forced.filter(|f| !f.is_empty()))
    }

    fn roll_idx(&self, rarity: u8, profile: usize, first: Option<usize>, forced: Option<&str>) -> Vec<Outcome> {
        let c = self.c();
        const MAGIC_COUNTS: [(usize, f64); 2] = [(1, 1.0 - MAGIC_TWO_MOD_CHANCE), (2, MAGIC_TWO_MOD_CHANCE)];
        let counts: &[(usize, f64)] = if rarity == MAGIC { &MAGIC_COUNTS } else { self.rare_counts() };
        let mut state = State { rarity, ..c.base };
        let mut placed = self.fixed.len();
        if let Some(f) = forced {
            state = self.force(&state, f);
            placed += 1;
        }
        let mut dist = Dist::one(state);
        let mut result = Dist::new();
        let mut alive = 1.0; // chance the roll still has more mods to add
        let mut scratch = Vec::new();
        let top = counts.iter().map(|c| c.0).max().unwrap_or(0);
        for k in placed..=top {
            let more: f64 = counts.iter().filter(|c| c.0 > k).map(|c| c.1).sum();
            let stop = alive - more;
            for &(s, p) in &dist.items {
                result.add(s, p * stop);
            }
            alive = more;
            if alive <= 1e-15 {
                break;
            }
            let prof = if k == placed && forced.is_none() { first.unwrap_or(profile) } else { profile };
            let mut next = Dist::new();
            for &(s, p) in &dist.items {
                self.add_mod_into(c, &s, prof, &[Side::Prefix, Side::Suffix], 0, &mut scratch);
                if scratch.is_empty() {
                    next.add(s, p);
                } else {
                    for &(q, n) in &scratch {
                        next.add(n, p * q);
                    }
                }
            }
            dist = next;
        }
        result.items.into_iter().filter(|(_, p)| *p > 0.0).map(|(s, p)| (p, s)).collect()
    }

    /// Bench "<keep side> Cannot Be Changed", then reroll (chaos, Harvest): only the other side rolls again.
    ///
    /// The metamod takes one slot on the rerolled side (removed afterwards); the
    /// roll's mod count includes the kept mods, as a normal reroll's would.
    pub fn lock_reroll(&self, s: &State, keep: Side, profile: &str, first_profile: Option<&str>) -> Out {
        let first = first_profile.filter(|f| !f.is_empty()).map(|f| self.profile_index(f));
        match self.lock_reroll_idx(s, keep, self.profile_index(profile), first) {
            Some(a) => Out::Shared(a),
            None => Out::Fresh(Vec::new()),
        }
    }

    fn lock_reroll_idx(&self, s: &State, keep: Side, profile: usize, first: Option<usize>) -> Option<Arc<Vec<Outcome>>> {
        let c = self.c();
        let other = keep.other();
        if s.rarity != RARE || self.counts(c, s)[other.index()] >= self.limit() {
            return None;
        }
        let mut state = *s;
        for i in 0..self.targets.len() {
            if c.t_side[i] != keep {
                state.status[i] = c.base.status[i];
            }
        }
        for slot in 0..JUNK_SLOTS {
            if !keep.slots().contains(&slot) {
                state.junk[slot] = c.base.junk[slot];
            }
        }
        // Only the kept side matters, so states that agree on it share one (identical) outcome list.
        let key = (keep.index() as u8, profile as u32, first.map_or(u32::MAX, |f| f as u32), state);
        if let Some(hit) = self.caches.locks.lock().unwrap().get(&key) {
            return Some(hit.clone());
        }
        let made = Arc::new(self.lock_reroll_dist(c, state, other, profile, first));
        Some(self.caches.locks.lock().unwrap().entry(key).or_insert(made).clone())
    }

    fn lock_reroll_dist(&self, c: &Compiled, state: State, other: Side, profile: usize, first: Option<usize>) -> Vec<Outcome> {
        let counts = self.rare_counts();
        let placed = self.counts(c, &state).iter().sum::<usize>() + 1; // the metamod
        let mut dist = Dist::one(state);
        let mut result = Dist::new();
        let mut alive = 1.0;
        let mut scratch = Vec::new();
        let top = counts.iter().map(|c| c.0).max().unwrap_or(0);
        for k in placed..=top {
            let more: f64 = counts.iter().filter(|c| c.0 > k).map(|c| c.1).sum();
            for &(st, p) in &dist.items {
                result.add(st, p * (alive - more));
            }
            alive = more;
            if alive <= 1e-15 {
                break;
            }
            let prof = if k == placed { first.unwrap_or(profile) } else { profile };
            let mut next = Dist::new();
            for &(st, p) in &dist.items {
                self.add_mod_into(c, &st, prof, &[other], 1, &mut scratch);
                if scratch.is_empty() {
                    next.add(st, p);
                } else {
                    for &(q, n) in &scratch {
                        next.add(n, p * q);
                    }
                }
            }
            dist = next;
        }
        for &(st, p) in &dist.items {
            // counts above the last k (if any) end here
            result.add(st, p * alive);
        }
        result.items.into_iter().filter(|(_, p)| *p > 0.0).map(|(s, p)| (p, s)).collect()
    }

    /// Remove one random mod that isn't fractured (eldritch annulment: from one side only).
    pub fn annul(&self, s: &State, side: Option<Side>) -> Vec<Outcome> {
        let mut out = Vec::new();
        self.annul_into(self.c(), s, side, &mut out);
        out
    }

    fn annul_into(&self, c: &Compiled, s: &State, side: Option<Side>, out: &mut Vec<Outcome>) {
        out.clear();
        for i in 0..self.targets.len() {
            if side.is_some_and(|sd| c.t_side[i] != sd) {
                continue;
            }
            if s.status[i] != MISSING && c.base.status[i] == MISSING {
                let mut n = *s;
                n.status[i] = MISSING;
                out.push((1.0, n));
            }
        }
        let all: [usize; JUNK_SLOTS] = [0, 1, 2, 3, 4, 5];
        for &slot in side.map_or(&all[..], |sd| &sd.slots()[..]) {
            let n = s.junk[slot] as i32 - c.base.junk[slot] as i32;
            if n > 0 {
                let mut m = *s;
                m.junk[slot] -= 1;
                out.push((n as f64, m));
            }
        }
        let total: f64 = out.iter().map(|o| o.0).sum();
        if total != 0.0 {
            for o in out.iter_mut() {
                o.0 /= total;
            }
        } else {
            out.clear();
        }
    }

    /// What a reroll's outcome depends on; None for actions that aren't rerolls.
    pub fn roll_key(&self, a: &Action) -> Option<RollKey> {
        match a.kind.as_str() {
            "transmute" | "alt" | "reroll_magic" => Some(RollKey::Magic(a.profile.clone())),
            "reroll_rare" => {
                let forced = a.forced.as_deref().filter(|f| !f.is_empty());
                Some(RollKey::Rare(a.profile.clone(), a.first_profile.clone(), forced.map(|f| self.classify(f))))
            }
            _ => None,
        }
    }

    /// A reroll's outcome distribution, computed once per `roll_key` (the actions sharing one share it); None for other actions.
    ///
    /// The cache outlives changes to `actions`: a roll depends only on the pool, targets and fractured mods.
    pub fn rolled(&self, a: &Action) -> Option<Arc<Vec<Outcome>>> {
        let key = self.roll_key(a)?;
        if let Some(hit) = self.caches.rolls.lock().unwrap().get(&key) {
            return Some(hit.clone());
        }
        let made = Arc::new(match &key {
            RollKey::Magic(_) => self.roll(MAGIC, &a.profile, None, None),
            RollKey::Rare(..) => self.roll(RARE, &a.profile, a.first_profile.as_deref(), a.forced.as_deref()),
        });
        Some(self.caches.rolls.lock().unwrap().entry(key).or_insert(made).clone())
    }

    /// Drop the cached lock-and-reroll lists (`solve` does, as its action list changes).
    pub(crate) fn clear_lock_cache(&self) {
        self.caches.locks.lock().unwrap().clear();
    }

    pub(crate) fn prepare<'a>(&self, a: &'a Action) -> ActionC<'a> {
        let kind = ActionKind::parse(&a.kind);
        let mask = a.from_rarity.iter().filter(|&&r| r < 8).fold(0u8, |m, &r| m | 1 << r);
        let mut c = ActionC { kind, mask, profile: 0, first: None, roll: None, sale: a.sale.as_ref() };
        match kind {
            ActionKind::Roll => c.roll = self.rolled(a),
            ActionKind::LockReroll(_) => {
                c.profile = self.profile_index(&a.profile);
                c.first = a.first_profile.as_deref().filter(|f| !f.is_empty()).map(|f| self.profile_index(f));
            }
            _ => {}
        }
        c
    }

    /// The outcomes of action `a` from `s`. Fresh outcomes go to `scratch`.
    pub(crate) fn outcomes_into(&self, s: &State, a: &ActionC, scratch: &mut Vec<Outcome>) -> Res {
        let c = self.c();
        if s.rarity >= 8 || a.mask & (1 << s.rarity) == 0 {
            return Res::Empty;
        }
        let both = [Side::Prefix, Side::Suffix];
        match a.kind {
            ActionKind::Sell => {
                let Some(sale) = a.sale else { return Res::Empty };
                if !self.sells_as(s, sale) {
                    return Res::Empty;
                }
                scratch.clear();
                scratch.push((1.0, self.start()));
            }
            ActionKind::Roll => {
                return match &a.roll {
                    Some(r) if !r.is_empty() => Res::Shared(r.clone()),
                    _ => Res::Empty,
                };
            }
            ActionKind::Aug => {
                if self.total(s) < 2 {
                    self.add_mod_into(c, s, 0, &both, 0, scratch);
                } else {
                    scratch.clear();
                }
            }
            ActionKind::Regal => self.add_mod_into(c, &State { rarity: RARE, ..*s }, 0, &both, 0, scratch),
            ActionKind::Exalt => {
                if self.total(s) < 2 * self.limit() {
                    self.add_mod_into(c, s, 0, &both, 0, scratch);
                } else {
                    scratch.clear();
                }
            }
            ActionKind::Annul => self.annul_into(c, s, None, scratch),
            ActionKind::ExaltSide(side) => {
                if self.total(s) < 2 * self.limit() {
                    self.add_mod_into(c, s, 0, &[side], 0, scratch);
                } else {
                    scratch.clear();
                }
            }
            ActionKind::AnnulSide(side) => self.annul_into(c, s, Some(side), scratch),
            ActionKind::Keep(side) => {
                *scratch = self.keep(s, side);
            }
            ActionKind::LockReroll(side) => {
                return match self.lock_reroll_idx(s, side, a.profile, a.first) {
                    Some(r) if !r.is_empty() => Res::Shared(r),
                    _ => Res::Empty,
                };
            }
            ActionKind::Other => return Res::Empty,
        }
        if scratch.is_empty() { Res::Empty } else { Res::Fresh }
    }

    pub fn outcomes(&self, s: &State, a: &Action) -> Out {
        let ac = self.prepare(a);
        let mut scratch = Vec::new();
        match self.outcomes_into(s, &ac, &mut scratch) {
            Res::Empty => Out::Fresh(Vec::new()),
            Res::Fresh => Out::Fresh(scratch),
            Res::Shared(r) => Out::Shared(r),
        }
    }
}
