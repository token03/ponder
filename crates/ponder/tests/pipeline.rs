//! The pipeline. Nothing here touches the network: every client is given a fake
//! transport, and the plan cache lives under the target directory.

mod common;
use common::*;

use ponder_core::{ActionKind, ProblemSpec};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::anyhow;
use ponder::pipeline::*;
use ponder_core::{Plan, Prices, Problem, Settings};
use ponder_game::item::parse_item;
use ponder_game::mods::IdMods;
use ponder_net::env::Clock;
use ponder_net::http::{Request, Response, Transport};
use ponder_net::ladder::Ladder;
use ponder_net::prices::Prices as PriceClient;
use ponder_net::trade::Trade;
use serde_json::{Value, json};

// -- helpers ----------------------------------------------------------------------------

/// A transport that fails every request: tests that reach the network by accident find out.
fn offline() -> Arc<dyn Transport> {
    Arc::new(|req: &Request| Err(anyhow!("no network in tests: {}", req.url)))
}

/// The plan cache shared by tests (in the target directory, so reruns are fast).
fn shared_plans() -> PlanCache {
    PlanCache::new(PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("plans"))
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
    let prices = PriceClient::with(
        Arc::new(|req: &Request| {
            if req.url.ends_with("/leagues") {
                return Ok(Response::json(200, &json!([{"id": "Standard"}])));
            }
            Err(anyhow!("no network in tests: {}", req.url))
        }),
        temp_dir("prices"),
        fixed_clock(),
    );
    let trade = Trade::with(trade_http, trade_cache, fixed_clock());
    let ladder = Ladder::with(offline(), temp_dir("ladder"), fixed_clock());
    Context::new(Settings { chaos_per_hour }, prices, trade, ladder, share_http).with_plans(shared_plans())
}

fn offline_context(chaos_per_hour: f64) -> Context {
    context(chaos_per_hour, offline(), temp_dir("trade"))
}

fn price_table() -> &'static Prices {
    static T: OnceLock<Prices> = OnceLock::new();
    T.get_or_init(|| serde_json::from_value(fixture("prices.json.gz")).unwrap())
}

fn gear_opts() -> GearOpts {
    GearOpts { solve: false, ..GearOpts::default() }
}

fn real_build(name: &str) -> ponder_game::pob::Build {
    let path = repo_root().join("fixtures/real").join(name);
    ponder_game::pob::load_with(path.to_str().unwrap(), |_| Err("no network".into())).unwrap()
}

fn cheap_table() -> Prices {
    [
        ("alt", 0.074),
        ("aug", 0.017),
        ("regal", 0.093),
        ("scour", 0.35),
        ("transmute", 0.004),
        ("chaos", 1.0),
        ("exalted", 5.0),
        ("annul", 11.0),
        ("alch", 0.03),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

// -- pipeline -----------------------------------------------------------------------------

fn estimate(title: &str, cost: f64, market: Option<f64>) -> Estimate {
    let mut e = Estimate::new("cluster", title, vec!["x".into()]);
    e.plan = Some(Plan { cost, ..Plan::default() });
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
    assert!(10.0 < cost && cost < 500.0, "{cost}");
    let url = est.trade_url.as_deref().unwrap();
    assert!(url.contains("Medium%20Cluster%20Jewel") && url.contains("available"), "{url}");
    serde_json::to_string(&est).unwrap();
    assert_eq!(est.to_dict()["kind"], "cluster");
}

#[test]
fn fractures_and_best_base() {
    common::setup();
    let cx = offline_context(0.0);
    let build = real_build("occultist_winter_orb.txt");
    let gloves = build
        .items
        .iter()
        .filter_map(|i| parse_item(&i.text).ok())
        .find(|i| i.base == "Warlock Gloves" && i.rarity == "RARE")
        .expect("gloves in the fixture");
    let table: Prices = [
        ("alt", 0.074),
        ("aug", 0.017),
        ("regal", 0.093),
        ("scour", 0.35),
        ("transmute", 0.004),
        ("chaos", 1.0),
        ("exalted", 5.0),
        ("annul", 11.0),
        ("alch", 0.03),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let mut est = estimate_gear(&cx, db(), &db().match_item(&gloves), &table, "Allflame", &gear_opts()).unwrap();
    assert_eq!((est.title.as_str(), est.base.as_str()), ("Gloves (ES)", "Warlock Gloves"));
    assert!(est.bases > 5);
    add_fractures(&cx, &mut est, "Allflame", None, true);
    {
        let mut items: Vec<&mut dyn Solvable> = vec![&mut est];
        solve_all(&cx, &mut items, &table, Some(1), None);
    }
    {
        let mut fr: Vec<&mut dyn Solvable> = est.fractures.iter_mut().map(|f| f as &mut dyn Solvable).collect();
        solve_all(&cx, &mut fr, &table, Some(1), None);
    }
    assert_eq!(est.fractures.len(), est.targets.len());
    let whole = est.craft_cost().unwrap();
    assert!(est.fractures.iter().all(|f| f.craft_cost().is_some()));
    assert!(est.fractures.iter().any(|f| f.craft_cost().unwrap() < whole));
    assert!(est.fractures.iter().all(|f| {
        let url = f.trade_url.as_deref().unwrap();
        url.contains("fractured.stat_") && url.contains("fractured_item")
    }));
}

#[test]
fn fracture_options_respect_base_restrictions_and_include_the_bench() {
    common::setup();
    let cx = offline_context(0.0);
    let mods = IdMods::from_json(&json!({"explicit": [{"id": "IncreasedLife7"}, {"id": "IncreasedEnergyShield10"}]}));
    let make = |influences: BTreeSet<String>, synthesised: bool| {
        let mut m = db().match_ids("Bone Ring", &mods, Some(84), &influences, "RARE", "", &Default::default());
        if synthesised {
            m.item.flags.insert("synthesised".into());
        }
        estimate_gear(&cx, db(), &m, price_table(), "Standard", &GearOpts { solve: false, exact: true, ..GearOpts::default() }).unwrap()
    };
    let mut plain = make(BTreeSet::new(), false);
    plain.bench = vec![("bench".into(), 5.0)];
    add_fractures(&cx, &mut plain, "Standard", None, false);
    assert_eq!(plain.fractures.len(), 2);
    for f in &mut plain.fractures {
        f.plan = Some(Plan { cost: 10.0, ..Plan::default() });
        f.base_price = Some((20.0, 1));
        assert_eq!(f.craft_cost(), Some(15.0));
        assert_eq!(f.total(), Some(35.0));
        assert_eq!(f.query.as_ref().unwrap()["query"]["filters"]["misc_filters"]["filters"]["corrupted"]["option"], "false");
    }
    for mut e in [make(BTreeSet::from(["shaper".into()]), false), make(BTreeSet::new(), true)] {
        add_fractures(&cx, &mut e, "Standard", Some(price_table()), false);
        assert!(e.fractures.is_empty());
    }
    let mut fractured = make(BTreeSet::new(), false);
    let fixed = fractured.problem.as_ref().unwrap().pool[0].clone();
    let mut spec = fractured.problem.as_ref().unwrap().to_spec();
    spec.fixed.push(fixed);
    fractured.problem = Some(spec.compile());
    add_fractures(&cx, &mut fractured, "Standard", Some(price_table()), false);
    assert!(fractured.fractures.is_empty());
}

fn problem_of(v: &Value) -> Problem {
    let mut v = v.clone();
    v.as_object_mut().unwrap().entry("begin").or_insert(Value::Null);
    serde_json::from_value(v).unwrap()
}

// -- trade ------------------------------------------------------------------------------------

/// A made-up trade site: a search's answer is a function of the
/// query's text, so what was searched and in what order shows in the answers.
fn fake_trade(calls: Arc<Mutex<Vec<String>>>) -> Arc<dyn Transport> {
    Arc::new(move |req: &Request| {
        if req.url.contains("/api/trade/search/") {
            let h = blake3::hash(req.body.as_deref().unwrap()).to_hex().to_string();
            calls.lock().unwrap().push(h.clone());
            let n = u32::from_str_radix(&h[..8], 16).unwrap() as u64;
            if n.is_multiple_of(7) {
                return Ok(Response::json(200, &json!({"id": "none", "total": 0, "result": []})));
            }
            let price = (5 + n % 1000) as f64 / 10.0;
            return Ok(Response::json(200, &json!({"id": price.to_string(), "total": 3 + n % 40, "result": ["a"]})));
        }
        let price: f64 = req.url.rsplit("query=").next().unwrap().parse().unwrap();
        Ok(Response::json(200, &json!({"result": [{"listing": {"price": {"amount": price, "currency": "chaos"}}}]})))
    })
}

fn status_transport(status: u16) -> Arc<dyn Transport> {
    Arc::new(move |_: &Request| Ok(Response::new(status, "{}")))
}

#[test]
fn lookup_reports_trade_errors_without_leaking_the_cookie() {
    let table = cheap_table();
    let query = json!({"query": {"status": {"option": "available"}}});
    for (transport, expect) in [
        (status_transport(403), "trade 403: check POESESSID in .env"),
        (status_transport(401), "trade 401: check POESESSID in .env"),
        (status_transport(500), "trade 500: try again later"),
        (
            Arc::new(|_: &Request| Ok(Response::json(200, &json!({"id": "x", "total": 0, "result": []})))) as Arc<dyn Transport>,
            "no listings",
        ),
    ] {
        let mut cx = context(0.0, transport, temp_dir("errors"));
        cx.trade.cookie = Arc::new(|| Some("SECRET-SESSION".into()));
        let mut e = Estimate::new("gear", "x", vec![]);
        e.query = Some(query.clone());
        let used = lookup(&cx, Lookup::Market(&mut e), &table, "Standard");
        assert_eq!(e.market_error.as_deref(), Some(expect));
        assert!(e.market.is_none());
        assert_eq!(used, 1);
        assert!(!serde_json::to_string(&e).unwrap().contains("SECRET"), "the cookie must not show");
    }
    let cx = context(0.0, Arc::new(|_: &Request| Err(anyhow!("connection reset"))), temp_dir("errors"));
    let mut e = Estimate::new("gear", "x", vec![]);
    e.query = Some(query);
    lookup(&cx, Lookup::Market(&mut e), &table, "Standard");
    assert_eq!(e.market_error.as_deref(), Some("trade failed: connection reset"));
    // nothing to search: no request, nothing used
    let mut none = Estimate::new("gear", "y", vec![]);
    assert_eq!(lookup(&cx, Lookup::Base(&mut none), &table, "Standard"), 0);
}

#[test]
fn lookup_market_prices_the_base_too() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let cx = context(0.0, fake_trade(calls.clone()), temp_dir("market"));
    let table = cheap_table();
    let mut e = Estimate::new("gear", "x", vec![]);
    e.query = Some(json!({"query": {"a": 1}}));
    e.base_query = Some(json!({"query": {"b": 2}}));
    e.fixed = vec!["Life".into()];
    lookup_market(&cx, &mut e, &table, "Standard");
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert!(e.market.is_some() || e.market_error.is_some());
    assert!(e.base_price.is_some() == (e.base_source == "trade"));
    // a priced base is not searched again; a repeated search comes from the cache
    lookup_market(&cx, &mut e, &table, "Standard");
    assert_eq!(calls.lock().unwrap().len(), 2);
}

#[test]
fn price_fractures_prices_each_items_best_few() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let cx = context(0.0, fake_trade(calls.clone()), temp_dir("fractures"));
    let table = cheap_table();
    let mut e = Estimate::new("gear", "x", vec![]);
    for i in 0..4 {
        let mut f = Fracture::new(format!("f{i}"), "m", problem_of(&json!({"pool": [], "targets": [], "need": 0, "actions": []})));
        f.query = Some(json!({"query": {"i": i}}));
        f.plan = Some(Plan { cost: [30.0, 10.0, 20.0, 40.0][i], ..Plan::default() });
        e.fractures.push(f);
    }
    let used = price_fractures(&cx, std::slice::from_mut(&mut e), &table, "Standard", 2, 40, None);
    assert_eq!(used, 2);
    let priced: Vec<&str> =
        e.fractures.iter().filter(|f| f.base_price.is_some() || f.base_error.is_some()).map(|f| f.target.as_str()).collect();
    assert_eq!(priced, ["f1", "f2"], "the two cheapest to finish");
    // a budget stops it
    let mut e2 = e.clone();
    e2.fractures.iter_mut().for_each(|f| (f.base_price, f.base_error) = (None, None));
    let cx2 = context(0.0, fake_trade(calls), temp_dir("fractures2"));
    assert_eq!(price_fractures(&cx2, std::slice::from_mut(&mut e2), &table, "Standard", 4, 1, None), 1);
}

// -- the product tree, by hand ----------------------------------------------------------------

fn target(name: &str, side: ponder_core::Side, good: &[&str], groups: &[&str]) -> ponder_core::Target {
    ponder_core::Target {
        name: name.into(),
        side,
        good: good.iter().map(|s| s.to_string()).collect(),
        groups: groups.iter().map(|s| s.to_string()).collect(),
        notable: false,
    }
}

fn tiny(targets: Vec<ponder_core::Target>, filler: &[&str], filler_need: usize, filler_sides: Option<(usize, usize)>) -> Problem {
    let mut p = ProblemSpec::new(vec![], targets, 0, vec![], ponder_core::Kind::Gear);
    p.need = p.targets.len();
    p.filler = filler.iter().map(|s| s.to_string()).collect();
    p.filler_need = filler_need;
    p.filler_sides = filler_sides;
    p.compile()
}

/// Two products of one base: A+B, and A+C. Crafting A+B now and then rolls A+C.
fn pivot_pair() -> Vec<Estimate> {
    use ponder_core::{Action, Kind, MAGIC, Mod, NORMAL, RARE, Side::*};
    let pool = vec![
        Mod::new("a", Prefix, 100.0, &["A"], false, "a", &[]),
        Mod::new("c", Prefix, 100.0, &["C"], false, "c", &[]),
        Mod::new("x", Prefix, 300.0, &["X"], false, "x", &[]),
        Mod::new("b", Suffix, 100.0, &["B"], false, "b", &[]),
        Mod::new("y", Suffix, 300.0, &["Y"], false, "y", &[]),
    ];
    let actions = vec![
        Action::new("transmute", ActionKind::Transmute, &[("transmute", 1.0)], &[NORMAL]),
        Action::new("alt", ActionKind::Alt, &[("alt", 1.0)], &[MAGIC]),
        Action::new("aug", ActionKind::Aug, &[("aug", 1.0)], &[MAGIC]),
        Action::new("regal", ActionKind::Regal, &[("regal", 1.0)], &[MAGIC]),
        Action::new("chaos", ActionKind::RerollRare, &[("chaos", 1.0)], &[RARE]),
    ];
    let product = |wants: &[&str]| {
        let targets: Vec<ponder_core::Target> =
            wants.iter().map(|w| target(w, pool.iter().find(|m| m.id == *w).unwrap().side, &[w], &[&w.to_uppercase()])).collect();
        let mut e = Estimate::new("gear", "Ring", wants.iter().map(|w| w.to_string()).collect());
        let p = ponder_core::ProblemSpec::new(pool.clone(), targets, wants.len(), actions.clone(), Kind::Jewel).compile();
        e.plan = Some(ponder_core::solve(&p, &cheap_table(), &Settings::default()).unwrap());
        e.problem = Some(p);
        e
    };
    vec![product(&["a", "b"]), product(&["a", "c"]), product(&["b"])]
}

#[test]
fn pivots_find_starts_on_the_other_products() {
    let mut ests = pivot_pair();
    add_pivots(&mut ests);
    let ab = &ests[0].pivots;
    assert!(!ab.is_empty(), "crafting a+b rolls a+c now and then");
    let steps: Vec<usize> = ests[0].steps().iter().map(|s| s.number).collect();
    for p in ab {
        assert!(steps.contains(&p.step), "pivots hang off recipe steps");
        assert!(p.to_finish <= (1.0 - PIVOT_SHARE) * p.fresh + 1e-9 && p.per_click > 0.0 && p.per_click <= 1.0);
        assert_eq!(p.product.title, "Ring");
    }
    assert!(ab.iter().any(|p| p.to == 1 && p.to_finish < 1e-9), "an a+c roll is a finished a+c");
    // a twin wanting the same mods is not a pivot, nor is another base group
    let mut twins = pivot_pair();
    twins[1] = twins[0].clone();
    twins[2].title = "Boots".into();
    add_pivots(&mut twins);
    assert!(twins[0].pivots.is_empty());
}

#[test]
fn dominated_crafts_resume_when_an_incumbent_or_base_quote_changes() {
    let mut cx = offline_context(0.0);
    cx.plans = PlanCache::new(temp_dir("competitive-plans"));
    let table = cheap_table();
    let mut estimate = pivot_pair().remove(0);
    let problem = estimate.problem.as_ref().unwrap();
    let mut spec = problem.to_spec();
    let target = spec.targets.remove(0);
    let fixed = spec.pool.iter().find(|modifier| target.good.contains(&modifier.id)).unwrap().clone();
    spec.fixed.push(ponder_core::Mod { weight: 0.0, ..fixed });
    spec.need -= 1;
    let fractured = spec.compile();
    let plan = ponder_core::solve(&fractured, &table, &cx.settings).unwrap();
    let mut fracture = Fracture::new("a", "a", fractured);
    fracture.plan = Some(plan);
    fracture.base_price = Some((0.0, 1));
    estimate.fractures.push(fracture);
    estimate.plan = None;
    solve_estimates(&cx, std::slice::from_mut(&mut estimate), &table, Some(1), None);
    assert!(estimate.plan.is_none());
    assert!(estimate.error.is_none());
    let bound = estimate.craft_lower_bound.expect("direct route is dominated");
    let key = plan_key(estimate.problem.as_ref().unwrap(), &table, &cx.settings);
    assert_eq!(cx.plans.lower_bound(&key), Some(bound));
    assert!(bound > estimate.cheapest().unwrap());

    let mut fracture = estimate.fractures.pop().unwrap();
    solve_estimates(&cx, std::slice::from_mut(&mut estimate), &table, Some(1), None);
    assert!(estimate.plan.is_some());
    assert!(estimate.craft_lower_bound.is_none());
    let direct = estimate.total().unwrap();
    fracture.plan = None;
    fracture.base_price = Some((direct * 2.0, 1));
    estimate.fractures.push(fracture);
    solve_estimates(&cx, std::slice::from_mut(&mut estimate), &table, Some(1), None);
    assert_eq!(estimate.fractures[0].craft_lower_bound, Some(0.0));
    assert!(estimate.fractures[0].plan.is_none());

    estimate.fractures[0].base_price = Some((0.0, 1));
    solve_estimates(&cx, std::slice::from_mut(&mut estimate), &table, Some(1), None);
    assert!(estimate.fractures[0].plan.is_some());
    assert!(estimate.fractures[0].craft_lower_bound.is_none());
    assert_eq!(estimate.cheapest_route(), Some(CraftRoute::Fracture(0)));
}

#[test]
fn short_names_drop_the_numbers() {
    for (name, want) in [
        ("+(100-114) to maximum Life (T1)", "Life T1"),
        ("(80-91)% increased Armour (T2+)", "Armour T2+"),
        ("+(12-15)% to Cold Resistance (T3+)", "Cold Resistance T3+"),
        ("(19-24)% increased Rarity of Items found (T2+)", "Rarity of Items found T2+"),
        ("Vast Power", "Vast Power"),
        ("(10-12)% increased Elemental Damage with Attack Skills and more words (T4)", "Elemental Damage with Att\u{2026} T4"),
        ("Adds (1-3) to (4-6) Physical Damage to Attacks", "Adds # to # Physical Dama\u{2026}"),
        ("40% increased Stun Duration on Enemies", "Stun Duration on Enemies"),
        ("+3 to Level of all Spell Skill Gems", "Level of all Spell Skill \u{2026}"),
    ] {
        assert_eq!(short_name(name, 26), want, "{name}");
    }
}

#[test]
fn round_sig_matches_pythons_3g() {
    for (x, want) in [(123456.0, 123000.0), (0.0123456, 0.0123), (1.005, 1.0), (2.5e-5, 2.5e-5), (999.5, 1000.0), (-4.567, -4.57)] {
        assert_eq!(round_sig(x, 3), want, "{x}");
    }
}

// -- finishing real items -----------------------------------------------------------------

/// A transport for a trade search that finds the given listings.
fn listings_transport(listings: Vec<Value>) -> Arc<dyn Transport> {
    Arc::new(move |req: &Request| {
        if req.url.contains("/api/trade/search/") {
            return Ok(Response::json(
                200,
                &json!({"id": "q", "total": listings.len(), "result": (0..listings.len()).map(|i| format!("id{i}")).collect::<Vec<_>>()}),
            ));
        }
        Ok(Response::json(200, &json!({"result": listings})))
    })
}

#[test]
fn finish_a_real_item_and_find_half_done_ones() {
    common::setup();
    // A ring with the wanted life already on finishes for less than a fresh one; a listed one is
    // priced as its price plus finishing it from its own mods.
    let listing = json!({
        "item": {"rarity": "Rare", "baseType": "Bone Ring", "ilvl": 84, "explicitMods": ["+105 to maximum Life"]},
        "listing": {"price": {"amount": 10, "currency": "chaos"}, "account": {"name": "seller"}, "whisper": "@seller hi"},
    });
    let cx = context(0.0, listings_transport(vec![listing]), temp_dir("starts"));
    let (table, _) = cx.prices.load().unwrap();
    let product = db().match_ids(
        "Bone Ring",
        &IdMods::from_json(&json!({"explicit": [{"id": "IncreasedLife7"}, {"id": "IncreasedEnergyShield10"}]})),
        Some(84),
        &Default::default(),
        "RARE",
        "",
        &Default::default(),
    );
    let mut est = estimate_gear(&cx, db(), &product, &table, "Standard", &GearOpts { exact: true, ..GearOpts::default() }).unwrap();
    assert!(est.plan.is_some(), "{:?}", est.error);
    let have =
        parse_item("Rarity: RARE\nGale Loop\nBone Ring\nItem Level: 84\nImplicits: 0\n+105 to maximum Life\n+30% to Fire Resistance")
            .unwrap();
    let f = finish(&cx, db(), &est, &db().match_item(&have), &table, true);
    assert!(f.error.is_none(), "{:?}", f.error);
    assert_eq!(f.have, est.targets.iter().filter(|t| t.contains("Life")).cloned().collect::<Vec<_>>());
    assert!(f.craft_cost.unwrap() <= est.craft_cost().unwrap() + table["scour"] + 1e-6); // never worse than scouring it
    assert!(f.steps()[0].have.join(" ").contains("Life"), "the recipe starts from the item");
    let both =
        parse_item("Rarity: RARE\nX\nBone Ring\nItem Level: 84\nImplicits: 0\n+105 to maximum Life\n+40 to maximum Energy Shield").unwrap();
    let done = finish(&cx, db(), &est, &db().match_item(&both), &table, true);
    assert!((done.craft_cost.unwrap() - est.bench_cost()).abs() < 1e-6, "already done");
    let corrupted = parse_item("Rarity: RARE\nX\nBone Ring\nImplicits: 0\n+105 to maximum Life\nCorrupted").unwrap();
    assert!(finish(&cx, db(), &est, &db().match_item(&corrupted), &table, true).error.is_some());

    let starts = find_starts(&cx, db(), &mut est, &table, "Standard", 20, 5).unwrap();
    assert!(!starts.is_empty());
    let (s, total) = (&starts[0], starts[0].total().unwrap());
    assert_eq!(s.price, Some(10.0));
    assert!((total - (10.0 + s.craft_cost.unwrap())).abs() < 1e-9);
    assert_eq!(est.starts.len(), starts.len());
    assert!(est.starts_url.is_some());
    let shown = s.to_dict();
    assert_eq!(
        (shown["seller"].as_str(), shown["whisper"].as_str(), shown["price"].as_f64()),
        (Some("seller"), Some("@seller hi"), Some(10.0))
    );
    assert!(shown["steps"].as_array().is_some_and(|s| !s.is_empty()), "the listing was finished with a plan");
    assert_eq!(serde_json::to_value(&est).unwrap()["half_done_for_sale"].as_array().unwrap().len(), starts.len());
}

#[test]
fn finishes_rank_products_by_what_they_save() {
    common::setup();
    let cx = offline_context(0.0);
    let (table, _) = cx.prices.load().unwrap();
    let make = |ids: &[&str]| {
        let explicit: Vec<Value> = ids.iter().map(|i| json!({"id": i})).collect();
        let m = db().match_ids(
            "Bone Ring",
            &IdMods::from_json(&json!({ "explicit": explicit })),
            Some(84),
            &Default::default(),
            "RARE",
            "",
            &Default::default(),
        );
        estimate_gear(&cx, db(), &m, &table, "Standard", &GearOpts { exact: true, ..GearOpts::default() }).unwrap()
    };
    let ests = vec![make(&["IncreasedLife7"]), make(&["IncreasedLife7", "IncreasedEnergyShield10"])];
    let have = parse_item("Rarity: RARE\nGale Loop\nBone Ring\nItem Level: 84\nImplicits: 0\n+105 to maximum Life").unwrap();
    let found = finishes(&cx, db(), &db().match_item(&have), &ests, &table, 5);
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|f| f.plan.is_some() && f.craft_cost.is_some()), "the top few get a plan");
    assert!(found[0].saves().unwrap() >= found[1].saves().unwrap());
    let top_one = finishes(&cx, db(), &db().match_item(&have), &ests, &table, 1);
    assert_eq!(top_one.len(), 1);
    // an item of another base group is no one's start
    let boots = parse_item("Rarity: RARE\nX\nStrapped Boots\nImplicits: 0\n+5 to Strength").unwrap();
    assert!(finishes(&cx, db(), &db().match_item(&boots), &ests, &table, 5).is_empty());
}

// -- plan cache, runs and sources -------------------------------------------------------------

#[test]
fn plans_are_cached_by_problem_and_prices() {
    common::setup();
    let dir = temp_dir("plans");
    let cx = offline_context(0.0).with_plans(PlanCache::new(&dir));
    let table = cheap_table();
    let notables = vec!["Vast Power".to_string(), "Assert Dominance".to_string()];
    let opts = ClusterOpts { solve: false, ..ClusterOpts::default() };
    let mut a = estimate_cluster(&cx, "medium", "area damage", &notables, &table, "Standard", &opts);
    let key = plan_key(a.problem.as_ref().unwrap(), &table, &cx.settings);
    assert_eq!(key.len(), 64);
    assert!(cx.plans.get(&key).is_none());
    solve_one(&cx, &mut a, &table);
    let plan = a.plan.clone().expect("solved");
    let saved = cx.plans.get(&key).expect("saved under the key of the unsolved problem");
    assert_eq!((saved.cost, saved.states, saved.order.len()), (plan.cost, plan.states, plan.order.len()));
    assert_eq!(saved.policy, plan.policy);
    assert_eq!(saved.values, plan.values);
    assert!(std::fs::read_dir(&dir).unwrap().any(|f| f.unwrap().file_name().to_string_lossy().ends_with(".plan")));
    // a second estimate of the same craft is answered from the cache, with the same attachments
    let mut b = estimate_cluster(&cx, "medium", "area damage", &notables, &table, "Standard", &opts);
    solve_one(&cx, &mut b, &table);
    assert_eq!(b.plan.as_ref().unwrap().cost, plan.cost);
    // prices count to 3 figures, the clicking rate exactly, and what the actions don't use not at all
    let fresh = estimate_cluster(&cx, "medium", "area damage", &notables, &table, "Standard", &opts);
    let p = fresh.problem.as_ref().unwrap();
    let mut nudged = table.clone();
    nudged.insert("alt".into(), table["alt"] * 1.0001);
    assert_eq!(plan_key(p, &nudged, &cx.settings), key);
    nudged.insert("alt".into(), table["alt"] * 1.1);
    assert_ne!(plan_key(p, &nudged, &cx.settings), key);
    nudged = table.clone();
    nudged.insert("divine".into(), 999.0);
    assert_eq!(plan_key(p, &nudged, &cx.settings), key);
    assert_ne!(plan_key(p, &table, &Settings { chaos_per_hour: 1500.0 }), key);
    // damaged files are no plan
    let file = dir.join(format!("{key}.plan"));
    std::fs::write(&file, b"not a plan").unwrap();
    assert!(cx.plans.get(&key).is_none());
    assert!(PlanCache::disabled().get(&key).is_none());
    PlanCache::disabled().put(&key, &plan); // and keeps nothing
}

#[test]
fn solve_all_solves_what_needs_solving_in_parallel() {
    common::setup();
    let cx = offline_context(0.0).with_plans(PlanCache::new(temp_dir("solve-all")));
    let table = cheap_table();
    let opts = ClusterOpts { solve: false, ..ClusterOpts::default() };
    let sets: [&[&str]; 4] = [&["Vast Power"], &["Assert Dominance"], &["Magnifier"], &["Vast Power", "Assert Dominance"]];
    let mut ests: Vec<Estimate> = sets
        .iter()
        .map(|n| {
            estimate_cluster(&cx, "medium", "area damage", &n.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &table, "Standard", &opts)
        })
        .collect();
    let broken = estimate_cluster(&cx, "medium", "no such enchant", &["x".to_string()], &table, "Standard", &opts);
    assert!(broken.error.is_some() && broken.problem.is_none());
    ests.push(broken);
    let said = Mutex::new(Vec::new());
    let say = |m: &str| said.lock().unwrap().push(m.to_string());
    {
        let mut items: Vec<&mut dyn Solvable> = ests.iter_mut().map(|e| e as &mut dyn Solvable).collect();
        solve_all(&cx, &mut items, &table, None, Some(&say));
    }
    for e in &ests[..4] {
        assert!(e.plan.is_some(), "{:?}", e.error);
    }
    assert!(ests[4].plan.is_none());
    assert!(said.lock().unwrap().iter().any(|m| m.starts_with("solved ")), "{:?}", said.lock().unwrap());
    // more notables cost more
    let costs: Vec<f64> = ests[..4].iter().map(|e| e.craft_cost().unwrap()).collect();
    assert!(costs[3] > costs[0] && costs[3] > costs[1], "{costs:?}");
    // solving again changes nothing and says nothing
    said.lock().unwrap().clear();
    let mut items: Vec<&mut dyn Solvable> = ests.iter_mut().map(|e| e as &mut dyn Solvable).collect();
    solve_all(&cx, &mut items, &table, Some(1), Some(&say));
    assert!(said.lock().unwrap().is_empty());
    // a problem the solver can't do is an error on the estimate, not a panic
    let mut hopeless = estimate_cluster(&cx, "medium", "area damage", &["Vast Power".to_string()], &table, "Standard", &opts);
    let mut spec = hopeless.problem.as_ref().unwrap().to_spec();
    spec.actions.clear();
    hopeless.problem = Some(spec.compile());
    solve_one(&cx, &mut hopeless, &table);
    assert!(hopeless.plan.is_none() && hopeless.error.is_some(), "{:?}", hopeless.error);
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
fn the_league_is_the_newest_or_standard() {
    let cx = offline_context(0.0);
    assert_eq!(current_league(&cx), "Standard"); // the fake poe.ninja lists only Standard
    let down = Context::new(
        Settings::default(),
        PriceClient::with(offline(), temp_dir("league"), fixed_clock()),
        Trade::with(offline(), temp_dir("league"), fixed_clock()),
        Ladder::with(offline(), temp_dir("league"), fixed_clock()),
        offline(),
    );
    assert_eq!(current_league(&down), "Standard");
    let up = Context::new(
        Settings::default(),
        PriceClient::with(
            Arc::new(|_: &Request| Ok(Response::json(200, &json!([{"id": "Allflame"}, {"id": "Standard"}])))),
            temp_dir("league"),
            fixed_clock(),
        ),
        Trade::with(offline(), temp_dir("league"), fixed_clock()),
        Ladder::with(offline(), temp_dir("league"), fixed_clock()),
        offline(),
    );
    assert_eq!(current_league(&up), "Allflame");
}

#[test]
fn run_turns_builds_into_a_report() {
    common::setup();
    // The builds come from a share link: the fake share site serves a fixture's code.
    let code = std::fs::read_to_string(repo_root().join("fixtures/real/occultist_winter_orb.txt")).unwrap();
    let share: Arc<dyn Transport> = Arc::new(move |req: &Request| {
        assert_eq!(req.url, "https://pobb.in/abc/raw", "only the raw build is fetched");
        Ok(Response::new(200, code.clone()))
    });
    let calls = Arc::new(Mutex::new(Vec::new()));
    let cx = context_with(1500.0, fake_trade(calls.clone()), temp_dir("run"), share);
    let sources = vec![
        "https://pobb.in/abc".to_string(),
        "fixtures/real/no_such_build.txt".to_string(),
        repo_root().join("fixtures/real/marauder_rf_leveling.txt").to_string_lossy().into_owned(),
    ];
    let opts = RunOpts { fractures: true, trade_lookup: true, trade_budget: 6, workers: Some(2), ..RunOpts::default() };
    let progress = Mutex::new(Vec::new());
    let report = run(&cx, db(), &sources, &opts, Some(&|m: &str| progress.lock().unwrap().push(m.to_string()))).unwrap();
    assert_eq!(report.builds.len(), 2);
    assert_eq!(report.builds[0].name, "Occultist lvl 97: Winter Orb");
    assert!(report.builds.iter().all(|b| b.rares > 3));
    assert!(report.errors.iter().any(|e| e.starts_with("fixtures/real/no_such_build.txt")));
    assert!(
        report.errors.iter().all(|e| e.starts_with("fixtures/real/no_such_build.txt") || e.contains("bench crafts conflict")),
        "{:?}",
        report.errors
    );
    assert_eq!(report.league, "Standard");
    assert!(report.estimates.len() >= 6);
    assert!(report.estimates.iter().filter(|e| e.plan.is_some()).count() >= 5);
    assert!(report.estimates.iter().all(|e| !e.builds.is_empty() && e.settings.chaos_per_hour == 1500.0));
    assert!(
        report.estimates.iter().any(|e| { !e.fractures.is_empty() && e.fractures.iter().all(|f| f.plan.is_some() || f.error.is_some()) })
    );
    assert!(report.estimates.iter().any(|e| e.slot.is_some()));
    let said = progress.lock().unwrap();
    assert!(said.iter().any(|m| m.starts_with("loading https://pobb.in/abc")) && said.iter().any(|m| m.starts_with("trade ")), "{said:?}");
    // the trade budget caps the searches; the answers are in the report
    assert!(calls.lock().unwrap().len() <= 6);
    assert!(report.estimates.iter().any(|e| e.market.is_some() || e.market_error.is_some()));
    // shared combinations are mined from both builds
    assert!(report.shared.len() <= 30);
    for s in &report.shared {
        assert!(s.candidate.builds >= 2);
    }
    // the JSON is Python's: ranked, with every estimate's keys
    let out = report.to_dict();
    assert_eq!(out["estimates"].as_array().unwrap().len(), report.estimates.len());
    assert_eq!(out["builds"][0]["rares"], report.builds[0].rares);
    let ranked = report.ranked();
    assert!(ranked.windows(2).all(|w| cmp_priority(w[0], w[1]) != std::cmp::Ordering::Greater));
    let text = serde_json::to_string(&report).unwrap();
    assert!(text.contains("\"chaos_per_divine\"") && !text.contains("POESESSID="));
}

#[test]
fn the_json_keys() {
    let e = estimate("x", 10.0, Some(100.0));
    let keys: Vec<String> = e.to_dict().as_object().unwrap().keys().cloned().collect();
    let want = [
        "kind",
        "title",
        "base",
        "build_base",
        "bases_in_group",
        "mods",
        "targets",
        "slot",
        "builds",
        "item_level",
        "craft_cost",
        "direct_lower_bound",
        "craft_bounds",
        "bench",
        "base_price",
        "base_source",
        "base_url",
        "total",
        "market",
        "listed",
        "margin",
        "currency",
        "clicking_hours",
        "currency_chaos",
        "recipe",
        "recipe_stages",
        "selected_route",
        "steps",
        "fractures",
        "not_crafted",
        "trade_url",
        "recomb",
        "recomb_total",
        "demand",
        "skills",
        "hole",
        "fracture_worth",
        "pivots",
        "half_done_for_sale",
        "error",
    ];
    assert_eq!(keys, want);
    let mut f = Fracture::new("t", "m", tiny(vec![], &[], 0, None));
    f.base_price = Some((12.0, 3));
    f.plan = Some(Plan { cost: 8.0, ..Plan::default() });
    f.made = Some(ponder_core::SelfFracture {
        target: 0,
        action: "scour+alch".into(),
        max_mods: 5,
        hit: 0.5,
        fracture_chance: 0.25,
        cost: 9.0,
        hours: 0.1,
    });
    let d = f.to_dict();
    assert_eq!(
        d.as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "target",
            "craft_cost",
            "craft_lower_bound",
            "base_price",
            "base_listed",
            "self_made",
            "total",
            "trade_url",
            "donor_query",
            "donor_url",
            "error"
        ]
    );
    assert_eq!(
        (d["total"].as_f64(), d["base_listed"].as_i64(), d["self_made"]["action"].as_str(), f.self_made()),
        (Some(17.0), Some(3), Some("scour+alch"), true)
    );
    assert_eq!(f.base_cost(), Some(9.0));
}

#[test]
fn estimate_arithmetic() {
    let mut e = estimate("x", 100.0, Some(500.0));
    e.bench = vec![("Life".into(), 5.0)];
    e.problem = Some(tiny(vec![], &[], 0, None));
    e.base_price = Some((20.0, 3));
    e.demand = 10;
    assert_eq!((e.total(), e.margin(), e.cheapest()), (Some(125.0), Some(375.0), Some(125.0)));
    assert_eq!(e.hole(), Some(375.0 * 10.0 / 10.0)); // listed 5, plus 5
    // a fractured base without a price leaves the total incomplete
    e.fixed = vec!["Life".into()];
    e.base_price = None;
    assert!(e.needs_base_price() && e.margin().is_none() && e.hole().is_none());
    assert!(e.total().is_none() && e.cheapest_route().is_none());
    e.fixed.clear();
    // a priced, cheaper fracture is the cheapest way
    let mut f = Fracture::new("a", "m", tiny(vec![], &[], 0, None));
    f.plan = Some(Plan { cost: 40.0, ..Plan::default() });
    f.bench_cost = e.bench_cost();
    f.base_price = Some((30.0, 2));
    e.fractures.push(f);
    assert_eq!((e.cheapest(), e.best_fracture().map(|f| f.target.as_str())), (Some(75.0), Some("a")));
    assert_eq!(e.cheapest_route(), Some(CraftRoute::Fracture(0)));
    assert_eq!(e.cheapest_plan().unwrap().1.cost, 40.0);
    assert_eq!(e.fracture_worth().map(|(w, f)| (w, f.target.clone())), Some((105.0 - 45.0, "a".to_string())));
    // a recombinator tree competes too, bench included
    e.recomb = Some(RecombInfo {
        cost: Some(50.0),
        json: json!({"cost": 50}),
        part: vec![],
        finish: 0.0,
        finishing: None,
        usage: [("exalt".into(), 2.0)].into(),
        hours: 0.0,
        donors: Vec::new(),
        recovery: None,
    });
    assert_eq!((e.recomb_total(), e.cheapest()), (Some(55.0), Some(55.0)));
    assert_eq!(e.cheapest_route(), Some(CraftRoute::Recomb));
    assert_eq!(e.usage(), vec![("exalt".into(), 2.0)]);
    assert_eq!(e.to_dict()["selected_route"], "recombination");
    assert_eq!(e.to_dict()["total"], 55.0);
    assert_eq!(e.to_dict()["craft_cost"], 50.0);
    assert!(e.to_dict()["craft_bounds"].is_null());
    assert_eq!(e.to_dict()["direct_total"], 105.0);
    assert!(e.cheapest_plan().is_none());
    assert_eq!(e.to_dict()["recomb"], json!({"cost": 50}));
    // no plan, no total
    let bare = Estimate::new("gear", "bare", vec![]);
    assert!(
        bare.total().is_none()
            && bare.cheapest().is_none()
            && bare.craft_cost().is_none()
            && bare.steps().is_empty()
            && bare.usage().is_empty()
    );
}

#[test]
fn protected_stages_remain_available_after_a_direct_search_failure() {
    use ponder_core::{Action, Kind, Mod, NORMAL, RARE, Side, Target};
    let cx = offline_context(0.0);
    let make_mod = |id: &str, side| Mod::new(id, side, 1.0, &[id], false, id, &[]);
    let pool = vec![make_mod("p0", Side::Prefix), make_mod("p1", Side::Prefix), make_mod("p2", Side::Prefix), make_mod("s0", Side::Suffix)];
    let targets = pool
        .iter()
        .map(|m| Target { name: m.id.clone(), side: m.side, good: [m.id.clone()].into(), groups: [m.id.clone()].into(), notable: false })
        .collect();
    let actions = vec![
        Action::new("roll", ActionKind::RerollRare, &[("roll", 1.0)], &[NORMAL, RARE]),
        Action::new("lock prefixes", ActionKind::MetaLockPrefixes, &[("lock", 1.0)], &[RARE]),
        Action::new("chaos", ActionKind::Reforge, &[("chaos", 1.0)], &[RARE]),
    ];
    let p = ProblemSpec::new(pool, targets, 4, actions, Kind::Gear).compile();
    let mut e = Estimate::new("gear", "test", p.targets.iter().map(|t| t.name.clone()).collect());
    e.problem = Some(p);
    e.error = Some("direct search incomplete".into());
    let prices = Prices::from([("roll".into(), 1.0), ("lock".into(), 2.0), ("chaos".into(), 1.0), ("scour".into(), 0.1)]);
    add_protected_routes(&cx, std::slice::from_mut(&mut e), &prices, Some(1));
    assert!(e.protected.is_some());
    assert_eq!(e.cheapest_route(), Some(CraftRoute::Protected));
    assert!(e.cheapest().unwrap().is_finite());
    assert!(e.failure().is_none());
    assert_eq!(e.to_dict()["direct_error"], "direct search incomplete");
    assert_eq!(e.recipe_stages().len(), 3);
}

#[test]
fn protected_finishing_handles_six_modifiers_on_an_influenced_ring() {
    common::setup();
    let cx = offline_context(0.0);
    let modifiers =
        ["ChaosResist5", "IncreasedEvasionRating6", "AllResistances5", "Strength4", "IncreasedLife4", "LifeGainedOnSpellHitUber1"];
    let m = db().match_ids(
        "Vermillion Ring",
        &IdMods { explicit: modifiers.map(str::to_owned).to_vec(), ..Default::default() },
        Some(86),
        &BTreeSet::from(["shaper".into()]),
        "RARE",
        "",
        &Default::default(),
    );
    let mut e = estimate_gear(&cx, db(), &m, price_table(), "Standard", &gear_opts()).unwrap();
    assert_eq!(e.problem.as_ref().unwrap().targets.len(), 6);
    add_protected_routes(&cx, std::slice::from_mut(&mut e), price_table(), Some(1));
    assert!(e.protected.is_some());
    assert!(e.protected.as_ref().unwrap().finishing.states < 10000);
}
#[test]
fn arbitrary_full_item_padding_does_not_change_the_goal() {
    common::setup();
    let cx = offline_context(0.0);
    let mut m = db().match_ids(
        "Bone Ring",
        &IdMods { explicit: vec!["IncreasedLife5".into()], ..Default::default() },
        Some(84),
        &Default::default(),
        "RARE",
        "",
        &Default::default(),
    );
    m.finish = Some(json!({"full": 4}));
    let a = estimate_gear(&cx, db(), &m, price_table(), "Standard", &gear_opts()).unwrap();
    m.finish = Some(json!({"full": 6}));
    let b = estimate_gear(&cx, db(), &m, price_table(), "Standard", &gear_opts()).unwrap();
    assert_eq!(a.key, b.key);
}

#[test]
fn donor_searches_allow_magic_starts_and_editable_modifier_counts() {
    common::setup();
    let cx = offline_context(0.0);
    let wanted = vec![BTreeSet::from(["IncreasedLife7".into()])];
    let q = donor_query(&cx, "Bone Ring", &wanted, Some(84), 1, 1).unwrap();
    assert_eq!(q["query"]["filters"]["type_filters"]["filters"]["rarity"]["option"], "nonunique");
    assert_eq!(q["query"]["stats"][0]["filters"][0]["id"], "pseudo.pseudo_number_of_affix_mods");
    assert_eq!(q["query"]["stats"][0]["filters"][0]["value"], json!({"min": 1, "max": 1}));
    let misc = &q["query"]["filters"]["misc_filters"]["filters"];
    for key in ["corrupted", "fractured_item", "mirrored", "synthesised_item"] {
        assert_eq!(misc[key]["option"], "false");
    }
    assert_eq!(q["query"]["stats"][2]["type"], "not");
    assert_eq!(q["query"]["stats"][2]["filters"].as_array().unwrap().len(), 6);
    let broad = donor_mod_range(&q, 1, 6);
    assert_eq!(broad["query"]["stats"][0]["filters"][0]["value"], json!({"min": 1, "max": 6}));
    assert_eq!(broad["query"]["stats"][1], q["query"]["stats"][1]);
    assert_eq!(q["query"]["stats"][0]["filters"][0]["value"]["max"], 1);
}

#[test]
fn finishing_rebuilds_the_actual_item_level_pool() {
    common::setup();
    let cx = offline_context(0.0);
    let product = db().match_ids(
        "Bone Ring",
        &IdMods { explicit: vec!["IncreasedLife5".into()], ..Default::default() },
        Some(84),
        &Default::default(),
        "RARE",
        "",
        &Default::default(),
    );
    let e = estimate_gear(&cx, db(), &product, price_table(), "Standard", &GearOpts::default()).unwrap();
    let item = db().match_ids("Bone Ring", &IdMods::default(), Some(30), &Default::default(), "NORMAL", "", &Default::default());
    let f = finish(&cx, db(), &e, &item, price_table(), true);
    assert!(f.error.is_none(), "{:?}", f.error);
    assert!(f.problem.as_ref().unwrap().pool.iter().all(|m| db().mods[&m.id].required_level <= 30));
}
#[test]
fn recombination_reports_trade_links_for_its_actual_donors() {
    common::setup();
    let cx = offline_context(0.0);
    let product = db().match_ids(
        "Bone Ring",
        &IdMods { explicit: vec!["IncreasedLife7".into(), "IncreasedEnergyShield10".into()], ..Default::default() },
        Some(84),
        &Default::default(),
        "RARE",
        "",
        &Default::default(),
    );
    let mut estimates = vec![estimate_gear(&cx, db(), &product, price_table(), "Test League", &GearOpts::default()).unwrap()];
    add_recombs_in_league(&cx, db(), &mut estimates, price_table(), "Test League", Some(1), None);
    let donors = estimates[0].recomb.as_ref().unwrap().json["donors"].as_array().unwrap();
    assert!(!donors.is_empty());
    for donor in donors {
        assert!(donor["trade_url"].as_str().unwrap().contains("/Test%20League?q="));
        assert_eq!(donor["query"]["query"]["type"], donor["base"]);
        assert_eq!(donor["query"]["query"]["stats"][0]["filters"][0]["value"]["min"], donor["required_mods"]);
        assert_eq!(donor["query"]["query"]["stats"][0]["filters"][0]["value"]["max"], donor["required_mods"]);
    }
}

#[test]
fn cached_search_failures_remain_visible_and_new_prices_trigger_a_new_solve() {
    use ponder_core::{Action, Kind, Mod, NORMAL, Side, Target};
    let cx = offline_context(0.0).with_plans(PlanCache::new(temp_dir("failed-plans")));
    let modifier = Mod::new("p", Side::Prefix, 1.0, &["p"], false, "p", &[]);
    let target = Target { name: "p".into(), side: Side::Prefix, good: ["p".into()].into(), groups: ["p".into()].into(), notable: false };
    let action = Action::new("transmute", ActionKind::Transmute, &[("missing", 1.0)], &[NORMAL]);
    let problem = ProblemSpec::new(vec![modifier], vec![target], 1, vec![action], Kind::Gear).compile();
    let make = || {
        let mut e = Estimate::new("gear", "p", vec![]);
        e.problem = Some(problem.clone());
        e
    };
    let mut first = make();
    solve_one(&cx, &mut first, &Prices::new());
    assert!(first.error.is_some() && first.plan.is_none());
    let mut repeated = make();
    let progress = Mutex::new(Vec::new());
    solve_all(&cx, &mut [&mut repeated], &Prices::new(), Some(1), Some(&|m: &str| progress.lock().unwrap().push(m.to_string())));
    assert_eq!(repeated.error, first.error);
    assert!(progress.lock().unwrap().is_empty());
    let mut repriced = make();
    solve_one(&cx, &mut repriced, &Prices::from([("missing".into(), 1.0)]));
    assert!(repriced.error.is_none());
    assert_eq!(repriced.plan.unwrap().cost, 1.0);
}
