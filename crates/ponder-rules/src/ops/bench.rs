//! Crafting bench: one crafted mod, or removing them. Options, mods and costs come from the
//! game's bench table; guides name an option by what it adds.

use std::sync::Arc;

use super::{Cost, Ctx, Operation, Step, View};
use crate::data::BenchOption;
use crate::game::{Game, GroupId, ModId, Side};
use crate::goal::{Rarity, Source};

/// Guide names for bench options: the templates of the mods they may add, in preference order.
/// `any-prefix`, `any-suffix` and `any-mod` instead pick the cheapest craft on that side whose
/// groups no goal wants (a blocker).
const NAMES: [(&str, &[&str]); 31] = [
    ("prefixes-cannot-be-changed", &["Prefixes Cannot Be Changed"]),
    ("suffixes-cannot-be-changed", &["Suffixes Cannot Be Changed"]),
    ("cannot-roll-attack", &["Cannot roll Attack Modifiers"]),
    ("cannot-roll-caster", &["Cannot roll Caster Modifiers"]),
    ("multimod", &["Can have up to # Crafted Modifiers"]),
    ("accuracy-strength-and-dexterity", &["+# to Accuracy Rating / +# to Strength and Dexterity"]),
    (
        "area-of-effect",
        &["#% increased Area Damage / #% increased Area of Effect", "#% increased Area of Effect / +# to Level of Socketed AoE Gems"],
    ),
    ("area-of-effect-and-socketed-aoe", &["#% increased Area of Effect / +# to Level of Socketed AoE Gems"]),
    ("armour", &["#% increased Armour", "+# to Armour"]),
    ("attributes", &["+# to all Attributes"]),
    ("cast-speed", &["#% increased Cast Speed"]),
    ("chaos-damage-over-time-multiplier", &["+#% to Chaos Damage over Time Multiplier"]),
    (
        "convert",
        &[
            "#% of Physical Damage Converted to Fire Damage",
            "#% of Physical Damage Converted to Cold Damage",
            "#% of Physical Damage Converted to Lightning Damage",
        ],
    ),
    ("convert-cold", &["#% of Physical Damage Converted to Cold Damage"]),
    ("convert-fire", &["#% of Physical Damage Converted to Fire Damage"]),
    ("critical-strike-chance", &["#% increased Critical Strike Chance", "#% increased Global Critical Strike Chance"]),
    ("critical-strike-multiplier", &["+#% to Global Critical Strike Multiplier"]),
    (
        "damage-over-time-multiplier",
        &[
            "+#% to Cold Damage over Time Multiplier",
            "+#% to Fire Damage over Time Multiplier",
            "+#% to Chaos Damage over Time Multiplier",
            "+#% to Physical Damage over Time Multiplier",
        ],
    ),
    ("dexterity-and-intelligence", &["+# to Dexterity and Intelligence"]),
    ("fire-and-chaos-resistance", &["+#% to Fire and Chaos Resistances"]),
    ("fire-damage-to-attacks", &["Adds # to # Fire Damage to Attacks"]),
    ("life", &["+# to maximum Life"]),
    ("minimum-frenzy-charges", &["+# to Minimum Frenzy Charges"]),
    ("minion-damage", &["Minions deal #% increased Damage"]),
    (
        "minion-damage-or-life",
        &[
            "Minions deal #% increased Damage / Minions have #% increased maximum Life",
            "Minions deal #% increased Damage",
            "Minions have #% increased maximum Life",
        ],
    ),
    ("physical-damage", &["#% increased Physical Damage", "Adds # to # Physical Damage"]),
    ("physical-damage-over-time-multiplier", &["+#% to Physical Damage over Time Multiplier"]),
    ("projectile-speed", &["#% increased Projectile Damage / #% increased Projectile Speed"]),
    ("resistance", &["+#% to Fire Resistance", "+#% to Cold Resistance", "+#% to Lightning Resistance"]),
    ("socketed-support-gems", &["+# to Level of Socketed Support Gems"]),
    (
        "speed-or-critical",
        &[
            "#% increased Attack Speed / +# to Dexterity and Intelligence",
            "#% increased Critical Strike Chance / +# to Strength and Intelligence",
        ],
    ),
];

/// More bench names that need no table: one template each.
const MORE: [(&str, &str); 3] = [
    ("spell-damage", "#% increased Spell Damage"),
    ("trigger-on-focus", "Trigger Socketed Spells when you Focus, with a # second Cooldown"),
    (
        "trigger-on-skill-use",
        "Trigger a Socketed Spell when you Use a Skill, with a # second Cooldown / Spells Triggered this way have #% more Cost",
    ),
];

/// Bench currency by metadata id: price id and how cheap it is (for blockers).
const CURRENCY: [(&str, &str, u8); 18] = [
    ("CurrencyUpgradeToMagic", "transmute", 1),
    ("CurrencyRerollMagic", "alt", 2),
    ("CurrencyAddModToMagic", "aug", 3),
    ("CurrencyConvertToNormal", "scour", 4),
    ("CurrencyUpgradeToRare", "alch", 5),
    ("CurrencyUpgradeMagicToRare", "regal", 6),
    ("CurrencyRerollRare", "chaos", 7),
    ("CurrencyRerollSocketColours", "chrome", 7),
    ("CurrencyRerollSocketNumbers", "jewellers", 7),
    ("CurrencyRerollSocketLinks", "fusing", 7),
    ("CurrencyArmourQuality", "scrap", 3),
    ("CurrencyGemQuality", "gcp", 8),
    ("CurrencyUpgradeRandomly", "chance", 8),
    ("CurrencyInstillingOrb", "instilling-orb", 8),
    ("CurrencyFlaskQuality", "glassblower", 8),
    ("CurrencyCorrupt", "vaal", 8),
    ("CurrencyAddModToRare", "exalted", 9),
    ("CurrencyModValues", "divine", 10),
];

pub enum Bench {
    /// Add the first of these crafted mods that fits.
    Craft {
        name: String,
        steps: [Step; 1],
        cost: Cost,
    },
    RemoveCrafted,
}

impl Bench {
    /// A bench option by guide name for an item class, e.g. `prefixes-cannot-be-changed`,
    /// `fire-and-chaos-resistance`, `any-suffix` or `remove-crafted`.
    pub fn new(game: &Game, name: &str, class: &str, avoid: &[GroupId]) -> Option<Bench> {
        if name == "remove-crafted" {
            return Some(Bench::RemoveCrafted);
        }
        let options: Vec<(ModId, &BenchOption)> = game
            .bench
            .iter()
            .filter(|o| o.item_classes.iter().any(|c| c == class))
            .filter_map(|o| Some((game.mod_id(o.actions.add_explicit_mod.as_deref()?)?, o)))
            .filter(|(m, _)| game.def(*m).source == Source::Crafted)
            .collect();
        let chosen: Vec<(ModId, &BenchOption)> = if let Some(side) = name.strip_prefix("any-") {
            let side = match side {
                "prefix" => Some(Side::Prefix),
                "suffix" => Some(Side::Suffix),
                "mod" => None,
                _ => return None,
            };
            let blocker = options
                .iter()
                .filter(|(m, _)| {
                    let def = game.def(*m);
                    side.is_none_or(|s| s == def.side) && !game.metamods.contains(m) && !def.groups.iter().any(|g| avoid.contains(g))
                })
                .min_by_key(|(m, o)| (cheapness(o), game.def(*m).level, *m))?;
            vec![*blocker]
        } else {
            let templates: Vec<&str> = match NAMES.iter().find(|(n, _)| *n == name) {
                Some((_, t)) => t.to_vec(),
                None => vec![MORE.iter().find(|(n, _)| *n == name)?.1],
            };
            // Per template, the highest-level craft for this class.
            templates
                .iter()
                .filter_map(|t| {
                    options
                        .iter()
                        .filter(|(m, _)| game.def(*m).template.eq_ignore_ascii_case(t))
                        .max_by_key(|(m, _)| (game.def(*m).level, *m))
                })
                .copied()
                .collect()
        };
        let first = chosen.first()?.1;
        let cost = Cost { items: first.cost.iter().map(|(id, n)| (price_id(id), *n as f64)).collect(), seconds: 5.0 };
        let mods = chosen.iter().map(|c| c.0).collect();
        Some(Bench::Craft { name: name.into(), steps: [Step::Craft(Arc::new(mods))], cost })
    }
}

fn currency(id: &str) -> Option<&(&'static str, &'static str, u8)> {
    let short = id.rsplit('/').next().unwrap_or(id);
    CURRENCY.iter().find(|c| c.0 == short)
}

fn price_id(id: &str) -> String {
    currency(id).map_or_else(|| id.to_string(), |c| c.1.to_string())
}

fn cheapness(o: &BenchOption) -> (u8, u32) {
    o.cost.iter().map(|(id, n)| (currency(id).map_or(u8::MAX, |c| c.2), *n)).max().unwrap_or((u8::MAX, 0))
}

impl Operation for Bench {
    fn name(&self) -> String {
        match self {
            Bench::Craft { name, .. } => format!("bench:{name}"),
            Bench::RemoveCrafted => "bench:remove-crafted".into(),
        }
    }

    /// One crafted mod at a time, three with the multimod craft (SPEC §4.6).
    fn applies(&self, ctx: &Ctx, item: &dyn View) -> bool {
        match self {
            Bench::Craft { steps: [Step::Craft(mods)], .. } => {
                let limit = if ctx.game.metamods.get(4).is_some_and(|mm| item.has(*mm)) { 3 } else { 1 };
                item.rarity() != Rarity::Normal && item.crafted() < limit && mods.iter().any(|m| item.fits(*m))
            }
            Bench::Craft { .. } => unreachable!(),
            Bench::RemoveCrafted => item.crafted() > 0,
        }
    }

    fn steps(&self) -> &[Step] {
        match self {
            Bench::Craft { steps, .. } => steps,
            Bench::RemoveCrafted => &[Step::RemoveCrafted],
        }
    }

    fn cost(&self) -> Cost {
        match self {
            Bench::Craft { cost, .. } => cost.clone(),
            Bench::RemoveCrafted => Cost::of("scour", 1.0, 3.0),
        }
    }
}
