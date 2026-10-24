//! demand.wear / archetypes / cluster_archetypes / archetype_match against the Python goldens
//! (scripts/golden/problems.py), on the cached ladder. Skipped when the data or ladder cache is missing.

use std::io::Read;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use indexmap::IndexMap;
use ponder::demand::{self, Archetype, ClusterArchetype, Counter, Worn};
use ponder_game::data;
use ponder_game::mods::{ItemMatch, ModDB};
use ponder_net::ladder::{Ladder, Lite};
use serde_json::{Map, Value, json};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

fn golden(name: &str) -> Value {
    let path = repo_root()
        .join("rust/goldens/problems")
        .join(format!("{name}.json.gz"));
    let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut text = String::new();
    flate2::read::GzDecoder::new(file)
        .read_to_string(&mut text)
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Point the data directory at the repo's cache unless the caller chose one; true when there is nothing to test on.
fn skip() -> bool {
    if std::env::var_os("PONDER_DATA").is_none() {
        // SAFETY: called before any thread reads the environment (Once-guarded by `world`).
        unsafe { std::env::set_var("PONDER_DATA", repo_root().join(".cache/repoe")) };
    }
    if !data::available(None) {
        eprintln!(
            "skipping: repoe data not found in {}",
            data::data_dir().display()
        );
        return true;
    }
    if world().1.is_empty() {
        eprintln!(
            "skipping: no cached ladder in {}",
            repo_root().join(".cache/ladder").display()
        );
        return true;
    }
    false
}

fn world() -> &'static (ModDB, Vec<Lite>) {
    static W: OnceLock<(ModDB, Vec<Lite>)> = OnceLock::new();
    W.get_or_init(|| {
        if std::env::var_os("PONDER_DATA").is_none() {
            // SAFETY: first thing any test does, under this OnceLock.
            unsafe { std::env::set_var("PONDER_DATA", repo_root().join(".cache/repoe")) };
        }
        if !data::available(None) {
            return (
                ModDB::new(
                    Some(Default::default()),
                    Some(Default::default()),
                    Some(Default::default()),
                ),
                Vec::new(),
            );
        }
        let ladder = Ladder::with(
            ponder_net::http::real(),
            repo_root().join(".cache/ladder"),
            ponder_net::env::Clock::real(),
        );
        (
            ModDB::load().expect("repoe data"),
            ladder.cached(None).expect("ladder cache"),
        )
    })
}

fn worn() -> &'static Vec<Worn> {
    static W: OnceLock<Vec<Worn>> = OnceLock::new();
    W.get_or_init(|| {
        let (db, chars) = world();
        let t = Instant::now();
        let worn = demand::wear(db, chars, None);
        eprintln!(
            "wear: {} items from {} characters in {:.2?}",
            worn.len(),
            chars.len(),
            t.elapsed()
        );
        worn
    })
}

fn found() -> &'static Vec<Archetype> {
    static F: OnceLock<Vec<Archetype>> = OnceLock::new();
    F.get_or_init(|| {
        let t = Instant::now();
        let found = demand::archetypes(worn(), 5, 4, 2, 0.7);
        eprintln!("archetypes: {} in {:.2?}", found.len(), t.elapsed());
        found
    })
}

// -- Rust -> the golden's JSON shape -------------------------------------------------------

fn counter<K: std::hash::Hash + Eq + Clone, F: Fn(&K) -> String>(c: &Counter<K>, key: F) -> Value {
    Value::Object(c.iter().map(|(k, n)| (key(k), json!(n))).collect())
}

fn counter_s<K: std::hash::Hash + Eq + Clone + ToString>(c: &Counter<K>) -> Value {
    counter(c, K::to_string)
}

fn counters<K: std::hash::Hash + Eq + Clone + ToString>(m: &IndexMap<String, Counter<K>>) -> Value {
    Value::Object(m.iter().map(|(k, c)| (k.clone(), counter_s(c))).collect())
}

fn archetype_json(a: &Archetype) -> Value {
    json!({
        "group": a.group, "families": a.families, "characters": a.characters, "items": a.items, "lift": a.lift, "niche": a.niche(),
        "skills": counter_s(&a.skills), "bases": counter_s(&a.bases), "tiers": counters(&a.tiers), "resistances": counter_s(&a.resistances),
        "others": counter_s(&a.others), "fillers": counter(&a.fillers, |(f, s)| format!("{f}|{s}")),
        "filler_tiers": counters(&a.filler_tiers), "filled": counters(&a.filled),
    })
}

fn cluster_json(a: &ClusterArchetype) -> Value {
    json!({
        "size": a.size, "enchant": a.enchant, "tag": a.tag, "notables": a.notables, "characters": a.characters, "items": a.items,
        "passives": counter_s(&a.passives), "item_levels": counter_s(&a.item_levels), "full": a.full, "tiers": counters(&a.tiers),
    })
}

fn match_json(m: &ItemMatch) -> Value {
    json!({
        "base": m.item.base, "base_id": m.base_id, "item_class": m.item_class, "item_level": m.item.item_level,
        "influences": m.item.influences, "tags": m.tags,
        "mods": m.mods.iter().map(|x| json!({"mod_id": x.mod_id, "kind": x.kind, "family": x.family, "source": x.source, "tier": x.tier, "fractured": x.fractured()})).collect::<Vec<_>>(),
        "finish": m.finish,
    })
}

// -- comparison ------------------------------------------------------------------------------

#[derive(Default)]
struct Diff {
    problems: Vec<String>,
    float_drift: usize,
    drifted: std::collections::BTreeMap<String, usize>,
}

impl Diff {
    /// Deep equality; object key order counts except where Python's order is arbitrary (set iteration):
    /// `unordered` lists those paths. Floats may differ by two ulps (counted as drift: serde_json parses decimals without exact rounding).
    fn same(&mut self, path: &str, a: &Value, b: &Value, unordered: &[&str]) {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                let (mut kx, mut ky): (Vec<&String>, Vec<&String>) =
                    (x.keys().collect(), y.keys().collect());
                if unordered.contains(&path) {
                    kx.sort();
                    ky.sort();
                }
                if kx != ky {
                    self.problems.push(format!("{path}: keys {kx:?} != {ky:?}"));
                    return;
                }
                for k in x.keys() {
                    self.same(&format!("{path}.{k}"), &x[k], &y[k], unordered);
                }
            }
            (Value::Array(x), Value::Array(y)) => {
                if x.len() != y.len() {
                    self.problems
                        .push(format!("{path}: {} entries != {}", x.len(), y.len()));
                    return;
                }
                for (i, (p, q)) in x.iter().zip(y).enumerate() {
                    self.same(&format!("{path}[{i}]"), p, q, unordered);
                }
            }
            (Value::Number(p), Value::Number(q)) if p != q => {
                let (p, q) = (p.as_f64().unwrap(), q.as_f64().unwrap());
                if (p - q).abs() <= 4.5e-16 * p.abs().max(q.abs()) {
                    self.float_drift += 1;
                    let field = path.rsplit('.').next().unwrap().to_string();
                    *self.drifted.entry(field).or_default() += 1;
                } else {
                    self.problems.push(format!("{path}: {p} != {q}"));
                }
            }
            _ if a != b => self.problems.push(format!("{path}: {a} != {b}")),
            _ => {}
        }
    }

    fn finish(&self, what: &str) {
        for p in self.problems.iter().take(25) {
            eprintln!("  {p}");
        }
        assert!(
            self.problems.is_empty(),
            "{what}: {} differences (first above)",
            self.problems.len()
        );
        if self.float_drift > 0 {
            eprintln!(
                "{what}: {} floats differ in the last digits: {:?}",
                self.float_drift, self.drifted
            );
        }
    }
}

// -- tests -----------------------------------------------------------------------------------

#[test]
fn worn_matches_python() {
    if skip() {
        return;
    }
    let g = golden("archetypes");
    assert_eq!(
        world().1.len() as u64,
        g["characters"].as_u64().unwrap(),
        "characters"
    );
    assert_eq!(
        worn().len() as u64,
        g["worn"].as_u64().unwrap(),
        "worn items"
    );
}

#[test]
fn archetypes_match_python() {
    if skip() {
        return;
    }
    let g = golden("archetypes");
    let want = g["gear"].as_array().unwrap();
    let got = found();
    assert_eq!(got.len(), want.len(), "archetype count");
    let mut diff = Diff::default();
    for (i, (a, w)) in got.iter().zip(want).enumerate() {
        // Python builds `others` and `tiers` by iterating a set, so their key order is arbitrary there.
        let mut a = archetype_json(a);
        let mut w = w.clone();
        for v in [&mut a, &mut w] {
            let o = v.as_object_mut().unwrap();
            for f in ["others", "tiers"] {
                let sorted: Map<String, Value> = {
                    let m = o[f].as_object().unwrap();
                    let mut keys: Vec<&String> = m.keys().collect();
                    keys.sort();
                    keys.into_iter()
                        .map(|k| (k.clone(), m[k].clone()))
                        .collect()
                };
                o.insert(f.into(), Value::Object(sorted));
            }
        }
        diff.same(&format!("gear[{i}]"), &a, &w, &[]);
    }
    diff.finish("archetypes");
    eprintln!("archetypes: all {} agree on every field", want.len());
}

#[test]
fn cluster_archetypes_match_python() {
    if skip() {
        return;
    }
    let (_, chars) = world();
    let g = golden("archetypes");
    let want = g["clusters"].as_array().unwrap();
    let t = Instant::now();
    let wc = demand::wear_clusters(chars, None);
    let t_wear = t.elapsed();
    let t = Instant::now();
    let got = demand::cluster_archetypes(&wc, 5, 0.8);
    eprintln!(
        "wear_clusters: {} in {t_wear:.2?}; cluster_archetypes: {} in {:.2?}",
        wc.len(),
        got.len(),
        t.elapsed()
    );
    assert_eq!(got.len(), want.len(), "cluster archetype count");
    let mut diff = Diff::default();
    for (i, (a, w)) in got.iter().zip(want).enumerate() {
        diff.same(&format!("clusters[{i}]"), &cluster_json(a), w, &[]);
    }
    diff.finish("cluster archetypes");
    eprintln!(
        "cluster archetypes: all {} agree on every field",
        want.len()
    );
}

#[test]
fn archetype_match_matches_python() {
    if skip() {
        return;
    }
    let (db, _) = world();
    let rows = golden("gear");
    let rows = rows.as_array().unwrap();
    let mut diff = Diff::default();
    let (mut none, mut some) = (0, 0);
    for (i, (a, row)) in found().iter().zip(rows).enumerate() {
        assert_eq!(
            (a.group.as_str(), &a.families),
            (
                row["group"].as_str().unwrap(),
                &row["families"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|f| f.as_str().unwrap().to_string())
                    .collect()
            ),
            "row {i}"
        );
        let m = demand::archetype_match(db, a);
        if m.is_some() {
            some += 1;
        } else {
            none += 1;
        }
        let mut got = m.as_ref().map_or(Value::Null, match_json);
        let mut want = row["match"].clone();
        // `finish.families` starts from a frozenset of fillers: its order is arbitrary in Python.
        for v in [&mut got, &mut want].into_iter().filter(|v| v.is_object()) {
            // A tag with two influence suffixes is spelled in Python's set order: compare their number only.
            const INF: [&str; 6] = [
                "shaper",
                "elder",
                "crusader",
                "eyrie",
                "basilisk",
                "adjudicator",
            ];
            let tags = v["tags"].as_array_mut().unwrap();
            let multi = |t: &Value| {
                let parts: Vec<&str> = t.as_str().unwrap().rsplitn(3, '_').collect();
                parts.len() == 3 && INF.contains(&parts[0]) && INF.contains(&parts[1])
            };
            let n = tags.iter().filter(|t| multi(t)).count();
            tags.retain(|t| !multi(t));
            v["multi_influence_tags"] = n.into();
            if let Some(Value::Object(f)) = v.pointer_mut("/finish/families") {
                let mut e: Vec<(String, Value)> = std::mem::take(f).into_iter().collect();
                e.sort_by(|a, b| a.0.cmp(&b.0));
                *f = e.into_iter().collect();
            }
        }
        diff.same(&format!("match[{i}]"), &got, &want, &[]);
    }
    diff.finish("archetype_match");
    eprintln!(
        "archetype_match: all {} agree ({some} items, {none} none)",
        some + none
    );
}

#[test]
fn niche_and_usual_tier() {
    let mut a = Archetype::new("g", vec!["x".into()], 25, 40);
    a.lift = 1.0;
    assert_eq!(a.niche(), 0.0);
    a.lift = std::f64::consts::E;
    assert!((a.niche() - 5.0).abs() < 1e-12);
    for t in [1, 1, 2, 3] {
        a.tiers.entry("x".into()).or_default().add(t);
    }
    assert_eq!(a.usual_tier("x", 0.5), Some(1));
    assert_eq!(a.usual_tier("x", 0.75), Some(2));
    assert_eq!(a.usual_tier("x", 1.0), Some(3));
    assert_eq!(a.usual_tier("y", 0.5), None);
}

#[test]
fn counter_most_common_keeps_insertion_order_on_ties() {
    let mut c = Counter::new();
    for k in ["b", "a", "c", "a", "c"] {
        c.add(k.to_string());
    }
    c.add("d".to_string());
    assert_eq!(
        c.most_common()
            .iter()
            .map(|(k, n)| (k.as_str(), *n))
            .collect::<Vec<_>>(),
        [("a", 2), ("c", 2), ("b", 1), ("d", 1)]
    );
}

#[test]
fn family_stats_match_python() {
    if skip() {
        return;
    }
    let (db, _) = world();
    let path = repo_root().join("rust/goldens/demand/family_stats.json.gz");
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&path).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    let want: Value = serde_json::from_str(&text).unwrap();
    let t = Instant::now();
    let got = demand::family_stats(db, worn(), 10);
    eprintln!("family_stats: {} in {:.2?}", got.len(), t.elapsed());
    let got: Vec<Value> = got
        .iter()
        .map(|s| {
            let at = |f: &dyn Fn(i64) -> f64| Value::Object([1, 2, 3].map(|t| (t.to_string(), json!(f(t)))).into_iter().collect());
            json!({
                "group": s.group, "family": s.family, "side": s.side, "text": s.text, "characters": s.characters,
                "tiers": counter_s(&s.tiers), "natural": Value::Object(s.natural.iter().map(|(t, p)| (t.to_string(), json!(p))).collect()),
                "lift": at(&|t| s.lift(t)), "share": at(&|t| s.share_at_or_above(t)),
            })
        })
        .collect();
    let mut diff = Diff::default();
    diff.same("family_stats", &Value::Array(got), &want, &[]);
    diff.finish("family_stats");
}
