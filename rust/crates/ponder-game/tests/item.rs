//! Port of tests/test_item.py, plus the parse half of the texts and fixtures goldens.

mod common;

use ponder_game::item::parse_item;
use std::collections::BTreeSet;

const HELMET: &str = "\
Rarity: RARE
Doom Crown
Hubris Circlet
Shaper Item
Item Level: 86
Quality: 20
Implicits: 0
+121 to maximum Life
{fractured}84% increased Energy Shield
{crafted}+23% to Lightning Resistance
Corrupted
";

const BOOTS: &str = "\
Rarity: RARE
Grim Stride
Two-Toned Boots
Selected Variant: 2
Implicits: 2
{variant:1}+(8-12)% to Fire and Cold Resistances
{variant:2}+(8-12)% to Cold and Lightning Resistances
{range:0.5}+(42-45)% to Lightning Resistance
";

const COPIED: &str = "\
Item Class: Rings
Rarity: Rare
Gale Loop
Bone Ring
--------
Requirements:
Level: 75
--------
Item Level: 84
--------
+15 to maximum Energy Shield (implicit)
--------
{ Prefix Modifier \"Virile\" (Tier: 1) — Life }
+105(100-114) to maximum Life
+40 to maximum Energy Shield (fractured)
+42% to Fire Resistance
+12 to Dexterity (crafted)
--------
Shaper Item
--------
Corrupted
";

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn texts(lines: &[ponder_game::item::ModLine]) -> Vec<&str> {
    lines.iter().map(|l| l.text.as_str()).collect()
}

#[test]
fn header_and_markers() {
    let item = parse_item(HELMET).unwrap();
    assert_eq!(
        (item.rarity.as_str(), item.name.as_str(), item.base.as_str()),
        ("RARE", "Doom Crown", "Hubris Circlet")
    );
    assert_eq!((item.item_level, item.quality), (Some(86), 20));
    assert_eq!(item.influences, set(&["shaper"]));
    assert_eq!(item.flags, set(&["corrupted"]));
    assert_eq!(
        texts(&item.explicits),
        [
            "+121 to maximum Life",
            "84% increased Energy Shield",
            "+23% to Lightning Resistance"
        ]
    );
    assert!(item.explicits[1].fractured() && item.explicits[2].crafted());
}

#[test]
fn variants_split_after_implicit_count() {
    let item = parse_item(BOOTS).unwrap();
    assert_eq!(
        texts(&item.implicits),
        ["+(8-12)% to Cold and Lightning Resistances"]
    );
    assert_eq!(item.explicits.len(), 1);
    assert_eq!(item.explicits[0].roll(), Some(0.5));
}

#[test]
fn magic_item_has_single_name_line() {
    let item = parse_item(
        "Rarity: MAGIC\nRunner's Two-Toned Boots\nImplicits: 0\n25% increased Movement Speed\n",
    )
    .unwrap();
    assert_eq!(item.base, "Runner's Two-Toned Boots");
    assert_eq!(texts(&item.explicits), ["25% increased Movement Speed"]);
}

#[test]
fn pob_crafter_affix_ids() {
    let item = parse_item("Rarity: RARE\nX\nOnyx Amulet\nPrefix: {range:0.5}IncreasedLife7\nSuffix: None\nImplicits: 0\n").unwrap();
    assert_eq!(
        item.pob_affixes,
        [("prefix".to_string(), "IncreasedLife7".to_string())]
    );
}

#[test]
fn item_copied_in_game() {
    let item = parse_item(COPIED).unwrap();
    assert_eq!(
        (
            item.rarity.as_str(),
            item.name.as_str(),
            item.base.as_str(),
            item.item_level
        ),
        ("RARE", "Gale Loop", "Bone Ring", Some(84))
    );
    assert_eq!(texts(&item.implicits), ["+15 to maximum Energy Shield"]);
    assert_eq!(
        texts(&item.explicits),
        [
            "+105 to maximum Life",
            "+40 to maximum Energy Shield",
            "+42% to Fire Resistance",
            "+12 to Dexterity"
        ]
    );
    assert!(item.explicits[1].fractured() && item.explicits[3].crafted());
    assert_eq!(item.influences, set(&["shaper"]));
    assert!(item.flags.is_superset(&set(&["corrupted", "fractured"])));
}

#[test]
fn not_an_item_is_an_error() {
    assert!(parse_item("hello").is_err());
    assert!(parse_item("Rarity: Rare").is_err());
    assert!(parse_item("nothing\n--------\nhere").is_err());
}

/// Every hand-written text parses to exactly what Python made of it (no repoe data needed).
#[test]
fn parse_matches_python_goldens() {
    let texts = common::golden("texts");
    let mut bad = 0;
    for entry in texts.as_array().unwrap() {
        let text = entry["text"].as_str().unwrap();
        match (parse_item(text), entry.get("item")) {
            (Ok(item), Some(want)) => {
                if common::item_json(&item) != *want {
                    bad += 1;
                    eprintln!(
                        "parse differs for {text:?}\n rust:   {}\n python: {want}",
                        common::item_json(&item)
                    );
                }
            }
            (Err(_), None) => {}
            (got, want) => {
                bad += 1;
                eprintln!(
                    "parse outcome differs for {text:?}: rust {:?}, python {:?}",
                    got.is_ok(),
                    want.is_some()
                );
            }
        }
    }
    assert_eq!(bad, 0);
}
