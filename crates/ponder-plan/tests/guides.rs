//! Guide acceptance (SPEC §8.3): for every guide whose mechanics exist, ponder's best plan costs
//! at most the guide's recipe + 5%, and simulating the recipe on real items agrees with its
//! exact cost. Prints a table with `-- --nocapture`.

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
use ponder_solve::{ends, solve};

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

    // Bench "any" crafts block groups the goal does not want.
    let avoid: Vec<GroupId> =
        targets[index["goal"]].needs.iter().flat_map(|n| n.ok.iter()).flat_map(|m| game.def(*m).groups.iter().copied()).collect();
    let mut ops: Vec<Box<dyn Operation>> = Currency::ALL.iter().map(|c| Box::new(*c) as Box<dyn Operation>).collect();
    ops.extend(parse(game, &class, "bench:remove-crafted", &avoid));
    // Besides the guide's own: every essence and reforge that serves some stage.
    let wanted = targets.iter().flat_map(|t| &t.needs).flat_map(|n| n.ok.iter().copied()).collect();
    ops.extend(catalogue(game, &class, &wanted));
    // Recombinations: (item made first, partner item, op for the guide's partner, op for ponder's).
    let mut recombinations: Vec<(usize, usize, Box<dyn Operation>, Box<dyn Operation>)> = Vec::new();
    for op in guide.steps.iter().flat_map(|s| &s.ops) {
        if op.starts_with("buy:") || ops.iter().any(|o| o.name() == *op) || recombinations.iter().any(|r| r.2.name() == *op) {
            continue;
        }
        if let Some((x, y)) = op.strip_prefix("recombinator:").and_then(|r| r.split_once('+')) {
            let (Some(i), Some(j)) = (items.iter().position(|n| n == x), items.iter().position(|n| n == y)) else {
                return Err(format!("needs {op}"));
            };
            let made = |k: usize| Box::new(Recombinator::new(&class, &format!("{x}+{y}"), products[y][k].clone())) as Box<dyn Operation>;
            recombinations.push((i, j, made(0), made(1)));
            continue;
        }
        ops.push(parse(game, &class, op, &avoid).ok_or_else(|| format!("needs {op}"))?);
    }
    let all: Vec<&dyn Operation> =
        ops.iter().map(|o| o.as_ref()).chain(recombinations.iter().flat_map(|r| [r.2.as_ref(), r.3.as_ref()])).collect();
    let lens = Lens::new(game, kind, targets.clone(), &all);

    // Things to buy: a plain base and any stage the guide buys, each at its price.
    let mut plain = Item::new(kind);
    plain.quality = guide.quality.unwrap_or(0);
    let mut stocks: Vec<Arc<dyn Supply>> = vec![Arc::new(Stock::new("base", market.base, plain, &lens))];
    for stage in guide.steps.iter().flat_map(|s| &s.ops).filter_map(|op| op.strip_prefix("buy:")) {
        if stocks.iter().any(|s| s.name() == stage) {
            continue;
        }
        let price = guide.stages.get(stage).and_then(|g| g.price).ok_or_else(|| format!("no price for {stage}"))?;
        let item = targets[index[stage]].witness(game, kind).ok_or_else(|| format!("cannot build {stage}"))?;
        stocks.push(Arc::new(Stock::new(stage, price, item, &lens)));
    }

    let mut outcome = None;
    let clock = std::time::Instant::now();
    let limit = std::env::var("LIMIT").map_or(4_000_000, |l| l.parse().unwrap());
    for (n, item) in items.iter().enumerate() {
        let last = n + 1 == items.len();
        let first = guide.steps.iter().find(|s| s.item == *item).unwrap();
        let goal = index[guide.steps.iter().rev().find(|s| s.item == *item).unwrap().until.as_deref().ok_or("no stage")?];
        // An item made from others starts from the first of them.
        let from = recombinations.iter().find(|r| first.ops.contains(&r.2.name())).map(|r| r.0);
        // Operations whose partner is made by now.
        let usable = |k: usize| -> Vec<&dyn Operation> {
            ops.iter()
                .map(|o| o.as_ref())
                .chain(recombinations.iter().filter(|r| r.1 < n).map(|r| if k == 0 { r.2.as_ref() } else { r.3.as_ref() }))
                .collect()
        };

        // The guide's recipe, exactly and on real items.
        let mut starts = stocks.clone();
        starts.extend(from.map(|i| products[items[i].as_str()][0].clone() as Arc<dyn Supply>));
        let guide_ops = usable(0);
        let craft = Craft::new(&ctx, &lens, &guide_ops, market, starts, goal);
        let actions: Vec<String> = (0..craft.ops.len() + craft.starts.len()).map(|i| craft.action_name(i)).collect();
        let start = from.map(|_| craft.ops.len() + craft.starts.len() - 1);
        let recipe = Recipe::new(guide, item, &index, &actions, craft.ops.len(), start)?;
        if last && std::env::var("TRACE").is_ok() {
            trace(&recipe, &craft, &targets);
        }
        let following = Following::new(&craft, &recipe);
        let guided = solve(&following, &[(1.0, Position::Fresh)], limit).map_err(|e| format!("recipe for {item}: {e:?}"))?;
        let given_up = guided.given_up.iter().cloned().collect();
        let simulated = recipe.simulate(&craft, &targets, &given_up, 500, 1);
        if std::env::var("RECIPE_ONLY").is_ok() && last {
            return Err(format!("recipe {:.1}c, simulated {:.1} ± {:.1}", guided.cost, simulated.mean, simulated.error));
        }
        let mut times = vec![clock.elapsed().as_secs_f64()];

        // Ponder's plan for the same item, free to use anything made before it.
        let mut starts = stocks.clone();
        starts.extend(items[..n].iter().map(|i| products[i.as_str()][1].clone() as Arc<dyn Supply>));
        let best_ops = usable(1);
        let craft = Craft::new(&ctx, &lens, &best_ops, market, starts, goal);
        if std::env::var("OPS").is_ok() {
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
        let best = solve(&craft, &[(1.0, Node::Market)], limit).map_err(|e| format!("best for {item}: {e:?}"))?;
        times.push(clock.elapsed().as_secs_f64());
        if std::env::var("STATS").is_ok() {
            println!("  {item}: {} states, {:?}", best.states.len(), best.stats);
            stats(&craft, &best, &lens);
        }
        let followed = craft.follow(&best, 500, 1_000_000);
        times.push(clock.elapsed().as_secs_f64());
        if !last {
            let at = |p: &Position| match p {
                Position::At(_, _, _, s) => s.clone(),
                _ => unreachable!("only items meet a stage"),
            };
            let ends_guided = ends(&following, &guided, &[(1.0, Position::Fresh)]).into_iter().map(|(p, s)| (p, at(&s))).collect();
            products[item.as_str()][0].fill(guided.cost, ends_guided, simulated.items.clone());
            let item_of = |s: Node| match s {
                Node::Item(s) => s,
                _ => unreachable!("only items meet a stage"),
            };
            let ends_best = ends(&craft, &best, &[(1.0, Node::Market)]).into_iter().map(|(p, s)| (p, item_of(s))).collect();
            products[item.as_str()][1].fill(best.cost, ends_best, followed.items.clone());
            if std::env::var("OPS").is_ok() {
                println!(
                    "  {item} ends in {} (guided) and {} (best) states",
                    products[item.as_str()][0].states().len(),
                    products[item.as_str()][1].states().len()
                );
            }
        }
        let start = craft.first_buy(&best).map_or("?".into(), |s| s.name());
        let done = Outcome {
            item: item.clone(),
            guide: guided.cost,
            best: best.cost,
            simulated: (simulated.mean, simulated.error),
            states: best.states.len(),
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
    Ok(outcome.unwrap())
}

/// Where the states of a solved craft come from: per rarity and affix count, and how many the
/// best policy can reach.
fn stats(craft: &Craft, best: &ponder_solve::Solution<Node>, lens: &Lens) {
    let tracked = (0..lens.classes()).filter(|&c| lens.is_tracked(c)).count();
    println!("  {} classes ({tracked} tracked), {} ops ({} not dominated)", lens.classes(), craft.ops.len(), craft.dominated.iter().filter(|d| !**d).count());
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
    // Which action first reaches each state, breadth first over every action.
    let mut first: std::collections::HashMap<Node, String> = Default::default();
    let mut queue = std::collections::VecDeque::from([Node::Market]);
    first.insert(Node::Market, "start".into());
    while let Some(n) = queue.pop_front() {
        let Some(&i) = best.index.get(&n) else { continue };
        let _ = i;
        for (a, _) in ponder_solve::Model::actions(craft, &n) {
            for (_, t) in ponder_solve::Model::outcomes(craft, &n, a) {
                if !first.contains_key(&t) {
                    first.insert(t.clone(), craft.action_name(a));
                    queue.push_back(t);
                }
            }
        }
    }
    let mut by: std::collections::BTreeMap<&str, usize> = Default::default();
    for a in first.values() {
        *by.entry(a.as_str()).or_default() += 1;
    }
    let mut by: Vec<_> = by.into_iter().collect();
    by.sort_by_key(|e| std::cmp::Reverse(e.1));
    println!("  first reached by: {by:?}");
    // Distinct states once each decoration is forgotten.
    let n = lens.classes();
    let items: Vec<&ponder_rules::lens::State> = best.states.iter().filter_map(|s| if let Node::Item(s) = s { Some(s) } else { None }).collect();
    let distinct = |f: &dyn Fn(&ponder_rules::lens::State) -> ponder_rules::lens::State| items.iter().map(|s| f(s)).collect::<std::collections::HashSet<_>>().len();
    let plain = |s: &ponder_rules::lens::State| ponder_rules::lens::State { eldritch: None, ..s.clone() };
    let unfractured = |s: &ponder_rules::lens::State| {
        let mut t = s.clone();
        for c in 0..n { t.counts[c] += t.counts[c + n]; t.counts[c + n] = 0; }
        t
    };
    let crafted: Vec<usize> = (0..n).filter(|&c| lens.describe(c).contains("crafted: true")).collect();
    let uncrafted = |s: &ponder_rules::lens::State| {
        let mut t = s.clone();
        for &c in &crafted { t.counts[c] = 0; }
        t
    };
    println!("  items {}; without eldritch {}; unfractured {}; uncrafted {}; all three {}", items.len(), distinct(&plain), distinct(&unfractured), distinct(&uncrafted), distinct(&|s| uncrafted(&unfractured(&plain(s)))));
    for c in 0..n {
        println!("    class {c}: {}", lens.describe(c));
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
        match check(&game, &market, &guide) {
            Ok(o) => {
                active += 1;
                report(&name, &o);
                let f = &o.followed;
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
