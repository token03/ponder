//! Eldritch implicits and currency. The dominant implicit picks the side Eldritch currency works
//! on: the Searing Exarch (embers) prefixes, the Eater of Worlds (ichors) suffixes. Implicits
//! are not modelled beyond that side.

use super::{Cost, Ctx, Operation, Pick, Pool, Sides, Step, View};
use crate::game::Side;
use crate::goal::Rarity;

/// Make one implicit dominant by applying an ember or ichor.
pub struct Implicit {
    name: String,
    price: &'static str,
    steps: [Step; 1],
}

impl Implicit {
    /// `lesser-ichor`, `lesser-ember`, `eater-dominant`, `exarch-dominant`, ...
    pub fn new(arg: &str) -> Option<Implicit> {
        let (side, price) = match arg {
            "lesser-ichor" | "eater-dominant" => (Side::Suffix, "lesser-eldritch-ichor"),
            "greater-ichor" => (Side::Suffix, "greater-eldritch-ichor"),
            "lesser-ember" | "exarch-dominant" => (Side::Prefix, "lesser-eldritch-ember"),
            "greater-ember" => (Side::Prefix, "greater-eldritch-ember"),
            _ => return None,
        };
        Some(Implicit { name: arg.into(), price, steps: [Step::Eldritch(Some(side))] })
    }
}

impl Operation for Implicit {
    fn name(&self) -> String {
        format!("eldritch-implicit:{}", self.name)
    }

    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        !item.influenced() && item.eldritch() != Some(side(&self.steps))
    }

    fn steps(&self) -> &[Step] {
        &self.steps
    }

    fn cost(&self) -> Cost {
        Cost::of(self.price, 1.0, 2.0)
    }
}

fn side(steps: &[Step; 1]) -> Side {
    match steps[0] {
        Step::Eldritch(Some(s)) => s,
        _ => unreachable!(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orb {
    Chaos,
    Exalted,
    Annulment,
}

const PICK: Pick = Pick { sides: Sides::Eldritch, tag: None, pool: Pool::Natural(None) };
const CHAOS: &[Step] =
    &[Step::Clear { sides: Sides::Eldritch, respect_lock: true }, Step::Rarity(Rarity::Rare), Step::RollCount, Step::Fill(PICK)];
const EXALTED: &[Step] = &[Step::Draw(PICK)];
const ANNUL: &[Step] = &[Step::Remove { sides: Sides::Eldritch, tag: None, respect_lock: true }];

impl Orb {
    pub fn parse(name: &str) -> Option<Orb> {
        match name {
            "eldritch-chaos" => Some(Orb::Chaos),
            "eldritch-exalted" => Some(Orb::Exalted),
            "eldritch-annulment" => Some(Orb::Annulment),
            _ => None,
        }
    }
}

impl Operation for Orb {
    fn name(&self) -> String {
        match self {
            Orb::Chaos => "eldritch-chaos",
            Orb::Exalted => "eldritch-exalted",
            Orb::Annulment => "eldritch-annulment",
        }
        .into()
    }

    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        let Some(side) = item.eldritch() else { return false };
        let locked = item.locked()[side as usize];
        match self {
            Orb::Chaos => item.rarity() == Rarity::Rare && !locked,
            Orb::Exalted => item.rarity() == Rarity::Rare && item.open(side) > 0,
            Orb::Annulment => item.rarity() != Rarity::Normal && !locked && item.count(side) > 0,
        }
    }

    fn steps(&self) -> &[Step] {
        match self {
            Orb::Chaos => CHAOS,
            Orb::Exalted => EXALTED,
            Orb::Annulment => ANNUL,
        }
    }

    fn cost(&self) -> Cost {
        let id = match self {
            Orb::Chaos => "eldritch-chaos-orb",
            Orb::Exalted => "eldritch-exalted-orb",
            Orb::Annulment => "eldritch-orb-of-annulment",
        };
        Cost::of(id, 1.0, 2.0)
    }
}
