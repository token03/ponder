//! Beastcrafts and imprints.
//!
//! - `imprint`: a Craicic Croaker saves a magic, unfractured item as an imprint.
//! - `restore`: turn the item back into its imprint; the imprint is used up.
//! - `beast:wild-bristle-matron`: add a random eligible crafted meta-modifier.
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

pub fn parse(game: &Game, class: &str, op: &str) -> Option<Box<dyn Operation>> {
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
            vec![Step::CraftAny(Arc::new(
                game.metamods
                    .iter()
                    .copied()
                    .filter(|&m| {
                        game.bench.iter().any(|option| {
                            option.item_classes.iter().any(|c| c == class)
                                && option.actions.add_explicit_mod.as_deref() == Some(game.def(m).key.as_str())
                        })
                    })
                    .collect(),
            ))],
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

    fn applies(&self, ctx: &Ctx, item: &dyn View) -> bool {
        match &self.steps[0] {
            Step::Imprint => item.rarity() == Rarity::Magic && item.fractured() == 0,
            Step::Restore => item.imprinted(),
            Step::Foresee => item.rarity() == Rarity::Magic && !item.foreseen(),
            Step::CraftAny(mods) => {
                let multimod = ctx.game.metamods.get(4).is_some_and(|&m| item.has(m));
                item.rarity() != Rarity::Normal
                    && (item.crafted() == 0 || (item.crafted() == 1 && multimod))
                    && mods.iter().any(|&m| item.fits(m))
            }
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
