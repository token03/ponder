//! Harvest crafts.
//!
//! - `reforge-X`: reroll any normal, magic or rare item as rare, guaranteeing one mod tagged X. The guaranteed mod is drawn
//!   first, then the rest as usual; "Cannot Be Changed" keeps the locked mods.
//! - `add-remove-X` (guides also say `augment-X`): add a new X mod, then remove another random
//!   mod; non-influenced items only. `add-X-remove-non-X` only removes mods without the tag.

use super::{Cost, Ctx, Operation, Pick, Pool, Sides, Step, View};
use crate::game::Game;
use crate::goal::Rarity;

/// (craft name, mod tag, lifeforce price id, reforge amount), from poedb's Horticrafting list.
const CRAFTS: [(&str, &str, &str, f64); 15] = [
    ("fire", "fire", "wild-lifeforce", 50.0),
    ("cold", "cold", "vivid-lifeforce", 50.0),
    ("lightning", "lightning", "primal-lifeforce", 50.0),
    ("physical", "physical", "vivid-lifeforce", 50.0),
    ("life", "life", "wild-lifeforce", 75.0),
    ("defence", "defences", "primal-lifeforce", 75.0),
    ("chaos", "chaos", "vivid-lifeforce", 100.0),
    ("attack", "attack", "wild-lifeforce", 75.0),
    ("caster", "caster", "primal-lifeforce", 75.0),
    ("speed", "speed", "vivid-lifeforce", 150.0),
    ("critical", "critical", "primal-lifeforce", 150.0),
    ("minion", "minion", "primal-lifeforce", 200.0),
    ("elemental", "elemental", "wild-lifeforce", 200.0),
    ("attribute", "attribute", "vivid-lifeforce", 200.0),
    ("mana", "mana", "primal-lifeforce", 200.0),
];

/// Lifeforce for an add/remove craft, as a multiple of the reforge's. Unverified (SPEC §10).
const ADD_REMOVE_FACTOR: f64 = 10.0;

pub fn parse(game: &Game, arg: &str) -> Option<Box<dyn Operation>> {
    let (kind, which) = if let Some(w) = arg.strip_prefix("reforge-") {
        (Kind::Reforge, w)
    } else if let Some(w) = arg.strip_prefix("add-remove-").or_else(|| arg.strip_prefix("augment-")) {
        (Kind::AddRemove { only_other: false }, w)
    } else {
        let (w, other) = arg.strip_prefix("add-")?.split_once("-remove-non-")?;
        if w != other {
            return None;
        }
        (Kind::AddRemove { only_other: true }, w)
    };
    let &(name, tag, lifeforce, amount) = CRAFTS.iter().find(|r| r.0 == which)?;
    let tag = game.tag_id(tag)?;
    let add = Pick { sides: Sides::Both, tag: Some((tag, true)), pool: Pool::Natural(None) };
    let (steps, amount) = match kind {
        Kind::Reforge => (
            vec![
                Step::Clear { sides: Sides::Both, respect_lock: true },
                Step::Rarity(Rarity::Rare),
                Step::RollCount,
                Step::Draw(add.clone()),
                Step::Fill(Pick::ANY),
            ],
            amount,
        ),
        Kind::AddRemove { only_other } => {
            (vec![Step::Swap { add, remove: only_other.then_some((tag, false)) }], amount * ADD_REMOVE_FACTOR)
        }
    };
    let label = match kind {
        Kind::Reforge => format!("reforge-{name}"),
        Kind::AddRemove { only_other: false } => arg.to_string(),
        Kind::AddRemove { only_other: true } => format!("add-{name}-remove-non-{name}"),
    };
    Some(Box::new(Harvest { name: label, kind, steps, cost: Cost::of(lifeforce, amount, 4.0) }))
}

/// Every reforge craft, with its tag.
pub fn reforges(game: &Game) -> Vec<(Box<dyn Operation>, crate::game::TagId)> {
    CRAFTS.iter().filter_map(|c| Some((parse(game, &format!("reforge-{}", c.0))?, game.tag_id(c.1)?))).collect()
}

#[derive(Clone, Copy)]
enum Kind {
    Reforge,
    AddRemove { only_other: bool },
}

pub struct Harvest {
    name: String,
    kind: Kind,
    steps: Vec<Step>,
    cost: Cost,
}

impl Operation for Harvest {
    fn name(&self) -> String {
        format!("harvest:{}", self.name)
    }

    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        match self.kind {
            // Any normal, magic or rare item.
            Kind::Reforge => true,
            Kind::AddRemove { .. } => item.rarity() != Rarity::Normal && !item.influenced() && item.room(Sides::Both) && item.mods() > 0,
        }
    }

    fn steps(&self) -> &[Step] {
        &self.steps
    }

    fn cost(&self) -> Cost {
        self.cost.clone()
    }
}
