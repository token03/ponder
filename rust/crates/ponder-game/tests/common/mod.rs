//! Shared test helpers: repo paths, goldens, the data skip, and Rust items/matches as the golden JSON shape.
#![allow(dead_code)]

use std::io::Read;
use std::path::PathBuf;
use std::sync::{Once, OnceLock};

use ponder_game::data;
use ponder_game::item::{Item, ModLine};
use ponder_game::mods::{ItemMatch, ModDB};
use serde_json::{Value, json};

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

pub fn fixtures() -> PathBuf {
    repo_root().join("tests/fixtures")
}

/// Point the data directory at the repo's cache unless the caller already chose one.
pub fn setup() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("PONDER_DATA").is_none() {
            // SAFETY: every test calls `setup` before touching the environment, and the Once
            // blocks them all until this has run.
            unsafe { std::env::set_var("PONDER_DATA", repo_root().join(".cache/repoe")) };
        }
    });
}

/// True (with a note) when the repoe data is missing: the test should return early.
pub fn skip_without_data() -> bool {
    setup();
    if data::available(None) {
        return false;
    }
    eprintln!(
        "skipping: repoe data not found in {}; run `ponder data fetch`",
        data::data_dir().display()
    );
    true
}

pub fn db() -> &'static ModDB {
    static DB: OnceLock<ModDB> = OnceLock::new();
    DB.get_or_init(|| ModDB::load().expect("repoe data"))
}

pub fn golden(name: &str) -> Value {
    // PONDER_GOLDENS points at a bigger set made with PONDER_GOLDEN_SAMPLE / PONDER_GOLDEN_SYNTH.
    let dir = std::env::var_os("PONDER_GOLDENS")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("rust/goldens/items"));
    let path = dir.join(format!("{name}.json.gz"));
    let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut text = String::new();
    flate2::read::GzDecoder::new(file)
        .read_to_string(&mut text)
        .unwrap();
    serde_json::from_str(&text).unwrap()
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
