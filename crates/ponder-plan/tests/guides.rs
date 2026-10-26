//! Guide acceptance (SPEC §8.3): for every guide whose mechanics exist, ponder's best plan costs
//! at most the guide's recipe + 5%, and simulating the recipe on real items agrees with its
//! exact cost. Prints a table with `-- --nocapture`.

use std::collections::HashMap;
use std::path::PathBuf;

use ponder_plan::craft::Craft;
use ponder_plan::market::Market;
use ponder_plan::recipe::{Following, Recipe};
use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind};
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

    let mut ops: Vec<Box<dyn Operation>> = Currency::ALL.iter().map(|c| Box::new(*c) as Box<dyn Operation>).collect();
    ops.extend(parse(game, &class, "bench:remove-crafted"));
    for op in guide.steps.iter().flat_map(|s| &s.ops) {
        if op.starts_with("buy:") || ops.iter().any(|o| o.name() == *op) {
            continue;
        }
        ops.push(parse(game, &class, op).ok_or_else(|| format!("needs {op}"))?);
    }
    let refs: Vec<&dyn Operation> = ops.iter().map(|o| o.as_ref()).collect();
    let lens = Lens::new(game, kind, targets.clone(), &refs);
    let craft = Craft::new(&ctx, &lens, &refs, market, index["goal"]);
    let op_names: Vec<String> = craft.ops.iter().map(|o| o.0.name()).collect();
    let recipe = Recipe::new(guide, &index, &op_names)?;

    let mut item = Item::new(kind);
    item.quality = guide.quality.unwrap_or(0);
    let start = lens.project(&item);
    let following = Following { craft: &craft, recipe: &recipe };
    let limit = 2_000_000;
    let guided = solve(&following, &[(1.0, following.start(start.clone()))], limit).map_err(|e| format!("recipe: {e:?}"))?;
    let best = solve(&craft, &[(1.0, start)], limit).map_err(|e| format!("best: {e:?}"))?;
    let simulated = recipe.simulate(&craft, &targets, &item, 500, &mut Rng::new(1));
    let followed = craft.follow(&best, &item, 500, &mut Rng::new(2));
    Ok(Outcome { guide: guided.cost, best: best.cost, simulated, states: best.states.len(), followed })
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
        let guide: Guide = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        match check(&game, &market, &guide) {
            Ok(o) => {
                active += 1;
                println!(
                    "{name:42} guide {:>9.1}c  ponder {:>9.1}c  simulated {:>9.1}c ± {:.1}  ({} states)",
                    o.guide, o.best, o.simulated.0, o.simulated.1, o.states
                );
                let f = &o.followed;
                println!("{:42} ponder's policy on real items {:.1}c ± {:.1}, lost {}; uses {:?}", "", f.mean, f.error, f.lost, f.uses);
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
    }
    println!("{active} active guides");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
