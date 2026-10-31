//! Awakener's Orb: a donor item's influence and one of its influenced mods join the current
//! item, which keeps one of its own and is reforged as a rare (poewiki, Awakener's Orb).

use std::sync::Arc;

use super::{Cost, Ctx, Operation, Pick, Spend, Step, Supply, SupplyRef, View};

pub struct Awakener {
    name: String,
    steps: Vec<Step>,
}

impl Awakener {
    /// `label` names the pairing (`receiver+donor`).
    pub fn new(label: &str, donor: Arc<dyn Supply>) -> Awakener {
        Awakener {
            name: format!("awakener:{label}"),
            steps: vec![Step::Awaken(SupplyRef(donor)), Step::RollCount, Step::Fill(Pick::ANY), Step::Forget(Spend::Upto(100))],
        }
    }

    fn donor(&self) -> &dyn Supply {
        match &self.steps[0] {
            Step::Awaken(d) => d.0.as_ref(),
            _ => unreachable!(),
        }
    }
}

impl Operation for Awakener {
    fn name(&self) -> String {
        self.name.clone()
    }

    /// Each item has exactly one influence and they differ. Fractured items are not admitted:
    /// what the reforge does to a fractured mod is not documented.
    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        let own = item.influences();
        own.count_ones() == 1
            && item.fractured() == 0
            && self.donor().items().iter().all(|d| d.kind.influences.0.count_ones() == 1 && d.kind.influences.0 != own)
    }

    fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The orb and the donor's making.
    fn cost(&self) -> Cost {
        Cost { items: vec![("awakeners-orb".into(), 1.0), ("chaos".into(), self.donor().price())], seconds: 15.0 }
    }
}
