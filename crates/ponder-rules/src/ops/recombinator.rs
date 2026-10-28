//! Recombinators: the current item and a partner made separately become one item.

use std::sync::Arc;

use super::{Cost, Ctx, Operation, Step, Supply, SupplyRef, View};
use crate::goal::Rarity;

pub struct Recombinator {
    name: String,
    price: String,
    steps: Vec<Step>,
}

impl Recombinator {
    /// `label` names the recombination (`a+b`); the recombinator kind follows the item class.
    pub fn new(class: &str, label: &str, partner: Arc<dyn Supply>) -> Recombinator {
        let kind = match class {
            "Amulet" | "Ring" | "Belt" => "jewellery",
            c if ["Body Armour", "Helmet", "Gloves", "Boots", "Shield"].contains(&c) => "armour",
            _ => "weapon",
        };
        Recombinator {
            name: format!("recombinator:{label}"),
            price: format!("{kind}-recombinator"),
            steps: vec![Step::Recombine(SupplyRef(partner)), Step::RecombineBonus],
        }
    }

    fn partner(&self) -> &dyn Supply {
        match &self.steps[0] {
            Step::Recombine(p) => p.0.as_ref(),
            _ => unreachable!(),
        }
    }
}

impl Operation for Recombinator {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        item.rarity() != Rarity::Normal && self.partner().items().iter().all(|i| i.rarity != Rarity::Normal)
    }

    fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The recombinator and the partner's making.
    fn cost(&self) -> Cost {
        Cost { items: vec![(self.price.clone(), 1.0), ("chaos".into(), self.partner().price())], seconds: 5.0 }
    }
}
