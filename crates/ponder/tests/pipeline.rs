//! The pipeline. Nothing here touches the network: every client is given a fake
//! transport, and the plan cache lives under the target directory.

#[path = "../../ponder-game/tests/common/mod.rs"]
mod common;
use anyhow::anyhow;
use common::*;
use ponder::gear::{Prices, Settings};
use ponder::pipeline::*;
use ponder_net::{
    env::Clock,
    http::{Request, Transport},
    ladder::Ladder,
    prices::Prices as PriceClient,
    trade::Trade,
};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

/// A transport that fails every request: tests that reach the network by accident find out.
fn offline() -> Arc<dyn Transport> {
    Arc::new(|req: &Request| Err(anyhow!("no network in tests: {}", req.url)))
}

/// The plan cache shared by tests (in the target directory, so reruns are fast).
fn shared_plans() -> Cache {
    Cache::new(PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("plans"))
}

fn fixed_clock() -> Clock {
    Clock::new(|| 1_700_000_000.0, |_| {})
}

/// A context over fake transports; `trade_http` answers the trade site's requests.
fn context(chaos_per_hour: f64, trade_http: Arc<dyn Transport>, trade_cache: PathBuf) -> Context {
    context_with(chaos_per_hour, trade_http, trade_cache, offline())
}

/// `context` with `share_http` answering downloads of build share links.
fn context_with(chaos_per_hour: f64, trade_http: Arc<dyn Transport>, trade_cache: PathBuf, share_http: Arc<dyn Transport>) -> Context {
    let prices = PriceClient::with(offline(), temp_dir("prices"), fixed_clock());
    let trade = Trade::with(trade_http, trade_cache, fixed_clock());
    let ladder = Ladder::with(offline(), temp_dir("ladder"), fixed_clock());
    Context::new(Settings { chaos_per_hour }, prices, trade, ladder, share_http).with_plans(shared_plans())
}

fn offline_context(chaos_per_hour: f64) -> Context {
    context(chaos_per_hour, offline(), temp_dir("trade"))
}

fn estimate(title: &str, cost: f64, market: Option<f64>) -> Estimate {
    let mut e = Estimate::new("cluster", title, vec!["x".into()]);
    e.summary = Some(Arc::new(Summary { cost, bases: 1.0, ..Summary::default() }));
    e.base_price = Some((0.0, 0));
    e.market = market.map(|m| (m, 5));
    e
}

#[test]
fn ranked_puts_margins_first_then_dearest() {
    let report = Report {
        builds: vec![],
        estimates: vec![estimate("cheap", 5.0, None), estimate("profit", 50.0, Some(500.0)), estimate("dear", 900.0, None)],
        extra: vec![],
        shared: vec![],
        prices_source: String::new(),
        league: String::new(),
        errors: vec![],
        divine: 0.0,
    };
    let titles: Vec<&str> = report.ranked().iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, ["profit", "dear", "cheap"]);
    assert_eq!(report.ranked()[0].margin(), Some(450.0));
    // the JSON is ranked too, and carries Python's keys
    let out = report.to_dict();
    assert_eq!(out["estimates"][0]["total"], json!(50.0));
    assert_eq!(out["estimates"][0]["margin"], json!(450.0));
}

#[test]
fn cluster_estimate_has_cost_link_and_json() {
    common::setup();
    let cx = offline_context(0.0);
    let table: Prices = [("alt", 0.074), ("aug", 0.017), ("regal", 0.093), ("scour", 0.35), ("transmute", 0.004), ("chaos", 1.0)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    let notables = vec!["Vast Power".to_string(), "Assert Dominance".to_string()];
    let est = estimate_cluster(&cx, "medium", "area damage", &notables, &table, "Allflame", &ClusterOpts::default());
    assert!(est.error.is_none(), "{:?}", est.error);
    let cost = est.craft_cost().unwrap();
    assert!(0.0 < cost && cost < 500.0, "{cost}");
    let url = est.trade_url.as_deref().unwrap();
    assert!(url.contains("Medium%20Cluster%20Jewel") && url.contains("available"), "{url}");
    serde_json::to_string(&est).unwrap();
    assert_eq!(est.to_dict()["kind"], "cluster");
}

#[test]
fn sources_expand_and_builds_get_names() {
    let dir = temp_dir("sources");
    let list = dir.join("builds.txt");
    std::fs::write(&list, "# my builds\nhttps://pobb.in/aaa  # the first\n\nhttps://pobb.in/bbb\n").unwrap();
    let not_links = dir.join("notes.txt");
    std::fs::write(&not_links, "some notes\nhttps://pobb.in/ccc\n").unwrap();
    let sources = [
        list.to_string_lossy().into_owned(),
        not_links.to_string_lossy().into_owned(),
        "https://pobb.in/ddd".into(),
        "/no/such/file.txt".into(),
    ];
    let expanded = expand_sources(&sources);
    assert_eq!(expanded[..2], ["https://pobb.in/aaa", "https://pobb.in/bbb"]);
    assert_eq!(expanded[2..], sources[1..]);
    let build = ponder_game::pob::Build {
        class_name: "Witch".into(),
        ascendancy: "Occultist".into(),
        level: 95,
        main_skill: Some("Winter Orb".into()),
        items: vec![],
    };
    assert_eq!(build_name(&build), "Occultist lvl 95: Winter Orb");
    let bare = ponder_game::pob::Build { ascendancy: String::new(), main_skill: None, ..build };
    assert_eq!(build_name(&bare), "Witch lvl 95: ?");
}

#[test]
fn the_league_matches_saved_prices_without_network_requests() {
    let directory = temp_dir("league");
    let cx = Context::new(
        Settings::default(),
        PriceClient::with(Arc::new(|_: &Request| panic!("league lookup must stay offline")), directory.clone(), fixed_clock()),
        Trade::with(offline(), temp_dir("league"), fixed_clock()),
        Ladder::with(offline(), temp_dir("league"), fixed_clock()),
        offline(),
    );
    assert_eq!(current_league(&cx), "Standard");
    std::fs::write(directory.join("prices.json"), json!({"league": "Allflame"}).to_string()).unwrap();
    assert_eq!(current_league(&cx), "Allflame");
}

#[test]
fn selecting_report_rows_preserves_metadata_and_serialization() {
    let report = LadderReport {
        characters: 10,
        items: 20,
        clusters: 5,
        estimates: vec![estimate("first", 1.0, None), estimate("second", 2.0, None)],
        archetypes: 2,
        cluster_archetypes: 1,
        prices_source: "fixture".into(),
        league: "Standard".into(),
        divine: 100.0,
    };
    let mut expected = report.to_dict();
    assert_eq!(serde_json::to_value(&report).unwrap(), expected);
    expected["estimates"].as_array_mut().unwrap().truncate(1);
    let selected = report.to_dict_with_estimates(report.ranked().into_iter().take(1).map(Estimate::to_dict));
    assert_eq!(selected, expected);
}
