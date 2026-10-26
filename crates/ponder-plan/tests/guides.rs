//! Guide acceptance (SPEC §8.3): for every guide whose mechanics exist, ponder's best plan costs
//! at most the guide's recipe + 5%, and simulating the recipe on real items agrees with its
//! exact cost. Prints a table with `-- --nocapture`.

use std::collections::HashMap;
use std::path::PathBuf;

use ponder_plan::craft::{Craft, Node};
use ponder_plan::market::Market;
use ponder_plan::recipe::{Following, Position, Recipe};
use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, GroupId, Influences, Kind};
use ponder_rules::guide::Guide;
use ponder_rules::item::Item;
use ponder_rules::lens::Lens;
use ponder_rules::ops::currency::Currency;
use ponder_rules::ops::{Ctx, Operation, parse};
use ponder_rules::rng::Rng;
use ponder_rules::target::Target;
use ponder_solve::solve;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Outcome {
    guide: f64,
    best: f64,
    simulated: (f64, f64),
    states: usize,
    followed: ponder_plan::craft::Followed,
    start: String,
    /// Seconds when the recipe was checked, the best plan solved, and followed.
    times: Vec<f64>,
}

fn check(game: &Game, market: &Market, guide: &Guide) -> Result<Outcome, String> {
    let assumptions = Assumptions::default();
    let ctx = Ctx { game, assumptions: &assumptions };
    let base = game.base(&guide.base).ok_or("unknown base")?;
    let kind = Kind { base, ilvl: guide.ilvl.unwrap_or(100), influences: Influences::of(&guide.influences) };
    let class = game.bases[base.0 as usize].class.clone();

    let names: Vec<String> = ["goal".to_string()].into_iter().chain(guide.stages.keys().cloned()).collect();
    let mut targets = Vec::new();
    let mut index = HashMap::new();
    for name in &names {
        let goal = guide.target(name).unwrap();
        if goal.base.as_deref() != Some(&guide.base) {
            return Err(format!("stage {name} is another base"));
        }
        index.insert(name.clone(), targets.len());
        targets.push(Target::compile(game, &goal)?);
    }

    // Bench "any" crafts block groups the goal does not want.
    let avoid: Vec<GroupId> =
        targets[index["goal"]].needs.iter().flat_map(|n| n.ok.iter()).flat_map(|m| game.def(*m).groups.iter().copied()).collect();
    let mut ops: Vec<Box<dyn Operation>> = Currency::ALL.iter().map(|c| Box::new(*c) as Box<dyn Operation>).collect();
    ops.extend(parse(game, &class, "bench:remove-crafted", &avoid));
    for op in guide.steps.iter().flat_map(|s| &s.ops) {
        if op.starts_with("buy:") || ops.iter().any(|o| o.name() == *op) {
            continue;
        }
        ops.push(parse(game, &class, op, &avoid).ok_or_else(|| format!("needs {op}"))?);
    }
    let refs: Vec<&dyn Operation> = ops.iter().map(|o| o.as_ref()).collect();
    let lens = Lens::new(game, kind, targets.clone(), &refs);

    // Starting points: a plain base and any stage the guide buys, each at its price.
    let mut base = Item::new(kind);
    base.quality = guide.quality.unwrap_or(0);
    let mut starts = vec![("base".to_string(), market.base, base)];
    for stage in guide.steps.iter().flat_map(|s| &s.ops).filter_map(|op| op.strip_prefix("buy:")) {
        if starts.iter().any(|s| s.0 == stage) {
            continue;
        }
        let price = guide.stages.get(stage).and_then(|g| g.price).ok_or_else(|| format!("no price for {stage}"))?;
        let item = targets[index[stage]].witness(game, kind).ok_or_else(|| format!("cannot build {stage}"))?;
        starts.push((stage.to_string(), price, item));
    }
    let craft = Craft::new(&ctx, &lens, &refs, market, starts, index["goal"]);
    let actions: Vec<String> = (0..craft.ops.len() + craft.starts.len()).map(|i| craft.action_name(i)).collect();
    let recipe = Recipe::new(guide, &index, &actions, craft.ops.len())?;
    if std::env::var("TRACE").is_ok() {
        // The ends of runs that never reach the goal.
        for run in 0..200 {
            let lines = recipe.trace(&craft, &targets, 20_000, &mut Rng::new(run));
            if run == 0 {
                let mut counts = std::collections::BTreeMap::new();
                for l in &lines {
                    *counts.entry(l.split(" [").next().unwrap().to_string()).or_insert(0) += 1;
                }
                println!("  attempts per step: {counts:?}");
                let mut last = String::new();
                for l in lines.iter().filter(|l| !l.starts_with("step 1 ") && !l.starts_with("step 3 ")).take(40) {
                    if *l != last {
                        println!("  {}", &l[..l.len().min(400)]);
                    }
                    last = l.clone();
                }
            }
            if lines.last().is_some_and(|l| l != "goal") {
                for line in &lines[lines.len().saturating_sub(6)..] {
                    println!("  {line}");
                }
                println!();
            }
        }
    }

    let limit = std::env::var("LIMIT").map_or(2_000_000, |l| l.parse().unwrap());
    let clock = std::time::Instant::now();
    let guided = solve(&Following::new(&craft, &recipe), &[(1.0, Position::Fresh)], limit).map_err(|e| format!("recipe: {e:?}"))?;
    if std::env::var("TRACE").is_ok() {
        println!("  the recipe gives up in {} states, such as:", guided.given_up.len());
        for p in guided.given_up.iter().take(3) {
            if let Position::At(i, j, k, s) | Position::Hub(i, j, k, s) = p {
                let item = (0..2 * lens.classes())
                    .filter(|&c| s.counts[c] > 0)
                    .map(|c| format!("{}×{}", s.counts[c], lens.describe(c % lens.classes()).split(" [").nth(1).unwrap_or("")))
                    .collect::<Vec<_>>();
                println!("    step {} op {j} try {k}: {:?} {}", i + 1, s.rarity, item.join(" "));
            }
        }
        for c in 0..lens.classes() {
            println!("    {c}: {}", lens.describe(c));
        }
        println!(
            "  {} lens classes; recipe solved over {} states in {:.1}s",
            lens.classes(),
            guided.states.len(),
            clock.elapsed().as_secs_f64()
        );
    }
    let simulated = recipe.simulate(&craft, &targets, 500, 1);
    if std::env::var("RECIPE_ONLY").is_ok() {
        return Err(format!("recipe {:.1}c, simulated {:.1} ± {:.1}", guided.cost, simulated.0, simulated.1));
    }
    let mut times = vec![clock.elapsed().as_secs_f64()];
    let best = solve(&craft, &[(1.0, Node::Market)], limit).map_err(|e| format!("best: {e:?}"))?;
    times.push(clock.elapsed().as_secs_f64());
    let start = craft.first_buy(&best).map_or("?".into(), |s| s.name.clone());
    let followed = craft.follow(&best, 500, 1_000_000);
    times.push(clock.elapsed().as_secs_f64());
    let (guide_cost, best_cost) = (guided.cost, best.cost);
    Ok(Outcome { guide: guide_cost, best: best_cost, simulated, states: best.states.len(), followed, start, times })
}

#[test]
fn guides_cost_no_less_than_ponder() {
    let game = Game::new(Repoe::load(&root().join("fixtures/repoe")).unwrap());
    let market = Market::load(&root().join("fixtures/prices.json.gz")).unwrap();
    let mut files: Vec<PathBuf> = std::fs::read_dir(root().join("fixtures/guides")).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    let (mut active, mut failures) = (0, Vec::new());
    for path in files {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        if std::env::var("GUIDE").is_ok_and(|g| !name.contains(&g)) {
            continue;
        }
        let clock = std::time::Instant::now();
        let guide: Guide = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        match check(&game, &market, &guide) {
            Ok(o) => {
                active += 1;
                println!(
                    "{name:42} guide {:>9.1}c  ponder {:>9.1}c  simulated {:>9.1}c ± {:.1}  ({} states, starts from {})",
                    o.guide, o.best, o.simulated.0, o.simulated.1, o.states, o.start
                );
                println!(
                    "{:42} recipe {:.1}s, best plan {:.1}s, following {:.1}s",
                    "",
                    o.times[0],
                    o.times[1] - o.times[0],
                    o.times[2] - o.times[1]
                );
                let f = &o.followed;
                println!("{:42} ponder's policy on real items {:.1}c ± {:.1}, lost {}; uses {:?}", "", f.mean, f.error, f.lost, f.uses);
                let example: Vec<String> = f.example.iter().map(|(a, n)| if *n > 1 { format!("{a}×{n}") } else { a.clone() }).collect();
                println!("{:42} one run: {}", "", example.join(", ").chars().take(1500).collect::<String>());
                if f.lost > 0 || (f.mean - o.best).abs() > 4.0 * f.error + 0.05 * o.best {
                    failures.push(format!(
                        "{name}: ponder's policy costs {:.1} ± {:.1} on real items vs {:.1} exact ({} lost)",
                        f.mean, f.error, o.best, f.lost
                    ));
                }
                if o.best > o.guide * 1.05 {
                    failures.push(format!("{name}: ponder {:.1} > guide {:.1} + 5%", o.best, o.guide));
                }
                if (o.simulated.0 - o.guide).abs() > 4.0 * o.simulated.1 + 0.05 * o.guide {
                    failures.push(format!("{name}: simulated {:.1} ± {:.1} vs exact {:.1}", o.simulated.0, o.simulated.1, o.guide));
                }
            }
            Err(why) => println!("{name:42} inactive: {why}"),
        }
        println!("{:42} {:.1}s", "", clock.elapsed().as_secs_f64());
    }
    println!("{active} active guides");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
