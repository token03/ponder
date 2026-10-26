//! Basic currency orbs.

use super::{Ctx, Operation, add, clear, fill, remove, roll_count};
use crate::goal::Rarity;
use crate::item::Item;
use crate::rng::Rng;

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
}

impl Operation for Currency {
    fn name(&self) -> String {
        format!("{self:?}").to_lowercase()
    }

    fn applies(&self, ctx: &Ctx, item: &Item) -> bool {
        let game = ctx.game;
        let room = || crate::game::Side::BOTH.iter().any(|s| item.open(game, *s) > 0);
        match self {
            Currency::Transmutation | Currency::Alchemy => item.rarity == Rarity::Normal,
            Currency::Alteration => item.rarity == Rarity::Magic,
            Currency::Augmentation => item.rarity == Rarity::Magic && room(),
            Currency::Regal => item.rarity == Rarity::Magic,
            Currency::Chaos => item.rarity == Rarity::Rare,
            Currency::Exalted => item.rarity == Rarity::Rare && room(),
            Currency::Annulment => item.rarity != Rarity::Normal && item.affixes.iter().any(|a| !a.fractured),
            Currency::Scouring => item.rarity != Rarity::Normal,
        }
    }

    fn apply(&self, ctx: &Ctx, item: &mut Item, rng: &mut Rng) {
        match self {
            Currency::Transmutation | Currency::Alteration => {
                clear(item, |_| false);
                item.rarity = Rarity::Magic;
                let n = roll_count(ctx, item, Rarity::Magic, rng);
                fill(ctx, item, rng, n);
            }
            Currency::Alchemy | Currency::Chaos => {
                clear(item, |_| false);
                item.rarity = Rarity::Rare;
                let n = roll_count(ctx, item, Rarity::Rare, rng);
                fill(ctx, item, rng, n);
            }
            Currency::Augmentation | Currency::Exalted => {
                add(ctx, item, rng, None);
            }
            Currency::Regal => {
                item.rarity = Rarity::Rare;
                add(ctx, item, rng, None);
            }
            Currency::Annulment => {
                remove(item, rng, |_| false);
            }
            Currency::Scouring => {
                clear(item, |_| false);
                if item.affixes.is_empty() {
                    item.rarity = Rarity::Normal;
                }
            }
        }
    }
}
