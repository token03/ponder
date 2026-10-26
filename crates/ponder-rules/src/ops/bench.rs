//! Crafting bench: one crafted mod, or removing them.

use super::{Cost, Ctx, Operation, Step, View};
use crate::game::{Game, ModId};

/// Bench option names the guides use, by crafted mod key.
const CRAFTS: [(&str, &str); 5] = [
    ("prefixes-cannot-be-changed", "StrMasterItemGenerationCannotChangePrefixes"),
    ("suffixes-cannot-be-changed", "DexMasterItemGenerationCannotChangeSuffixes"),
    ("cannot-roll-attack", "IntMasterItemGenerationCannotRollAttackAffixes"),
    ("cannot-roll-caster", "StrDexMasterItemGenerationCannotRollCasterAffixes"),
    ("multimod", "StrIntMasterItemGenerationCanHaveMultipleCraftedMods"),
];

pub enum Bench {
    Craft { name: String, steps: [Step; 1], cost: Cost },
    RemoveCrafted,
}

impl Bench {
    /// A named bench option (`prefixes-cannot-be-changed`, ..., `remove-crafted`).
    pub fn new(game: &Game, name: &str) -> Option<Bench> {
        if name == "remove-crafted" {
            return Some(Bench::RemoveCrafted);
        }
        let key = CRAFTS.iter().find(|(n, _)| *n == name)?.1;
        let m: ModId = game.mod_id(key)?;
        // Every metamod costs two Divine Orbs on the 3.29 bench.
        let cost = Cost { items: vec![("divine".into(), 2.0)], seconds: 5.0 };
        Some(Bench::Craft { name: name.into(), steps: [Step::Force(m)], cost })
    }
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
            Bench::Craft { steps: [Step::Force(m)], .. } => {
                let limit = if ctx.game.metamods.get(4).is_some_and(|mm| item.has(*mm)) { 3 } else { 1 };
                item.rarity() != crate::goal::Rarity::Normal && item.crafted() < limit && item.fits(*m)
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
            Bench::RemoveCrafted => Cost { items: vec![("scour".into(), 1.0)], seconds: 3.0 },
        }
    }
}
