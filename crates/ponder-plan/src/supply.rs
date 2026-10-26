//! Where items come from: bought at a price, or made by another solved craft.

use std::sync::OnceLock;

use ponder_rules::item::Item;
use ponder_rules::ops::Supply;
use ponder_rules::rng::Rng;

/// One item for sale.
pub struct Stock {
    name: String,
    price: f64,
    items: [Item; 1],
}

impl Stock {
    pub fn new(name: &str, price: f64, item: Item) -> Stock {
        Stock { name: name.into(), price, items: [item] }
    }
}

impl Supply for Stock {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn price(&self) -> f64 {
        self.price
    }
    fn items(&self) -> &[Item] {
        &self.items
    }
    fn item(&self, _: &mut Rng) -> Item {
        self.items[0].clone()
    }
}

pub struct Product {
    name: String,
    made: OnceLock<Made>,
}

struct Made {
    price: f64,
    items: Vec<Item>,
}

impl Product {
    pub fn new(name: &str) -> Product {
        Product { name: name.into(), made: OnceLock::new() }
    }

    pub fn fill(&self, price: f64, items: Vec<Item>) {
        assert!(!items.is_empty(), "{}: no real items made", self.name);
        assert!(self.made.set(Made { price, items }).is_ok(), "{} filled twice", self.name);
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
    fn items(&self) -> &[Item] {
        &self.made().items
    }
    fn item(&self, rng: &mut Rng) -> Item {
        let items = &self.made().items;
        items[rng.below(items.len())].clone()
    }
}
