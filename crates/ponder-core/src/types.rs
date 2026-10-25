//! Plain data: mods, targets, lumped item states and currency actions.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

// Assumptions not in the game data; adjust here if testing shows otherwise.
pub const MAGIC_TWO_MOD_CHANCE: f64 = 0.5; // transmute/alteration roll 1 or 2 mods
/// Mod count when an item becomes rare (alchemy, chaos, essence, fossil, Harvest), as (count, chance).
/// Gear follows a 1,000-chaos sample (4:5:6 = 9:3:1); jewels assume the same 3:1 step.
pub const RARE_MOD_COUNTS_JEWEL: [(usize, f64); 2] = [(3, 0.75), (4, 0.25)];
pub const RARE_MOD_COUNTS_GEAR: [(usize, f64); 3] = [(4, 9.0 / 13.0), (5, 3.0 / 13.0), (6, 1.0 / 13.0)];
pub const RARE_AFFIX_LIMIT_JEWEL: usize = 2; // per side
pub const RARE_AFFIX_LIMIT_GEAR: usize = 3;
/// `State.junk` slots on each side, indexed by `Side::index`.
pub const SIDE_SLOTS: [[usize; 3]; 2] = [[0, 1, 4], [2, 3, 5]];
pub const JUNK_LABELS: [&str; 6] =
    ["other prefix notable", "other prefix", "other suffix notable", "other suffix", "filler prefix", "filler suffix"];

pub const NORMAL: u8 = 0;
pub const MAGIC: u8 = 1;
pub const RARE: u8 = 2;
pub const MISSING: u8 = 0;
pub const GOOD: u8 = 1;
pub const BLOCKED: u8 = 2;

/// Most targets a problem may have (`State` stores their status inline).
pub const MAX_TARGETS: usize = 8;
pub const JUNK_SLOTS: usize = 6;
pub const UNVEIL_TARGET_CLASSES: u8 = (2 * MAX_TARGETS) as u8;
pub const UNVEIL_CLASSES: u8 = UNVEIL_TARGET_CLASSES + JUNK_SLOTS as u8;
pub const VEILED_PREFIX: u8 = 31;
pub const VEILED_SUFFIX: u8 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Prefix,
    Suffix,
}

impl Side {
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn other(self) -> Side {
        match self {
            Side::Prefix => Side::Suffix,
            Side::Suffix => Side::Prefix,
        }
    }

    pub fn slots(self) -> &'static [usize; 3] {
        &SIDE_SLOTS[self.index()]
    }

    pub fn name(self) -> &'static str {
        ["prefix", "suffix"][self.index()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Jewel,
    Gear,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mod {
    pub id: String,
    pub side: Side,
    pub weight: f64,
    pub groups: Vec<String>,
    pub notable: bool,
    pub label: String,
    #[serde(default)]
    pub tags: Vec<String>, // implicit tags, used by fossils and Harvest
}

impl Mod {
    pub fn new(id: &str, side: Side, weight: f64, groups: &[&str], notable: bool, label: &str, tags: &[&str]) -> Mod {
        Mod {
            id: id.into(),
            side,
            weight,
            groups: groups.iter().map(|g| g.to_string()).collect(),
            notable,
            label: label.into(),
            tags: tags.iter().map(|g| g.to_string()).collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub name: String,
    pub side: Side,
    pub good: BTreeSet<String>,   // mod ids that satisfy it
    pub groups: BTreeSet<String>, // other mods in these groups block it
    #[serde(default)]
    pub notable: bool,
}

/// Bench metamods an item can carry (`State.meta`; 0 is none). Each is a crafted mod holding a
/// slot (`meta_side`). What respects them (Craft of Exile's table, poedb.tw/metamods):
/// - "Prefixes/Suffixes Cannot Be Changed": chaos, Harvest reforges, scour, annul and Harvest
///   add/remove leave the locked side alone. A reforge still adds mods to the locked side if it
///   has room, and removes the metamod with the rest of the other side.
/// - "Cannot roll Attack/Caster Modifiers": exalts, regals, augments and Harvest add/remove
///   don't add mods with the tag, and annuls can't remove them.
/// - Essences and fossils can't be used on an item with any metamod.
pub const META_LOCK_PREFIXES: u8 = 1;
pub const META_LOCK_SUFFIXES: u8 = 2;
pub const META_NO_ATTACK: u8 = 3;
pub const META_NO_CASTER: u8 = 4;
pub const META_MULTIMOD: u8 = 5;
pub const META_NAMES: [&str; 6] = [
    "",
    "Prefixes Cannot Be Changed",
    "Suffixes Cannot Be Changed",
    "Cannot roll Attack Modifiers",
    "Cannot roll Caster Modifiers",
    "Can have up to 3 Crafted Modifiers",
];
/// The mod tags "Cannot roll Attack/Caster Modifiers" keep off the item, by `meta_tag`.
pub const META_TAGS: [&str; 2] = ["attack", "caster"];

/// The side a metamod's slot is on.
pub fn meta_side(meta: u8) -> Option<Side> {
    match meta {
        0 => None,
        META_LOCK_SUFFIXES => Some(Side::Prefix),
        _ => Some(Side::Suffix),
    }
}

/// The side a lock metamod keeps.
pub fn meta_locks(meta: u8) -> Option<Side> {
    match meta {
        META_LOCK_PREFIXES => Some(Side::Prefix),
        META_LOCK_SUFFIXES => Some(Side::Suffix),
        _ => None,
    }
}

/// Which of `META_TAGS` a "cannot roll" metamod keeps off.
pub fn meta_tag(meta: u8) -> Option<usize> {
    match meta {
        META_NO_ATTACK => Some(0),
        META_NO_CASTER => Some(1),
        _ => None,
    }
}

/// The lumped item: rarity, each target's status, the count of other mods per slot, and the
/// bench metamod on it (`META_*`, 0 for none).
///
/// `junk` slots: prefix notable, prefix other, suffix notable, suffix other, prefix
/// filler, suffix filler (filler: wanted, but any will do). Unused status entries are 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub rarity: u8,
    pub status: [u8; MAX_TARGETS],
    pub junk: [u8; JUNK_SLOTS],
    pub meta: u8,
    pub veiled: u8,
    pub unveil: u32,
}

impl State {
    pub fn new(rarity: u8, status: &[u8], junk: &[u8]) -> State {
        assert!(status.len() <= MAX_TARGETS, "at most {MAX_TARGETS} targets");
        assert!(junk.len() <= JUNK_SLOTS, "at most {JUNK_SLOTS} junk slots");
        let mut s = State { rarity, status: [0; MAX_TARGETS], junk: [0; JUNK_SLOTS], meta: 0, veiled: 0, unveil: 0 };
        s.status[..status.len()].copy_from_slice(status);
        s.junk[..junk.len()].copy_from_slice(junk); // short junk is padded with zeros
        s
    }

    pub fn with_meta(mut self, meta: u8) -> State {
        self.meta = meta;
        self
    }
}

impl Hash for State {
    // States are hashed millions of times while solving: two words, no length prefixes.
    fn hash<H: Hasher>(&self, h: &mut H) {
        let mut j = [0u8; 8];
        j[..JUNK_SLOTS].copy_from_slice(&self.junk);
        j[JUNK_SLOTS] = self.rarity;
        j[JUNK_SLOTS + 1] = self.meta;
        h.write_u64(u64::from_le_bytes(self.status));
        h.write_u64(u64::from_le_bytes(j));
        if self.veiled != 0 || self.unveil != 0 {
            h.write_u64(self.veiled as u64 | (self.unveil as u64) << 8);
        }
    }
}

fn is_zero(x: &u8) -> bool {
    *x == 0
}

fn is_zero_u32(x: &u32) -> bool {
    *x == 0
}

#[derive(Serialize, Deserialize)]
struct StateRepr {
    rarity: u8,
    status: Vec<u8>,
    #[serde(default)]
    junk: Vec<u8>,
    #[serde(default, skip_serializing_if = "is_zero")]
    meta: u8,
    #[serde(default, skip_serializing_if = "is_zero")]
    veiled: u8,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    unveil: u32,
}

impl Serialize for State {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        // Trailing status zeros can't be told from missing targets, so all slots are written.
        StateRepr {
            rarity: self.rarity,
            status: self.status.to_vec(),
            junk: self.junk.to_vec(),
            meta: self.meta,
            veiled: self.veiled,
            unveil: self.unveil,
        }
        .serialize(ser)
    }
}

impl<'de> Deserialize<'de> for State {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<State, D::Error> {
        let r = StateRepr::deserialize(de)?;
        if r.status.len() > MAX_TARGETS
            || r.junk.len() > JUNK_SLOTS
            || r.meta as usize >= META_NAMES.len()
            || (r.veiled > UNVEIL_CLASSES && !matches!(r.veiled, VEILED_PREFIX | VEILED_SUFFIX))
            || r.unveil >> UNVEIL_CLASSES != 0
            || (r.unveil != 0 && !matches!(r.veiled, VEILED_PREFIX | VEILED_SUFFIX))
        {
            return Err(serde::de::Error::custom("state has too many targets or junk slots, or an unknown metamod"));
        }
        Ok(State { veiled: r.veiled, unveil: r.unveil, ..State::new(r.rarity, &r.status, &r.junk).with_meta(r.meta) })
    }
}

/// What the solver needs that isn't in the game data. Python reads the clicking rate from
/// the environment; here the caller passes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// What an hour of clicking is worth, in chaos. Charging for time stops plans
    /// that spam a cheap orb ten thousand times for a long shot.
    pub chaos_per_hour: f64,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { chaos_per_hour: 1500.0 }
    }
}

pub type Prices = std::collections::HashMap<String, f64>;
pub type WeightProfile = std::sync::Arc<std::collections::BTreeMap<String, f64>>;

/// One entry of an outcome distribution: (probability, next state).
pub type Outcome = (f64, State);
