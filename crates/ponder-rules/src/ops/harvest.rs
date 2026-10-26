//! Harvest reforges: reroll as rare, guaranteeing one mod with a tag.
//!
//! The guaranteed mod is drawn first from the tagged pool, then the rest as usual; "Cannot Be
//! Changed" keeps its side.

use super::{Cost, Ctx, Operation, Step, View};
use crate::game::Game;
use crate::goal::Rarity;

/// (craft name, mod tag, lifeforce price id, amount), from poedb's Horticrafting list.
const REFORGES: [(&str, &str, &str, f64); 15] = [
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

pub struct Reforge {
    name: &'static str,
    steps: [Step; 5],
    cost: Cost,
}

impl Reforge {
    /// `reforge-critical` and friends.
    pub fn new(game: &Game, name: &str) -> Option<Reforge> {
        let which = name.strip_prefix("reforge-")?;
        let &(name, tag, lifeforce, amount) = REFORGES.iter().find(|r| r.0 == which)?;
        let tag = game.tag_id(tag)?;
        Some(Reforge {
            name,
            steps: [
                Step::Clear { keep_locked: true },
                Step::Rarity(Rarity::Rare),
                Step::RollCount,
                Step::Draw { side: None, tag: Some(tag) },
                Step::Fill,
            ],
            cost: Cost { items: vec![(lifeforce.into(), amount)], seconds: 4.0 },
        })
    }
}

impl Operation for Reforge {
    fn name(&self) -> String {
        format!("harvest:reforge-{}", self.name)
    }

    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        item.rarity() == Rarity::Rare
    }

    fn steps(&self) -> &[Step] {
        &self.steps
    }

    fn cost(&self) -> Cost {
        self.cost.clone()
    }
}
