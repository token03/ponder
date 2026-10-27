//! Guide acceptance (SPEC §8.3): for every guide whose mechanics exist, ponder's best plan costs
//! no more than the guide's recipe, and simulating either on real items agrees with its exact
//! cost, each within the margin of error. Prints a table with `-- --nocapture`.

use std::collections::HashMap;
use std::path::PathBuf;

use std::sync::Arc;

use ponder_plan::craft::{Craft, Node};
use ponder_plan::market::Market;
use ponder_plan::recipe::{Following, Position, Recipe};
use ponder_plan::supply::{Product, Stock};
use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, GroupId, Influences, Kind};
use ponder_rules::guide::Guide;
use ponder_rules::item::Item;
use ponder_rules::lens::Lens;
use ponder_rules::ops::currency::Currency;
use ponder_rules::ops::recombinator::Recombinator;
use ponder_rules::ops::{Ctx, Operation, Supply, catalogue, parse};
use ponder_rules::rng::Rng;
use ponder_rules::target::Target;
use ponder_solve::solve;

/// How far an exact cost may sit from what it is checked against (SPEC immutables 5 and 6),
/// beyond the simulation's own noise.
const MARGIN: f64 = 0.05;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Outcome {
    item: String,
    guide: f64,
    best: f64,
    simulated: (f64, f64),
    states: usize,
    followed: ponder_plan::craft::Followed,
    start: String,
    /// Seconds when the recipe was checked, the best plan solved, and followed.
    times: Vec<f64>,
}

#[derive(Debug)]
enum CheckError {
    Inactive(String),
    Solve(String, ponder_solve::Error),
    Recipe(String),
}

impl From<String> for CheckError {
    fn from(value: String) -> Self {
        Self::Inactive(value)
    }
}

impl From<&str> for CheckError {
    fn from(value: &str) -> Self {
        Self::Inactive(value.into())
    }
}

fn check(game: &Game, market: &Market, guide: &Guide, best_plan: bool) -> Result<Option<Outcome>, CheckError> {
    let assumptions = Assumptions::default();
    let ctx = Ctx { game, assumptions: &assumptions };
    let base = game.base(&guide.base_name()).ok_or("unknown base")?;
    let kind = Kind { base, ilvl: guide.ilvl.unwrap_or(100), influences: Influences::of(&guide.influences) };
    let class = game.bases[base.0 as usize].class.clone();

    let names: Vec<String> = ["goal".to_string()].into_iter().chain(guide.stages.keys().cloned()).collect();
    let mut targets = Vec::new();
    let mut index = HashMap::new();
    for name in &names {
        let goal = guide.target(name).unwrap();
        if goal.base != Some(guide.base_name()) {
            return Err(format!("stage {name} is another base").into());
        }
        index.insert(name.clone(), targets.len());
        targets.push(Target::compile(game, &goal)?);
    }

    // The items the guide works on, in order: each is made by its own steps, the last is the goal.
    let mut items: Vec<String> = Vec::new();
    for step in &guide.steps {
        if !items.contains(&step.item) {
            items.push(step.item.clone());
        }
    }
    // What making each item yields, by the guide's recipe [0] and by ponder's plan [1].
    let products: HashMap<&str, [Arc<Product>; 2]> =
        items.iter().map(|i| (i.as_str(), [Arc::new(Product::new(i)), Arc::new(Product::new(i))])).collect();

    let mut recombinations = Vec::new();
    for op in guide.steps.iter().flat_map(|s| &s.ops) {
        if let Some((x, y)) = op.strip_prefix("recombinator:").and_then(|r| r.split_once('+')) {
            let (Some(i), Some(j)) = (items.iter().position(|n| n == x), items.iter().position(|n| n == y)) else {
                return Err(format!("needs {op}").into());
            };
            if !recombinations.iter().any(|r: &(usize, usize, String)| r.2 == *op) {
                recombinations.push((i, j, op.clone()));
            }
        }
    }
    let mut plain = Item::new(kind);
    plain.quality = guide.quality.unwrap_or(0);
    let mut stocks: Vec<Arc<dyn Supply>> = vec![Arc::new(Stock::new("base", market.base, plain))];
    for stage in guide.steps.iter().flat_map(|s| &s.ops).filter_map(|op| op.strip_prefix("buy:")) {
        if stocks.iter().any(|s| s.name() == stage) {
            continue;
        }
        let price = guide.stages.get(stage).and_then(|g| g.price).ok_or_else(|| format!("no price for {stage}"))?;
        let item = targets[index[stage]].witness(game, kind).ok_or_else(|| format!("cannot build {stage}"))?;
        stocks.push(Arc::new(Stock::new(stage, price, item)));
    }

    let mut outcome = None;
    let limit = std::env::var("LIMIT").map_or(200_000, |l| l.parse().unwrap());
    for (n, item) in items.iter().enumerate() {
        let clock = std::time::Instant::now();
        let last = n + 1 == items.len();
        let own: Vec<_> = guide.steps.iter().filter(|s| s.item == *item).collect();
        let goal_name = own.last().unwrap().until.as_deref().ok_or("no stage")?;
        let goal = &targets[index[goal_name]];
        let from = recombinations.iter().find(|r| own[0].ops.contains(&r.2)).map(|r| r.0);
        let avoid: Vec<GroupId> = goal.needs.iter().flat_map(|n| &n.ok).flat_map(|m| game.def(*m).groups.iter().copied()).collect();
        let operations = |which: usize| -> Result<Vec<Box<dyn Operation>>, String> {
            let mut ops: Vec<Box<dyn Operation>> = Vec::new();
            if which == 1 {
                ops.extend(Currency::ALL.iter().map(|c| Box::new(*c) as Box<dyn Operation>));
                ops.extend(parse(game, &class, "bench:remove-crafted", &avoid));
                let wanted = goal.needs.iter().flat_map(|n| n.ok.iter().copied()).collect();
                ops.extend(catalogue(game, &class, &wanted));
            }
            for op in own.iter().flat_map(|s| &s.ops) {
                if op.starts_with("buy:") || ops.iter().any(|o| o.name() == *op) {
                    continue;
                }
                let operation = if let Some((_, j, _)) = recombinations.iter().find(|r| r.2 == *op) {
                    Box::new(Recombinator::new(
                        &class,
                        op.strip_prefix("recombinator:").unwrap(),
                        products[items[*j].as_str()][which].clone(),
                    )) as Box<dyn Operation>
                } else {
                    parse(game, &class, op, &avoid).ok_or_else(|| format!("needs {op}"))?
                };
                ops.push(operation);
            }
            if which == 1 {
                for (_, j, op) in recombinations.iter().filter(|r| r.1 < n) {
                    if !ops.iter().any(|o| o.name() == *op) {
                        ops.push(Box::new(Recombinator::new(
                            &class,
                            op.strip_prefix("recombinator:").unwrap(),
                            products[items[*j].as_str()][which].clone(),
                        )));
                    }
                }
            }
            Ok(ops)
        };

        let mut local_names = vec![goal_name.to_string()];
        for name in own.iter().filter_map(|s| s.until.as_ref()) {
            if !local_names.contains(name) {
                local_names.push(name.clone());
            }
        }
        let local_index: HashMap<_, _> = local_names.iter().enumerate().map(|(i, name)| (name.clone(), i)).collect();
        let local_targets: Vec<_> = local_names.iter().map(|name| targets[index[name]].clone()).collect();
        let mut starts = stocks.clone();
        starts.extend(from.map(|i| products[items[i].as_str()][0].clone() as Arc<dyn Supply>));
        let guide_ops = operations(0)?;
        let refs: Vec<_> = guide_ops.iter().map(|o| o.as_ref()).collect();
        let inputs: Vec<_> = starts.iter().flat_map(|s| s.items()).collect();
        let guide_lens = Lens::new(game, kind, local_targets.clone(), &refs, &inputs);
        let guide_craft = Craft::new(&ctx, &guide_lens, &refs, market, starts, 0);
        let actions: Vec<String> = (0..guide_craft.ops.len() + guide_craft.starts.len()).map(|i| guide_craft.action_name(i)).collect();
        let start = from.map(|_| guide_craft.ops.len() + guide_craft.starts.len() - 1);
        let recipe = Recipe::new(guide, item, &local_index, &actions, guide_craft.ops.len(), start)?;
        if last && std::env::var("TRACE").is_ok() {
            trace(&recipe, &guide_craft, &local_targets);
        }
        let following = Following::new(&guide_craft, &recipe);
        let guided = solve(&following, &[(1.0, Position::Fresh)], limit).map_err(|e| CheckError::Solve(format!("recipe for {item}"), e))?;
        println!(
            "  {item} recipe: {} classes, {} explored states, {} outcomes, {:.3}s, {:.1}c",
            guide_lens.classes(),
            guided.explored,
            guided.outcomes,
            clock.elapsed().as_secs_f64(),
            guided.cost
        );
        let given_up = guided.given_up.iter().cloned().collect();
        let simulated = recipe.simulate(&guide_craft, &local_targets, &given_up, 500, 1);
        println!("  {item} recipe on real items: {:.1}c ± {:.1}", simulated.mean, simulated.error);
        if !best_plan {
            if simulated.items.len() != 500 || (simulated.mean - guided.cost).abs() > 4.0 * simulated.error + 0.05 * guided.cost {
                return Err(CheckError::Recipe(format!(
                    "{item}: simulated {:.1} ± {:.1} vs {:.1} exact ({} finished)",
                    simulated.mean,
                    simulated.error,
                    guided.cost,
                    simulated.items.len()
                )));
            }
            if !last {
                products[item.as_str()][0].fill(guided.cost, simulated.items);
            }
            continue;
        }
        if std::env::var("RECIPE_ONLY").is_ok() && last {
            return Err(format!("recipe {:.1}c, simulated {:.1} ± {:.1}", guided.cost, simulated.mean, simulated.error).into());
        }
        let mut times = vec![clock.elapsed().as_secs_f64()];

        let mut starts = stocks.clone();
        starts.extend(items[..n].iter().map(|i| products[i.as_str()][1].clone() as Arc<dyn Supply>));
        let best_ops = operations(1)?;
        let refs: Vec<_> = best_ops.iter().map(|o| o.as_ref()).collect();
        let inputs: Vec<_> = starts.iter().flat_map(|s| s.items()).collect();
        let lens = Lens::new(game, kind, vec![goal.clone()], &refs, &inputs);
        let mut craft = Craft::new(&ctx, &lens, &refs, market, starts, 0);
        println!("  {item} best: {} classes, {} operations", lens.classes(), craft.dominated.iter().filter(|d| !**d).count());
        if std::env::var("OPS").is_ok() {
            for c in 0..lens.classes() {
                println!("  class {c}: {}", lens.describe(c));
            }
            println!(
                "  {} classes; ops: {:?}",
                lens.classes(),
                craft
                    .ops
                    .iter()
                    .zip(&craft.dominated)
                    .map(|(o, d)| format!("{} {:.1}{}", o.0.name(), o.1, if *d { " (dominated)" } else { "" }))
                    .collect::<Vec<_>>()
            );
        }
        let best = craft.solve(limit).map_err(|e| CheckError::Solve(format!("best for {item}"), e))?;
        println!(
            "  {item} best: {} policy states ({} items, {} continuations), {} explored, {} actions, {} outcomes, {:.3}s",
            best.states.len(),
            best.states.iter().filter(|s| matches!(s, Node::Item(_))).count(),
            best.states.iter().filter(|s| matches!(s, Node::Hub(..))).count(),
            best.explored,
            best.actions,
            best.outcomes,
            clock.elapsed().as_secs_f64() - times[0]
        );
        times.push(clock.elapsed().as_secs_f64());
        if std::env::var("STATS").is_ok() {
            let mut found: Vec<(String, usize)> = best.stats.found.iter().map(|(&a, &n)| (craft.action_name(a), n)).collect();
            found.sort_by_key(|f| std::cmp::Reverse(f.1));
            println!("  {item}: {} explored, {} kept; found by {found:?}", best.explored, best.states.len());
            println!("  {item}: {:?}", ponder_solve::Stats { found: Default::default(), ..best.stats.clone() });
            stats(&craft, &best, &lens);
        }
        let followed = craft.follow(&best, 500, 1_000_000);
        times.push(clock.elapsed().as_secs_f64());
        if !last {
            products[item.as_str()][0].fill(guided.cost, simulated.items.clone());
            products[item.as_str()][1].fill(best.cost, followed.items.clone());
        }
        let start = craft.first_buy(&best).map_or("?".into(), |s| s.name());
        let done = Outcome {
            item: item.clone(),
            guide: guided.cost,
            best: best.cost,
            simulated: (simulated.mean, simulated.error),
            states: best.explored,
            followed,
            start,
            times,
        };
        if last {
            outcome = Some(done);
        } else {
            report("", &done);
        }
    }
    Ok(outcome)
}

/// Where the states of a solved craft come from: per rarity and affix count, and how many the
/// best policy can reach.
fn stats(craft: &Craft, best: &ponder_solve::Solution<Node>, lens: &Lens) {
    let tracked = (0..lens.classes()).filter(|&c| lens.is_tracked(c)).count();
    println!(
        "  {} classes ({tracked} tracked), {} ops ({} not dominated)",
        lens.classes(),
        craft.ops.len(),
        craft.dominated.iter().filter(|d| !**d).count()
    );
    let mut by: std::collections::BTreeMap<String, usize> = Default::default();
    for n in &best.states {
        let key = match n {
            Node::Item(s) => format!("{:?} {} affixes", s.rarity, s.counts.iter().map(|&c| c as usize).sum::<usize>()),
            Node::Hub(..) => "hub".into(),
            _ => "other".into(),
        };
        *by.entry(key).or_default() += 1;
    }
    println!("  states by kind: {by:?}");
    // Reachable under the policy.
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![Node::Market];
    while let Some(n) = stack.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        let Some(&i) = best.index.get(&n) else { continue };
        if let Some(a) = best.policy[i] {
            for (_, t) in ponder_solve::Model::outcomes(craft, &n, a) {
                stack.push(t);
            }
        }
    }
    println!("  states the best policy visits: {}", seen.len());
    // Distinct states once each decoration is forgotten.
    let n = lens.classes();
    let items: Vec<&ponder_rules::lens::State> =
        best.states.iter().filter_map(|s| if let Node::Item(s) = s { Some(s) } else { None }).collect();
    let distinct = |f: &dyn Fn(&ponder_rules::lens::State) -> ponder_rules::lens::State| {
        items.iter().map(|s| f(s)).collect::<std::collections::HashSet<_>>().len()
    };
    let plain = |s: &ponder_rules::lens::State| ponder_rules::lens::State { eldritch: None, ..*s };
    let unfractured = |s: &ponder_rules::lens::State| {
        let mut t = *s;
        for c in 0..n {
            t.counts[c] += t.counts[c + n];
            t.counts[c + n] = 0;
        }
        t
    };
    let crafted: Vec<usize> = (0..n).filter(|&c| lens.describe(c).contains("crafted: true")).collect();
    let uncrafted = |s: &ponder_rules::lens::State| {
        let mut t = *s;
        for &c in &crafted {
            t.counts[c] = 0;
        }
        t
    };
    println!(
        "  items {}; without eldritch {}; unfractured {}; uncrafted {}; all three {}",
        items.len(),
        distinct(&plain),
        distinct(&unfractured),
        distinct(&uncrafted),
        distinct(&|s| uncrafted(&unfractured(&plain(s))))
    );
    // States by their decorations: fractured, crafted, unveiled and veiled mods.
    let has = |s: &ponder_rules::lens::State, word: &str| -> usize {
        (0..n).filter(|&c| lens.describe(c).contains(word)).map(|c| s.counts[c] as usize).sum()
    };
    let mut by: std::collections::BTreeMap<String, usize> = Default::default();
    for s in &items {
        let fractured: usize = s.counts[n..].iter().map(|&c| c as usize).sum();
        let key = format!(
            "{:?}{} fr{} cr{} unveiled{} veil{}",
            s.rarity,
            s.counts.iter().map(|&c| c as usize).sum::<usize>(),
            fractured,
            has(s, "crafted: true"),
            has(s, "rank: 1") + has(s, "rank: 2") + has(s, "rank: 3"),
            has(s, "veil: true"),
        );
        *by.entry(key).or_default() += 1;
    }
    let mut by: Vec<_> = by.into_iter().collect();
    by.sort_by_key(|e| std::cmp::Reverse(e.1));
    println!("  by decoration: {:?}", &by[..by.len().min(25)]);
    for c in 0..n {
        println!("    class {c}: {}", lens.describe(c).chars().take(300).collect::<String>());
    }
}

/// The ends of runs that never reach the goal.
fn trace(recipe: &Recipe, craft: &Craft, targets: &[Target]) {
    for run in 0..200 {
        let lines = recipe.trace(craft, targets, 20_000, &mut Rng::new(run));
        if run == 0 {
            let mut counts = std::collections::BTreeMap::new();
            for l in &lines {
                *counts.entry(l.split(" [").next().unwrap().to_string()).or_insert(0) += 1;
            }
            println!("  attempts per step: {counts:?}");
        }
        if lines.last().is_some_and(|l| l != "goal") {
            for line in &lines[lines.len().saturating_sub(6)..] {
                println!("  {line}");
            }
            println!();
        }
    }
}

/// One item's lines of the table.
fn report(name: &str, o: &Outcome) {
    let name = if name.is_empty() { format!("  item {}", o.item) } else { name.to_string() };
    println!(
        "{name:42} guide {:>9.1}c  ponder {:>9.1}c  simulated {:>9.1}c ± {:.1}  ({} states, starts from {})",
        o.guide, o.best, o.simulated.0, o.simulated.1, o.states, o.start
    );
    println!("{:42} recipe {:.1}s, best plan {:.1}s, following {:.1}s", "", o.times[0], o.times[1] - o.times[0], o.times[2] - o.times[1]);
    let f = &o.followed;
    println!("{:42} ponder's policy on real items {:.1}c ± {:.1}, lost {}; uses {:?}", "", f.mean, f.error, f.lost, f.uses);
    let example: Vec<String> = f.example.iter().map(|(a, n)| if *n > 1 { format!("{a}×{n}") } else { a.clone() }).collect();
    println!("{:42} one run: {}", "", example.join(", ").chars().take(1500).collect::<String>());
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
        match check(&game, &market, &guide, true) {
            Ok(Some(o)) => {
                active += 1;
                report(&name, &o);
                let f = &o.followed;
                if f.lost > 0 || (f.mean - o.best).abs() > 4.0 * f.error + MARGIN * o.best {
                    failures.push(format!(
                        "{name}: ponder's policy costs {:.1} ± {:.1} on real items vs {:.1} exact ({} lost)",
                        f.mean, f.error, o.best, f.lost
                    ));
                }
                if o.best > o.guide * (1.0 + MARGIN) {
                    failures.push(format!("{name}: ponder {:.1} > guide {:.1}", o.best, o.guide));
                }
                if (o.simulated.0 - o.guide).abs() > 4.0 * o.simulated.1 + MARGIN * o.guide {
                    failures.push(format!("{name}: simulated {:.1} ± {:.1} vs exact {:.1}", o.simulated.0, o.simulated.1, o.guide));
                }
            }
            Ok(None) => unreachable!(),
            Err(CheckError::Inactive(why)) => println!("{name:42} inactive: {why}"),
            Err(CheckError::Solve(stage, why)) => {
                let why = format!("{name}: {stage}: {why:?}");
                println!("{why}");
                failures.push(why);
            }
            Err(CheckError::Recipe(why)) => failures.push(format!("{name}: {why}")),
        }
        println!("{:42} {:.1}s", "", clock.elapsed().as_secs_f64());
    }
    println!("{active} active guides");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn ama_recipes_match_real_items() {
    let game = Game::new(Repoe::load(&root().join("fixtures/repoe")).unwrap());
    let market = Market::load(&root().join("fixtures/prices.json.gz")).unwrap();
    let mut files: Vec<_> = std::fs::read_dir(root().join("fixtures/guides")).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    let mut checked = 0;
    for path in files {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        if !name.starts_with("ama-") || std::env::var("GUIDE").is_ok_and(|g| !name.contains(&g)) {
            continue;
        }
        let guide: Guide = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        match check(&game, &market, &guide, false) {
            Ok(None) => checked += 1,
            Ok(Some(_)) => unreachable!(),
            Err(CheckError::Inactive(why)) => println!("{name}: inactive: {why}"),
            Err(error) => panic!("{name}: {error:?}"),
        }
    }
    println!("{checked} AMA recipe references checked against real-item sampling");
}
