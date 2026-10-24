//! Times `solve` on every fixture problem: `cargo run --release -p ponder-core --example bench [reps]`.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::time::Instant;

use flate2::read::GzDecoder;
use ponder_core::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    name: String,
    problem: Problem,
    prices: Prices,
    chaos_per_hour: f64,
}

fn main() {
    let reps: usize = std::env::args().nth(1).and_then(|r| r.parse().ok()).unwrap_or(5);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/solver");
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    let mut total = [0.0; 2];
    println!("{:<34} {:>7} {:>12} {:>12}", "problem", "states", "split ms", "dense ms");
    for path in files {
        let mut text = String::new();
        GzDecoder::new(File::open(&path).unwrap()).read_to_string(&mut text).unwrap();
        let g: Fixture = serde_json::from_str(&text).unwrap();
        let settings = Settings { chaos_per_hour: g.chaos_per_hour };
        let mut best = [f64::INFINITY; 2];
        let mut states = 0;
        for (b, backend) in [Backend::Split, Backend::Dense].into_iter().enumerate() {
            for _ in 0..reps {
                let p = g.problem.to_spec().compile();
                let t = Instant::now();
                states = solve_with(&p, &g.prices, &settings, SolveOptions { backend, ..Default::default() }).map_or(0, |plan| plan.states);
                best[b] = best[b].min(t.elapsed().as_secs_f64() * 1000.0);
            }
            total[b] += best[b];
        }
        println!("{:<34} {:>7} {:>12.2} {:>12.2}", g.name, states, best[0], best[1]);
    }
    println!("{:<34} {:>7} {:>12.2} {:>12.2}", "total", "", total[0], total[1]);
}
