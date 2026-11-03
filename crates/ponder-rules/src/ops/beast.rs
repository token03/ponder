//! Beastcrafts and imprints.
//!
//! - `imprint`: a Craicic Croaker saves a magic, unfractured item as an imprint.
//! - `restore`: turn the item back into its imprint; the imprint is used up.
//! - `beast:wild-bristle-matron`: add a random eligible crafted meta-modifier.
//! - `beast:wild-bristle-matron:<prefixes|suffixes>:<k>`: Matrons until that side's lock comes, on
//!   an item where `k` meta-modifiers fit; anything else one adds is removed at the bench. About
//!   `k` Matrons and `k - 1` removals, and no other outcome: plans use these, as the random
//!   outcomes multiplied states for a cheap beast (SPEC §10).
//! - `beast:lock`: Hinekora's Lock on a magic item, from a Craicic Croaker and a Black Mórrígan:
//!   the next currency's result shows before it is used, and any currency uses the lock up.

use std::sync::Arc;

use super::{Cost, Ctx, Operation, Step, View};
use crate::game::{Game, ModId};
use crate::goal::Rarity;

pub struct Beast {
    name: &'static str,
    steps: Vec<Step>,
    cost: Cost,
}

pub fn parse(game: &Game, class: &str, op: &str) -> Option<Box<dyn Operation>> {
    if let Some(rest) = op.strip_prefix("beast:wild-bristle-matron:") {
        return Matron::parse(game, class, rest).map(|matron| Box::new(matron) as Box<dyn Operation>);
    }
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
            vec![Step::CraftAny(Arc::new(eligible(game, class)))],
            Cost::of("wild-bristle-matron", 1.0, MATRON_SECONDS),
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
            Step::CraftAny(mods) => takes_metamod(ctx, item) && mods.iter().any(|&m| item.fits(m)),
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

const MATRON_SECONDS: f64 = 10.0;
const REMOVE_SECONDS: f64 = 3.0;

/// The meta-modifiers a Matron can add to an item of `class`: those its bench offers.
fn eligible(game: &Game, class: &str) -> Vec<ModId> {
    game.metamods
        .iter()
        .copied()
        .filter(|&m| {
            game.bench.iter().any(|option| {
                option.item_classes.iter().any(|c| c == class)
                    && option.actions.add_explicit_mod.as_deref() == Some(game.def(m).key.as_str())
            })
        })
        .collect()
}

/// Can a Matron add a meta-modifier: a magic or rare item with no crafted mod but a multimod.
fn takes_metamod(ctx: &Ctx, item: &dyn View) -> bool {
    let multimod = ctx.game.metamods.get(4).is_some_and(|&m| item.has(m));
    item.rarity() != Rarity::Normal && (item.crafted() == 0 || (item.crafted() == 1 && multimod))
}

/// Wild Bristle Matrons until one adds `lock`, where `tries` meta-modifiers fit.
pub struct Matron {
    name: String,
    eligible: Vec<ModId>,
    tries: usize,
    steps: [Step; 1],
}

impl Matron {
    /// `prefixes:3`: the Prefixes Cannot Be Changed lock, where three meta-modifiers fit.
    fn parse(game: &Game, class: &str, arg: &str) -> Option<Matron> {
        let (side, tries) = arg.split_once(':')?;
        let lock = match side {
            "prefixes" => *game.metamods.first()?,
            "suffixes" => *game.metamods.get(1)?,
            _ => return None,
        };
        let tries: usize = tries.parse().ok()?;
        let eligible = eligible(game, class);
        if tries == 0 || tries > eligible.len() || !eligible.contains(&lock) {
            return None;
        }
        Some(Matron { name: format!("beast:wild-bristle-matron:{arg}"), eligible, tries, steps: [Step::Craft(Arc::new(vec![lock]))] })
    }
}

impl Operation for Matron {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn applies(&self, ctx: &Ctx, item: &dyn View) -> bool {
        let Step::Craft(lock) = &self.steps[0] else { unreachable!() };
        takes_metamod(ctx, item) && item.fits(lock[0]) && self.eligible.iter().filter(|&&m| item.fits(m)).count() == self.tries
    }

    fn steps(&self) -> &[Step] {
        &self.steps
    }

    fn cost(&self) -> Cost {
        let (matrons, removals) = (self.tries as f64, self.tries as f64 - 1.0);
        let mut items = vec![("wild-bristle-matron".into(), matrons)];
        if removals > 0.0 {
            items.push(("scour".into(), removals));
        }
        Cost { items, seconds: matrons * MATRON_SECONDS + removals * REMOVE_SECONDS }
    }
}
