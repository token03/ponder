//! Times `solve` on every golden problem: `cargo run --release -p ponder-core --example bench [reps]`.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::time::Instant;

use flate2::read::GzDecoder;
use ponder_core::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Golden {
    name: String,
    problem: Problem,
    prices: Prices,
    chaos_per_hour: f64,
}

fn main() {
    let reps: usize = std::env::args().nth(1).and_then(|r| r.parse().ok()).unwrap_or(5);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../goldens/solver");
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    let mut total = 0.0;
    println!("{:<34} {:>7} {:>10}", "golden", "states", "ms/solve");
    for path in files {
        let mut text = String::new();
        GzDecoder::new(File::open(&path).unwrap()).read_to_string(&mut text).unwrap();
        let g: Golden = serde_json::from_str(&text).unwrap();
        let settings = Settings { chaos_per_hour: g.chaos_per_hour };
        let mut best = f64::INFINITY;
        let mut states = 0;
        for _ in 0..reps {
            let mut p = g.problem.clone(); // fresh caches, as a first solve in Python
            let t = Instant::now();
            states = solve(&mut p, &g.prices, &settings).map_or(0, |plan| plan.states);
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        total += best;
        println!("{:<34} {:>7} {:>10.2}", g.name, states, best);
    }
    println!("{:<34} {:>7} {:>10.2}", "total", "", total);
}
