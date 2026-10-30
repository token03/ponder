//! The price model: least squares and the T1-premium fit. No network: listings are synthetic.

#[path = "../../ponder-game/tests/common/mod.rs"]
mod common;
use common::*;

use std::collections::{BTreeSet, HashMap};

use ponder::candidate::RES;
use ponder::gear;
use ponder::pricing::{self};
use ponder_game::item::Item;
use ponder_rules::rng::Rng;
use serde_json::json;

// -- helpers ----------------------------------------------------------------------------

fn near(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0), "{a} != {b}");
}

#[test]
fn lstsq_fits_a_line() {
    let x: Vec<Vec<f64>> = (0..6).map(|i| vec![1.0, i as f64]).collect();
    let y: Vec<f64> = (0..6).map(|i| 2.0 + 3.0 * i as f64).collect();
    let c = pricing::lstsq(&x, &y);
    near(c[0], 2.0);
    near(c[1], 3.0);
    // noisy: the normal equations' answer, 1.5 + 0.5 x on these four points
    let x: Vec<Vec<f64>> = (0..4).map(|i| vec![1.0, i as f64]).collect();
    let c = pricing::lstsq(&x, &[1.0, 3.0, 2.0, 3.0]);
    near(c[0], 1.5);
    near(c[1], 0.5);
}

#[test]
fn lstsq_gives_the_minimum_norm_answer_when_columns_repeat() {
    // two identical columns share the weight; a zero column gets none
    let x = vec![vec![1.0, 1.0, 0.0], vec![1.0, 1.0, 0.0], vec![1.0, 1.0, 0.0]];
    let c = pricing::lstsq(&x, &[2.0, 2.0, 2.0]);
    near(c[0], 1.0);
    near(c[1], 1.0);
    assert_eq!(c[2], 0.0);
}

// -- ladder -----------------------------------------------------------------------------

#[test]
fn price_model_recovers_a_t1_premium() {
    common::setup();
    let db = db();
    // Life tiers as they roll on Vaal Gauntlets, best first.
    let base = db.find_base("Vaal Gauntlets").unwrap();
    let tags = db.item_tags(base, &BTreeSet::new());
    let mut life: Vec<String> =
        gear::pool(db, base, &tags, 86).into_iter().filter(|m| db.mods[m.id.as_str()].type_ == "IncreasedLife").map(|m| m.id).collect();
    life.sort_by_key(|i| db.tier(i, &tags));
    let mut rng = Rng::new(1);
    let mut listings = Vec::new();
    for _ in 0..60 {
        let tier = [0, 0, 1, 2, 3][(rng.unit() * 5.0) as usize];
        let low = &db.mods[life[tier].as_str()].stats[0].min;
        let mult = if tier == 0 {
            3.0
        } else if tier == 1 {
            1.5
        } else {
            1.0
        };
        let price = 100.0 * mult * (0.9 + 0.2 * rng.unit());
        listings.push(json!({
            "item": {"baseType": "Vaal Gauntlets", "ilvl": 86, "explicitMods": [format!("+{low} to maximum Life"), "+40% to Fire Resistance"]},
            "listing": {"price": {"amount": price, "currency": "chaos"}},
        }));
    }
    let rates = HashMap::from([("chaos".to_string(), 1.0)]);
    let model = pricing::fit(db, "Gloves (Armour)", &["base_maximum_life", RES], &listings, &rates).expect("enough listings");
    let life = &model.tiers[0];
    assert!((life.t1.unwrap() / 3.0 - 1.0).abs() < 0.15, "{:?}", life.t1);
    assert!((life.t2.unwrap() / 1.5 - 1.0).abs() < 0.15, "{:?}", life.t2);
    assert_eq!(model.listings, 60);
    assert!(model.r2 > 0.9);
}

#[test]
fn too_few_listings_make_no_model() {
    common::setup();
    let one = json!({"item": {"baseType": "Vaal Gauntlets", "explicitMods": ["+100 to maximum Life"]}, "listing": {"price": {"amount": 5, "currency": "chaos"}}});
    let rates = HashMap::from([("chaos".to_string(), 1.0)]);
    assert!(pricing::fit(db(), "Gloves", &["base_maximum_life", RES], &vec![one; 5], &rates).is_none());
}

#[test]
fn a_listing_without_a_known_base_has_no_match() {
    common::setup();
    let item = |base: &str| json!({"item": {"baseType": base, "ilvl": 84, "explicitMods": ["+100 to maximum Life"], "influences": {"shaper": false}}});
    assert!(pricing::listing_match(db(), &item("Not A Real Base")).is_none());
    let m = pricing::listing_match(db(), &item("Vaal Gauntlets")).unwrap();
    assert!(m.base_id.is_some() && m.item.influences.is_empty());
    assert!(pricing::listing_match(db(), &json!({})).is_none());
    let _ = Item::default();
}
