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
    pub bench: Vec<Mod>,
    #[serde(default)]
    pub influences: BTreeSet<String>,
    #[serde(default)]
    pub filler: BTreeSet<String>, // mods that count as filler (e.g. any resistance at T3+)
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
mod pool;
mod projection;
mod rolls;

pub(crate) use effects::PreparedAction;
use interval::{Dist, normalise};
use pool::{Compiled, ITEM_MAX, Lumped, SLOT_MAX};
pub use rolls::RollKey;

type LockKey = (u8, u32, u32, State);

/// (profile, first profile, `META_TAGS` index): a Harvest reforge under a "cannot roll" metamod.
type RestrictedKey = (usize, usize, usize);

#[derive(Default)]
struct Caches {
    compiled: OnceLock<Compiled>,
    rolls: Mutex<HashMap<RollKey, CachedRoll>>,
    locks: Mutex<FxHashMap<LockKey, Arc<Distribution>>>,
    restricted_rolls: Mutex<HashMap<RestrictedKey, CachedRoll>>,
}

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
        ProblemSpec {
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
            bench: Vec::new(),
            influences: BTreeSet::new(),
            filler: BTreeSet::new(),
            filler_need: 0,
            filler_sides: None,
            junk_cap: None,
            min_mods: 0,
            begin: None,
        }
    }

    pub fn compile(self) -> Problem {
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

    fn c(&self) -> &Compiled {
        self.caches.compiled.get_or_init(|| self.compile_pool())
    }
}
