//! The solver against the simulator on real problems (fixtures/solver): playing the plan out
//! on real mods, with exact group exclusion instead of lumped weights, costs about what the
//! solver says.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

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

fn fixtures() -> Vec<Fixture> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/solver");
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    files
        .iter()
        .map(|p| {
            let mut text = String::new();
            GzDecoder::new(File::open(p).unwrap()).read_to_string(&mut text).unwrap();
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
        })
        .collect()
}

#[test]
fn plans_cost_what_the_simulator_says() {
    let all = fixtures();
    assert!(all.len() >= 20, "found only {} fixtures", all.len());
    let mut bad = Vec::new();
    // The simulator starts from a fresh base: a half-done `begin` item has no real mods to replay.
    for f in all.into_iter().filter(|f| f.problem.begin.is_none()) {
        let settings = Settings { chaos_per_hour: f.chaos_per_hour };
        let p = f.problem.clone();
        let plan = match solve(&p, &f.prices, &settings) {
            Ok(plan) => plan,
            Err(e) => {
                bad.push(format!("{}: {e}", f.name));
                continue;
            }
        };
        // Batches with their own seeds give the replay's standard error.
        let batches: Vec<f64> = (0..6).map(|seed| simulate(&p, &plan, &f.prices, &settings, 500, 11 + seed, 100_000)).collect();
        let replay = batches.iter().sum::<f64>() / batches.len() as f64;
        let se = (batches.iter().map(|b| (b - replay).powi(2)).sum::<f64>() / (batches.len() * (batches.len() - 1)) as f64).sqrt();
        let off = (replay - plan.cost) / plan.cost;
        eprintln!(
            "{:<34} plan {:>10.1}  replay {:>10.1} ±{:>6.1}  {:>+6.1}%{}",
            f.name,
            plan.cost,
            replay,
            se,
            100.0 * off,
            if plan.approximate { "  (lumped)" } else { "" }
        );
        if off.abs() > TOLERANCE && (replay - plan.cost).abs() > 4.0 * se {
            bad.push(format!("{}: replay {replay:.1} vs plan {:.1} ({:+.1}%)", f.name, plan.cost, 100.0 * off));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Rolls are exact; what is left is the lumped state not knowing which groups its other mods
/// are, so exalts and annulments after a roll use expected weights.
const TOLERANCE: f64 = 0.06;

#[test]
fn solving_twice_gives_the_same_plan() {
    let f = fixtures().into_iter().find(|f| f.name == "gear_begin").unwrap();
    let settings = Settings { chaos_per_hour: f.chaos_per_hour };
    let p = f.problem.clone();
    let a = solve(&p, &f.prices, &settings).unwrap();
    let b = solve(&p, &f.prices, &settings).unwrap();
    assert!((a.cost - b.cost).abs() <= 1e-9 * a.cost);
}

#[test]
fn focused_search_matches_eager_search_on_crafting_fixtures() {
    for f in fixtures() {
        let settings = Settings { chaos_per_hour: f.chaos_per_hour };
        let eager = solve_with(
            &f.problem.fresh_transition_cache(),
            &f.prices,
            &settings,
            SolveOptions { algorithm: search::Algorithm::Eager, ..Default::default() },
        )
        .unwrap();
        for algorithm in [search::Algorithm::Ilao, search::Algorithm::CgIlao] {
            let focused =
                solve_with(&f.problem.fresh_transition_cache(), &f.prices, &settings, SolveOptions { algorithm, ..Default::default() })
                    .unwrap_or_else(|error| panic!("{} {algorithm:?}: {error}", f.name));
            assert!(
                (focused.cost - eager.cost).abs() <= 1e-7 * eager.cost.abs().max(1.0),
                "{} {algorithm:?}: focused={} eager={}",
                f.name,
                focused.cost,
                eager.cost
            );
        }
    }
}

#[test]
fn backends_agree() {
    for f in fixtures().into_iter().filter(|f| f.name.starts_with("toy") || f.name.starts_with("cluster")) {
        let settings = Settings { chaos_per_hour: f.chaos_per_hour };
        let costs: Vec<f64> = [Backend::Dense, Backend::Split]
            .into_iter()
            .map(|b| solve_with(&f.problem, &f.prices, &settings, SolveOptions { backend: b, ..Default::default() }).unwrap().cost)
            .collect();
        assert!((costs[0] - costs[1]).abs() <= 1e-7 * costs[0], "{}: {costs:?}", f.name);
    }
}
