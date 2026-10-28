//! Basic currency orbs.

use super::{Cost, Ctx, Operation, Pick, Sides, Step, View};
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
    Fracturing,
}

const ANY: Pick = Pick::ANY;
const REMAKE_MAGIC: &[Step] =
    &[Step::Clear { sides: Sides::Both, respect_lock: false }, Step::Rarity(Rarity::Magic), Step::RollCount, Step::Fill(ANY)];
const REMAKE_RARE: &[Step] =
    &[Step::Clear { sides: Sides::Both, respect_lock: false }, Step::Rarity(Rarity::Rare), Step::RollCount, Step::Fill(ANY)];
/// Under a lock, chaos keeps the locked mods; new ones may land on either side (SPEC §4.7).
const CHAOS: &[Step] =
    &[Step::Clear { sides: Sides::Both, respect_lock: true }, Step::Rarity(Rarity::Rare), Step::RollCount, Step::Fill(ANY)];
const ADD: &[Step] = &[Step::Draw(ANY)];
const REGAL: &[Step] = &[Step::Rarity(Rarity::Rare), Step::Draw(ANY)];
const ANNUL: &[Step] = &[Step::Remove { sides: Sides::Both, tag: None, respect_lock: true, respect_bans: true }];
const SCOUR: &[Step] = &[Step::Clear { sides: Sides::Both, respect_lock: true }, Step::Settle];
const FRACTURE: &[Step] = &[Step::Fracture];

impl Currency {
    pub const ALL: [Currency; 10] = [
        Currency::Transmutation,
        Currency::Alteration,
        Currency::Augmentation,
        Currency::Regal,
        Currency::Alchemy,
        Currency::Chaos,
        Currency::Exalted,
        Currency::Annulment,
        Currency::Scouring,
        Currency::Fracturing,
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
            Currency::Fracturing => "fracturing-orb",
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
            Currency::Annulment => item.rarity() != Rarity::Normal && item.removable(true, true) > 0,
            Currency::Scouring => item.rarity() != Rarity::Normal,
            // SPEC §4.5: a rare with at least four mods, not influenced, nothing fractured yet.
            Currency::Fracturing => item.rarity() == Rarity::Rare && item.mods() >= 4 && !item.influenced() && item.fractured() == 0,
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
            Currency::Fracturing => FRACTURE,
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
