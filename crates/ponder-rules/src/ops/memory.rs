//! Memory strands: the Orb of Remembrance randomises them on a normal item. Currencies use them
//! up ([`super::Step::Forget`]) and they keep low tiers from rolling ([`crate::game::Game::strand_factor`]).

use super::{Cost, Ctx, Operation, Step, View};
use crate::goal::Rarity;

pub struct Remembrance;

impl Operation for Remembrance {
    fn name(&self) -> String {
        "remembrance".into()
    }

    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        item.rarity() == Rarity::Normal
    }

    fn steps(&self) -> &[Step] {
        &[Step::Remember]
    }

    fn cost(&self) -> Cost {
        Cost::of("orb-of-remembrance", 1.0, 2.0)
    }
}
