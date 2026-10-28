//! What an item must look like: the language goals, intermediates and stop conditions share.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Influence {
    Shaper,
    Elder,
    Crusader,
    Redeemer,
    Hunter,
    Warlord,
}

impl Influence {
    pub const ALL: [Influence; 6] =
        [Influence::Shaper, Influence::Elder, Influence::Crusader, Influence::Redeemer, Influence::Hunter, Influence::Warlord];

    /// The suffix influenced spawn tags carry, e.g. `gloves_basilisk` for Hunter gloves.
    pub fn tag(self) -> &'static str {
        match self {
            Influence::Shaper => "shaper",
            Influence::Elder => "elder",
            Influence::Crusader => "crusader",
            Influence::Redeemer => "eyrie",
            Influence::Hunter => "basilisk",
            Influence::Warlord => "adjudicator",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rarity {
    Normal,
    Magic,
    Rare,
}

/// Where a mod on the item came from. Matters to later operations (recombinators treat essence
/// and other non-natural mods as exclusive; the bench removes only crafted mods).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Natural,
    Influence,
    /// An influence mod upgraded by an Orb of Dominance.
    Elevated,
    Essence,
    Crafted,
    Veiled,
    /// Only added by fossils (Faceted, ...).
    Fossil,
    /// No longer obtainable by rolling (bought, legacy).
    Legacy,
}

/// One requirement on the item's mods. `text` names mods by template (see `data::template`);
/// several texts mean "any of these", and `count` distinct mods must match.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModReq {
    #[serde(deserialize_with = "one_or_many")]
    pub text: Vec<String>,
    /// Worst acceptable tier, 1 being the best that can roll on this base.
    #[serde(default = "default_tier", deserialize_with = "tier")]
    pub tier: Option<u8>,
    #[serde(default = "one")]
    pub count: u8,
    #[serde(default)]
    pub fractured: bool,
    #[serde(default)]
    pub source: Option<Source>,
}

/// A predicate over items. Unset fields don't constrain. Goals are monotone: better tiers of
/// required mods always satisfy them.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Goal {
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub ilvl: Option<u8>,
    #[serde(default)]
    pub quality: Option<u8>,
    #[serde(default)]
    pub influences: Vec<Influence>,
    #[serde(default)]
    pub rarity: Option<Rarity>,
    #[serde(default)]
    pub mods: Vec<ModReq>,
    /// Free affix slots required on each side.
    #[serde(default)]
    pub open_prefix: u8,
    #[serde(default)]
    pub open_suffix: u8,
    /// Most affixes allowed on each side, or in total.
    #[serde(default)]
    pub max_prefix: Option<u8>,
    #[serde(default)]
    pub max_suffix: Option<u8>,
    #[serde(default)]
    pub max_mods: Option<u8>,
    #[serde(default)]
    pub min_mods: Option<u8>,
    /// Chaos to buy an item meeting this stage (`buy:<stage>` in guides). Rough estimates.
    #[serde(default)]
    pub price: Option<f64>,
}

fn one() -> u8 {
    1
}

fn default_tier() -> Option<u8> {
    Some(2)
}

fn tier<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u8>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Tier {
        Number(u8),
        Name(String),
    }
    match Tier::deserialize(d)? {
        Tier::Number(n) if n > 0 => Ok(Some(n)),
        Tier::Name(name) if name == "any" => Ok(None),
        _ => Err(serde::de::Error::custom("tier must be a positive integer or \"any\"")),
    }
}

fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}
