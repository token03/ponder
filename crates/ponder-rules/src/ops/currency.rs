//! Basic currency orbs.

use super::{Cost, Ctx, Operation, Step, View};
use crate::game::Side;
use crate::goal::Rarity;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Currency {
    Transmutation,
    Alteration,
    Augmentation,
    Regal,
    Alchemy,
    Chaos,
    Exalted,
    Annulment,
    Scouring,
}

const REMAKE_MAGIC: &[Step] = &[Step::Clear { keep_locked: false }, Step::Rarity(Rarity::Magic), Step::RollCount, Step::Fill];
const REMAKE_RARE: &[Step] = &[Step::Clear { keep_locked: false }, Step::Rarity(Rarity::Rare), Step::RollCount, Step::Fill];
const CHAOS: &[Step] = &[Step::Clear { keep_locked: true }, Step::Rarity(Rarity::Rare), Step::RollCount, Step::Fill];
const ADD: &[Step] = &[Step::Draw { side: None, tag: None }];
const REGAL: &[Step] = &[Step::Rarity(Rarity::Rare), Step::Draw { side: None, tag: None }];
const ANNUL: &[Step] = &[Step::Remove { respect_lock: true }];
const SCOUR: &[Step] = &[Step::Clear { keep_locked: true }, Step::Settle];

impl Currency {
    pub const ALL: [Currency; 9] = [
        Currency::Transmutation,
        Currency::Alteration,
        Currency::Augmentation,
        Currency::Regal,
        Currency::Alchemy,
        Currency::Chaos,
        Currency::Exalted,
        Currency::Annulment,
        Currency::Scouring,
    ];

    pub fn parse(name: &str) -> Option<Currency> {
        Currency::ALL.into_iter().find(|c| c.name() == name)
    }

    /// poe.ninja price id.
    fn price(self) -> &'static str {
        match self {
            Currency::Transmutation => "transmute",
            Currency::Alteration => "alt",
            Currency::Augmentation => "aug",
            Currency::Regal => "regal",
            Currency::Alchemy => "alch",
            Currency::Chaos => "chaos",
            Currency::Exalted => "exalted",
            Currency::Annulment => "annul",
            Currency::Scouring => "scour",
        }
    }
}

impl Operation for Currency {
    fn name(&self) -> String {
        format!("{self:?}").to_lowercase()
    }

    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        let room = Side::BOTH.iter().any(|s| item.open(*s) > 0);
        match self {
            Currency::Transmutation | Currency::Alchemy => item.rarity() == Rarity::Normal,
            Currency::Alteration => item.rarity() == Rarity::Magic,
            Currency::Augmentation => item.rarity() == Rarity::Magic && room,
            Currency::Regal => item.rarity() == Rarity::Magic,
            Currency::Chaos => item.rarity() == Rarity::Rare,
            Currency::Exalted => item.rarity() == Rarity::Rare && room,
            Currency::Annulment => item.rarity() != Rarity::Normal && item.removable(true) > 0,
            Currency::Scouring => item.rarity() != Rarity::Normal,
        }
    }

    fn steps(&self) -> &[Step] {
        match self {
            Currency::Transmutation | Currency::Alteration => REMAKE_MAGIC,
            Currency::Alchemy => REMAKE_RARE,
            Currency::Chaos => CHAOS,
            Currency::Augmentation | Currency::Exalted => ADD,
            Currency::Regal => REGAL,
            Currency::Annulment => ANNUL,
            Currency::Scouring => SCOUR,
        }
    }

    fn cost(&self) -> Cost {
        let seconds = match self {
            Currency::Transmutation | Currency::Alteration | Currency::Augmentation | Currency::Scouring => 1.0,
            _ => 2.0,
        };
        Cost { items: vec![(self.price().into(), 1.0)], seconds }
    }
}
