//! Plain data: mods, targets, lumped item states and currency actions.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::Arc;

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

/// The lumped item: rarity, each target's status, and the count of other mods per slot.
///
/// `junk` slots: prefix notable, prefix other, suffix notable, suffix other, prefix
/// filler, suffix filler (filler: wanted, but any will do). Unused status entries are 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub rarity: u8,
    pub status: [u8; MAX_TARGETS],
    pub junk: [u8; JUNK_SLOTS],
}

impl State {
    pub fn new(rarity: u8, status: &[u8], junk: &[u8]) -> State {
        assert!(status.len() <= MAX_TARGETS, "at most {MAX_TARGETS} targets");
        assert!(junk.len() <= JUNK_SLOTS, "at most {JUNK_SLOTS} junk slots");
        let mut s = State { rarity, status: [0; MAX_TARGETS], junk: [0; JUNK_SLOTS] };
        s.status[..status.len()].copy_from_slice(status);
        s.junk[..junk.len()].copy_from_slice(junk); // short junk is padded with zeros
        s
    }
}

impl Hash for State {
    // States are hashed millions of times while solving: two words, no length prefixes.
    fn hash<H: Hasher>(&self, h: &mut H) {
        let mut j = [0u8; 8];
        j[..JUNK_SLOTS].copy_from_slice(&self.junk);
        j[JUNK_SLOTS] = self.rarity;
        h.write_u64(u64::from_le_bytes(self.status));
        h.write_u64(u64::from_le_bytes(j));
    }
}

#[derive(Serialize, Deserialize)]
struct StateRepr {
    rarity: u8,
    status: Vec<u8>,
    #[serde(default)]
    junk: Vec<u8>,
}

impl Serialize for State {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        // Trailing status zeros can't be told from missing targets, so all slots are written.
        StateRepr { rarity: self.rarity, status: self.status.to_vec(), junk: self.junk.to_vec() }.serialize(ser)
    }
}

impl<'de> Deserialize<'de> for State {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<State, D::Error> {
        let r = StateRepr::deserialize(de)?;
        if r.status.len() > MAX_TARGETS || r.junk.len() > JUNK_SLOTS {
            return Err(serde::de::Error::custom("state has too many targets or junk slots"));
        }
        Ok(State::new(r.rarity, &r.status, &r.junk))
    }
}

/// What a bricked item must have to sell as a smaller product: its targets on, and its
/// filler (prefix, suffix, any) from the filler slots and the `spare` targets that count as filler.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sale {
    pub need: BTreeSet<usize>,
    pub prefix: usize,
    pub suffix: usize,
    pub any: usize,
    pub spare: BTreeSet<usize>,
}

fn base_profile() -> String {
    "base".into()
}

/// One currency use. `cost` is [(price id, quantity)].
///
/// `kind`: transmute alt aug regal exalt annul reroll_magic reroll_rare keep_prefixes
/// keep_suffixes lockreroll_{prefixes,suffixes} exalt_{prefix,suffix} annul_{prefix,suffix} sell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
    pub kind: String,
    pub cost: Vec<(String, f64)>,
    pub from_rarity: Vec<u8>,
    #[serde(default = "base_profile")]
    pub profile: String, // weight profile for rolled mods
    #[serde(default)]
    pub first_profile: Option<String>, // Harvest: the first mod comes from a tag-filtered pool
    #[serde(default)]
    pub forced: Option<String>, // essence: this mod is always added
    /// Selling a bricked item as a smaller product and starting over: chaos back, and what it must have.
    #[serde(default)]
    pub credit: f64,
    #[serde(default)]
    pub sale: Option<Sale>,
}

impl Action {
    pub fn new(name: &str, kind: &str, cost: &[(&str, f64)], from_rarity: &[u8]) -> Action {
        Action {
            name: name.into(),
            kind: kind.into(),
            cost: cost.iter().map(|(p, q)| (p.to_string(), *q)).collect(),
            from_rarity: from_rarity.to_vec(),
            profile: base_profile(),
            first_profile: None,
            forced: None,
            credit: 0.0,
            sale: None,
        }
    }

    pub fn with_profile(mut self, profile: &str) -> Action {
        self.profile = profile.into();
        self
    }

    pub fn with_first_profile(mut self, profile: &str) -> Action {
        self.first_profile = Some(profile.into());
        self
    }

    pub fn with_forced(mut self, mod_id: &str) -> Action {
        self.forced = Some(mod_id.into());
        self
    }

    pub fn with_sale(mut self, credit: f64, sale: Sale) -> Action {
        self.credit = credit;
        self.sale = Some(sale);
        self
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

/// One entry of an outcome distribution: (probability, next state).
pub type Outcome = (f64, State);

/// An action's outcomes from one state: owned, or shared with every state that rolls the same.
#[derive(Clone, Debug)]
pub enum Out {
    Fresh(Vec<Outcome>),
    Shared(Arc<Vec<Outcome>>),
}

impl Deref for Out {
    type Target = [Outcome];

    fn deref(&self) -> &[Outcome] {
        match self {
            Out::Fresh(v) => v,
            Out::Shared(v) => v,
        }
    }
}
