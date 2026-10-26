//! Operations that change nothing the model tracks (sockets, quality) but still cost.

use super::{Cost, Ctx, Operation, Step, View, eldritch, veiled};

pub struct Noop {
    name: String,
    cost: Cost,
}

impl Operation for Noop {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn applies(&self, _: &Ctx, _: &dyn View) -> bool {
        true
    }

    fn steps(&self) -> &[Step] {
        &[]
    }

    fn cost(&self) -> Cost {
        self.cost.clone()
    }
}

/// Eldritch and veiled orbs, chromatics and catalysts by guide name.
pub fn parse(op: &str) -> Option<Box<dyn Operation>> {
    if let Some(o) = eldritch::Orb::parse(op) {
        return Some(Box::new(o));
    }
    if let Some(o) = veiled::Orb::parse(op) {
        return Some(Box::new(o));
    }
    let cost = match op.split_once(':') {
        None if op == "chromatic" => Cost::of("chrome", 1.0, 1.0),
        Some(("catalyst", kind)) => Cost::of(&format!("{kind}-catalyst"), 10.0, 5.0),
        _ => return None,
    };
    Some(Box::new(Noop { name: op.into(), cost }))
}
