//! Problem building against the Python goldens (scripts/golden/problems.py), plus the pieces of
//! tests/test_craft.py, test_pipeline.py and test_recomb.py that touch actions, gear and cluster.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Once, OnceLock};
use std::time::Instant;

use ponder::gear::{self, BuildOpts, Pricing};
use ponder::{actions, cluster};
use ponder_core::*;
use ponder_game::data;
use ponder_game::item::ModLine;
use ponder_game::mods::{IdMods, ItemMatch, ModDB};
use rayon::prelude::*;
use serde_json::{Value, json};

// -- helpers ----------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    match std::env::var_os("PONDER_REPO") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().unwrap(),
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

fn db() -> &'static ModDB {
    static DB: OnceLock<ModDB> = OnceLock::new();
    DB.get_or_init(|| ModDB::load().expect("repoe data"))
}

fn golden(path: &str) -> Value {
    let file = std::fs::File::open(repo_root().join("rust/goldens").join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut text = String::new();
    flate2::read::GzDecoder::new(file).read_to_string(&mut text).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn table() -> &'static Prices {
    static T: OnceLock<Prices> = OnceLock::new();
    T.get_or_init(|| serde_json::from_value(golden("problems/prices.json.gz")).unwrap())
}

const SETTINGS: Settings = Settings { chaos_per_hour: 1500.0 };

fn pricing() -> Pricing<'static> {
    Pricing::new(table(), &SETTINGS)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9f64.max(1e-9 * a.abs().max(b.abs()))
}

/// Differences between two JSON documents, numbers compared to a relative 1e-9.
fn diff(a: &Value, b: &Value, path: &str, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            if !close(x.as_f64().unwrap(), y.as_f64().unwrap()) {
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

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
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

/// A tag with two influence suffixes ("amulet_shaper_adjudicator") is spelled in Python's set
/// iteration order, which varies per run; no mod spawns on one, so they are left out of comparisons.
fn single_influence_tags(tags: &BTreeSet<String>) -> Vec<&String> {
    const INF: [&str; 6] = ["shaper", "elder", "crusader", "eyrie", "basilisk", "adjudicator"];
    let multi = |t: &str| {
        let parts: Vec<&str> = t.rsplitn(3, '_').collect();
        parts.len() == 3 && INF.contains(&parts[0]) && INF.contains(&parts[1])
    };
    tags.iter().filter(|t| !multi(t)).collect()
}

fn match_json(m: &ItemMatch) -> Value {
    json!({
        "base": m.item.base, "base_id": m.base_id, "item_class": m.item_class, "item_level": m.item.item_level,
        "influences": m.item.influences, "tags": single_influence_tags(&m.tags),
        "mods": m.mods.iter().map(|x| json!({"mod_id": x.mod_id, "kind": x.kind, "family": x.family, "source": x.source, "tier": x.tier, "fractured": x.fractured()})).collect::<Vec<_>>(),
    })
}

fn built_json(gp: &gear::GearProblem) -> Value {
    json!({
        "problem": serde_json::to_value(&gp.problem).unwrap(),
        "item_level": gp.item_level,
        "skipped": gp.skipped,
        "bench": gp.bench.iter().map(|(l, c)| json!([l, c])).collect::<Vec<_>>(),
        "rows": gp.rows.iter().map(|r| json!([r.role, r.side.name(), r.text, r.tier])).collect::<Vec<_>>(),
        "bench_ids": gp.bench_ids,
        "filler_families": gp.filler_families,
    })
}

/// Fields of a serialised Problem that carry no information for comparing builds.
fn tidy(mut v: Value) -> Value {
    if let Some(o) = v.as_object_mut() {
        // The golden writes a missing begin as null; so do we. Nothing to drop today.
        o.entry("begin").or_insert(Value::Null);
    }
    v
}

fn solved(p: &Problem) -> Result<f64, SolveError> {
    solve(&mut p.clone(), table(), &SETTINGS).map(|plan| plan.cost)
}

// -- goldens ----------------------------------------------------------------------------

#[test]
fn gear_problems_match_python_goldens() {
    if skip_without_data() {
        return;
    }
    let goldens = golden("problems/gear.json.gz");
    let rows = goldens.as_array().unwrap();
    assert_eq!(rows.len(), 120);
    // Load the data (ModDB, fossils, ...) first so the timing below is the building alone.
    let warm = Instant::now();
    let first = rows.iter().find(|r| !r["match"].is_null()).unwrap();
    gear::build(db(), &match_of(&first["match"]), pricing(), BuildOpts::default()).unwrap();
    eprintln!("gear: data load and first build {:?}", warm.elapsed());
    let started = Instant::now();
    // Build everything in parallel (like the pipeline will); compare afterwards.
    let built: Vec<Option<(ItemMatch, anyhow::Result<gear::GearProblem>)>> = rows
        .par_iter()
        .map(|row| {
            if row["match"].is_null() {
                return None;
            }
            let m = match_of(&row["match"]);
            let gp = gear::build(db(), &m, pricing(), BuildOpts::default());
            Some((m, gp))
        })
        .collect();
    let elapsed = started.elapsed();
    let (mut same, mut total, mut skipped) = (0, 0, 0);
    let mut bad: Vec<String> = Vec::new();
    for (row, got) in rows.iter().zip(&built) {
        let Some((m, gp)) = got else {
            skipped += 1;
            continue;
        };
        total += 1;
        let name = format!("{} {:?}", row["group"].as_str().unwrap(), row["families"]);
        let mut d = Vec::new();
        // The match we rebuild must be the one Python had.
        let mut want_match = row["match"].clone();
        want_match.as_object_mut().unwrap().remove("finish");
        let mut want_tags = want_match["tags"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect::<BTreeSet<_>>();
        want_tags.retain(|t| single_influence_tags(&BTreeSet::from([t.clone()])).len() == 1);
        want_match["tags"] = json!(want_tags);
        diff(&match_json(m), &want_match, "match", &mut d);
        let gp = gp.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut want = row["build"].clone();
        want["problem"] = tidy(want["problem"].clone());
        let mut have = built_json(gp);
        have["problem"] = tidy(have["problem"].clone());
        diff(&have, &want, "build", &mut d);
        // The estimate's base group.
        let grp = gear::group(db(), m);
        let est = &row["estimate"];
        if est["title"] != json!(grp.label) || est["base"] != json!(grp.best()) || est["bases"] != json!(grp.bases.len()) {
            d.push(format!("group {} / {} / {} != {} / {} / {}", grp.label, grp.best(), grp.bases.len(), est["title"], est["base"], est["bases"]));
        }
        if d.is_empty() {
            same += 1;
        } else {
            d.truncate(6);
            bad.push(format!("{name}:\n    {}", d.join("\n    ")));
        }
    }
    eprintln!("gear: built {total} problems in {elapsed:?} ({skipped} archetypes without a match); {same}/{total} identical to Python");
    assert!(bad.is_empty(), "{} of {total} differ:\n{}", bad.len(), bad.join("\n"));
    assert_eq!((total, skipped), (113, 7));
}

#[test]
fn built_gear_problems_solve_like_the_golden_ones() {
    if skip_without_data() {
        return;
    }
    let goldens = golden("problems/gear.json.gz");
    type Costs = (Result<f64, SolveError>, Result<f64, SolveError>);
    let costs: Vec<(String, Option<Costs>)> = goldens
        .as_array()
        .unwrap()
        .par_iter()
        .map(|row| {
            let name = row["group"].as_str().unwrap().to_string();
            if row["match"].is_null() {
                return (name, None);
            }
            let theirs: Problem = serde_json::from_value(row["build"]["problem"].clone()).unwrap();
            let ours = gear::build(db(), &match_of(&row["match"]), pricing(), BuildOpts::default()).unwrap();
            (name, Some((solved(&ours.problem), solved(&theirs))))
        })
        .collect();
    let mut bad = Vec::new();
    let mut n = 0;
    for (name, pair) in costs {
        let Some((ours, theirs)) = pair else { continue };
        n += 1;
        match (ours, theirs) {
            (Ok(a), Ok(b)) if (a - b).abs() <= 1e-7 * a.abs().max(b.abs()).max(1.0) => {}
            (Err(a), Err(b)) if a == b => {}
            (a, b) => bad.push(format!("{name}: {a:?} != {b:?}")),
        }
    }
    assert!(bad.is_empty(), "{} of {n} differ:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn cluster_problems_match_python_goldens() {
    if skip_without_data() {
        return;
    }
    let goldens = golden("problems/clusters.json.gz");
    let rows = goldens.as_array().unwrap();
    assert_eq!(rows.len(), 60);
    let started = Instant::now();
    let built: Vec<anyhow::Result<Problem>> = rows
        .par_iter()
        .map(|row| {
            let (size, enchant) = (row["size"].as_str().unwrap(), row["enchant"].as_str().unwrap());
            let found = cluster::find_enchant(size, enchant)?;
            cluster::problem(size, &found.name, &strings(&row["notables"]), None, 84, &[])
        })
        .collect();
    let elapsed = started.elapsed();
    let (mut same, mut errors) = (0, 0);
    let mut bad = Vec::new();
    for (row, got) in rows.iter().zip(&built) {
        let name = format!("{} {} {:?}", row["size"], row["enchant"], row["notables"]);
        match (got, &row["problem"], &row["error"]) {
            (Ok(p), want, Value::Null) if !want.is_null() => {
                let mut d = Vec::new();
                diff(&serde_json::to_value(p).unwrap(), want, "problem", &mut d);
                if d.is_empty() {
                    same += 1;
                } else {
                    d.truncate(6);
                    bad.push(format!("{name}:\n    {}", d.join("\n    ")));
                }
            }
            (Err(e), Value::Null, Value::String(want)) => {
                // Python's title wording for a missing enchant differs; the message is what we compare.
                if e.to_string() == *want {
                    same += 1;
                    errors += 1;
                } else {
                    bad.push(format!("{name}: error {e} != {want}"));
                }
            }
            (got, p, e) => bad.push(format!("{name}: {:?} vs problem {} / error {e}", got.as_ref().map(|_| ()).map_err(|e| e.to_string()), !p.is_null())),
        }
    }
    eprintln!("clusters: built {} in {elapsed:?}; {same}/{} identical to Python ({errors} errors)", rows.len(), rows.len());
    assert!(bad.is_empty(), "{} of {} differ:\n{}", bad.len(), rows.len(), bad.join("\n"));
    // And they solve like the golden ones.
    let costs: Vec<_> = rows
        .par_iter()
        .zip(&built)
        .filter_map(|(row, got)| {
            let p = got.as_ref().ok()?;
            let theirs: Problem = serde_json::from_value(row["problem"].clone()).unwrap();
            Some((solved(p), solved(&theirs)))
        })
        .collect();
    for (a, b) in costs {
        match (a, b) {
            (Ok(a), Ok(b)) => assert!((a - b).abs() <= 1e-7 * a.abs().max(b.abs()).max(1.0), "{a} != {b}"),
            (a, b) => assert_eq!(a, b),
        }
    }
}

// -- tests/test_craft.py ----------------------------------------------------------------

fn craft_prices() -> Prices {
    [
        ("transmute", 0.1),
        ("alt", 1.0),
        ("aug", 0.2),
        ("regal", 3.0),
        ("exalted", 50.0),
        ("annul", 20.0),
        ("alch", 0.5),
        ("chaos", 2.0),
        ("scour", 1.0),
        ("lifeforce", 0.1),
        ("fossil", 5.0),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), *v))
    .collect()
}

const FREE: Settings = Settings { chaos_per_hour: 0.0 };

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn cluster_two_prefix_notables_need_a_rare() {
    if skip_without_data() {
        return;
    }
    let prices = craft_prices();
    let (problem, plan) =
        cluster::cost("medium", "area damage", &names(&["Vast Power", "Assert Dominance"]), Pricing::new(&prices, &FREE), None, 84, &[]).unwrap();
    assert!(problem.targets.iter().all(|t| t.side == Side::Prefix));
    // a magic jewel holds one prefix, so every plan must make the item rare
    assert!(["regal", "alch", "chaos", "scour+alch"].iter().any(|a| plan.usage.get(*a).is_some_and(|u| *u > 0.0)));
    let simulated = simulate(&problem, &plan, &prices, &FREE, 4000, 2, 100_000);
    assert!((simulated - plan.cost).abs() <= 0.15 * plan.cost, "{simulated} vs {}", plan.cost);
}

#[test]
fn cluster_notable_cap() {
    if skip_without_data() {
        return;
    }
    let err = cluster::cost("medium", "area damage", &names(&["Vast Power", "Assert Dominance", "Magnifier"]), Pricing::new(&craft_prices(), &FREE), None, 84, &[]);
    assert!(err.unwrap_err().to_string().contains("at most 2 notables"));
}

#[test]
fn gear_targets_respect_tier_and_essences() {
    if skip_without_data() {
        return;
    }
    let item = ponder_game::item::parse_item(
        "Rarity: RARE\nX\nTwo-Toned Boots\nItem Level: 84\nImplicits: 0\n+117 to maximum Life\n30% increased Movement Speed\n",
    )
    .unwrap();
    let m = db().match_item(&item);
    let gp = gear::build(db(), &m, Pricing::new(&craft_prices(), &FREE), BuildOpts::default()).unwrap();
    let life = gp.problem.targets.iter().find(|t| t.name.contains("Life")).unwrap();
    assert!(life.good.iter().all(|id| db().mods[id.as_str()].required_level >= 54)); // T1 only on boots
    assert!(gp.problem.actions.iter().any(|a| a.name.contains("essence")));
    let mut problem = gp.problem.clone();
    let plan = solve(&mut problem, &craft_prices(), &FREE).unwrap(); // essences unpriced here, so left out
    assert!(!plan.unpriced.is_empty() && plan.cost > 0.0);
}

// -- tests/test_pipeline.py -------------------------------------------------------------

#[test]
fn best_base_of_a_real_item() {
    if skip_without_data() {
        return;
    }
    let path = repo_root().join("tests/fixtures/real/occultist_winter_orb.txt");
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let build = ponder_game::pob::load_text(&text).unwrap();
    let gloves = build
        .items
        .iter()
        .filter_map(|i| ponder_game::item::parse_item(&i.text).ok())
        .find(|i| i.base == "Warlock Gloves" && i.rarity == "RARE")
        .unwrap();
    let m = db().match_item(&gloves);
    let grp = gear::group(db(), &m);
    assert_eq!(grp.label, "Gloves (ES)");
    assert_eq!(grp.best(), "Warlock Gloves");
    assert!(grp.bases.len() > 5);
    let gp = gear::build(db(), &m, Pricing::new(&craft_prices(), &FREE), BuildOpts::default()).unwrap();
    assert!(!gp.problem.targets.is_empty());
}

#[test]
fn finished_item_from_a_ladder_archetype() {
    // tests/test_ladder.py::test_archetypes_become_finished_items: belts worn as life + armour + 3 suffixes
    // cost as that, not as the mods everyone shares.
    if skip_without_data() {
        return;
    }
    let ids = IdMods { explicit: names(&["IncreasedLife6", "IncreasedPhysicalDamageReductionRating5"]), ..Default::default() };
    let mut m = db().match_ids("Heavy Belt", &ids, None, &BTreeSet::new(), "RARE", "", &Default::default());
    let order = ["ChaosResistance", "FireResistance", "ColdResistance", "LightningResistance", "Strength"];
    let families: serde_json::Map<String, Value> = order.iter().map(|f| (f.to_string(), json!(if *f == "Strength" { 5 } else { 3 }))).collect();
    let sides = (2 - m.prefixes().count(), 3 - m.suffixes().count());
    m.finish = Some(json!({"families": families, "sides": [sides.0, sides.1], "order": order}));
    let gp = gear::build(db(), &m, Pricing::new(table(), &SETTINGS), BuildOpts { item_level: Some(84), ..Default::default() }).unwrap();
    let p = &gp.problem;
    assert_eq!(p.targets.len() + p.filler_need + gp.bench.len(), 5); // a full item
    assert!(!gp.bench.is_empty() && !p.reserve.is_empty()); // one open suffix is a bench resistance
    assert!(p.filler.iter().any(|x| db().mods[x.as_str()].type_ == "ColdResistance"));
    assert!(p.filler.iter().any(|x| db().mods[x.as_str()].type_ == "Strength"));
}

// -- tests/test_recomb.py ---------------------------------------------------------------

#[test]
fn problem_on_a_plain_base() {
    if skip_without_data() {
        return;
    }
    let base = db().find_base("Warlock Gloves").unwrap().to_string();
    let tags = db().item_tags(&base, &BTreeSet::new());
    let pool = gear::pool(db(), &base, &tags, 84);
    assert!(!pool.is_empty());
    let target = |family: &str, name: &str| {
        let ids: BTreeSet<String> = pool.iter().filter(|m| db().mods[m.id.as_str()].type_ == family && db().tier(&m.id, &tags).unwrap_or(99) <= 2).map(|m| m.id.clone()).collect();
        let side = pool.iter().find(|m| ids.contains(&m.id)).unwrap().side;
        let groups = pool.iter().filter(|m| ids.contains(&m.id)).flat_map(|m| m.groups.iter().cloned()).collect();
        Target { name: name.into(), side, good: ids, groups, notable: false }
    };
    let targets = vec![target("IncreasedLife", "life"), target("Intelligence", "int")];
    let p = gear::problem_on(db(), &base, targets, 84, &BTreeSet::new(), Some((0, 0)), pricing());
    assert_eq!(p.junk_cap, Some((0, 0)));
    assert_eq!((p.need, p.kind, p.pool.len()), (2, Kind::Gear, pool.len()));
    assert!(p.actions.iter().any(|a| a.name.contains("fossil")) && p.actions.iter().any(|a| a.name.starts_with("lock ")));
    assert!(p.actions.iter().any(|a| a.name.starts_with("eldritch")));
    // Influenced bases lose the eldritch currencies.
    let shaped = gear::problem_on(db(), &base, Vec::new(), 84, &BTreeSet::from(["shaper".to_string()]), None, pricing());
    assert!(!shaped.actions.iter().any(|a| a.name.starts_with("eldritch")));
}

// -- smaller facts -----------------------------------------------------------------------

#[test]
fn spawn_tags_and_enchants() {
    if skip_without_data() {
        return;
    }
    assert!(gear::spawn_tags().contains("ring") && gear::spawn_tags().contains("default"));
    let e = cluster::find_enchant("large", "minion damage").unwrap();
    assert_eq!(e.short, "Minions deal 10% increased Damage");
    assert!(cluster::find_enchant("large", "zzz").is_err());
    let p = cluster::problem("large", "minion damage", &names(&["HaveIncreasedEffect@1", "GrantMinionAttackAndCastSpeed@1"]), None, 84, &[]).unwrap();
    let effect = p.targets.iter().find(|t| t.name.contains("Effect")).unwrap();
    assert!(effect.name == "35% increased Effect" && effect.good.len() == 1); // T1 only
    assert_eq!(cluster::resolve(&p.pool, &names(&["HaveIncreasedEffect@2"])).unwrap()[0].good.len(), 2); // T2+: 25% or 35%
    let fixed = cluster::resolve_fixed(&p.pool, &names(&["HaveIncreasedEffect@1"])).unwrap();
    assert!(fixed[0].label.ends_with("35% increased Effect"));
}

#[test]
fn bench_costs_and_essences() {
    if skip_without_data() {
        return;
    }
    let cost = actions::bench_cost("Gloves", actions::LOCK_PREFIXES).unwrap();
    assert!(!cost.is_empty());
    assert!(actions::bench_cost("Gloves", "NoSuchMod").is_none());
    let (acts, forced) = actions::essences("Gloves", 84, 6);
    assert_eq!(acts.len(), 2 * forced.len());
    assert!(acts.iter().all(|a| a.forced.is_some()));
    let mods: BTreeMap<_, _> = forced.into_iter().collect();
    assert!(!mods.is_empty());
}
