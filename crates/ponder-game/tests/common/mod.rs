#![allow(dead_code, unused_imports)]

#[path = "../../../../tests/support/mod.rs"]
mod support;
#[path = "../../../../tests/support/game.rs"]
mod game;

pub use support::*;
pub use game::*;
use ponder_game::item::{Item, ModLine};
use ponder_game::mods::ItemMatch;
use serde_json::{Value, json};

pub fn expected(name: &str) -> Value {
    fixture(&format!("expected/items/{name}.json.gz"))
}

fn line_json(l: &ModLine) -> Value {
    json!({"text": l.text, "markers": l.markers})
}

pub fn item_json(item: &Item) -> Value {
    json!({
        "rarity": item.rarity,
        "name": item.name,
        "base": item.base,
        "item_level": item.item_level,
        "quality": item.quality,
        "influences": item.influences,
        "flags": item.flags,
        "implicits": item.implicits.iter().map(line_json).collect::<Vec<_>>(),
        "explicits": item.explicits.iter().map(line_json).collect::<Vec<_>>(),
        "pob_affixes": item.pob_affixes.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
        "header": item.header,
    })
}

pub fn match_json(m: &ItemMatch) -> Value {
    json!({
        "base_id": m.base_id,
        "item_class": m.item_class,
        "tags": m.tags,
        "mods": m.mods.iter().map(|x| json!({
            "mod_id": x.mod_id, "kind": x.kind, "family": x.family, "source": x.source, "tier": x.tier,
            "alternatives": x.alternatives, "merged": x.merged,
            "lines": x.lines.iter().map(line_json).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "unmatched": m.unmatched.iter().map(|l| &l.text).collect::<Vec<_>>(),
        "notes": m.notes.iter().map(|l| &l.text).collect::<Vec<_>>(),
    })
}

/// A match with every list that Python orders arbitrarily sorted, for comparing sets.
pub fn normalized(mut m: Value, with_tags: bool) -> Value {
    let o = m.as_object_mut().unwrap();
    if !with_tags {
        o.remove("tags");
    } else {
        // A tag with two influence suffixes ("helmet_shaper_elder") is spelled in Python's set
        // iteration order, which varies per run: keep their number, not their spelling.
        const INF: [&str; 6] = [
            "shaper",
            "elder",
            "crusader",
            "eyrie",
            "basilisk",
            "adjudicator",
        ];
        let tags = o.get_mut("tags").unwrap().as_array_mut().unwrap();
        let multi = |t: &Value| {
            let parts: Vec<&str> = t.as_str().unwrap().rsplitn(3, '_').collect();
            parts.len() == 3 && INF.contains(&parts[0]) && INF.contains(&parts[1])
        };
        let n = tags.iter().filter(|t| multi(t)).count();
        tags.retain(|t| !multi(t));
        o.insert("multi_influence_tags".into(), n.into());
    }
    for key in ["unmatched", "notes"] {
        sort_values(o.get_mut(key).unwrap());
    }
    sort_values(o.get_mut("mods").unwrap());
    m
}

fn sort_values(v: &mut Value) {
    v.as_array_mut().unwrap().sort_by_key(|x| x.to_string());
}

/// Do the mods agree, in the same order too?
pub fn same_order(a: &Value, b: &Value) -> bool {
    a["mods"] == b["mods"]
}
