use serde::{Deserialize, Serialize};

use crate::types::*;

pub const FOSSIL_SECONDS: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Transmute,
    Alt,
    Aug,
    Regal,
    Exalt,
    Annul,
    RerollMagic,
    RerollRare,
    Reforge,
    Augment,
    ExaltPrefix,
    ExaltSuffix,
    AnnulPrefix,
    AnnulSuffix,
    ChaosPrefix,
    ChaosSuffix,
    BeastPrefixToSuffix,
    BeastSuffixToPrefix,
    BristleMatron,
    VeiledChaos,
    VeiledExalt,
    Unveil(u8),
    RevealUnveil(u8),
    MetaLockPrefixes,
    MetaLockSuffixes,
    MetaNoAttack,
    MetaNoCaster,
    RemoveCrafted,
    Scour,
    ProtectedScour(Side),
    MatronScour,
    FinishBench,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Roll(u8),
    Add(u8, Option<Side>),
    Annul(Option<Side>),
    Reforge,
    Augment,
    RerollSide(Side),
    RemoveAdd(Side),
    RandomMeta,
    Veil(bool),
    Unveil(u8),
    RevealUnveil(u8),
    Meta(u8),
    Unmeta,
    Scour,
    ProtectedScour { side: Side, require_full_side: bool },
    FinishBench,
}

impl Effect {
    pub(crate) fn available_observation(self, pending: bool, offers: bool) -> bool {
        if offers {
            matches!(self, Self::Unveil(_))
        } else if pending {
            matches!(self, Self::RevealUnveil(_) | Self::Unmeta)
        } else {
            !matches!(self, Self::Unveil(_) | Self::RevealUnveil(_))
        }
    }
}

impl ActionKind {
    pub fn effect(self) -> Effect {
        use ActionKind::*;
        match self {
            Transmute | Alt | RerollMagic => Effect::Roll(MAGIC),
            RerollRare => Effect::Roll(RARE),
            Aug => Effect::Add(MAGIC, None),
            Regal | Exalt => Effect::Add(RARE, None),
            ExaltPrefix => Effect::Add(RARE, Some(Side::Prefix)),
            ExaltSuffix => Effect::Add(RARE, Some(Side::Suffix)),
            Annul => Effect::Annul(None),
            AnnulPrefix => Effect::Annul(Some(Side::Prefix)),
            AnnulSuffix => Effect::Annul(Some(Side::Suffix)),
            ChaosPrefix => Effect::RerollSide(Side::Prefix),
            ChaosSuffix => Effect::RerollSide(Side::Suffix),
            BeastPrefixToSuffix => Effect::RemoveAdd(Side::Prefix),
            BeastSuffixToPrefix => Effect::RemoveAdd(Side::Suffix),
            BristleMatron => Effect::RandomMeta,
            VeiledChaos => Effect::Veil(true),
            VeiledExalt => Effect::Veil(false),
            Unveil(class) => Effect::Unveil(class),
            RevealUnveil(blocker) => Effect::RevealUnveil(blocker),
            Reforge => Effect::Reforge,
            Augment => Effect::Augment,
            MetaLockPrefixes => Effect::Meta(META_LOCK_PREFIXES),
            MetaLockSuffixes => Effect::Meta(META_LOCK_SUFFIXES),
            MetaNoAttack => Effect::Meta(META_NO_ATTACK),
            MetaNoCaster => Effect::Meta(META_NO_CASTER),
            RemoveCrafted => Effect::Unmeta,
            Scour => Effect::Scour,
            ProtectedScour(side) => Effect::ProtectedScour { side, require_full_side: false },
            MatronScour => Effect::ProtectedScour { side: Side::Suffix, require_full_side: true },
            FinishBench => Effect::FinishBench,
        }
    }
}

fn default_seconds<'a>(kind: ActionKind, inputs: impl Iterator<Item = &'a str>) -> f64 {
    let time = match kind {
        ActionKind::RerollMagic | ActionKind::RemoveCrafted => 3.0,
        _ => match kind.effect() {
            Effect::Roll(MAGIC) | Effect::Add(MAGIC, _) | Effect::Scour => 1.0,
            Effect::Meta(_) => 5.0,
            _ => 2.0,
        },
    };
    if matches!(kind, ActionKind::RerollRare | ActionKind::Reforge) {
        inputs.fold(time, |t, id| {
            t + if id == "scour" {
                1.0
            } else if id.ends_with("-fossil") {
                FOSSIL_SECONDS
            } else {
                0.0
            }
        })
    } else {
        time
    }
}

fn base_profile() -> String {
    "base".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
    pub kind: ActionKind,
    pub cost: Vec<(String, f64)>,
    pub seconds: f64,
    pub from_rarity: Vec<u8>,
    #[serde(default = "base_profile")]
    pub profile: String, // weight profile for rolled mods
    #[serde(default)]
    pub first_profile: Option<String>, // Harvest: the first mod comes from a tag-filtered pool
    #[serde(default)]
    pub forced: Option<String>, // essence: this mod is always added
}

impl Action {
    pub fn new(name: &str, kind: ActionKind, cost: &[(&str, f64)], from_rarity: &[u8]) -> Action {
        Action {
            name: name.into(),
            kind,
            seconds: default_seconds(kind, cost.iter().map(|(id, _)| *id)),
            cost: cost.iter().map(|(p, q)| (p.to_string(), *q)).collect(),
            from_rarity: from_rarity.to_vec(),
            profile: base_profile(),
            first_profile: None,
            forced: None,
        }
    }

    pub fn with_seconds(mut self, seconds: f64) -> Action {
        self.seconds = seconds;
        self
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
}

pub fn action_cost(a: &Action, prices: &Prices) -> f64 {
    a.cost.iter().map(|(pid, qty)| prices.get(pid).copied().unwrap_or(0.0) * qty).sum()
}

pub fn step_cost(a: &Action, prices: &Prices, settings: &Settings) -> f64 {
    action_cost(a, prices) + a.seconds * settings.chaos_per_hour / 3600.0
}
