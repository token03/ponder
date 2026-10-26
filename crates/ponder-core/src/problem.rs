//! The crafting problem and its mechanics: what each currency does to a lumped state.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Deref;
use std::sync::{Arc, Mutex, OnceLock};

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::action::{Action, Effect};
use crate::transition::{Distribution, ProbabilityRange as Bound, TransitionResult};
use crate::types::*;

fn default_kind() -> Kind {
    Kind::Gear
}

fn default_notable_cap() -> usize {
    99
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProblemSpec {
    #[serde(serialize_with = "shared::pool")]
    pub pool: Arc<Vec<Mod>>,
    pub targets: Vec<Target>,
    pub need: usize,
    pub actions: Vec<Action>,
    #[serde(default = "default_kind")]
    pub kind: Kind,
    #[serde(default)]
    pub affix_limits: Option<[usize; 2]>,
    #[serde(default = "default_notable_cap")]
    pub notable_cap: usize,
    #[serde(default)]
    pub fixed: Vec<Mod>, // fractured mods, never removed
    #[serde(default, serialize_with = "shared::profiles")]
    pub profiles: Arc<BTreeMap<String, WeightProfile>>, // name -> mod id -> weight
    #[serde(default)]
    pub extra_mods: BTreeMap<String, Mod>, // forced mods outside the pool
    #[serde(default)]
    pub reserve: BTreeMap<Side, usize>, // side -> slots left open for bench crafts at the end
    #[serde(default)]
    pub bench: Vec<Mod>,
    #[serde(default)]
    pub allow_terminal_bench: bool,
    #[serde(default)]
    pub blockers: Vec<Mod>,
    #[serde(default)]
    pub unveils: Vec<Mod>,
    #[serde(default)]
    pub influences: BTreeSet<String>,
    #[serde(default)]
    pub filler: BTreeSet<String>, // mods that count as filler (e.g. any resistance at T3+)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub filler_label: String,
    #[serde(default)]
    pub filler_need: usize, // how many filler mods the finished item needs, on top of the targets
    #[serde(default)]
    pub filler_sides: Option<(usize, usize)>, // (prefix, suffix) filler each side needs; None: any mix of filler_need
    #[serde(default)]
    pub junk_cap: Option<(usize, usize)>, // most other mods allowed (prefix, suffix): recombinator donors want clean items
    #[serde(default)]
    pub min_mods: usize, // a finished item has at least this many mods, bench crafts (`reserve`) included: any mod fills the rest
    #[serde(default)]
    pub begin: Option<State>, // where crafting starts when not a fresh base: a bought or dropped item (`state_of`)
}

mod effects;
mod pool;
mod rolls;
mod unveil;

use crate::transition::{Dist, normalise};
pub(crate) use effects::PreparedAction;
use pool::{Compiled, ITEM_MAX, Lumped, SLOT_MAX};
pub use rolls::RollKey;

type LockKey = (u8, u32, u32, State);

/// (profile, first profile, `META_TAGS` index): a Harvest reforge under a "cannot roll" metamod.
type RestrictedKey = (usize, usize, usize);
type OfferKey = (Side, u8, bool, u8);
type OfferMask = (u32, f64, f64, f64);

#[derive(Default)]
struct Caches {
    compiled: OnceLock<Arc<Compiled>>,
    fingerprint: OnceLock<blake3::Hash>,
    rolls: Mutex<FxHashMap<RollKey, CachedRoll>>,
    locks: Mutex<FxHashMap<LockKey, Arc<Distribution>>>,
    unveils: Mutex<FxHashMap<(bool, State), Arc<Distribution>>>,
    offer_masks: Mutex<FxHashMap<OfferKey, Vec<OfferMask>>>,
    offer_relevance: OnceLock<[u8; 2]>,
    restricted_rolls: Mutex<FxHashMap<RestrictedKey, CachedRoll>>,
}

#[derive(Clone)]
struct CachedRoll {
    distribution: Arc<Distribution>,
    attempted_exact: bool,
}

impl std::fmt::Debug for Caches {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Caches")
    }
}

impl ProblemSpec {
    pub fn new(pool: Vec<Mod>, targets: Vec<Target>, need: usize, actions: Vec<Action>, kind: Kind) -> ProblemSpec {
        Self::from_shared_pool(Arc::new(pool), targets, need, actions, kind)
    }

    pub fn from_shared_pool(pool: Arc<Vec<Mod>>, targets: Vec<Target>, need: usize, actions: Vec<Action>, kind: Kind) -> ProblemSpec {
        ProblemSpec {
            pool,
            targets,
            need,
            actions,
            kind,
            affix_limits: None,
            notable_cap: default_notable_cap(),
            fixed: Vec::new(),
            profiles: Arc::default(),
            extra_mods: BTreeMap::new(),
            reserve: BTreeMap::new(),
            bench: Vec::new(),
            allow_terminal_bench: false,
            blockers: Vec::new(),
            unveils: Vec::new(),
            influences: BTreeSet::new(),
            filler: BTreeSet::new(),
            filler_label: String::new(),
            filler_need: 0,
            filler_sides: None,
            junk_cap: None,
            min_mods: 0,
            begin: None,
        }
    }

    pub fn compile(mut self) -> Problem {
        self.refresh_unveil_actions();
        Problem { spec: Arc::new(self), caches: Arc::new(Caches::default()) }
    }
}

#[derive(Debug, Clone)]
pub struct Problem {
    spec: Arc<ProblemSpec>,
    caches: Arc<Caches>,
}

impl Deref for Problem {
    type Target = ProblemSpec;

    fn deref(&self) -> &ProblemSpec {
        &self.spec
    }
}

impl Serialize for Problem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.spec.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Problem {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Problem, D::Error> {
        Ok(ProblemSpec::deserialize(deserializer)?.compile())
    }
}

impl Problem {
    pub fn to_spec(&self) -> ProblemSpec {
        self.spec.as_ref().clone()
    }

    pub fn fresh_transition_cache(&self) -> Problem {
        let caches = Caches {
            rolls: Mutex::new(self.caches.rolls.lock().unwrap().clone()),
            restricted_rolls: Mutex::new(self.caches.restricted_rolls.lock().unwrap().clone()),
            ..Caches::default()
        };
        if let Some(compiled) = self.caches.compiled.get() {
            let _ = caches.compiled.set(compiled.clone());
        }
        if let Some(fingerprint) = self.caches.fingerprint.get() {
            let _ = caches.fingerprint.set(*fingerprint);
        }
        Problem { spec: self.spec.clone(), caches: Arc::new(caches) }
    }

    pub fn fingerprint(&self) -> blake3::Hash {
        *self.caches.fingerprint.get_or_init(|| shared::fingerprint(self.spec.as_ref()))
    }

    fn c(&self) -> &Compiled {
        self.caches.compiled.get_or_init(|| Arc::new(self.compile_pool()))
    }
}

pub(super) fn classify_in(targets: &[Target], filler: &BTreeSet<String>, m: &Mod) -> (i32, u8) {
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
    pub fn limit(&self) -> usize {
        self.side_limit(RARE, Side::Prefix).max(self.side_limit(RARE, Side::Suffix))
    }

    pub fn rare_counts(&self) -> Vec<(usize, f64)> {
        crate::mechanics::ItemRules::new(self.kind, self.notable_cap, self.affix_limits).counts(RARE)
    }

    pub fn side_limit(&self, rarity: u8, side: Side) -> usize {
        crate::mechanics::ItemRules::new(self.kind, self.notable_cap, self.affix_limits).limit(rarity, side)
    }

    /// A fresh base (with its fractured mods).
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
        if let Some(side) = meta_side(s.meta) {
            n[side.index()] += 1;
        }
        if let Some(side) = Self::pending_side(s) {
            n[side.index()] += 1;
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

    pub fn total(&self, s: &State) -> usize {
        let n = self.counts(self.c(), s);
        n[0] + n[1]
    }

    pub fn done(&self, s: &State) -> bool {
        if s.kind != StateKind::Item
            || s.unveil != 0
            || matches!(s.veiled, VEILED_PREFIX | VEILED_SUFFIX)
            || s.meta != 0
            || s.status[self.targets.len()..self.targets.len() + self.bench.len()].iter().any(|&v| v != MISSING)
        {
            return false; // the bench has to take it off
        }
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
        let counts = self.counts(c, s);
        if counts[0] + counts[1] + self.reserve.values().sum::<usize>() < self.min_mods {
            return false;
        }
        self.reserve.iter().all(|(side, n)| counts[side.index()] + n <= self.side_limit(s.rarity, *side))
    }
}

impl ProblemSpec {
    /// `classify` for any mod, also one outside the pool (a crafted or influence mod on a bought item).
    pub fn classify_mod(&self, m: &Mod) -> (i32, u8) {
        let found = classify_in(&self.targets, &self.filler, m);
        if found.0 >= 0 {
            return found;
        }
        self.bench
            .iter()
            .chain(&self.blockers)
            .position(|b| m.groups.iter().any(|g| b.groups.contains(g)))
            .map_or(found, |i| ((self.targets.len() + i) as i32, BLOCKED))
    }
}

mod shared {
    //! Fingerprints that hash each shared modifier pool and weight-profile table once.
    //!
    //! Many problems (every donor of a product, every fracture variant) share one `Arc`'d pool
    //! and profile table. Hashing a problem serialises its own fields and writes those shared parts
    //! as their content hash, remembered per allocation, instead of re-serialising thousands of
    //! weights for each problem.

    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, Weak};

    use rustc_hash::FxHashMap;
    use serde::{Serialize, Serializer};

    use crate::types::{Mod, WeightProfile};

    thread_local! {
        static FINGERPRINTING: Cell<bool> = const { Cell::new(false) };
    }

    type Hashes<T> = Mutex<FxHashMap<usize, (Weak<T>, blake3::Hash)>>;

    static POOLS: std::sync::LazyLock<Hashes<Vec<Mod>>> = std::sync::LazyLock::new(Default::default);
    static PROFILES: std::sync::LazyLock<Hashes<BTreeMap<String, WeightProfile>>> = std::sync::LazyLock::new(Default::default);

    /// The content hash of a shared value, computed once per live allocation.
    fn content_hash<T: Serialize>(value: &Arc<T>, hashes: &Hashes<T>) -> blake3::Hash {
        let address = Arc::as_ptr(value) as usize;
        if let Some((weak, hash)) = hashes.lock().unwrap().get(&address)
            && weak.upgrade().is_some_and(|live| Arc::ptr_eq(&live, value))
        {
            return *hash;
        }
        let mut hasher = blake3::Hasher::new();
        serde_json::to_writer(&mut hasher, value.as_ref()).expect("shared problem data serialises");
        let hash = hasher.finalize();
        let mut hashes = hashes.lock().unwrap();
        if hashes.len() > 4096 {
            hashes.retain(|_, (weak, _)| weak.strong_count() > 0);
        }
        hashes.insert(address, (Arc::downgrade(value), hash));
        hash
    }

    /// A problem's identity: its own fields, with shared pools and profiles by content hash.
    pub(super) fn fingerprint(spec: &impl Serialize) -> blake3::Hash {
        let mut hasher = blake3::Hasher::new();
        FINGERPRINTING.set(true);
        let written = serde_json::to_writer(&mut hasher, spec);
        FINGERPRINTING.set(false);
        written.expect("a problem serialises");
        hasher.finalize()
    }

    pub(super) fn pool<S: Serializer>(pool: &Arc<Vec<Mod>>, serializer: S) -> Result<S::Ok, S::Error> {
        if FINGERPRINTING.get() { content_hash(pool, &POOLS).as_bytes().serialize(serializer) } else { pool.serialize(serializer) }
    }

    pub(super) fn profiles<S: Serializer>(profiles: &Arc<BTreeMap<String, WeightProfile>>, serializer: S) -> Result<S::Ok, S::Error> {
        if FINGERPRINTING.get() {
            content_hash(profiles, &PROFILES).as_bytes().serialize(serializer)
        } else {
            profiles.serialize(serializer)
        }
    }
}
