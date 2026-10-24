//! Recombinator plans: tests/test_recomb.py, plus the Python goldens (scripts/golden/recomb.py).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Once, OnceLock};
use std::time::Instant;

use ponder::gear::{self, Pricing};
use ponder::recomb::{self, Counts, Donor, Node, Planner, Step, Want};
use ponder_core::{Plan, Prices, Problem, Rng, Settings, Side, Target};
use ponder_game::data;
use ponder_game::mods::ModDB;
use rayon::prelude::*;
use serde_json::{Value, json};

// -- helpers ----------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    match std::env::var_os("PONDER_REPO") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .unwrap(),
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
    let have = data::available(None);
    if !have {
        eprintln!(
            "skipping: repoe data not found in {}; run `ponder data fetch`",
            data::data_dir().display()
        );
    }
    !have
}

fn db() -> &'static ModDB {
    static DB: OnceLock<ModDB> = OnceLock::new();
    DB.get_or_init(|| ModDB::load().expect("repoe data"))
}

fn golden_path(path: &str) -> PathBuf {
    repo_root().join("rust/goldens").join(path)
}

/// A golden file, plain or gzipped; None (with a note) when it hasn't been exported.
fn golden(path: &str) -> Option<Value> {
    let file = match std::fs::File::open(golden_path(path)) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("skipping: {path}: {e}; run `uv run python scripts/golden/recomb.py`");
            return None;
        }
    };
    let mut text = String::new();
    if path.ends_with(".gz") {
        flate2::read::GzDecoder::new(file)
            .read_to_string(&mut text)
            .unwrap();
    } else {
        std::io::BufReader::new(file)
            .read_to_string(&mut text)
            .unwrap();
    }
    Some(serde_json::from_str(&text).unwrap())
}

fn table() -> Option<&'static Prices> {
    static T: OnceLock<Option<Prices>> = OnceLock::new();
    T.get_or_init(|| golden("problems/prices.json.gz").map(|v| serde_json::from_value(v).unwrap()))
        .as_ref()
}

const SETTINGS: Settings = Settings {
    chaos_per_hour: 1500.0,
};
const FREE_CLICKS: Settings = Settings {
    chaos_per_hour: 0.0,
};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9f64.max(1e-9 * a.abs().max(b.abs()))
        || (a - b).abs() <= 1e-9 * a.abs().max(b.abs())
}

fn approx(a: f64, b: f64) {
    assert!(
        (a - b).abs() <= 1e-12 + 1e-9 * a.abs().max(b.abs()),
        "{a} != {b}"
    );
}

/// Differences between two JSON documents, numbers compared to a relative 1e-9.
fn diff(a: &Value, b: &Value, path: &str, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            if (x - y).abs() > 1e-9 * x.abs().max(y.abs()) {
                out.push(format!("{path}: {x} != {y}"));
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys()).collect::<BTreeSet<_>>() {
                match (x.get(k), y.get(k)) {
                    (Some(p), Some(q)) => diff(p, q, &format!("{path}.{k}"), out),
                    (p, q) => out.push(format!(
                        "{path}.{k}: {} != {}",
                        p.map_or("missing".into(), |v| v.to_string()),
                        q.map_or("missing".into(), |v| v.to_string())
                    )),
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

fn plan_costing(cost: f64) -> Option<Plan> {
    Some(Plan {
        cost,
        ..Plan::default()
    })
}

fn donor(base: &str, base_id: &str, carries: &[&str], cost: f64) -> Donor {
    let mut d = Donor::new(base, base_id, carries);
    d.plan = plan_costing(cost);
    d
}

const NONE: Counts = [(0, 0); 2];

fn counts(prefix: (i64, i64), suffix: (i64, i64)) -> Counts {
    [prefix, suffix]
}

fn leaf_node(d: &Donor) -> Node {
    Node::Donor(Arc::new(d.leaf()))
}

// -- tests/test_recomb.py ---------------------------------------------------------------

#[test]
fn side_counts_are_distributions() {
    for (n, row) in recomb::SIDE_COUNTS.iter().enumerate() {
        approx(row.iter().map(|(_, p)| p).sum::<f64>(), 1.0);
        assert!(
            row.iter().map(|(k, _)| *k).max().unwrap() <= n.min(3),
            "{n}"
        );
    }
}

#[test]
fn nnn_padding_raises_the_chance_to_keep_everything() {
    // Two wanted prefixes, one per donor: a pool of 2 keeps both a third of the time.
    let plain = recomb::chance(&counts((1, 0), (0, 0)), &counts((1, 0), (0, 0)), true);
    approx(plain, 0.33);
    // Two NNN prefixes on a partner base make a pool of 4: kept 2+ 90% of the time, but the base is a coin flip.
    let padded = recomb::chance(&counts((1, 0), (0, 0)), &counts((1, 2), (0, 0)), false);
    approx(padded, 0.5 * 0.9);
    // Sides multiply; a side with nothing wanted doesn't matter.
    let both = recomb::chance(&counts((1, 0), (1, 0)), &counts((1, 0), (0, 0)), true);
    approx(both, 0.33 * 0.59);
    assert_eq!(NONE, counts((0, 0), (0, 0)));
}

#[test]
fn step_cost_divides_by_the_chance() {
    let a = donor("A", "a", &["x"], 10.0);
    let b = donor("B", "b", &["y"], 20.0);
    let step = Step::new(leaf_node(&a), leaf_node(&b), 0.25, 0.0, 0.0, 0.0);
    approx(step.cost.unwrap(), 120.0);
    let kinds: Vec<String> = step.steps().into_iter().map(|s| s.0).collect();
    assert_eq!(kinds, ["craft", "craft", "recombine"]);
    // With clicking charged, the recombination's 15 seconds are added before dividing.
    let paid = Step::new(leaf_node(&a), leaf_node(&b), 0.25, 0.0, 0.0, 1500.0);
    approx(paid.cost.unwrap(), (30.0 + 15.0 * 1500.0 / 3600.0) / 0.25);
    // A donor without a plan, or no chance, has no cost.
    let unsolved = Donor::new("C", "c", &["z"]);
    assert!(
        Step::new(leaf_node(&a), leaf_node(&unsolved), 0.25, 0.0, 0.0, 0.0)
            .cost
            .is_none()
    );
    assert!(
        Step::new(leaf_node(&a), leaf_node(&b), 0.0, 0.0, 0.0, 0.0)
            .cost
            .is_none()
    );
}

#[test]
fn step_text_names_the_odds_salvage_and_doubling() {
    let a = donor("A", "a", &["life", "es"], 10.0);
    let b = donor("B", "b", &["es"], 20.0);
    let first = Step::new(leaf_node(&a), leaf_node(&b), 0.6, 0.0, 0.0, 0.0);
    assert_eq!(first.carries, ["life", "es"]);
    assert_eq!(first.doubled, ["es"]);
    let c = donor("C", "c", &["int"], 5.0);
    let top = Step::new(
        Node::Step(Arc::new(first)),
        leaf_node(&c),
        0.59,
        0.34,
        0.001,
        0.0,
    );
    let steps = top.steps();
    assert_eq!(steps.len(), 5);
    assert_eq!(steps[0].1, "A: life + es");
    assert_eq!(steps[2].1, "#1 + #2 \u{2192} life + es  (60%; es doubled)");
    assert_eq!(
        steps[4].1,
        "#3 + #4 \u{2192} life + es + int  (59%; #3 back 34%)"
    );
    assert_eq!(top.donors().len(), 3);
    let dict = top.to_dict();
    assert_eq!(dict["table"], "poewiki / Butsicles (Aug 2026)");
    assert_eq!(dict["salvage"], json!([0.34, 0.001]));
    assert_eq!(
        dict["steps"][0],
        json!({"do": "craft", "what": "A: life + es", "chaos": 10.0})
    );
}

fn target(
    db: &ModDB,
    pool: &[ponder_core::Mod],
    tags: &BTreeSet<String>,
    family: &str,
    name: &str,
) -> Target {
    let ids: BTreeSet<String> = pool
        .iter()
        .filter(|m| db.mods[m.id.as_str()].type_ == family && db.tier(&m.id, tags).unwrap() <= 2)
        .map(|m| m.id.clone())
        .collect();
    let side = pool.iter().find(|m| ids.contains(&m.id)).unwrap().side;
    let groups = pool
        .iter()
        .filter(|m| ids.contains(&m.id))
        .flat_map(|m| m.groups.iter().cloned())
        .collect();
    Target {
        name: name.into(),
        side,
        good: ids,
        groups,
        notable: false,
    }
}

#[test]
fn planner_builds_padded_leaves_and_a_tree() {
    if skip_without_data() {
        return;
    }
    let Some(table) = table() else { return };
    let pricing = Pricing::new(table, &FREE_CLICKS);
    let db = db();
    let base = db.find_base("Warlock Gloves").unwrap().to_string();
    let tags = db.item_tags(&base, &BTreeSet::new());
    let pool = gear::pool(db, &base, &tags, 84);
    let targets = vec![
        target(db, &pool, &tags, "IncreasedLife", "life"),
        target(db, &pool, &tags, "Intelligence", "int"),
    ];
    let problem = gear::problem_on(db, &base, targets, 84, &BTreeSet::new(), None, pricing);
    let mut planner = Planner::new(db, &problem, &base, 84, pricing);
    assert!(
        planner
            .leaves()
            .iter()
            .any(|d| !d.on_target && !d.padding.is_empty())
    ); // partner bases with NNN pads
    assert!(
        planner
            .leaves()
            .iter()
            .all(|d| d.problem.as_ref().unwrap().junk_cap == Some((0, 0)))
    ); // donors must be clean
    for d in planner.leaves_mut() {
        // stand-in costs: padding costs more
        d.plan = plan_costing(5.0 + 3.0 * d.padding.values().sum::<usize>() as f64);
    }
    let best = planner.best().unwrap();
    let mut carries = best.carries.clone();
    carries.sort();
    assert_eq!(carries, ["int", "life"]);
    let plain = 10.0 / (0.59 * 0.59); // life on one donor, int on the other, both on the target base
    assert!(best.cost.unwrap() <= plain + 1e-9);
}

#[test]
fn filler_slots_get_distinct_resistance_families() {
    if skip_without_data() {
        return;
    }
    let Some(table) = table() else { return };
    let pricing = Pricing::new(table, &FREE_CLICKS);
    let db = db();
    let base = db.find_base("Warlock Gloves").unwrap().to_string();
    let mut problem = gear::problem_on(db, &base, vec![], 84, &BTreeSet::new(), None, pricing);
    let res: BTreeSet<String> = problem
        .mods()
        .keys()
        .filter(|m| gear::is_filler_family(&db.mods[m.as_str()].type_))
        .cloned()
        .collect();
    problem.filler = res;
    problem.filler_need = 3;
    problem.reset_caches();
    let slots: Vec<Target> = recomb::wants(&problem)
        .into_iter()
        .map(|w| w.target.unwrap())
        .collect();
    assert!(slots.len() == 3 && slots.iter().all(|t| !t.good.is_empty()));
    let groups: Vec<BTreeSet<String>> = slots
        .iter()
        .map(|t| {
            t.good
                .iter()
                .flat_map(|m| problem.mods()[m].groups.iter().cloned())
                .collect()
        })
        .collect();
    // two donors can't both bring fire
    assert!(
        groups[0].is_disjoint(&groups[1])
            && groups[0].is_disjoint(&groups[2])
            && groups[1].is_disjoint(&groups[2])
    );
}

fn lists<'a>(prefix: &[&'a str], suffix: &[&'a str]) -> [Vec<&'a str>; 2] {
    [prefix.to_vec(), suffix.to_vec()]
}

#[test]
fn chance_matches_a_simulation_with_nnn_and_junk_free_donors() {
    let a = lists(&["life"], &["res"]);
    let b = lists(&["es", "nnn1", "nnn2"], &["nnn3"]);
    let legal: HashSet<&str> = ["life", "res", "es"].into();
    let sim = recomb::simulate(&a, &b, &legal, false, &legal, 40000, 0);
    let exact = recomb::chance(&counts((1, 0), (1, 0)), &counts((1, 2), (0, 1)), false);
    assert!((sim - exact).abs() < 0.01, "{sim} vs {exact}");
}

#[test]
fn failures_that_hand_back_an_input_cut_the_cost() {
    // life (prefix) + int (suffix) on two target-base donors: each side keeps its mod 59% of the time.
    let (a, b) = (counts((1, 0), (0, 0)), counts((0, 0), (1, 0)));
    approx(recomb::salvage(&a, &b, true), 0.59 * 0.41); // life kept, int lost
    // Exact salvage chances plus success and the rest add up over the four outcomes.
    let total = recomb::chance(&a, &b, true)
        + recomb::salvage(&a, &b, true)
        + recomb::salvage(&b, &a, true)
        + 0.41 * 0.41;
    approx(total, 1.0);
    let x = donor("A", "a", &["life"], 10.0);
    let y = donor("A", "a", &["int"], 10.0);
    let step = Step::new(
        leaf_node(&x),
        leaf_node(&y),
        0.59 * 0.59,
        0.59 * 0.41,
        0.59 * 0.41,
        0.0,
    );
    approx(
        step.cost.unwrap(),
        20.0 * (1.0 - 0.59 * 0.41) / (0.59 * 0.59),
    );
}

#[test]
fn salvage_matches_a_simulation() {
    let a = lists(&["life", "es"], &["res"]);
    let b = lists(&["nnn1"], &["int", "nnn2"]);
    let legal: HashSet<&str> = ["life", "es", "res", "int"].into();
    let exact_set: HashSet<&str> = ["life", "es", "res"].into();
    let mut rng = Rng::new(1);
    let trials = 40000;
    let hits = (0..trials)
        .filter(|_| {
            recomb::draw(&mut rng, &a, &b, &legal, false).is_some_and(|kept| kept == exact_set)
        })
        .count();
    let exact = recomb::salvage(&counts((2, 0), (1, 0)), &counts((0, 1), (1, 1)), false);
    assert!(
        (hits as f64 / trials as f64 - exact).abs() < 0.01,
        "{} vs {exact}",
        hits as f64 / trials as f64
    );
}

#[test]
fn doubling_a_wanted_mod_pads_the_pool_on_the_target_base() {
    // Two prefixes wanted (life, es). Plain: pool 2 keeps both 33%. With life on both
    // donors the pool is 3: two picks always get both (life's group blocks its copy).
    let want = |name: &str, id: &str, group: &str| Want {
        name: name.into(),
        side: Side::Prefix,
        target: Some(Target {
            name: name.into(),
            side: Side::Prefix,
            good: [id.to_string()].into(),
            groups: [group.to_string()].into(),
            notable: false,
        }),
    };
    let one = |names: &[&str], cost: f64| donor("T", "t", names, cost);
    let leaves = vec![
        (vec!["life"], vec![one(&["life"], 10.0)]),
        (vec!["es"], vec![one(&["es"], 10.0)]),
        (vec!["es", "life"], vec![one(&["es", "life"], 12.0)]),
    ];
    let planner = Planner::from_leaves(
        vec![want("life", "l", "L"), want("es", "e", "E")],
        leaves,
        0.0,
    );
    let best = planner.best().unwrap();
    assert_eq!(best.doubled.len(), 1); // either mod, doubled, does it
    approx(best.chance, 0.60); // pool 3 keeps 2+ 60%
    approx(best.cost.unwrap(), (10.0 + 12.0) / 0.60); // beats (10 + 10) / 0.33 without doubling
}

#[test]
fn a_planner_without_a_plan_or_with_one_want_has_no_tree() {
    let w = |name: &str| Want {
        name: name.into(),
        side: Side::Prefix,
        target: None,
    };
    let leaves = vec![
        (vec!["a"], vec![Donor::new("T", "t", &["a"])]),
        (vec!["b"], vec![Donor::new("T", "t", &["b"])]),
    ];
    // unsolved donors are dropped from the options
    assert!(
        Planner::from_leaves(vec![w("a"), w("b")], leaves, 0.0)
            .best()
            .is_none()
    );
}

#[test]
fn combination_counts_follow_the_pool_table() {
    // Pools over 6 use row 6; an empty side keeps nothing; wanting more than is kept never succeeds.
    approx(recomb::keep_at_least(9, 2), 1.0);
    approx(recomb::keep_at_least(0, 0), 1.0);
    approx(recomb::keep_at_least(2, 3), 0.0);
    approx(recomb::keep_exactly(3, 3, 3), 0.10);
    approx(recomb::keep_exactly(4, 0, 0), 1.0);
}

// -- goldens ----------------------------------------------------------------------------

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

#[test]
fn kernel_matches_python_goldens() {
    let Some(g) = golden("recomb/kernel.json") else {
        return;
    };
    // the table itself
    for (n, row) in g["side_counts"].as_object().unwrap() {
        let ours = recomb::SIDE_COUNTS[n.parse::<usize>().unwrap()];
        assert_eq!(row.as_object().unwrap().len(), ours.len(), "pool {n}");
        for (k, p) in row.as_object().unwrap() {
            let mine = ours
                .iter()
                .find(|(kk, _)| *kk == k.parse::<usize>().unwrap())
                .unwrap()
                .1;
            assert_eq!(mine, f(p), "pool {n} keeps {k}");
        }
    }
    assert_eq!(g["table_version"], recomb::TABLE_VERSION);
    let mut n = 0;
    for row in g["keep_at_least"].as_array().unwrap() {
        let (pool, wanted) = (
            row[0].as_u64().unwrap() as usize,
            row[1].as_u64().unwrap() as usize,
        );
        assert!(
            close(recomb::keep_at_least(pool, wanted), f(&row[2])),
            "keep_at_least {row}"
        );
        n += 1;
    }
    for row in g["keep_exactly"].as_array().unwrap() {
        let (pool, wanted, subset) = (
            row[0].as_u64().unwrap() as usize,
            row[1].as_u64().unwrap() as usize,
            row[2].as_u64().unwrap() as usize,
        );
        assert!(
            close(recomb::keep_exactly(pool, wanted, subset), f(&row[3])),
            "keep_exactly {row}"
        );
        n += 1;
    }
    let pair = |v: &Value| (v[0].as_i64().unwrap(), v[1].as_i64().unwrap());
    for row in g["one_side"].as_array().unwrap() {
        let (mut a, mut b): (Counts, Counts) = (NONE, NONE);
        let side = usize::from(row[0] == "suffix");
        (a[side], b[side]) = (pair(&row[1]), pair(&row[2]));
        let same = row[3].as_bool().unwrap();
        assert!(
            close(recomb::chance(&a, &b, same), f(&row[4])),
            "chance {row}"
        );
        assert!(
            close(recomb::salvage(&a, &b, same), f(&row[5])),
            "salvage {row}"
        );
        assert!(
            close(recomb::salvage(&b, &a, same), f(&row[6])),
            "salvage {row}"
        );
        n += 3;
    }
    for row in g["two_sides"].as_array().unwrap() {
        let side = |v: &Value| -> Counts { [pair(&v[0]), pair(&v[1])] };
        let (a, b, same) = (side(&row[0]), side(&row[1]), row[2].as_bool().unwrap());
        assert!(
            close(recomb::chance(&a, &b, same), f(&row[3])),
            "chance {row}"
        );
        assert!(
            close(recomb::salvage(&a, &b, same), f(&row[4])),
            "salvage {row}"
        );
        assert!(
            close(recomb::salvage(&b, &a, same), f(&row[5])),
            "salvage {row}"
        );
        n += 3;
    }
    eprintln!("kernel: {n} probabilities agree with Python");
    assert!(n > 10000);
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

fn crc(text: &str) -> u64 {
    let mut c = flate2::Crc::new();
    c.update(text.as_bytes());
    u64::from(c.sum())
}

/// The same fingerprint of a leaf's problem as scripts/golden/recomb.py, field by field.
fn fingerprint(p: &Problem) -> Value {
    let opt = |s: &Option<String>| s.clone().unwrap_or_else(|| "None".into());
    let pool: String = p
        .pool
        .iter()
        .map(|m| format!("{}|{}|{};", m.id, m.weight, m.side.name()))
        .collect();
    let acts: String = p
        .actions
        .iter()
        .map(|a| {
            let rarity: Vec<String> = a.from_rarity.iter().map(u8::to_string).collect();
            format!(
                "{}|{}|{}|{}|{}|{};",
                a.name,
                a.kind,
                a.profile,
                opt(&a.first_profile),
                opt(&a.forced),
                rarity.join(",")
            )
        })
        .collect();
    let profs: String = p
        .profiles
        .iter()
        .map(|(k, v)| format!("{k}:{};", v.keys().cloned().collect::<Vec<_>>().join(",")))
        .collect();
    let extra = p.extra_mods.keys().cloned().collect::<Vec<_>>().join(",");
    json!({
        "pool": crc(&pool), "actions": crc(&acts), "profiles": crc(&profs), "extra": crc(&extra), "n_pool": p.pool.len(), "n_actions": p.actions.len(),
        "cost_sum": p.actions.iter().flat_map(|a| a.cost.iter().map(|(_, q)| *q)).sum::<f64>(),
        "targets": p.targets.iter().map(|t| json!([t.name, t.side.name(), t.good, t.groups, t.notable])).collect::<Vec<_>>(),
        "junk_cap": p.junk_cap.map(|(a, b)| json!([a, b])), "filler": p.filler, "filler_need": p.filler_need,
        "kind": match p.kind { ponder_core::Kind::Gear => "gear", ponder_core::Kind::Jewel => "jewel" }, "need": p.need,
    })
}

fn leaf_json(d: &Donor) -> Value {
    json!({
        "base": d.base, "base_id": d.base_id, "carries": d.carries, "padding": d.padding, "on_target": d.on_target,
        "problem": fingerprint(d.problem.as_ref().unwrap()),
    })
}

fn want_json(w: &Want) -> Value {
    json!({"name": w.name, "side": w.side.name(), "target": w.target.as_ref().map(|t| json!({"good": t.good, "groups": t.groups}))})
}

fn best_json(planner: &Planner) -> Value {
    match planner.best() {
        None => Value::Null,
        Some(b) => {
            let mut v = b.to_dict();
            v["carries"] = json!(b.carries);
            v["doubled"] = json!(b.doubled);
            v["donors"] = json!(b.donors().iter().map(|d| d.describe()).collect::<Vec<_>>());
            v
        }
    }
}

/// The estimates the golden planners were made from: problems/gear.json.gz rows by index.
fn gear_problem(rows: &Value, i: usize) -> Problem {
    serde_json::from_value(rows[i]["build"]["problem"].clone()).unwrap()
}

fn stand_in(planner: &mut Planner) -> Vec<Option<f64>> {
    let mut costs = Vec::new();
    for (i, d) in planner.leaves_mut().into_iter().enumerate() {
        if i % 17 == 5 {
            d.plan = None;
            costs.push(None);
        } else {
            let cost = 20.0
                + 7.0 * d.carries.len() as f64
                + 9.0 * d.padding.values().sum::<usize>() as f64
                + if d.on_target { 0.0 } else { 3.0 }
                + (i * 37 % 11) as f64 * 0.5;
            d.plan = plan_costing(cost);
            costs.push(Some(cost));
        }
    }
    costs
}

#[test]
fn planners_match_python_goldens() {
    if skip_without_data() {
        return;
    }
    let (Some(planners), Some(rows), Some(table)) = (
        golden("recomb/planner.json.gz"),
        golden("problems/gear.json.gz"),
        table(),
    ) else {
        return;
    };
    let pricing = Pricing::new(table, &SETTINGS);
    let planners = planners.as_array().unwrap();
    // warm the data so the timing is the planning alone
    ponder::gear::pool(
        db(),
        db().find_base("Warlock Gloves").unwrap(),
        &BTreeSet::new(),
        84,
    );
    let started = Instant::now();
    let built: Vec<(Planner, Vec<Option<f64>>, Value)> = planners
        .par_iter()
        .map(|g| {
            let problem = gear_problem(&rows, g["row"].as_u64().unwrap() as usize);
            let mut planner = Planner::new(
                db(),
                &problem,
                g["base_id"].as_str().unwrap(),
                g["item_level"].as_i64().unwrap(),
                pricing,
            );
            let costs = stand_in(&mut planner);
            let best = best_json(&planner);
            (planner, costs, best)
        })
        .collect();
    let elapsed = started.elapsed();
    let (mut same, mut leaves, mut trees, mut bad) = (0, 0, 0, Vec::new());
    for (g, (planner, costs, best)) in planners.iter().zip(&built) {
        let name = format!("row {} {}", g["row"], g["title"]);
        let mut d = Vec::new();
        if g["ok"] != json!(planner.ok) {
            d.push(format!("ok {} != {}", planner.ok, g["ok"]));
        }
        diff(
            &json!(planner.want.iter().map(want_json).collect::<Vec<_>>()),
            &g["wants"],
            "wants",
            &mut d,
        );
        if planner.ok && g["ok"] == json!(true) {
            let ours: Vec<Value> = planner.leaves().into_iter().map(leaf_json).collect();
            leaves += ours.len();
            diff(&json!(ours), &g["leaves"], "leaves", &mut d);
            let sets: Vec<Value> = planner
                .leaf_sets()
                .into_iter()
                .map(|(k, ds)| {
                    let at = |x: &Donor| {
                        planner
                            .leaves()
                            .iter()
                            .position(|y| std::ptr::eq(*y, x))
                            .unwrap()
                    };
                    json!([
                        k.iter().collect::<Vec<_>>(),
                        ds.into_iter().map(at).collect::<Vec<_>>()
                    ])
                })
                .collect();
            diff(&json!(sets), &g["leaf_sets"], "leaf_sets", &mut d);
            diff(&json!(costs), &g["costs"], "costs", &mut d);
            diff(best, &g["best"], "best", &mut d);
            trees += usize::from(!best.is_null());
        }
        if d.is_empty() {
            same += 1;
        } else {
            d.truncate(6);
            bad.push(format!("{name}:\n    {}", d.join("\n    ")));
        }
    }
    eprintln!(
        "planners: {} items, {leaves} leaves, {trees} trees in {elapsed:?}; {same}/{} identical to Python (leaf problems by fingerprint, stand-in costs)",
        planners.len(),
        planners.len()
    );
    assert!(
        bad.is_empty(),
        "{} of {} differ:\n{}",
        bad.len(),
        planners.len(),
        bad.join("\n")
    );
}

#[test]
fn solved_planners_match_python_goldens() {
    if skip_without_data() {
        return;
    }
    let (Some(solved), Some(rows), Some(planners), Some(table)) = (
        golden("recomb/solved.json.gz"),
        golden("problems/gear.json.gz"),
        golden("recomb/planner.json.gz"),
        table(),
    ) else {
        return;
    };
    let pricing = Pricing::new(table, &SETTINGS);
    let by_row: BTreeMap<u64, &Value> = planners
        .as_array()
        .unwrap()
        .iter()
        .map(|g| (g["row"].as_u64().unwrap(), g))
        .collect();
    let solved = solved.as_array().unwrap();
    let (mut bad, mut checked) = (Vec::new(), 0);
    for g in solved {
        let row = g["row"].as_u64().unwrap();
        let info = by_row[&row];
        let problem = gear_problem(&rows, row as usize);
        let started = Instant::now();
        let mut planner = Planner::new(
            db(),
            &problem,
            info["base_id"].as_str().unwrap(),
            info["item_level"].as_i64().unwrap(),
            pricing,
        );
        let planned = started.elapsed();
        planner.solve(pricing);
        let elapsed = started.elapsed();
        let mut d = Vec::new();
        let costs: Vec<Value> = planner.leaves().iter().map(|l| json!(l.cost())).collect();
        let errors: Vec<Value> = planner.leaves().iter().map(|l| json!(l.error)).collect();
        diff(&json!(costs), &g["costs"], "costs", &mut d);
        diff(&json!(errors), &g["errors"], "errors", &mut d);
        diff(&best_json(&planner), &g["best"], "best", &mut d);
        eprintln!(
            "solved row {row} {}: {} leaves, planned in {planned:?}, solved in {elapsed:?}; {} differences",
            g["title"],
            costs.len(),
            d.len()
        );
        checked += costs.len();
        if !d.is_empty() {
            d.truncate(6);
            bad.push(format!(
                "row {row} {}:\n    {}",
                g["title"],
                d.join("\n    ")
            ));
        }
    }
    eprintln!(
        "solved: {checked} leaf plans over {} items; {} items differ",
        solved.len(),
        bad.len()
    );
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert!(!solved.is_empty());
}

#[test]
fn plan_solves_and_merges_in_one_call() {
    if skip_without_data() {
        return;
    }
    let (Some(rows), Some(planners), Some(table)) = (
        golden("problems/gear.json.gz"),
        golden("recomb/planner.json.gz"),
        table(),
    ) else {
        return;
    };
    let pricing = Pricing::new(table, &SETTINGS);
    let Some(g) = planners
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["ok"] == json!(true) && g["leaves"].as_array().unwrap().len() < 25)
    else {
        return;
    };
    let problem = gear_problem(&rows, g["row"].as_u64().unwrap() as usize);
    let step = recomb::plan(
        db(),
        &problem,
        g["base_id"].as_str().unwrap(),
        g["item_level"].as_i64().unwrap(),
        pricing,
    );
    let step = step.expect("a tree for a small item");
    assert!(step.cost.unwrap() > 0.0 && step.donors().iter().all(|d| d.cost.is_some()));
    assert_eq!(strings(&json!(step.carries)).len(), step.carries.len());
}
