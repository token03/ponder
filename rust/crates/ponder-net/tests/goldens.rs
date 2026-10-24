//! Byte-for-byte checks against what the Python makes (`scripts/golden/net.py` writes rust/goldens/net).

use ponder_net::ladder;
use ponder_net::pyjson::{dumps, dumps_compact, dumps_sorted, sha1_hex};
use ponder_net::trade::{self, ItemQuery, Repoe};
use serde_json::{Number, Value, json};
use std::path::PathBuf;

fn golden(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../goldens/net").join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap()
}

fn text(v: &Value) -> &str {
    v.as_str().unwrap()
}

#[test]
fn search_urls_match() {
    let cases = golden("search_url.json");
    assert!(cases.as_array().unwrap().len() >= 18);
    for c in cases.as_array().unwrap() {
        assert_eq!(trade::search_url(text(&c["input"]["league"]), &c["input"]["query"]), text(&c["output"]));
    }
}

#[test]
fn cache_keys_match() {
    for c in golden("cache_keys.json").as_array().unwrap() {
        let i = &c["input"];
        let doc = match text(&i["kind"]) {
            "price" => json!([i["league"], i["query"], i["n"]]),
            _ => json!(["listings", i["league"], i["query"], i["n"]]),
        };
        assert_eq!(sha1_hex(dumps_sorted(&doc).as_bytes()), text(&c["output"]), "{}", dumps(&doc));
    }
}

#[test]
fn item_queries_match() {
    for c in golden("item_query.json").as_array().unwrap() {
        let q: ItemQuery = serde_json::from_value(c["input"].clone()).unwrap();
        let built = trade::item_query(&q);
        assert_eq!(dumps_compact(&built), text(&c["output"]), "{}", c["input"]);
        assert_eq!(dumps(&built), text(&c["output_default"]));
    }
}

#[test]
fn cluster_queries_match() {
    let g = golden("cluster_query.json");
    let repoe = Repoe::new(&g["repoe"]["mods"], &g["repoe"]["stat_translations"]);
    for c in g["cases"].as_array().unwrap() {
        let i = &c["input"];
        let notables: Vec<(String, Option<Number>)> = i["notables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| match n {
                Value::String(s) => (s.clone(), None),
                pair => (text(&pair[0]).to_string(), pair[1].as_number().cloned()),
            })
            .collect();
        let fractured: Vec<String> = i["fractured_ids"].as_array().unwrap().iter().map(|s| text(s).to_string()).collect();
        let q = trade::cluster_query(&repoe, text(&i["size"]), i["enchant_option"].as_str(), i["passives"].as_i64(), &notables, i["item_level"].as_i64(), &fractured);
        assert_eq!(dumps_compact(&q), text(&c["output"]), "{i}");
    }
}

#[test]
fn stat_min_takes_the_lowest_roll() {
    let g = golden("cluster_query.json");
    let mods = &g["repoe"]["mods"];
    assert_eq!(trade::stat_min(mods, &["SmallPassive1", "SmallPassive2"]).unwrap().to_string(), "0.5");
    assert_eq!(trade::stat_min(mods, &["Nope"]), None);
}

#[test]
fn lite_characters_match() {
    let cases = golden("lite.json");
    assert!(cases.as_array().unwrap().len() >= 4);
    for c in cases.as_array().unwrap() {
        let lite = ladder::lite(&c["input"]).unwrap();
        assert_eq!(lite.to_json(), text(&c["output"]), "{}", c["input"]["name"]);
        // and the written file reads back as the same record
        assert_eq!(serde_json::from_str::<ladder::Lite>(text(&c["output"])).unwrap(), lite);
    }
}
