//! The price model: the T1-premium test of tests/test_ladder.py, and the Python goldens
//! (scripts/golden/recomb.py) for trade queries and fits. No network: listings are synthetic.

use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Once, OnceLock};

use ponder::demand::RES;
use ponder::gear;
use ponder::pricing::{self, FLOOR_TIER, TradeData};
use ponder_core::Rng;
use ponder_game::data;
use ponder_game::item::Item;
use ponder_game::mods::{ItemMatch, ModDB};
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

fn golden(path: &str) -> Option<Value> {
    let file = match std::fs::File::open(repo_root().join("rust/goldens").join(path)) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("skipping: {path}: {e}; run `uv run python scripts/golden/recomb.py`");
            return None;
        }
    };
    let mut text = String::new();
    flate2::read::GzDecoder::new(file)
        .read_to_string(&mut text)
        .unwrap();
    Some(serde_json::from_str(&text).unwrap())
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

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

fn rates(v: &Value) -> HashMap<String, f64> {
    v.as_object()
        .unwrap()
        .iter()
        .map(|(k, r)| (k.clone(), r.as_f64().unwrap()))
        .collect()
}

fn model_json(m: &Option<pricing::PriceModel>) -> Value {
    match m {
        None => Value::Null,
        Some(m) => json!({
            "group": m.group, "listings": m.listings, "base_price": m.base_price, "per_res": m.per_res, "per_open_affix": m.per_open_affix, "r2": m.r2,
            "tiers": m.tiers.iter().map(|t| json!({"key": t.key, "text": t.text, "t1": t.t1, "t2": t.t2, "n_t1": t.n_t1, "n_t2": t.n_t2})).collect::<Vec<_>>(),
        }),
    }
}

fn matched_json(m: Option<ItemMatch>) -> Value {
    match m {
        None => Value::Null,
        Some(m) => json!(
            m.mods
                .iter()
                .map(|x| json!([x.mod_id, x.kind, x.source, x.tier, x.family]))
                .collect::<Vec<_>>()
        ),
    }
}

// -- least squares ----------------------------------------------------------------------

fn near(a: f64, b: f64) {
    assert!(
        (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0),
        "{a} != {b}"
    );
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
    let x = vec![
        vec![1.0, 1.0, 0.0],
        vec![1.0, 1.0, 0.0],
        vec![1.0, 1.0, 0.0],
    ];
    let c = pricing::lstsq(&x, &[2.0, 2.0, 2.0]);
    near(c[0], 1.0);
    near(c[1], 1.0);
    assert_eq!(c[2], 0.0);
}

// -- tests/test_ladder.py ---------------------------------------------------------------

#[test]
fn price_model_recovers_a_t1_premium() {
    if skip_without_data() {
        return;
    }
    let db = db();
    // Life tiers as they roll on Vaal Gauntlets, best first.
    let base = db.find_base("Vaal Gauntlets").unwrap();
    let tags = db.item_tags(base, &BTreeSet::new());
    let mut life: Vec<String> = gear::pool(db, base, &tags, 86)
        .into_iter()
        .filter(|m| db.mods[m.id.as_str()].type_ == "IncreasedLife")
        .map(|m| m.id)
        .collect();
    life.sort_by_key(|i| db.tier(i, &tags));
    let mut rng = Rng::new(1);
    let mut listings = Vec::new();
    for _ in 0..60 {
        let tier = [0, 0, 1, 2, 3][(rng.random() * 5.0) as usize];
        let low = &db.mods[life[tier].as_str()].stats[0].min;
        let mult = if tier == 0 {
            3.0
        } else if tier == 1 {
            1.5
        } else {
            1.0
        };
        let price = 100.0 * mult * (0.9 + 0.2 * rng.random());
        listings.push(json!({
            "item": {"baseType": "Vaal Gauntlets", "ilvl": 86, "explicitMods": [format!("+{low} to maximum Life"), "+40% to Fire Resistance"]},
            "listing": {"price": {"amount": price, "currency": "chaos"}},
        }));
    }
    let rates = HashMap::from([("chaos".to_string(), 1.0)]);
    let model = pricing::fit(
        db,
        "Gloves (Armour)",
        &["base_maximum_life", RES],
        &listings,
        &rates,
    )
    .expect("enough listings");
    let life = &model.tiers[0];
    assert!((life.t1.unwrap() / 3.0 - 1.0).abs() < 0.15, "{:?}", life.t1);
    assert!((life.t2.unwrap() / 1.5 - 1.0).abs() < 0.15, "{:?}", life.t2);
    assert_eq!(model.listings, 60);
    assert!(model.r2 > 0.9);
}

#[test]
fn too_few_listings_make_no_model() {
    if skip_without_data() {
        return;
    }
    let one = json!({"item": {"baseType": "Vaal Gauntlets", "explicitMods": ["+100 to maximum Life"]}, "listing": {"price": {"amount": 5, "currency": "chaos"}}});
    let rates = HashMap::from([("chaos".to_string(), 1.0)]);
    assert!(
        pricing::fit(
            db(),
            "Gloves",
            &["base_maximum_life", RES],
            &vec![one; 5],
            &rates
        )
        .is_none()
    );
}

#[test]
fn a_listing_without_a_known_base_has_no_match() {
    if skip_without_data() {
        return;
    }
    let item = |base: &str| json!({"item": {"baseType": base, "ilvl": 84, "explicitMods": ["+100 to maximum Life"], "influences": {"shaper": false}}});
    assert!(pricing::listing_match(db(), &item("Not A Real Base")).is_none());
    let m = pricing::listing_match(db(), &item("Vaal Gauntlets")).unwrap();
    assert!(m.base_id.is_some() && m.item.influences.is_empty());
    assert!(pricing::listing_match(db(), &json!({})).is_none());
    let _ = Item::default();
}

// -- goldens ----------------------------------------------------------------------------

fn example_match(e: &Value) -> ItemMatch {
    let mut item = Item::new("RARE", "", e["base"].as_str().unwrap());
    item.influences = strings(&e["influences"]).into_iter().collect();
    ItemMatch {
        item,
        base_id: Some(e["base_id"].as_str().unwrap().to_string()),
        item_class: Some(e["item_class"].as_str().unwrap().to_string()),
        tags: strings(&e["tags"]).into_iter().collect(),
        ..Default::default()
    }
}

#[test]
fn archetype_queries_match_python_goldens() {
    if skip_without_data() {
        return;
    }
    let Some(g) = golden("recomb/pricing.json.gz") else {
        return;
    };
    let td = TradeData::load();
    let queries = g["queries"].as_array().unwrap();
    let (mut same, mut none, mut bad) = (0, 0, Vec::new());
    for q in queries {
        let keys = strings(&q["families"]);
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        let got =
            pricing::archetype_query(db(), &example_match(&q["example"]), &keys, FLOOR_TIER, td);
        none += usize::from(got.is_none());
        let mut d = Vec::new();
        diff(&got.unwrap_or(Value::Null), &q["query"], "query", &mut d);
        // key order and int-versus-float spelling matter to the site's cache keys: compare text too
        if d.is_empty() {
            same += 1;
        } else {
            d.truncate(4);
            bad.push(format!(
                "row {} {}:\n    {}",
                q["row"],
                q["group"],
                d.join("\n    ")
            ));
        }
    }
    eprintln!(
        "archetype_query: {same}/{} identical to Python ({none} without a trade stat)",
        queries.len()
    );
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert!(queries.len() > 50);
}

#[test]
fn fits_match_python_goldens() {
    if skip_without_data() {
        return;
    }
    let Some(g) = golden("recomb/pricing.json.gz") else {
        return;
    };
    let rates = rates(&g["rates"]);
    let fits = g["fits"].as_array().unwrap();
    let (mut same, mut matched, mut bad) = (0, 0, Vec::new());
    for f in fits {
        let keys = strings(&f["families"]);
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        let listings = f["listings"].as_array().unwrap();
        let mut d = Vec::new();
        let ours: Vec<Value> = listings
            .iter()
            .map(|l| matched_json(pricing::listing_match(db(), l)))
            .collect();
        matched += ours.len();
        diff(&json!(ours), &f["matched"], "matched", &mut d);
        let model = pricing::fit(db(), f["group"].as_str().unwrap(), &keys, listings, &rates);
        diff(&model_json(&model), &f["model"], "model", &mut d);
        if d.is_empty() {
            same += 1;
        } else {
            d.truncate(6);
            bad.push(format!(
                "row {} {}:\n    {}",
                f["row"],
                f["group"],
                d.join("\n    ")
            ));
        }
    }
    eprintln!(
        "fit: {same}/{} models identical to Python (relative 1e-9), {matched} listings matched",
        fits.len()
    );
    // fewer listings than coefficients: no model
    let short = &g["too_few"];
    let first = &fits[0];
    let keys = strings(&first["families"]);
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    let few = &first["listings"].as_array().unwrap()[..short["n"].as_u64().unwrap() as usize];
    let model = pricing::fit(db(), "g", &keys, few, &rates);
    assert_eq!(model.is_none(), short["model"].is_null());
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert!(fits.len() >= 8);
}
