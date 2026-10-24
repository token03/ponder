use ponder_game::item::{Item, ModLine};
use ponder_net::trade::ListingItem;

pub fn listing_to_item(l: &ListingItem) -> Item {
    let mut item = Item::new(l.rarity.as_str(), l.name.as_str(), l.base.as_str());
    item.item_level = l.ilvl;
    item.influences = l.influences.clone();
    item.flags = l.flags.iter().cloned().collect();
    item.implicits = l.implicits.iter().map(ModLine::new).collect();
    item.explicits = l
        .explicits
        .iter()
        .map(|e| {
            if e.fractured {
                ModLine::with_marker(&e.text, "fractured")
            } else if e.crafted {
                ModLine::with_marker(&e.text, "crafted")
            } else {
                ModLine::new(&e.text)
            }
        })
        .collect();
    item
}
