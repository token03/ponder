//! Veiled orbs and unveiling.
//!
//! A Veiled Chaos Orb rerolls a rare with one veiled mod among the new ones (on a side with
//! room, evenly); a Veiled Exalted Orb adds one. Unveiling shows three offers from the veiled
//! mod's side and keeps one: guides name what they keep (`unveil:speed-or-critical`).

use std::sync::Arc;

use super::{Cost, Ctx, Operation, Pick, Sides, Step, View};
use crate::game::{Game, ModId};
use crate::goal::{Rarity, Source};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orb {
    Chaos,
    Exalted,
}

const CHAOS: &[Step] = &[
    Step::Clear { sides: Sides::Both, respect_lock: true },
    Step::Rarity(Rarity::Rare),
    Step::RollCount,
    Step::Veil(Sides::Both),
    Step::Fill(Pick::ANY),
];
const EXALTED: &[Step] = &[Step::Veil(Sides::Both)];

impl Orb {
    pub fn parse(name: &str) -> Option<Orb> {
        match name {
            "veiled-chaos" => Some(Orb::Chaos),
            "veiled-exalted" => Some(Orb::Exalted),
            _ => None,
        }
    }
}

impl Operation for Orb {
    fn name(&self) -> String {
        match self {
            Orb::Chaos => "veiled-chaos",
            Orb::Exalted => "veiled-exalted",
        }
        .into()
    }

    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        match self {
            Orb::Chaos => item.rarity() == Rarity::Rare,
            Orb::Exalted => item.rarity() == Rarity::Rare && !item.veiled() && item.room(Sides::Both),
        }
    }

    fn steps(&self) -> &[Step] {
        match self {
            Orb::Chaos => CHAOS,
            Orb::Exalted => EXALTED,
        }
    }

    fn cost(&self) -> Cost {
        match self {
            Orb::Chaos => Cost::of("veiled-chaos-orb", 1.0, 2.0),
            Orb::Exalted => Cost::of("veiled-exalted-orb", 1.0, 2.0),
        }
    }
}

/// What a guide keeps from an unveil, as mod templates in preference order.
const KEEPS: [(&str, &[&str]); 12] = [
    (
        "area-of-effect",
        &["#% increased Area of Effect / +# to Level of Socketed AoE Gems", "#% increased Area Damage / #% increased Area of Effect"],
    ),
    ("chaos-penetration", &["Attacks with this Weapon Penetrate #% Chaos Resistance"]),
    ("convert-cold", &["#% of Physical Damage Converted to Cold Damage"]),
    ("convert-fire", &["#% of Physical Damage Converted to Fire Damage"]),
    (
        "critical-or-speed",
        &[
            "#% increased Critical Strike Chance / +# to Strength and Intelligence",
            "#% increased Attack Speed / +# to Dexterity and Intelligence",
        ],
    ),
    (
        "speed-or-critical",
        &[
            "#% increased Attack Speed / +# to Dexterity and Intelligence",
            "#% increased Critical Strike Chance / +# to Strength and Intelligence",
        ],
    ),
    ("energy-shield-regeneration", &["Regenerate # Energy Shield per second while a Rare or Unique Enemy is Nearby"]),
    ("fire-and-chaos", &["+#% to Fire and Chaos Resistances"]),
    ("minion-cast-speed", &["Minions have #% increased Attack Speed / Minions have #% increased Cast Speed"]),
    ("movement-speed", &["#% increased Movement Speed / #% chance to gain Onslaught for # seconds on Kill", "#% increased Movement Speed"]),
    (
        "physical-damage",
        &[
            "#% increased Physical Damage / #% chance to cause Bleeding on Hit",
            "#% increased Physical Damage / #% chance to Impale Enemies on Hit with Attacks",
        ],
    ),
    ("trigger-on-focus", &["Trigger Socketed Spells when you Focus, with a # second Cooldown"]),
];

pub struct Unveil {
    name: String,
    steps: [Step; 1],
}

impl Unveil {
    pub fn new(game: &Game, arg: &str) -> Option<Unveil> {
        let templates = KEEPS.iter().find(|(n, _)| *n == arg)?.1;
        let prefs: Vec<Vec<ModId>> = templates
            .iter()
            .map(|t| game.with_template(t).iter().copied().filter(|m| game.def(*m).source == Source::Veiled).collect::<Vec<_>>())
            .filter(|p| !p.is_empty())
            .collect();
        if prefs.is_empty() {
            return None;
        }
        Some(Unveil { name: arg.into(), steps: [Step::Unveil(Arc::new(prefs))] })
    }
}

impl Operation for Unveil {
    fn name(&self) -> String {
        format!("unveil:{}", self.name)
    }

    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        item.veiled()
    }

    fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Unveiling at Jun's bench is free; only time.
    fn cost(&self) -> Cost {
        Cost { items: Vec::new(), seconds: 6.0 }
    }
}
