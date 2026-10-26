//! Influence: conqueror and Shaper/Elder exalted orbs (add the influence and one of its mods),
//! and influence orbs (add the influence only).

use super::{Cost, Ctx, Operation, Pick, Pool, Sides, Step, View};
use crate::goal::{Influence, Rarity};

pub struct Exalt {
    influence: Influence,
    /// Influence orbs add no mod.
    orb: bool,
    steps: Vec<Step>,
}

impl Exalt {
    /// `elder`, `warlord`, ... as in `influence-exalted:elder`, or `influence-orb:crusader`.
    pub fn new(arg: &str, orb: bool) -> Option<Exalt> {
        let influence = Influence::ALL.into_iter().find(|i| name(*i) == arg)?;
        let mut steps = vec![Step::Influence(influence)];
        if !orb {
            steps.push(Step::Draw(Pick { sides: Sides::Both, tag: None, pool: Pool::Influence(influence) }));
        }
        Some(Exalt { influence, orb, steps })
    }
}

fn name(i: Influence) -> &'static str {
    match i {
        Influence::Shaper => "shaper",
        Influence::Elder => "elder",
        Influence::Crusader => "crusader",
        Influence::Redeemer => "redeemer",
        Influence::Hunter => "hunter",
        Influence::Warlord => "warlord",
    }
}

impl Operation for Exalt {
    fn name(&self) -> String {
        let mechanic = if self.orb { "influence-orb" } else { "influence-exalted" };
        format!("{mechanic}:{}", name(self.influence))
    }

    /// A rare (any rarity for the orb) without influence, with room for the mod.
    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        !item.influenced()
            && if self.orb { item.rarity() != Rarity::Normal } else { item.rarity() == Rarity::Rare && item.room(Sides::Both) }
    }

    fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The influence orb is priced like the matching exalted orb (no market quote; SPEC §10).
    fn cost(&self) -> Cost {
        let id = match self.influence {
            Influence::Shaper => "shapers-exalted-orb",
            Influence::Elder => "elders-exalted-orb",
            Influence::Crusader => "crusaders-exalted-orb",
            Influence::Redeemer => "redeemers-exalted-orb",
            Influence::Hunter => "hunters-exalted-orb",
            Influence::Warlord => "warlords-exalted-orb",
        };
        Cost::of(id, 1.0, 2.0)
    }
}
