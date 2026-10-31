//! Beastcrafts and imprints.
//!
//! - `imprint`: a Craicic Croaker saves a magic, unfractured item as an imprint.
//! - `restore`: turn the item back into its imprint; the imprint is used up.
//! - `beast:wild-bristle-matron`: add a Prefixes or Suffixes Cannot Be Changed lock, either one
//!   that fits (SPEC §10: as likely as the other).
//! - `beast:lock`: Hinekora's Lock on a magic item, from a Craicic Croaker and a Black Mórrígan:
//!   the next currency's result shows before it is used, and any currency uses the lock up.

use std::sync::Arc;

use super::{Cost, Ctx, Operation, Step, View};
use crate::game::Game;
use crate::goal::Rarity;

pub struct Beast {
    name: &'static str,
    steps: Vec<Step>,
    cost: Cost,
}

pub fn parse(game: &Game, op: &str) -> Option<Box<dyn Operation>> {
    let (name, steps, cost) = match op {
        "imprint" => ("imprint", vec![Step::Imprint], Cost::of("craicic-croaker", 1.0, 10.0)),
        "restore" => ("restore", vec![Step::Restore], Cost { items: Vec::new(), seconds: 2.0 }),
        "beast:lock" => (
            "beast:lock",
            vec![Step::Foresee],
            Cost { items: vec![("craicic-croaker".into(), 1.0), ("black-morrigan".into(), 1.0)], seconds: 10.0 },
        ),
        "beast:wild-bristle-matron" => (
            "beast:wild-bristle-matron",
            vec![Step::CraftAny(Arc::new(game.metamods.get(..2)?.to_vec()))],
            Cost::of("wild-bristle-matron", 1.0, 10.0),
        ),
        _ => return None,
    };
    Some(Box::new(Beast { name, steps, cost }))
}

impl Operation for Beast {
    fn name(&self) -> String {
        self.name.into()
    }

    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        match &self.steps[0] {
            Step::Imprint => item.rarity() == Rarity::Magic && item.fractured() == 0,
            Step::Restore => item.imprinted(),
            Step::Foresee => item.rarity() == Rarity::Magic && !item.foreseen(),
            Step::CraftAny(locks) => item.rarity() != Rarity::Normal && locks.iter().any(|&m| item.fits(m)),
            _ => unreachable!(),
        }
    }

    fn steps(&self) -> &[Step] {
        &self.steps
    }

    fn cost(&self) -> Cost {
        self.cost.clone()
    }
}
