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
mod interval;
mod shared;
mod pool;
mod projection;
mod recipes;
mod rolls;
mod unveil;

pub(crate) use effects::PreparedAction;
use interval::{Dist, normalise};
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
