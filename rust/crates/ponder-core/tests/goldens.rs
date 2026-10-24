//! The Python solver's answers (scripts/golden/solver.py) reproduced state by state.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use flate2::read::GzDecoder;
use ponder_core::*;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Golden {
    name: String,
    problem: Problem,
    prices: Prices,
    chaos_per_hour: f64,
    plan: Value,
    steps: Vec<Value>,
    self_fracture: Vec<Option<SelfFracture>>,
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../goldens/solver")
}

fn load() -> Vec<Golden> {
    let mut files: Vec<_> =
        std::fs::read_dir(dir()).unwrap().map(|e| e.unwrap().path()).filter(|p| p.to_string_lossy().ends_with(".json.gz")).collect();
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

fn close(a: f64, b: f64, rel: f64, abs: f64) -> bool {
    (a - b).abs() <= abs.max(rel * a.abs().max(b.abs()))
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

fn state(v: &Value) -> State {
    serde_json::from_value(v.clone()).unwrap()
}

fn check(g: &Golden, backend: Backend, bad: &mut Vec<String>) {
    let settings = Settings { chaos_per_hour: g.chaos_per_hour };
    let mut fail = |what: String| bad.push(format!("{} ({backend:?}): {what}", g.name));

    // Fractured bases are priced before the solve rewrites the action list.
    for (i, want) in g.self_fracture.iter().enumerate() {
        let got = self_fracture(&g.problem, i, &g.prices, &settings);
        match (got, want) {
            (None, None) => {}
            (Some(a), Some(b)) => {
                let same = a.target == b.target
                    && a.max_mods == b.max_mods
                    && close(a.hit, b.hit, 1e-7, 1e-12)
                    && close(a.fracture_chance, b.fracture_chance, 1e-7, 1e-12)
                    && close(a.cost, b.cost, 1e-7, 1e-9)
                    && close(a.hours, b.hours, 1e-7, 1e-12);
                if !same {
                    fail(format!("self_fracture[{i}] {a:?} != {b:?}"));
                }
            }
            (a, b) => fail(format!("self_fracture[{i}] {a:?} != {b:?}")),
        }
    }

    let mut problem = g.problem.clone();
    let solved = solve_with(&mut problem, &g.prices, &settings, 60, backend);
    if let Some(e) = g.plan.get("error") {
        if solved.is_ok() {
            fail(format!("expected error {e}, got a plan"));
        }
        return;
    }
    let plan = match solved {
        Ok(p) => p,
        Err(e) => return fail(format!("solve failed: {e}")),
    };
    let want = &g.plan;
    if !close(plan.cost, num(&want["cost"]), 1e-7, 1e-9) {
        fail(format!("cost {} != {}", plan.cost, want["cost"]));
    }
    if !close(plan.hours, num(&want["hours"]), 1e-7, 1e-12) {
        fail(format!("hours {} != {}", plan.hours, want["hours"]));
    }
    if !close(plan.currency(&settings), num(&want["currency"]), 1e-7, 1e-9) {
        fail(format!("currency {} != {}", plan.currency(&settings), want["currency"]));
    }
    if plan.states != want["states"].as_u64().unwrap() as usize {
        fail(format!("states {} != {}", plan.states, want["states"]));
    }
    let after: Vec<&str> = want["actions_after"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    let ours: Vec<&str> = problem.actions.iter().map(|a| a.name.as_str()).collect();
    if after != ours {
        fail(format!("actions after solve {ours:?} != {after:?}"));
    }
    let unpriced: Vec<&str> = want["unpriced"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    if plan.unpriced != unpriced {
        fail(format!("unpriced {:?} != {unpriced:?}", plan.unpriced));
    }
    for (name, v) in want["usage"].as_object().unwrap() {
        match plan.usage.get(name) {
            Some(u) if close(*u, num(v), 1e-5, 1e-6) => {}
            u => fail(format!("usage[{name}] {u:?} != {v}")),
        }
    }
    if plan.usage.len() != want["usage"].as_object().unwrap().len() {
        fail("usage has different actions".into());
    }
    let values = want["values"].as_array().unwrap();
    if plan.values.len() != values.len() {
        fail(format!("{} values != {}", plan.values.len(), values.len()));
    }
    let mut worst = 0.0f64;
    for pair in values {
        let s = state(&pair[0]);
        match plan.values.get(&s) {
            Some(v) if close(*v, num(&pair[1]), 1e-6, 1e-9) => {}
            v => {
                worst += 1.0;
                if worst < 4.0 {
                    fail(format!("value at {s:?}: {v:?} != {}", pair[1]));
                }
            }
        }
    }
    for pair in want["visits"].as_array().unwrap() {
        let s = state(&pair[0]);
        match plan.visits.get(&s) {
            Some(v) if close(*v, num(&pair[1]), 1e-5, 1e-6) => {}
            v => fail(format!("visits at {s:?}: {v:?} != {}", pair[1])),
        }
    }

    // The golden policy may break ties differently; it must cost the same.
    let policy: StateMap<String> =
        want["policy"].as_array().unwrap().iter().map(|p| (state(&p[0]), p[1].as_str().unwrap().to_string())).collect();
    match evaluate_with(&problem, &policy, &g.prices, &settings, backend) {
        Ok(c) if close(c, plan.cost, 1e-7, 1e-9) => {}
        c => fail(format!("evaluate(golden policy) {c:?} != cost {}", plan.cost)),
    }

    // Steps tied to the last bit (mirror-image targets) may be numbered either way round, so
    // compare them by what they are: rarity and targets on, and exits by what they lead to.
    let steps = steps(&problem, &plan, 0.02);
    if steps.len() != g.steps.len() {
        return fail(format!("{} steps != {}", steps.len(), g.steps.len()));
    }
    let id = |rarity: u64, have: &[&str]| format!("{rarity}:{}", have.join("|"));
    let want_have = |w: &Value| -> Vec<String> {
        let mut h: Vec<String> = w["have"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        h.sort();
        h
    };
    let ids_ours: HashMap<usize, String> = steps
        .iter()
        .map(|s| {
            let mut h = s.have.clone();
            h.sort();
            (s.number, id(s.rarity as u64, &h.iter().map(String::as_str).collect::<Vec<_>>()))
        })
        .collect();
    let ids_want: HashMap<usize, String> = g
        .steps
        .iter()
        .map(|w| {
            let h = want_have(w);
            (w["number"].as_u64().unwrap() as usize, id(w["rarity"].as_u64().unwrap(), &h.iter().map(String::as_str).collect::<Vec<_>>()))
        })
        .collect();
    let want_by_id: HashMap<&String, &Value> = g.steps.iter().map(|w| (&ids_want[&(w["number"].as_u64().unwrap() as usize)], w)).collect();
    for st in &steps {
        let sid = &ids_ours[&st.number];
        let Some(w) = want_by_id.get(sid) else {
            fail(format!("step {sid} not in the golden"));
            continue;
        };
        if !close(st.reached, num(&w["reached"]), 1e-6, 1e-9) || !close(st.uses, num(&w["uses"]), 1e-6, 1e-9) {
            fail(format!("step {sid} reached/uses {}/{} != {}/{}", st.reached, st.uses, w["reached"], w["uses"]));
        }
        let key = |kind: &str, what: &str, to: Option<&String>| format!("{kind}/{what}/{}", to.map_or("-", |s| s.as_str()));
        let mut ours: Vec<(String, f64)> =
            st.exits.iter().map(|x| (key(&x.kind, &x.what, x.to.map(|t| &ids_ours[&t])), x.chance)).collect();
        let mut theirs: Vec<(String, f64)> = w["exits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| {
                (
                    key(x["kind"].as_str().unwrap(), x["what"].as_str().unwrap(), x["to"].as_u64().map(|t| &ids_want[&(t as usize)])),
                    num(&x["chance"]),
                )
            })
            .collect();
        ours.sort_by(|a, b| a.0.cmp(&b.0));
        theirs.sort_by(|a, b| a.0.cmp(&b.0));
        if ours.len() != theirs.len() || ours.iter().zip(&theirs).any(|(a, b)| a.0 != b.0 || !close(a.1, b.1, 1e-6, 1e-9)) {
            fail(format!("step {sid} exits {ours:?} != {theirs:?}"));
        }
    }
}

#[test]
fn rust_solver_matches_python_goldens() {
    let goldens = load();
    assert!(goldens.len() >= 20, "found only {} goldens", goldens.len());
    let mut bad = Vec::new();
    for g in &goldens {
        // The dense solver is the reference; the split one is what runs by default on big systems.
        check(g, Backend::Split, &mut bad);
        if g.plan.get("states").and_then(Value::as_u64).unwrap_or(0) <= 500 {
            check(g, Backend::Dense, &mut bad);
        }
    }
    assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn solving_twice_gives_the_same_plan() {
    // `solve` rewrites the action list; a second solve on the result must agree.
    let g = load().into_iter().find(|g| g.name == "gear_begin").unwrap();
    let settings = Settings { chaos_per_hour: g.chaos_per_hour };
    let mut p = g.problem.clone();
    let a = solve(&mut p, &g.prices, &settings).unwrap();
    let b = solve(&mut p, &g.prices, &settings).unwrap();
    assert!(close(a.cost, b.cost, 1e-9, 1e-12));
    let _: HashMap<_, _> = a.usage.clone();
}

#[test]
fn replaying_the_plan_on_real_mods_agrees_with_the_solver() {
    // Exact group exclusion instead of lumping: the numbers stay close. (Big gear pools drift further,
    // e.g. gear_fractured replays ~13% under its plan in Python too: the lumped group weights are approximate.)
    for (name, tolerance) in [("cluster_0", 0.08), ("cluster_1", 0.08), ("toy_three", 0.08), ("toy_harvest_fossil", 0.08)] {
        let g = load().into_iter().find(|g| g.name == name).unwrap();
        let settings = Settings { chaos_per_hour: g.chaos_per_hour };
        let mut p = g.problem.clone();
        let plan = solve(&mut p, &g.prices, &settings).unwrap();
        let replay = simulate(&p, &plan, &g.prices, &settings, 3000, 11, 100_000);
        assert!(close(replay, plan.cost, tolerance, 0.0), "{name}: replay {replay} vs plan {}", plan.cost);
    }
}
