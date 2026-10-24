//! The pipeline: the original test suite, plus the goldens (goldens/problems and
//! goldens/pipeline). Nothing here touches the network: every client is given a fake
//! transport, and the plan cache lives under the target directory.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once, OnceLock};

use anyhow::anyhow;
use ponder::pipeline::*;
use ponder_core::{Plan, Prices, Problem, Settings};
use ponder_game::data;
use ponder_game::item::{ModLine, parse_item};
use ponder_game::mods::{IdMods, ItemMatch, ModDB};
use ponder_net::env::Clock;
use ponder_net::http::{Request, Response, Transport};
use ponder_net::ladder::Ladder;
use ponder_net::prices::Prices as PriceClient;
use ponder_net::trade::Trade;
use serde_json::{Value, json};

// -- helpers ----------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    match std::env::var_os("PONDER_REPO") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap(),
    }
}

fn skip_without_data() -> bool {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("PONDER_DATA").is_none() {
            // SAFETY: every test calls this first and the Once blocks the others until it has run.
            unsafe { std::env::set_var("PONDER_DATA", repo_root().join(".cache/repoe")) };
        }
    });
    let have = data::available(None) && data::data_dir().join("cluster_jewels.json").is_file();
    if !have {
        eprintln!("skipping: repoe data not found in {}; run `ponder data fetch`", data::data_dir().display());
    }
    !have
}

fn skip_without_goldens(dir: &str) -> bool {
    let have = repo_root().join("goldens").join(dir).is_dir();
    if !have {
        eprintln!("skipping: no goldens in goldens/{dir}");
    }
    !have
}

fn db() -> &'static ModDB {
    static DB: OnceLock<ModDB> = OnceLock::new();
    DB.get_or_init(|| ModDB::load().expect("repoe data"))
}

fn golden(path: &str) -> Value {
    let file = std::fs::File::open(repo_root().join("goldens").join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut text = String::new();
    flate2::read::GzDecoder::new(file).read_to_string(&mut text).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// A transport that fails every request: tests that reach the network by accident find out.
fn offline() -> Arc<dyn Transport> {
    Arc::new(|req: &Request| Err(anyhow!("no network in tests: {}", req.url)))
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("pipeline-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The plan cache shared by tests (in the target directory, so reruns are fast).
fn shared_plans() -> PlanCache {
    PlanCache::new(PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("plans-rs"))
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
    let root = repo_root();
    let prices = PriceClient::with(
        Arc::new(|req: &Request| {
            if req.url.ends_with("/leagues") {
                return Ok(Response::json(200, &json!([{"id": "Standard"}])));
            }
            Err(anyhow!("no network in tests: {}", req.url))
        }),
        root.join(".cache/ninja"),
        fixed_clock(),
    );
    let trade = Trade::with(trade_http, trade_cache, fixed_clock());
    let ladder = Ladder::with(offline(), root.join(".cache/ladder"), fixed_clock());
    Context::new(Settings { chaos_per_hour }, prices, trade, ladder, share_http).with_plans(shared_plans())
}

fn offline_context(chaos_per_hour: f64) -> Context {
    context(chaos_per_hour, offline(), temp_dir("trade"))
}

fn golden_table() -> &'static Prices {
    static T: OnceLock<Prices> = OnceLock::new();
    T.get_or_init(|| serde_json::from_value(golden("problems/prices.json.gz")).unwrap())
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

fn close_to(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel.max(rel * a.abs().max(b.abs()))
}

/// Differences between two JSON documents, numbers compared to a relative 1e-9.
fn diff(a: &Value, b: &Value, path: &str, out: &mut Vec<String>) {
    diff_with(a, b, path, out, 1e-9)
}

/// `diff` with a chosen relative tolerance (the solver's answers agree to about 1e-7).
fn diff_with(a: &Value, b: &Value, path: &str, out: &mut Vec<String>, rel: f64) {
    let diff = |a: &Value, b: &Value, path: &str, out: &mut Vec<String>| diff_with(a, b, path, out, rel);
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            if !close_to(x.as_f64().unwrap(), y.as_f64().unwrap(), rel) {
                out.push(format!("{path}: {x} != {y}"));
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys()).collect::<BTreeSet<_>>() {
                match (x.get(k), y.get(k)) {
                    (Some(p), Some(q)) => diff(p, q, &format!("{path}.{k}"), out),
                    (p, q) => out.push(format!("{path}.{k}: {} != {}", p.map_or("missing".into(), |v| v.to_string()), q.map_or("missing".into(), |v| v.to_string()))),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                out.push(format!("{path}: {} items != {}", x.len(), y.len()));
            }
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                diff(p, q, &format!("{path}[{i}]"), out);
            }
        }
        _ if a == b => {}
        _ => out.push(format!("{path}: {a} != {b}")),
    }
}

/// A trade query with the order of its filler (count-group) stats ignored: Python builds that list
/// by iterating a set, so its order differs from run to run.
fn query_unordered(q: &Value) -> Value {
    let mut q = q.clone();
    if let Some(groups) = q.pointer_mut("/query/stats").and_then(Value::as_array_mut) {
        for g in groups.iter_mut() {
            if g["type"] == "count"
                && let Some(f) = g["filters"].as_array_mut() {
                    f.sort_by_key(|x| x.to_string());
                }
        }
    }
    q
}

/// The archetype's match as `demand.archetype_match` made it: mods by id, with the finish spec.
fn match_of(g: &Value) -> ItemMatch {
    let mut ids = IdMods::default();
    for m in g["mods"].as_array().unwrap() {
        let id = m["mod_id"].as_str().unwrap().to_string();
        if m["fractured"].as_bool().unwrap() {
            ids.fractured.push(id);
        } else if m["source"] == "crafted" {
            ids.crafted.push(id);
        } else {
            ids.explicit.push(id);
        }
    }
    let influences: BTreeSet<String> = strings(&g["influences"]).into_iter().collect();
    let mut m = db().match_ids(g["base"].as_str().unwrap(), &ids, g["item_level"].as_i64(), &influences, "RARE", "", &Default::default());
    // `demand.archetype_match` gives each mod all lines of its text (hybrids have two), and no markers.
    for (x, want) in m.mods.iter_mut().zip(g["mods"].as_array().unwrap()) {
        let text = db().mods[x.mod_id.as_str()].text_or_empty();
        let marker = |l: ModLine| if want["fractured"] == true { ModLine::with_marker(&l.text, "fractured") } else { l };
        let lines: Vec<ModLine> = text.lines().map(|t| marker(ModLine::new(t))).collect();
        if !lines.is_empty() {
            x.lines = lines;
        }
    }
    if !g["finish"].is_null() {
        m.finish = Some(g["finish"].clone());
    }
    m
}

fn gear_opts() -> GearOpts {
    GearOpts { solve: false, ..GearOpts::default() }
}

fn real_build(name: &str) -> ponder_game::pob::Build {
    let path = repo_root().join("fixtures/real").join(name);
    ponder_game::pob::load_with(path.to_str().unwrap(), |_| Err("no network".into())).unwrap()
}

fn cheap_table() -> Prices {
    [("alt", 0.074), ("aug", 0.017), ("regal", 0.093), ("scour", 0.35), ("transmute", 0.004), ("chaos", 1.0), ("exalted", 5.0), ("annul", 11.0), ("alch", 0.03)]
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
    if skip_without_data() {
        return;
    }
    let cx = offline_context(0.0);
    let table: Prices = [("alt", 0.074), ("aug", 0.017), ("regal", 0.093), ("scour", 0.35), ("transmute", 0.004), ("chaos", 1.0)].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
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
    if skip_without_data() {
        return;
    }
    let cx = offline_context(0.0);
    let build = real_build("occultist_winter_orb.txt");
    let gloves = build
        .items
        .iter()
        .filter_map(|i| parse_item(&i.text).ok())
        .find(|i| i.base == "Warlock Gloves" && i.rarity == "RARE")
        .expect("gloves in the fixture");
    let table: Prices =
        [("alt", 0.074), ("aug", 0.017), ("regal", 0.093), ("scour", 0.35), ("transmute", 0.004), ("chaos", 1.0), ("exalted", 5.0), ("annul", 11.0), ("alch", 0.03)]
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
    // Starting with one target already on the item can only make the rest cheaper.
    let whole = est.craft_cost().unwrap();
    assert!(est.fractures.iter().all(|f| f.craft_cost().unwrap() <= whole + 1e-6));
    assert!(est.fractures.iter().all(|f| {
        let url = f.trade_url.as_deref().unwrap();
        url.contains("fractured.stat_") && url.contains("fractured_item")
    }));
}

// -- goldens/problems ---------------------------------------------------------------

#[test]
fn gear_estimates_match_python_goldens() {
    if skip_without_data() {
        return;
    }
    let cx = offline_context(1500.0);
    let rows = golden("problems/gear.json.gz");
    let (mut total, mut bad) = (0, Vec::new());
    for row in rows.as_array().unwrap().iter().filter(|r| !r["match"].is_null()) {
        total += 1;
        let name = format!("{} {:?}", row["group"].as_str().unwrap(), row["families"]);
        let want = &row["estimate"];
        let est = estimate_gear(&cx, db(), &match_of(&row["match"]), golden_table(), "Standard", &gear_opts()).unwrap();
        let mut d = Vec::new();
        let have = json!({
            "title": est.title, "base": est.base, "bases": est.bases, "targets": est.targets, "error": est.error,
            "query": est.query.as_ref().map(query_unordered), "search": est.search.to_value(),
        });
        let mut want = want.clone();
        want["query"] = if want["query"].is_null() { Value::Null } else { query_unordered(&want["query"]) };
        diff(&have, &want, "estimate", &mut d);
        if !d.is_empty() {
            d.truncate(4);
            bad.push(format!("{name}:\n    {}", d.join("\n    ")));
        }
    }
    eprintln!("gear estimates: {}/{total} identical to Python", total - bad.len());
    assert!(bad.is_empty(), "{} of {total} differ:\n{}", bad.len(), bad.join("\n"));
    assert_eq!(total, 113);
}

#[test]
fn fractures_match_python_goldens() {
    if skip_without_data() {
        return;
    }
    let cx = offline_context(1500.0);
    let rows = golden("problems/gear.json.gz");
    let fr = golden("problems/fractures.json.gz");
    let mut ests: Vec<Estimate> = rows
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| !r["match"].is_null())
        .map(|r| estimate_gear(&cx, db(), &match_of(&r["match"]), golden_table(), "Standard", &gear_opts()).unwrap())
        .filter(|e| e.error.is_none())
        .take(15)
        .collect();
    assert_eq!(ests.len(), fr.as_array().unwrap().len());
    let mut bad = Vec::new();
    let mut made = 0;
    for (e, want) in ests.iter_mut().zip(fr.as_array().unwrap()) {
        add_fractures(&cx, e, "Standard", Some(golden_table()), false);
        let mut d = Vec::new();
        assert_eq!((e.title.as_str(), json!(e.targets)), (want["title"].as_str().unwrap(), want["targets"].clone()));
        let want_fr = want["fractures"].as_array().unwrap();
        if e.fractures.len() != want_fr.len() {
            d.push(format!("{} fractures != {}", e.fractures.len(), want_fr.len()));
        }
        for (f, w) in e.fractures.iter().zip(want_fr) {
            let mut f = f.clone();
            // Of tiers with equal weight Python's set order picked one at random; any of them will do.
            let weight = |id: &str| e.problem.as_ref().unwrap().pool.iter().find(|m| m.id == id).map(|m| m.weight);
            let want_id = w["mod_id"].as_str().unwrap();
            if want_id != f.mod_id {
                assert!(weight(want_id).is_some() && weight(want_id) == weight(&f.mod_id), "{}: {} is not a tie with {}", f.target, f.mod_id, want_id);
                let fixed = f.problem.as_mut().unwrap().fixed.last_mut().unwrap();
                fixed.id = want_id.to_string();
                fixed.label = w["problem"]["fixed"].as_array().unwrap().last().unwrap()["label"].as_str().unwrap().to_string();
                f.mod_id = want_id.to_string();
            }
            let mut have_problem = serde_json::to_value(f.problem.as_ref().unwrap()).unwrap();
            have_problem.as_object_mut().unwrap().entry("begin").or_insert(Value::Null);
            let have = json!({
                "target": f.target, "mod_id": f.mod_id, "problem": have_problem,
                "query": f.query.as_ref().map(query_unordered),
                "made": f.made.as_ref().map(|m| json!({"action": m.action, "max_mods": m.max_mods, "hit": m.hit, "fracture_chance": m.fracture_chance, "cost": m.cost})),
            });
            made += f.made.is_some() as usize;
            let mut w = w.clone();
            if !w["query"].is_null() {
                w["query"] = query_unordered(&w["query"]);
            }
            diff(&have, &w, &format!("{}/{}", e.title, f.target), &mut d);
        }
        if !d.is_empty() {
            d.truncate(5);
            bad.push(d.join("\n    "));
        }
    }
    eprintln!("fractures: {} estimates, {made} self-made costs; {} differ", ests.len(), bad.len());
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert!(made > 0);
}

// -- goldens/pipeline ---------------------------------------------------------------

fn problem_of(v: &Value) -> Problem {
    let mut v = v.clone();
    v.as_object_mut().unwrap().entry("begin").or_insert(Value::Null);
    serde_json::from_value(v).unwrap()
}

fn base_price_of(v: &Value) -> Option<(f64, i64)> {
    v.as_array().map(|a| (a[0].as_f64().unwrap(), a[1].as_i64().unwrap()))
}

/// The tree golden's estimates as they were before the product tree: problems, fractures, queries.
fn tree_estimates(doc: &Value, settings: Settings) -> Vec<Estimate> {
    doc["before"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let mut est = Estimate::new(e["kind"].as_str().unwrap(), e["title"].as_str().unwrap(), strings(&e["targets"]));
            est.settings = settings;
            est.problem = Some(problem_of(&e["problem"]));
            est.base_price = base_price_of(&e["base_price"]);
            est.base_source = e["base_source"].as_str().unwrap().to_string();
            est.query = Some(e["query"].clone()).filter(|q| !q.is_null());
            est.base_query = Some(e["base_query"].clone()).filter(|q| !q.is_null());
            est.trade_url = e["trade_url"].as_str().map(String::from);
            est.base_url = e["base_url"].as_str().map(String::from);
            est.item_level = e["item_level"].as_i64().unwrap();
            est.demand = e["demand"].as_u64().unwrap() as usize;
            est.base = e["base"].as_str().unwrap().into();
            est.used_base = e["used_base"].as_str().unwrap().into();
            est.bases = e["bases"].as_u64().unwrap() as usize;
            est.bench = e["bench"].as_array().unwrap().iter().map(|b| (b[0].as_str().unwrap().to_string(), b[1].as_f64().unwrap())).collect();
            est.skipped = strings(&e["skipped"]);
            est.fixed = strings(&e["fixed"]);
            est.fixed_ids = strings(&e["fixed_ids"]);
            est.filler_families = strings(&e["filler_families"]);
            est.skills = strings(&e["skills"]);
            est.rows = e["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| ponder::gear::Row {
                    role: ["target", "essence", "fractured", "bench", "skipped", "filler"].into_iter().find(|x| *x == r[0].as_str().unwrap()).unwrap(),
                    side: if r[1] == "prefix" { ponder_core::Side::Prefix } else { ponder_core::Side::Suffix },
                    text: r[2].as_str().unwrap().into(),
                    tier: r[3].as_str().unwrap().into(),
                })
                .collect();
            let s = &e["search"];
            est.search = if s.get("size").is_some() {
                Search::Cluster { size: s["size"].as_str().unwrap().into(), enchant: s["enchant"].as_str().map(String::from), passives: s["passives"].as_i64().unwrap() }
            } else if let Some(b) = s.get("base") {
                Search::Base(b.as_str().unwrap().into())
            } else if let Some(c) = s.get("category") {
                Search::Category(c.as_str().map(String::from))
            } else {
                Search::None
            };
            est.fractures = e["fractures"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    let mut fr = Fracture::new(f["target"].as_str().unwrap(), f["mod_id"].as_str().unwrap(), problem_of(&f["problem"]));
                    fr.query = Some(f["query"].clone()).filter(|q| !q.is_null());
                    fr.trade_url = f["trade_url"].as_str().map(String::from);
                    fr.made = serde_json::from_value(f["made"].clone()).unwrap();
                    fr
                })
                .collect();
            est
        })
        .collect()
}

fn sale_json(s: &ponder_core::Sale) -> Value {
    json!({"need": s.need, "prefix": s.prefix, "suffix": s.suffix, "any": s.any, "spare": s.spare})
}

fn sells_json(p: &Problem) -> Value {
    Value::Array(p.actions.iter().filter(|a| a.kind == "sell").map(|a| json!({"name": a.name, "credit": a.credit, "sale": sale_json(a.sale.as_ref().unwrap())})).collect())
}

fn tree_context(doc: &Value) -> (Context, Prices, Settings) {
    let table: Prices = serde_json::from_value(doc["prices"]["table"].clone()).unwrap();
    let settings = Settings { chaos_per_hour: doc["prices"]["chaos_per_hour"].as_f64().unwrap() };
    (context(settings.chaos_per_hour, offline(), temp_dir("tree")), table, settings)
}

/// The tree golden run through the Rust pipeline in the order `ladder` does it: link, then wave by wave sales and solving.
/// `checks` get the sale actions each estimate was given before its wave was solved.
fn run_tree(doc: &Value, cx: &Context, table: &Prices, settings: Settings) -> (Vec<Estimate>, Vec<Value>) {
    let mut ests = tree_estimates(doc, settings);
    link_products(&mut ests);
    let mut sales = vec![Value::Null; ests.len()];
    for n in doc["sizes"].as_array().unwrap().iter().map(|n| n.as_u64().unwrap() as usize) {
        let wave: Vec<usize> = (0..ests.len()).filter(|&i| ests[i].problem.as_ref().is_some_and(|p| p.targets.len() == n)).collect();
        for &i in &wave {
            add_sales(&mut ests, i);
            sales[i] = json!({
                "estimate": sells_json(ests[i].problem.as_ref().unwrap()),
                "fractures": ests[i].fractures.iter().map(|f| sells_json(f.problem.as_ref().unwrap())).collect::<Vec<_>>(),
            });
        }
        solve_slots(cx, slots_of(&mut ests, &wave), table, None, None);
    }
    (ests, sales)
}

fn solved_tree() -> &'static (Value, Vec<Estimate>, Vec<Value>) {
    static TREE: OnceLock<(Value, Vec<Estimate>, Vec<Value>)> = OnceLock::new();
    TREE.get_or_init(|| {
        let doc = golden("pipeline/tree.json.gz");
        let (cx, table, settings) = tree_context(&doc);
        let (ests, sales) = run_tree(&doc, &cx, &table, settings);
        (doc, ests, sales)
    })
}

#[test]
fn product_tree_covers_and_links_match_python() {
    if skip_without_data() || skip_without_goldens("pipeline") {
        return;
    }
    let doc = golden("pipeline/tree.json.gz");
    let ests = tree_estimates(&doc, Settings::default());
    let mut bad = Vec::new();
    let covers_golden = doc["covers"].as_array().unwrap();
    for c in covers_golden {
        let (i, j) = (c[0].as_u64().unwrap() as usize, c[1].as_u64().unwrap() as usize);
        let have = covers(ests[i].problem.as_ref().unwrap(), ests[j].problem.as_ref().unwrap()).map_or(Value::Null, |s| sale_json(&s));
        if have != c[2] {
            bad.push(format!("covers({i}, {j}): {have} != {}", c[2]));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    let mut linked = ests.clone();
    link_products(&mut linked);
    for (i, e) in linked.iter().enumerate() {
        let want = |k: &str| doc[k][i].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as usize).collect::<Vec<_>>();
        assert_eq!((&e.smaller, &e.bigger), (&want("smaller"), &want("bigger")), "{}", e.title);
    }
    let n = covers_golden.iter().filter(|c| !c[2].is_null()).count();
    eprintln!("tree: {} pairs ({n} covered), {} products with smaller ones", covers_golden.len(), linked.iter().filter(|e| !e.smaller.is_empty()).count());
    assert!(n > 10);
}

#[test]
fn sales_and_plans_match_python() {
    if skip_without_data() || skip_without_goldens("pipeline") {
        return;
    }
    let (doc, ests, sales) = solved_tree();
    let mut bad = Vec::new();
    let mut n_sales = 0;
    for (i, e) in ests.iter().enumerate() {
        // the sale actions: names and what an item must have exactly; their credits to the 3 figures they are rounded to
        let (have, want) = (&sales[i], &doc["sales"][i]);
        let mut d = Vec::new();
        diff_with(have, want, &e.title, &mut d, 6e-3);
        let names = |v: &Value| -> Vec<Value> { v["estimate"].as_array().unwrap().iter().map(|a| json!([a["name"], a["sale"]])).collect() };
        if names(have) != names(want) {
            d.push(format!("{}: sale actions {:?} != {:?}", e.title, names(have), names(want)));
        }
        n_sales += have["estimate"].as_array().unwrap().len();
        // the plans
        let after = &doc["after"][i];
        let mut costs = vec![(&after["estimate"], e.craft_cost(), &e.error)];
        costs.extend(after["fractures"].as_array().unwrap().iter().zip(&e.fractures).map(|(w, f)| (w, f.craft_cost(), &f.error)));
        for (k, (w, got, err)) in costs.into_iter().enumerate() {
            match (w["cost"].as_f64(), got) {
                (Some(want_cost), Some(g)) if close_to(want_cost, g, 1e-6) => {}
                (None, None) => {}
                (want_cost, g) => d.push(format!("{} #{k}: cost {g:?} != {want_cost:?} (error {err:?} / {})", e.title, w["error"])),
            }
        }
        if let Some(w) = after["cheapest"].as_f64()
            && !close_to(w, e.cheapest().unwrap_or(f64::NAN), 1e-6) {
                d.push(format!("{}: cheapest {:?} != {w}", e.title, e.cheapest()));
            }
        if !d.is_empty() {
            d.truncate(4);
            bad.push(d.join("\n    "));
        }
    }
    eprintln!("sales: {n_sales} sale actions on {} estimates", ests.len());
    assert!(bad.is_empty(), "{} estimates differ:\n{}", bad.len(), bad.join("\n"));
    assert!(n_sales > 5);
}

#[test]
fn estimates_serialise_like_python() {
    if skip_without_data() || skip_without_goldens("pipeline") {
        return;
    }
    let (doc, ests, _) = solved_tree();
    let mut ests = ests.clone();
    snapshot_products(&mut ests);
    let shown = doc["to_dict"].as_object().unwrap();
    assert!(shown.len() >= 2);
    for (i, want) in shown {
        let e = &ests[i.parse::<usize>().unwrap()];
        let have = serde_json::to_value(e).unwrap();
        assert_eq!(have.as_object().unwrap().keys().collect::<Vec<_>>(), want.as_object().unwrap().keys().collect::<Vec<_>>(), "{}: keys, in Python's order", e.title);
        // the plan's walk-through can break ties differently; the rest must agree
        let (mut have, mut want) = (have, want.clone());
        for k in ["recipe", "steps", "currency"] {
            let (h, w) = (have[k].take(), want[k].take());
            assert_eq!((h.is_array(), h.is_object()), (w.is_array(), w.is_object()), "{}: {k}", e.title);
            assert_eq!(h.as_array().map(Vec::is_empty), w.as_array().map(Vec::is_empty), "{}: {k}", e.title);
        }
        let mut d = Vec::new();
        diff_with(&have, &want, &e.title, &mut d, 1e-6);
        assert!(d.is_empty(), "{}", d.iter().take(8).cloned().collect::<Vec<_>>().join("\n"));
    }
}

fn run_ladder_summary(opts: &LadderOpts) -> (Value, std::time::Duration) {
    let cx = offline_context(1500.0);
    let started = std::time::Instant::now();
    let report = ladder(&cx, db(), None, opts, None).unwrap();
    let elapsed = started.elapsed();
    (summarise(&report), elapsed)
}

fn summarise(r: &LadderReport) -> Value {
    let sells = |e: &Estimate| e.problem.as_ref().map_or(0, |p| p.actions.iter().filter(|a| a.kind == "sell").count());
    let ests = &r.estimates;
    json!({
        "characters": r.characters, "items": r.items, "clusters": r.clusters, "archetypes": r.archetypes, "cluster_archetypes": r.cluster_archetypes,
        "estimates": ests.len(), "gear": ests.iter().filter(|e| e.kind == "gear").count(), "cluster": ests.iter().filter(|e| e.kind == "cluster").count(),
        "fractures": ests.iter().map(|e| e.fractures.len()).sum::<usize>(), "planned": ests.iter().filter(|e| e.plan.is_some()).count(),
        "planned_fractures": ests.iter().flat_map(|e| &e.fractures).filter(|f| f.plan.is_some()).count(),
        "linked": ests.iter().filter(|e| !e.smaller.is_empty()).count(), "sale_actions": ests.iter().map(sells).sum::<usize>(),
        "per_estimate": ests.iter().map(|e| json!({
            "kind": e.kind, "title": e.title, "targets": e.targets, "demand": e.demand, "cheapest": e.cheapest(), "total": e.total(),
            "craft_cost": e.craft_cost(), "fractures": e.fractures.len(), "smaller": e.smaller.len(), "bigger": e.bigger.len(), "sales": sells(e), "error": e.error,
        })).collect::<Vec<_>>(),
    })
}

/// How closely a ladder summary has to follow Python's.
struct Slack {
    /// share the solved/linked/sale counts may be apart
    counts: f64,
    /// relative difference in cheapest costs
    cost: f64,
    /// estimates that may differ (picked or linked differently): Python breaks ties among mods
    /// worn equally often by set order, which changes with each process's hash seed
    ties: usize,
    /// compare sale actions: a parallel Python run keeps, on the estimate, the sales its
    /// worker's solve dropped as singular (the plan doesn't use them); Rust and an inline
    /// Python run drop them
    sales: bool,
}

/// Compare two ladder summaries the way a drifting run can be compared: the counts of what was
/// found and built exactly (they come before any sale credit), the rest to a tolerance.
fn compare_summaries(have: &Value, want: &Value, slack: &Slack) -> Vec<String> {
    let (count_slack, cost_rel) = (slack.counts, slack.cost);
    let mut bad = Vec::new();
    for k in ["characters", "items", "clusters", "archetypes", "cluster_archetypes", "estimates", "gear", "cluster", "fractures"] {
        if have[k] != want[k] {
            bad.push(format!("{k}: {} != {}", have[k], want[k]));
        }
    }
    for k in ["planned", "planned_fractures", "linked", "sale_actions"] {
        if k == "sale_actions" && !slack.sales {
            continue;
        }
        let (h, w) = (have[k].as_f64().unwrap(), want[k].as_f64().unwrap());
        if (h - w).abs() > count_slack * w.max(10.0) {
            bad.push(format!("{k}: {h} vs {w} (more than {:.0}% apart)", count_slack * 100.0));
        }
    }
    // Two archetypes can build the same title and targets (on different bases): pair them in order.
    // targets as a set: their order follows the same tie-breaks
    let key = |e: &Value| {
        let mut t: Vec<&str> = e["targets"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
        t.sort_unstable();
        format!("{} {t:?}", e["title"])
    };
    let mut per = Vec::new();
    let mut theirs: std::collections::HashMap<String, std::collections::VecDeque<&Value>> = std::collections::HashMap::new();
    for e in want["per_estimate"].as_array().unwrap() {
        theirs.entry(key(e)).or_default().push_back(e);
    }
    let (mut off, mut compared) = (0, 0);
    for e in have["per_estimate"].as_array().unwrap() {
        let Some(w) = theirs.get_mut(&key(e)).and_then(|q| q.pop_front()) else {
            per.push(format!("{} has no Python counterpart", key(e)));
            continue;
        };
        let differs: Vec<String> = ["demand", "fractures", "smaller", "bigger"].iter().filter(|&&k| e[k] != w[k]).map(|k| format!("{k} {} != {}", e[k], w[k])).collect();
        if !differs.is_empty() {
            per.push(format!("{}: {}", key(e), differs.join(", ")));
        }
        if let (Some(h), Some(p)) = (e["cheapest"].as_f64(), w["cheapest"].as_f64()) {
            compared += 1;
            if !close_to(h, p, cost_rel) {
                off += 1;
            }
        } else if e["cheapest"].is_null() != w["cheapest"].is_null() {
            off += 1;
        }
    }
    if per.len() > slack.ties {
        bad.extend(per);
    }
    if off as f64 > 0.05 * compared.max(1) as f64 {
        bad.push(format!("{off} of {compared} cheapest costs are more than {:.0}% off", cost_rel * 100.0));
    }
    bad
}

#[test]
fn mid_sized_ladder_matches_python() {
    if skip_without_data() || skip_without_goldens("pipeline") || !repo_root().join(".cache/ladder").is_dir() {
        return;
    }
    let doc = golden("pipeline/tree.json.gz");
    let o = &doc["options"];
    let opts = LadderOpts {
        gear_top: o["gear_top"].as_u64().unwrap() as usize,
        per_group: o["per_group"].as_u64().unwrap() as usize,
        niche: o["niche"].as_u64().unwrap() as usize,
        cluster_top: o["cluster_top"].as_u64().unwrap() as usize,
        min_characters: o["min_characters"].as_u64().unwrap() as usize,
        ..LadderOpts::default()
    };
    let (have, elapsed) = run_ladder_summary(&opts);
    eprintln!("mid ladder: {} estimates in {elapsed:?} (Python: {:.1} s with a warm plan cache)", have["estimates"], doc["seconds"].as_f64().unwrap());
    let bad = compare_summaries(&have, &doc["summary"], &Slack { counts: 0.05, cost: 0.05, ties: 0, sales: true }); // an inline (workers=1) Python run
    assert!(bad.is_empty(), "{}", bad.iter().take(15).cloned().collect::<Vec<_>>().join("\n"));
}

/// The default `ladder()` on the cached ladder, with an empty plan cache (the timing the rewrite is for).
/// `cargo test -p ponder --release --test pipeline -- --ignored --nocapture full_ladder`
#[test]
#[ignore = "a few minutes in debug builds; run with --release --ignored"]
fn full_ladder_matches_python() {
    if skip_without_data() || skip_without_goldens("pipeline") || !repo_root().join(".cache/ladder").is_dir() {
        return;
    }
    let want = golden("pipeline/ladder_full.json.gz");
    let cx = offline_context(want["chaos_per_hour"].as_f64().unwrap()).with_plans(PlanCache::new(temp_dir("full-plans")));
    let started = std::time::Instant::now();
    let report = ladder(&cx, db(), None, &LadderOpts::default(), Some(&|m: &str| eprintln!("  {m}"))).unwrap();
    let elapsed = started.elapsed();
    eprintln!("full ladder: {} estimates in {elapsed:?} with an empty plan cache (Python: {:.0} s cold)", report.estimates.len(), want["seconds"].as_f64().unwrap_or(0.0));
    let again = std::time::Instant::now();
    ladder(&cx, db(), None, &LadderOpts::default(), None).unwrap();
    eprintln!("full ladder, plans cached: {:?}", again.elapsed());
    let bad = compare_summaries(&summarise(&report), &want, &Slack { counts: 0.05, cost: 0.10, ties: 5, sales: false }); // a parallel one
    assert!(bad.is_empty(), "{}", bad.iter().take(15).cloned().collect::<Vec<_>>().join("\n"));
}

// -- trade ------------------------------------------------------------------------------------

/// The trade site as the goldens/pipeline trade answers were recorded: a search's answer is a function of the
/// query's text, so what was searched and in what order shows in the answers.
fn fake_trade(calls: Arc<Mutex<Vec<String>>>) -> Arc<dyn Transport> {
    Arc::new(move |req: &Request| {
        if req.url.contains("/api/trade/search/") {
            let h = ponder_net::pyjson::sha1_hex(req.body.as_deref().unwrap());
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

#[test]
fn price_on_trade_follows_python() {
    if skip_without_data() || skip_without_goldens("pipeline") {
        return;
    }
    let (doc, solved, _) = solved_tree();
    let want = golden("pipeline/trade.json.gz");
    let shared = Arc::new(Mutex::new(Vec::new()));
    let cx = context(1500.0, fake_trade(shared.clone()), temp_dir("trade"));
    let table: Prices = serde_json::from_value(doc["prices"]["table"].clone()).unwrap();
    let mut ests = solved.clone();
    let mut order: Vec<usize> = (0..ests.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(ests[i].demand));
    let said = Mutex::new(Vec::new());
    let used = price_on_trade(&cx, &mut ests, &order, &table, "Standard", want["budget"].as_u64().unwrap() as usize, Some(&|m: &str| said.lock().unwrap().push(m.to_string())));
    let calls = shared.lock().unwrap().clone(); // not held: the second pass below searches again
    let want_calls = strings(&want["calls"]);
    assert_eq!((used, calls.len()), (want["used"].as_u64().unwrap() as usize, want_calls.len()));
    assert_eq!(calls, want_calls, "the same searches in the same order");
    assert!(said.lock().unwrap().iter().any(|m| m.contains("budget of")), "{:?}", said.lock().unwrap().last());
    let pair = |v: &Value| v.as_array().map(|a| (a[0].as_f64().unwrap(), a[1].as_i64().unwrap()));
    let text = |v: &Value| v.as_str().map(String::from);
    for (e, w) in ests.iter().zip(want["estimates"].as_array().unwrap()) {
        assert_eq!((e.market, &e.market_error), (pair(&w["market"]), &text(&w["market_error"])), "{} market", e.title);
        assert_eq!((e.base_price, &e.base_error, e.base_source.as_str()), (pair(&w["base_price"]), &text(&w["base_error"]), w["base_source"].as_str().unwrap()), "{} base", e.title);
        for (f, wf) in e.fractures.iter().zip(w["fractures"].as_array().unwrap()) {
            assert_eq!((f.base_price, &f.base_error), (pair(&wf["base_price"]), &text(&wf["base_error"])), "{} / {}", e.title, f.target);
        }
    }
    // a second pass, with budget to spare, only searches what the first had no budget for:
    // everything already asked is answered from the cache
    shared.lock().unwrap().clear();
    let again = price_on_trade(&cx, &mut ests, &order, &table, "Standard", 1000, None);
    let repeated = shared.lock().unwrap().iter().filter(|h| calls.contains(h)).count();
    assert_eq!(repeated, 0, "{repeated} of {again} searches asked again");
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
        (Arc::new(|_: &Request| Ok(Response::json(200, &json!({"id": "x", "total": 0, "result": []})))) as Arc<dyn Transport>, "no listings"),
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
    let priced: Vec<&str> = e.fractures.iter().filter(|f| f.base_price.is_some() || f.base_error.is_some()).map(|f| f.target.as_str()).collect();
    assert_eq!(priced, ["f1", "f2"], "the two cheapest to finish");
    // a budget stops it
    let mut e2 = e.clone();
    e2.fractures.iter_mut().for_each(|f| (f.base_price, f.base_error) = (None, None));
    let cx2 = context(0.0, fake_trade(calls), temp_dir("fractures2"));
    assert_eq!(price_fractures(&cx2, std::slice::from_mut(&mut e2), &table, "Standard", 4, 1, None), 1);
}

// -- the product tree, by hand ----------------------------------------------------------------

fn target(name: &str, side: ponder_core::Side, good: &[&str], groups: &[&str]) -> ponder_core::Target {
    ponder_core::Target { name: name.into(), side, good: good.iter().map(|s| s.to_string()).collect(), groups: groups.iter().map(|s| s.to_string()).collect(), notable: false }
}

fn tiny(targets: Vec<ponder_core::Target>, filler: &[&str], filler_need: usize, filler_sides: Option<(usize, usize)>) -> Problem {
    let mut p = Problem::new(vec![], targets, 0, vec![], ponder_core::Kind::Gear);
    p.need = p.targets.len();
    p.filler = filler.iter().map(|s| s.to_string()).collect();
    p.filler_need = filler_need;
    p.filler_sides = filler_sides;
    p
}

#[test]
fn covers_needs_every_target_at_the_same_tier_or_better() {
    use ponder_core::Side::{Prefix, Suffix};
    let life = |tiers: &[&str]| target("life", Prefix, tiers, &["Life"]);
    let es = target("es", Prefix, &["es1", "es2"], &["ES"]);
    let big = tiny(vec![life(&["life1"]), es.clone()], &[], 0, None);
    // life T1 only is wanted by the small one at T1 or T2: big's T1 qualifies
    let small = tiny(vec![life(&["life1", "life2"])], &[], 0, None);
    let sale = covers(&big, &small).expect("big makes what small wants");
    assert_eq!((sale.need.iter().copied().collect::<Vec<_>>(), sale.prefix, sale.suffix, sale.any), (vec![0], 0, 0, 0));
    // a worse tier on big does not cover a better one wanted
    let strict = tiny(vec![life(&["life1"])], &[], 0, None);
    let loose_big = tiny(vec![life(&["life1", "life2"])], &[], 0, None);
    assert!(covers(&loose_big, &strict).is_none());
    // a fractured mod stands in for the target
    let mut with_fixed = tiny(vec![es.clone()], &[], 0, None);
    with_fixed.fixed.push(ponder_core::Mod::new("life1", Prefix, 0.0, &["Life"], false, "life", &[]));
    let sale = covers(&with_fixed, &small).expect("fractured life covers it");
    assert!(sale.need.is_empty());
    // two targets of small cannot both use one of big's
    let two = tiny(vec![life(&["life1", "life2"]), life(&["life1", "life2"])], &[], 0, None);
    assert!(covers(&big, &two).is_none());
    // nothing wanted covers nothing
    assert!(covers(&big, &tiny(vec![], &[], 0, None)).is_none());
    // filler: big's spare target that small counts as filler, and big's own filler slots
    let res = target("res", Suffix, &["fire1"], &["Fire"]);
    let filler_small = tiny(vec![life(&["life1", "life2"])], &["fire1"], 1, Some((0, 1)));
    let big_with_res = tiny(vec![life(&["life1"]), res.clone()], &[], 0, None);
    let sale = covers(&big_with_res, &filler_small).expect("the resistance target is the filler");
    assert_eq!((sale.spare.iter().copied().collect::<Vec<_>>(), sale.suffix, sale.any), (vec![1], 1, 1));
    assert!(covers(&big, &filler_small).is_none(), "no suffix room");
    let big_with_slots = tiny(vec![life(&["life1"])], &[], 2, Some((0, 2)));
    assert!(covers(&big_with_slots, &filler_small).is_some());
    let any_mix = tiny(vec![life(&["life1"])], &[], 1, None);
    let mix_small = tiny(vec![life(&["life1", "life2"])], &["fire1"], 1, None);
    assert!(covers(&any_mix, &mix_small).is_some(), "an unsided filler slot is room for any");
}

#[test]
fn link_products_and_sales_on_a_made_up_group() {
    use ponder_core::{MAGIC, RARE, Side::Prefix};
    let t = |name: &str, good: &[&str]| target(name, Prefix, good, &[name]);
    let mut big = Estimate::new("gear", "Ring", vec!["a".into(), "b".into()]);
    big.problem = Some(tiny(vec![t("A (T1)", &["a1"]), t("B (T1)", &["b1"])], &[], 0, None));
    let mut small = Estimate::new("gear", "Ring", vec!["A (T1)".into()]);
    small.problem = Some(tiny(vec![t("A (T1)", &["a1", "a2"])], &[], 0, None));
    let mut other = Estimate::new("gear", "Boots", vec!["A (T1)".into()]);
    other.problem = Some(tiny(vec![t("A (T1)", &["a1", "a2"])], &[], 0, None));
    let mut broken = Estimate::new("gear", "Ring", vec![]);
    broken.problem = Some(tiny(vec![], &[], 0, None));
    broken.error = Some("no".into());
    let mut ests = vec![big, small, other, broken];
    link_products(&mut ests);
    assert_eq!((&ests[0].smaller, &ests[0].bigger, &ests[1].smaller, &ests[1].bigger), (&vec![1], &vec![], &vec![], &vec![0]));
    assert!(ests[2].smaller.is_empty() && ests[2].bigger.is_empty(), "another base group is another tree");
    // the small one is worth 100 chaos; the big one's base costs 10 to buy again
    ests[1].plan = Some(Plan { cost: 100.0, ..Plan::default() });
    ests[0].base_price = Some((10.0, 4));
    add_sales(&mut ests, 0);
    let sells: Vec<_> = ests[0].problem.as_ref().unwrap().actions.iter().filter(|a| a.kind == "sell").collect();
    assert_eq!(sells.len(), 1);
    assert_eq!((sells[0].name.as_str(), sells[0].credit, sells[0].from_rarity.clone()), ("sell as A T1", 60.0, vec![MAGIC, RARE]));
    assert_eq!(sells[0].sale.as_ref().unwrap().need.iter().copied().collect::<Vec<_>>(), vec![0]);
    // adding again replaces the sales instead of piling them up
    add_sales(&mut ests, 0);
    assert_eq!(ests[0].problem.as_ref().unwrap().actions.iter().filter(|a| a.kind == "sell").count(), 1);
    // below MIN_SALE nothing is worth listing; a solved estimate is left alone
    ests[1].plan = Some(Plan { cost: 20.0, ..Plan::default() });
    add_sales(&mut ests, 0);
    assert_eq!(ests[0].problem.as_ref().unwrap().actions.iter().filter(|a| a.kind == "sell").count(), 0);
    ests[1].plan = Some(Plan { cost: 100.0, ..Plan::default() });
    ests[0].plan = Some(Plan::default());
    add_sales(&mut ests, 0);
    assert_eq!(ests[0].problem.as_ref().unwrap().actions.iter().filter(|a| a.kind == "sell").count(), 0);
    // a fracture is restarted from the cost of making its base
    ests[0].plan = None;
    let mut f = Fracture::new("A (T1)", "a1", tiny(vec![t("B (T1)", &["b1"])], &[], 0, None));
    f.problem.as_mut().unwrap().fixed.push(ponder_core::Mod::new("a1", Prefix, 0.0, &["A (T1)"], false, "a", &[]));
    f.made = Some(ponder_core::SelfFracture { target: 0, action: "x".into(), max_mods: 5, hit: 0.1, fracture_chance: 0.1, cost: 30.0, hours: 0.0 });
    ests[0].fractures.push(f);
    add_sales(&mut ests, 0);
    let credit = |a: &ponder_core::Action| a.credit;
    let own: Vec<f64> = ests[0].problem.as_ref().unwrap().actions.iter().filter(|a| a.kind == "sell").map(credit).collect();
    let frac: Vec<f64> = ests[0].fractures[0].problem.as_ref().unwrap().actions.iter().filter(|a| a.kind == "sell").map(credit).collect();
    assert_eq!((own, frac), (vec![60.0], vec![40.0]));
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
            return Ok(Response::json(200, &json!({"id": "q", "total": listings.len(), "result": (0..listings.len()).map(|i| format!("id{i}")).collect::<Vec<_>>()})));
        }
        Ok(Response::json(200, &json!({"result": listings})))
    })
}

#[test]
fn finish_a_real_item_and_find_half_done_ones() {
    if skip_without_data() {
        return;
    }
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
    let have = parse_item("Rarity: RARE\nGale Loop\nBone Ring\nItem Level: 84\nImplicits: 0\n+105 to maximum Life\n+30% to Fire Resistance").unwrap();
    let f = finish(&cx, db(), &est, &db().match_item(&have), &table, true);
    assert!(f.error.is_none(), "{:?}", f.error);
    assert_eq!(f.have, est.targets.iter().filter(|t| t.contains("Life")).cloned().collect::<Vec<_>>());
    assert!(f.craft_cost.unwrap() <= est.craft_cost().unwrap() + table["scour"] + 1e-6); // never worse than scouring it
    assert!(f.steps()[0].have.join(" ").contains("Life"), "the recipe starts from the item");
    let both = parse_item("Rarity: RARE\nX\nBone Ring\nItem Level: 84\nImplicits: 0\n+105 to maximum Life\n+40 to maximum Energy Shield").unwrap();
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
    assert_eq!((shown["seller"].as_str(), shown["whisper"].as_str(), shown["price"].as_f64()), (Some("seller"), Some("@seller hi"), Some(10.0)));
    assert!(shown["steps"].as_array().is_some_and(|s| !s.is_empty()), "the listing was finished with a plan");
    assert_eq!(serde_json::to_value(&est).unwrap()["half_done_for_sale"].as_array().unwrap().len(), starts.len());
}

#[test]
fn finishes_rank_products_by_what_they_save() {
    if skip_without_data() {
        return;
    }
    let cx = offline_context(0.0);
    let (table, _) = cx.prices.load().unwrap();
    let make = |ids: &[&str]| {
        let explicit: Vec<Value> = ids.iter().map(|i| json!({"id": i})).collect();
        let m = db().match_ids("Bone Ring", &IdMods::from_json(&json!({ "explicit": explicit })), Some(84), &Default::default(), "RARE", "", &Default::default());
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

#[test]
fn finish_matches_python_goldens() {
    if skip_without_data() || skip_without_goldens("pipeline") {
        return;
    }
    let want = golden("pipeline/finish.json.gz");
    let cx = offline_context(want["prices"]["chaos_per_hour"].as_f64().unwrap());
    let table: Prices = serde_json::from_value(want["prices"]["table"].clone()).unwrap();
    let (mut items, mut ests): (Vec<(String, ItemMatch)>, Vec<Estimate>) = (Vec::new(), Vec::new());
    let mut keys = std::collections::HashSet::new();
    for name in strings(&want["builds"]) {
        for pob_item in real_build(&name).items {
            let item = parse_item(&pob_item.text).unwrap();
            if item.rarity != "RARE" {
                continue;
            }
            let m = db().match_item(&item);
            if let Some(est) = estimate_item(&cx, db(), &m, &table, "Standard", &GearOpts::default()).unwrap()
                && !est.targets.is_empty() && keys.insert(est.key.clone()) {
                    ests.push(est);
                }
            items.push((name.clone(), m));
        }
    }
    let want_ests = want["estimates"].as_array().unwrap();
    assert_eq!(ests.len(), want_ests.len());
    for (e, w) in ests.iter().zip(want_ests) {
        assert_eq!((e.title.as_str(), json!(e.targets), e.base.as_str()), (w["title"].as_str().unwrap(), w["targets"].clone(), w["base"].as_str().unwrap()));
        match (e.craft_cost(), w["craft_cost"].as_f64()) {
            (Some(a), Some(b)) => assert!(close_to(a, b, 1e-6), "{}: {a} != {b}", e.title),
            (a, b) => assert_eq!(a.is_some(), b.is_some(), "{}: {a:?} vs {b:?} ({:?})", e.title, e.error),
        }
    }
    let rows = want["items"].as_array().unwrap();
    assert_eq!(items.len(), rows.len());
    let (mut finished, mut bad) = (0, Vec::new());
    for ((build, m), row) in items.iter().zip(rows) {
        assert_eq!((build.as_str(), m.item.base.as_str()), (row["build"].as_str().unwrap(), row["base"].as_str().unwrap()));
        for (j, e) in ests.iter().enumerate().filter(|(_, e)| e.kind == "gear" && e.plan.is_some()) {
            let f = finish(&cx, db(), e, m, &table, false);
            let name = format!("{} -> {}", m.item.base, e.title);
            let Some(w) = row["finish"].get(j.to_string()) else {
                if f.craft_cost.is_some() {
                    bad.push(format!("{name}: finished, Python says it can't"));
                }
                continue;
            };
            finished += 1;
            let mut d = Vec::new();
            let have = json!({"error": f.error, "have": f.have, "craft_cost": f.craft_cost, "saves": f.saves(), "worth": f.worth(),
                "state": match (&f.problem, &f.state) { (Some(p), Some(s)) => Some(ponder_core::describe(p, s)), _ => None }});
            let mut w = w.clone();
            w.as_object_mut().unwrap().remove("plan");
            diff_with(&have, &w, &name, &mut d, 1e-6);
            bad.extend(d);
        }
        // the best few, with plans
        let top = finishes(&cx, db(), m, &ests, &table, 3);
        let want_top = row["finishes"].as_array().unwrap();
        if top.len() != want_top.len() {
            bad.push(format!("{}: {} finishes != {}", m.item.base, top.len(), want_top.len()));
        }
        // Finishes saving the same (up to float noise; often many products are one bench craft
        // short) are ranked by Rust cheapest first and by Python in float-noise order: at each
        // rank the saving must match, and the details when both picked the same product.
        for (f, w) in top.iter().zip(want_top) {
            let have = json!({"product": f.product, "targets": f.targets, "craft_cost": f.craft_cost, "saves": f.saves(), "have": f.have,
                "plan_cost": f.plan.as_ref().map(|p| p.cost), "total": f.total()});
            let mut d = Vec::new();
            // `saves` is a difference of two costs: compare it absolutely
            diff_with(&have["saves"], &w["saves"], &format!("{}.saves", m.item.base), &mut d, 1e-3);
            if have["targets"] == w["targets"] {
                let (mut have, mut w) = (have.clone(), w.clone());
                have.as_object_mut().unwrap().remove("saves");
                w.as_object_mut().unwrap().remove("saves");
                diff_with(&have, &w, &m.item.base, &mut d, 1e-6);
            }
            bad.extend(d);
        }
    }
    eprintln!("finish: {} items against {} estimates, {finished} finishes compared", items.len(), ests.len());
    assert!(bad.is_empty(), "{} differ:\n{}", bad.len(), bad.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
    assert!(finished > 20);
}

// -- plan cache, runs and sources -------------------------------------------------------------

#[test]
fn plans_are_cached_by_problem_and_prices() {
    if skip_without_data() {
        return;
    }
    let dir = temp_dir("plans");
    let cx = offline_context(0.0).with_plans(PlanCache::new(&dir));
    let table = cheap_table();
    let notables = vec!["Vast Power".to_string(), "Assert Dominance".to_string()];
    let opts = ClusterOpts { solve: false, ..ClusterOpts::default() };
    let mut a = estimate_cluster(&cx, "medium", "area damage", &notables, &table, "Standard", &opts);
    let key = plan_key(a.problem.as_ref().unwrap(), &table, &cx.settings);
    assert_eq!(key.len(), 40);
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
    if skip_without_data() {
        return;
    }
    let cx = offline_context(0.0).with_plans(PlanCache::new(temp_dir("solve-all")));
    let table = cheap_table();
    let opts = ClusterOpts { solve: false, ..ClusterOpts::default() };
    let sets: [&[&str]; 4] = [&["Vast Power"], &["Assert Dominance"], &["Magnifier"], &["Vast Power", "Assert Dominance"]];
    let mut ests: Vec<Estimate> = sets
        .iter()
        .map(|n| estimate_cluster(&cx, "medium", "area damage", &n.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &table, "Standard", &opts))
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
    hopeless.problem.as_mut().unwrap().actions.clear();
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
    let sources = [list.to_string_lossy().into_owned(), not_links.to_string_lossy().into_owned(), "https://pobb.in/ddd".into(), "/no/such/file.txt".into()];
    let expanded = expand_sources(&sources);
    assert_eq!(expanded[..2], ["https://pobb.in/aaa", "https://pobb.in/bbb"]);
    assert_eq!(expanded[2..], sources[1..]);
    let build = ponder_game::pob::Build { class_name: "Witch".into(), ascendancy: "Occultist".into(), level: 95, main_skill: Some("Winter Orb".into()), items: vec![] };
    assert_eq!(build_name(&build), "Occultist lvl 95: Winter Orb");
    let bare = ponder_game::pob::Build { ascendancy: String::new(), main_skill: None, ..build };
    assert_eq!(build_name(&bare), "Witch lvl 95: ?");
}

#[test]
fn the_league_is_the_newest_or_standard() {
    let cx = offline_context(0.0);
    assert_eq!(current_league(&cx), "Standard"); // the fake poe.ninja lists only Standard
    let down = Context::new(Settings::default(), PriceClient::with(offline(), temp_dir("league"), fixed_clock()), Trade::with(offline(), temp_dir("league"), fixed_clock()), Ladder::with(offline(), temp_dir("league"), fixed_clock()), offline());
    assert_eq!(current_league(&down), "Standard");
    let up = Context::new(
        Settings::default(),
        PriceClient::with(Arc::new(|_: &Request| Ok(Response::json(200, &json!([{"id": "Allflame"}, {"id": "Standard"}])))), temp_dir("league"), fixed_clock()),
        Trade::with(offline(), temp_dir("league"), fixed_clock()),
        Ladder::with(offline(), temp_dir("league"), fixed_clock()),
        offline(),
    );
    assert_eq!(current_league(&up), "Allflame");
}

#[test]
fn run_turns_builds_into_a_report() {
    if skip_without_data() {
        return;
    }
    // The builds come from a share link: the fake share site serves a fixture's code.
    let code = std::fs::read_to_string(repo_root().join("fixtures/real/occultist_winter_orb.txt")).unwrap();
    let share: Arc<dyn Transport> = Arc::new(move |req: &Request| {
        assert_eq!(req.url, "https://pobb.in/abc/raw", "only the raw build is fetched");
        Ok(Response::new(200, code.clone()))
    });
    let calls = Arc::new(Mutex::new(Vec::new()));
    let cx = context_with(1500.0, fake_trade(calls.clone()), temp_dir("run"), share);
    let sources = vec!["https://pobb.in/abc".to_string(), "fixtures/real/no_such_build.txt".to_string(), repo_root().join("fixtures/real/marauder_rf_leveling.txt").to_string_lossy().into_owned()];
    let opts = RunOpts { fractures: true, trade_lookup: true, trade_budget: 6, workers: Some(2), ..RunOpts::default() };
    let progress = Mutex::new(Vec::new());
    let report = run(&cx, db(), &sources, &opts, Some(&|m: &str| progress.lock().unwrap().push(m.to_string()))).unwrap();
    assert_eq!(report.builds.len(), 2);
    assert_eq!(report.builds[0].name, "Occultist lvl 97: Winter Orb");
    assert!(report.builds.iter().all(|b| b.rares > 3));
    assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
    assert!(report.errors[0].starts_with("fixtures/real/no_such_build.txt"));
    assert_eq!(report.league, "Standard");
    assert!(report.estimates.len() >= 6);
    assert!(report.estimates.iter().filter(|e| e.plan.is_some()).count() >= 5);
    assert!(report.estimates.iter().all(|e| !e.builds.is_empty() && e.settings.chaos_per_hour == 1500.0));
    assert!(report.estimates.iter().any(|e| !e.fractures.is_empty() && e.fractures.iter().all(|f| f.plan.is_some() || f.error.is_some())));
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
fn the_json_has_pythons_keys() {
    let e = estimate("x", 10.0, Some(100.0));
    let keys: Vec<String> = e.to_dict().as_object().unwrap().keys().cloned().collect();
    let python = [
        "kind", "title", "base", "build_base", "bases_in_group", "mods", "targets", "slot", "builds", "item_level", "craft_cost", "bench", "base_price", "base_source", "base_url",
        "total", "market", "listed", "margin", "currency", "clicking_hours", "currency_chaos", "recipe", "steps", "fractures", "not_crafted", "trade_url", "recomb", "recomb_total",
        "demand", "skills", "hole", "fracture_worth", "smaller_products", "bigger_products", "half_done_for_sale", "error",
    ];
    assert_eq!(keys, python);
    let mut f = Fracture::new("t", "m", tiny(vec![], &[], 0, None));
    f.base_price = Some((12.0, 3));
    f.plan = Some(Plan { cost: 8.0, ..Plan::default() });
    f.made = Some(ponder_core::SelfFracture { target: 0, action: "scour+alch".into(), max_mods: 5, hit: 0.5, fracture_chance: 0.25, cost: 9.0, hours: 0.1 });
    let d = f.to_dict();
    assert_eq!(d.as_object().unwrap().keys().collect::<Vec<_>>(), ["target", "craft_cost", "base_price", "base_listed", "self_made", "total", "trade_url", "error"]);
    assert_eq!((d["total"].as_f64(), d["base_listed"].as_i64(), d["self_made"]["action"].as_str(), f.self_made()), (Some(17.0), Some(3), Some("scour+alch"), true));
    assert_eq!(f.base_cost(), Some(9.0));
}

#[test]
fn estimate_arithmetic() {
    let mut e = estimate("x", 100.0, Some(500.0));
    e.bench = vec![("Life".into(), 5.0)];
    e.base_price = Some((20.0, 3));
    e.demand = 10;
    assert_eq!((e.total(), e.margin(), e.cheapest()), (Some(125.0), Some(375.0), Some(125.0)));
    assert_eq!(e.hole(), Some(375.0 * 10.0 / 10.0)); // listed 5, plus 5
    // a fractured base without a price leaves the total incomplete
    e.fixed = vec!["Life".into()];
    e.base_price = None;
    assert!(e.needs_base_price() && e.margin().is_none() && e.hole().is_none());
    // a priced, cheaper fracture is the cheapest way
    let mut f = Fracture::new("a", "m", tiny(vec![], &[], 0, None));
    f.plan = Some(Plan { cost: 40.0, ..Plan::default() });
    f.base_price = Some((30.0, 2));
    e.fractures.push(f);
    assert_eq!((e.cheapest(), e.best_fracture().map(|f| f.target.as_str())), (Some(70.0), Some("a")));
    assert_eq!(e.fracture_worth().map(|(w, f)| (w, f.target.clone())), Some((105.0 - 40.0, "a".to_string())));
    // a recombinator tree competes too, bench included
    e.recomb = Some(RecombInfo { cost: Some(50.0), json: json!({"cost": 50}) });
    assert_eq!((e.recomb_total(), e.cheapest()), (Some(55.0), Some(55.0)));
    assert_eq!(e.to_dict()["recomb"], json!({"cost": 50}));
    // no plan, no total
    let bare = Estimate::new("gear", "bare", vec![]);
    assert!(bare.total().is_none() && bare.cheapest().is_none() && bare.craft_cost().is_none() && bare.steps().is_empty() && bare.usage().is_empty());
}
