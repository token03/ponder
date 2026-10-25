use std::time::Instant;

use ponder::pipeline::{self, Context, GearOpts};
use ponder::{demand, pipeline::LadderOpts};
use ponder_game::mods::ModDB;

fn main() {
    let started = Instant::now();
    let db = ModDB::load().unwrap();
    let mut cx = Context::real();
    if let Some(path) = std::env::args().skip_while(|arg| arg != "--plan-cache").nth(1) {
        cx.plans = pipeline::PlanCache::new(path);
    }
    if std::env::args().any(|a| a == "--cold") {
        cx.plans = pipeline::PlanCache::disabled();
    }
    let (table, _) = cx.prices.load().unwrap();
    if let Some(needle) = std::env::args().skip_while(|arg| arg != "--dump-case").nth(1) {
        let characters = cx.ladder.cached(None).unwrap();
        let worn = demand::wear(&db, &characters, None);
        std::fs::create_dir_all("target/craft-cases").unwrap();
        for a in demand::archetypes(&worn, 5, 4, 2, 0.7).into_iter().filter(|a| a.families.iter().any(|f| f.contains(&needle))) {
            let Some(m) = demand::archetype_match(&db, &a) else { continue };
            let Ok(e) = pipeline::estimate_gear(&cx, &db, &m, &table, "Standard", &GearOpts { solve: false, ..Default::default() }) else {
                continue;
            };
            let p = e.problem.as_ref().unwrap();
            let key = p.fingerprint();
            std::fs::write(format!("target/craft-cases/{key}.json"), serde_json::to_vec(p).unwrap()).unwrap();
            println!("{key} {} {}", e.base, e.targets.join(" + "));
        }
        return;
    }
    if let Some(path) = std::env::args().skip_while(|arg| arg != "--solve-case").nth(1) {
        let p: ponder_core::Problem = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let plan = ponder_core::solve(&p, &table, &cx.settings);
        println!("{:.3}s {:?}", started.elapsed().as_secs_f64(), plan.map(|p| (p.cost, p.states, p.usage)));
        return;
    }
    if std::env::args().any(|a| a == "--builds") {
        let characters = cx.ladder.cached(None).unwrap();
        let worn = demand::wear(&db, &characters, None);
        for a in demand::archetypes(&worn, 5, 4, 2, 0.7).into_iter().take(30) {
            let Some(m) = demand::archetype_match(&db, &a) else {
                continue;
            };
            let t = Instant::now();
            let Ok(mut e) = pipeline::estimate_gear(&cx, &db, &m, &table, "Standard", &GearOpts { solve: false, ..Default::default() })
            else {
                continue;
            };
            let built = t.elapsed().as_secs_f64();
            pipeline::add_fractures(&cx, &mut e, "Standard", Some(&table), false);
            println!(
                "{} build={built:.3}s fracture={:.3}s unveils={}",
                a.group,
                t.elapsed().as_secs_f64() - built,
                e.problem.as_ref().unwrap().unveils.len()
            );
        }
    } else {
        let opts = if std::env::args().any(|a| a == "--full") {
            LadderOpts::default()
        } else {
            LadderOpts { gear_top: 12, per_group: 0, niche: 0, cluster_top: 12, ..Default::default() }
        };
        let say = |message: &str| {
            if std::env::args().any(|arg| arg == "--profile") || (!message.starts_with("solved") && !message.contains(':')) {
                eprintln!("{:.3}s {message}", started.elapsed().as_secs_f64());
            }
        };
        let report = pipeline::ladder(&cx, &db, None, &opts, Some(&say)).unwrap();
        println!(
            "estimates={} direct_failures={} unresolved={} elapsed={:.3}s",
            report.estimates.len(),
            report.estimates.iter().filter(|e| e.error.is_some()).count(),
            report.estimates.iter().filter(|e| e.cheapest().is_none()).count(),
            started.elapsed().as_secs_f64()
        );
        let mut routes = std::collections::BTreeMap::new();
        for estimate in &report.estimates {
            let route = match estimate.cheapest_route() {
                Some(pipeline::CraftRoute::Craft) => "direct",
                Some(pipeline::CraftRoute::Fracture(_)) => "fracture",
                Some(pipeline::CraftRoute::Recomb) => "recombination",
                Some(pipeline::CraftRoute::Awakener) => "awakener",
                Some(pipeline::CraftRoute::Protected) => "protected",
                None => "unresolved",
            };
            *routes.entry(route).or_insert(0usize) += 1;
        }
        println!("routes={routes:?}");
        if let Some(path) = std::env::args().skip_while(|arg| arg != "--snapshot").nth(1) {
            let estimates: Vec<_> = report
                .estimates
                .iter()
                .map(|estimate| {
                    serde_json::json!({
                        "key": estimate.problem.as_ref().map(|problem| problem.fingerprint().to_string()),
                        "title": estimate.title,
                        "base": estimate.base,
                        "cost": estimate.cheapest(),
                        "route": format!("{:?}", estimate.cheapest_route()),
                        "direct_lower_bound": estimate.craft_lower_bound,
                        "pruned_fractures": estimate.fractures.iter().filter(|fracture| fracture.craft_lower_bound.is_some()).count(),
                    })
                })
                .collect();
            std::fs::write(path, serde_json::to_vec_pretty(&estimates).unwrap()).unwrap();
        }
        if std::env::args().any(|arg| arg == "--pruning-profile") {
            let mut total = 0;
            let mut dominated = 0;
            let mut certified = 0;
            let mut bounded_recipes = 0;
            let mut dominated_direct = 0;
            for e in &report.estimates {
                let upper = e.plan.as_ref().map_or(f64::INFINITY, |p| p.bounds.upper + e.bench_cost() + e.base_price.map_or(0.0, |b| b.0));
                if let Some(upper) = e.upper_bound() {
                    bounded_recipes += 1;
                    dominated_direct += usize::from(e.plan.as_ref().is_some_and(|p| p.bounds.lower + e.bench_cost() > upper));
                }
                for f in &e.fractures {
                    total += 1;
                    if let Some(base) = f.base_cost() {
                        dominated += usize::from(e.cheapest().is_some_and(|cost| base + f.bench_cost >= cost));
                        certified += usize::from(base + f.bench_cost >= upper);
                    }
                }
            }
            println!("fractures={total} nominal_dominated={dominated} certified_by_direct={certified}");
            println!("bounded_recipes={bounded_recipes} dominated_direct={dominated_direct}");
        }
        for estimate in &report.estimates {
            if let Some(error) = &estimate.error {
                eprintln!(
                    "{} {}: {error}; route={:?}; targets={}",
                    estimate.title,
                    estimate.base,
                    estimate.cheapest_route(),
                    estimate.targets.join(" + ")
                );
            }
        }
    }
}
