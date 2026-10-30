use ponder_net::ladder;
use ponder_net::trade::{self, ItemQuery, Repoe};
use serde_json::{Number, Value};
#[path = "../../../fixtures/support.rs"]
mod common;

fn expected(name: &str) -> Value {
    common::fixture(&format!("expected/net/{name}"))
}

fn decoded_search(url: &str) -> (String, Value) {
    let (base, query) = url.split_once("?q=").unwrap();
    (
        percent_encoding::percent_decode_str(base).decode_utf8().unwrap().into_owned(),
        serde_json::from_str(&percent_encoding::percent_decode_str(query).decode_utf8().unwrap()).unwrap(),
    )
}

fn text(v: &Value) -> &str {
    v.as_str().unwrap()
}

#[test]
fn search_urls_match() {
    let cases = expected("search_url.json");
    assert!(cases.as_array().unwrap().len() >= 18);
    for c in cases.as_array().unwrap() {
        assert_eq!(
            decoded_search(&trade::search_url(text(&c["input"]["league"]), &c["input"]["query"])),
            decoded_search(text(&c["output"]))
        );
    }
}

#[test]
fn item_queries_match() {
    for c in expected("item_query.json").as_array().unwrap() {
        let q: ItemQuery = serde_json::from_value(c["input"].clone()).unwrap();
        let built = trade::item_query(&q);
        assert_eq!(built, c["output"], "{}", c["input"]);
    }
}

#[test]
fn cluster_queries_match() {
    let g = expected("cluster_query.json");
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
        let q = trade::cluster_query(
            &repoe,
            text(&i["size"]),
            i["enchant_option"].as_str(),
            i["passives"].as_i64(),
            &notables,
            i["item_level"].as_i64(),
            &fractured,
        );
        assert_eq!(q, c["output"], "{i}");
    }
}

#[test]
fn stat_min_takes_the_lowest_roll() {
    let g = expected("cluster_query.json");
    let mods = &g["repoe"]["mods"];
    assert_eq!(trade::stat_min(mods, &["SmallPassive1", "SmallPassive2"]).unwrap().to_string(), "0.5");
    assert_eq!(trade::stat_min(mods, &["Nope"]), None);
}

#[test]
fn lite_characters_match() {
    let cases = expected("lite.json");
    assert!(cases.as_array().unwrap().len() >= 4);
    for c in cases.as_array().unwrap() {
        let lite = ladder::lite(&c["input"]).unwrap();
        assert_eq!(serde_json::to_value(&lite).unwrap(), c["output"], "{}", c["input"]["name"]);
        assert_eq!(serde_json::from_value::<ladder::Lite>(c["output"].clone()).unwrap(), lite);
    }
}
