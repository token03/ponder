//! Where items come from: bought at a price, or made by another solved craft.

use std::sync::OnceLock;

use ponder_rules::item::Item;
use ponder_rules::lens::{Lens, State};
use ponder_rules::ops::Supply;
use ponder_rules::rng::Rng;

/// One item for sale.
pub struct Stock {
    name: String,
    price: f64,
    item: Item,
    states: Vec<(f64, State)>,
}

impl Stock {
    pub fn new(name: &str, price: f64, item: Item, lens: &Lens) -> Stock {
        Stock { name: name.into(), price, states: vec![(1.0, lens.project(&item))], item }
    }
}

impl Supply for Stock {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn price(&self) -> f64 {
        self.price
    }
    fn states(&self) -> &[(f64, State)] {
        &self.states
    }
    fn item(&self, _: &mut Rng) -> Item {
        self.item.clone()
    }
}

/// What a craft makes: empty until the craft is solved, then its expected cost, the states it
/// ends in, and real items made by following it.
pub struct Product {
    name: String,
    made: OnceLock<Made>,
}

struct Made {
    price: f64,
    states: Vec<(f64, State)>,
    items: Vec<Item>,
}

impl Product {
    pub fn new(name: &str) -> Product {
        Product { name: name.into(), made: OnceLock::new() }
    }

    /// Fill once: the craft's cost, its end states, and real items it made.
    pub fn fill(&self, price: f64, states: Vec<(f64, State)>, items: Vec<Item>) {
        assert!(!items.is_empty(), "{}: no real items made", self.name);
        assert!(self.made.set(Made { price, states, items }).is_ok(), "{} filled twice", self.name);
    }

    fn made(&self) -> &Made {
        self.made.get().unwrap_or_else(|| panic!("{} is used before it is made", self.name))
    }
}

impl Supply for Product {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn price(&self) -> f64 {
        self.made().price
    }
    fn states(&self) -> &[(f64, State)] {
        &self.made().states
    }
    fn item(&self, rng: &mut Rng) -> Item {
        let items = &self.made().items;
        items[rng.below(items.len())].clone()
    }
}
