//! Essences: reforge as rare with one forced mod.

use super::{Cost, Ctx, Operation, Step, View};
use crate::game::{Game, ModId};
use crate::goal::Rarity;

pub struct Essence {
    /// e.g. "deafening-woe".
    pub name: String,
    price: String,
    steps: [Step; 5],
}

impl Essence {
    /// The essence named like `deafening-woe` for an item class, if it has a mod there.
    pub fn new(game: &Game, name: &str, class: &str) -> Option<Essence> {
        let (tier, kind) = name.split_once('-').unwrap_or(("", name));
        let display = |e: &crate::data::Essence| e.name.to_lowercase().replace(' ', "-");
        let wanted = if tier.is_empty() { format!("essence-of-{kind}") } else { format!("{tier}-essence-of-{kind}") };
        let essence = game.essences.iter().find(|e| display(e) == wanted)?;
        let forced: ModId = game.mod_id(essence.mods.get(class)?.as_deref()?)?;
        Some(Essence {
            name: name.into(),
            price: wanted,
            steps: [Step::Clear { keep_locked: false }, Step::Rarity(Rarity::Rare), Step::RollCount, Step::Force(forced), Step::Fill],
        })
    }

    pub fn forced(&self) -> ModId {
        match self.steps[3] {
            Step::Force(m) => m,
            _ => unreachable!(),
        }
    }
}

impl Operation for Essence {
    fn name(&self) -> String {
        format!("essence:{}", self.name)
    }

    /// Normal or rare items without a bench metamod (SPEC §4.7), unless a fractured mod
    /// blocks the forced one.
    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        matches!(item.rarity(), Rarity::Normal | Rarity::Rare) && !item.has_metamod() && !item.fixed_blocks(self.forced())
    }

    fn steps(&self) -> &[Step] {
        &self.steps
    }

    fn cost(&self) -> Cost {
        Cost { items: vec![(self.price.clone(), 1.0)], seconds: 2.0 }
    }
}
