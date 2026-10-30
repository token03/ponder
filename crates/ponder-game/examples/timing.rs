//! Load and match timings: `cargo run -p ponder-game --release --example timing` from the repo root.

use std::collections::BTreeSet;
use std::time::Instant;

use ponder_game::mods::{IdMods, ModDB};
use ponder_game::{data, item};

fn main() {
    let t = Instant::now();
    let mods = data::mods().expect("repoe data (run from the repo root or set PONDER_DATA)");
    println!("mods.json    {:>7.0} ms ({} mods)", t.elapsed().as_secs_f64() * 1e3, mods.len());
    let t = Instant::now();
    let bases = data::base_items().unwrap();
    println!("base_items   {:>7.0} ms ({} bases)", t.elapsed().as_secs_f64() * 1e3, bases.len());
    let t = Instant::now();
    let _ = data::crafting_bench_options().unwrap();
    let _ = data::stat_translations().unwrap();
    println!("bench+stats  {:>7.0} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let db = ModDB::load().unwrap();
    println!("ModDB::load  {:>7.0} ms (indexes, files cached)", t.elapsed().as_secs_f64() * 1e3);

    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/expected/items");
    let read = |name: &str| -> serde_json::Value {
        let f = std::fs::File::open(dir.join(format!("{name}.json.gz"))).unwrap();
        serde_json::from_reader(flate2::read::GzDecoder::new(f)).unwrap()
    };
    let ladder = read("ladder");
    let items = ladder["items"].as_array().unwrap();
    let inputs: Vec<_> = items
        .iter()
        .map(|e| {
            let ids = |k: &str| -> Vec<String> { e["mods"][k].as_array().unwrap().iter().map(|s| s.as_str().unwrap().into()).collect() };
            let inf: BTreeSet<String> = e["influences"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().into()).collect();
            (
                e["base"].as_str().unwrap().to_string(),
                IdMods { explicit: ids("explicit"), fractured: ids("fractured"), crafted: ids("crafted") },
                e["ilvl"].as_i64(),
                inf,
            )
        })
        .collect();
    let t = Instant::now();
    let mut n = 0;
    for (b, m, l, i) in &inputs {
        n += db.match_ids(b, m, *l, i, "RARE", "", &Default::default()).mods.len();
    }
    println!("match_ids x{} {:>7.0} ms ({n} mods)", inputs.len(), t.elapsed().as_secs_f64() * 1e3);

    let synth = read("synth");
    let parsed: Vec<_> = synth.as_array().unwrap().iter().map(|e| item::parse_item(e["text"].as_str().unwrap()).unwrap()).collect();
    let t = Instant::now();
    let mut n = 0;
    for it in &parsed {
        n += db.match_item(it).mods.len();
    }
    println!("match x{}    {:>7.0} ms ({n} mods)", parsed.len(), t.elapsed().as_secs_f64() * 1e3);
}
