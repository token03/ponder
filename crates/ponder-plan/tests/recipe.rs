use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use ponder_plan::craft::{Craft, Node};
use ponder_plan::market::Market;
use ponder_plan::recipe::{Following, Position, Recipe};
use ponder_plan::supply::{Product, Stock};
use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind, Side};
use ponder_rules::goal::Rarity;
use ponder_rules::guide::Guide;
use ponder_rules::item::Item;
use ponder_rules::lens::Lens;
use ponder_rules::ops::recombinator::Recombinator;
use ponder_rules::ops::{Cost, Ctx, Operation, Step, Supply, View, parse};
use ponder_rules::target::Target;
use ponder_solve::solve;

fn game() -> Game {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    Game::new(Repoe::load(&root.join("fixtures/repoe")).unwrap())
}

#[test]
fn a_recipe_buys_its_start_once() {
    let game = game();
    let guide: Guide = toml::from_str(
        r#"
sheet = "Purchase"
base = "Twilight Regalia"
[goal]
rarity = "magic"
[[steps]]
do = ["buy:base"]
[[steps]]
do = ["transmutation"]
until = "goal"
"#,
    )
    .unwrap();
    let kind = Kind { base: game.base(&guide.base_name()).unwrap(), ilvl: 86, influences: Influences::default() };
    let targets = vec![Target::compile(&game, &guide.target("goal").unwrap()).unwrap()];
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: &game, assumptions: &assumptions };
    let market = Market { prices: HashMap::from([("transmute".into(), 1.0)]), chaos_per_hour: 0.0, base: 5.0 };
    let op = parse(&game, "Body Armour", "transmutation", &[]).unwrap();
    let ops: Vec<&dyn Operation> = vec![op.as_ref()];
    let lens = Lens::new(&game, kind, targets.clone(), &ops, &[]);
    let starts: Vec<Arc<dyn Supply>> = vec![Arc::new(Stock::new("base", market.base, Item::new(kind)))];
    let craft = Craft::new(&ctx, &lens, &ops, &market, starts, 0);
    let actions = (0..craft.ops.len() + craft.starts.len()).map(|i| craft.action_name(i)).collect::<Vec<_>>();
    let recipe = Recipe::new(&guide, "main", &HashMap::from([("goal".into(), 0)]), &actions, craft.ops.len(), None).unwrap();
    let exact = solve(&Following::new(&craft, &recipe), &[(1.0, Position::Fresh)], 200_000).unwrap();
    let simulated = recipe.simulate(&craft, &targets, &HashSet::new(), 10, 1);
    assert_eq!(exact.cost, 6.0);
    assert_eq!(simulated.mean, exact.cost);
    assert_eq!(simulated.items.len(), 10);
}

#[test]
fn products_are_projected_for_the_consuming_goal() {
    let game = game();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let guide: Guide = toml::from_str(&std::fs::read_to_string(root.join("fixtures/guides/cheap-high-es-regalia.toml")).unwrap()).unwrap();
    let kind = Kind { base: game.base(&guide.base_name()).unwrap(), ilvl: 86, influences: Influences::default() };
    let mut flat = Target::compile(&game, &guide.target("flat").unwrap()).unwrap();
    let mut percent = Target::compile(&game, &guide.target("percent").unwrap()).unwrap();
    let hybrid = Target::compile(&game, &guide.target("hybrid").unwrap()).unwrap();
    flat.rarity = None;
    percent.rarity = None;
    let source = Lens::new(&game, kind, vec![flat.clone()], &[], &[]);
    let mut first = flat.witness(&game, kind).unwrap();
    first.affixes.extend(percent.witness(&game, kind).unwrap().affixes);
    let mut second = flat.witness(&game, kind).unwrap();
    second.affixes.extend(hybrid.witness(&game, kind).unwrap().affixes);
    let items = vec![first, second];
    assert!(source.holds(0, &source.project(&items[0])));
    assert!(source.holds(0, &source.project(&items[1])));
    assert_eq!(source.project(&items[0]), source.project(&items[1]));
    let product = Arc::new(Product::new("mixed"));
    product.fill(10.0, items);
    let inputs = product.items().iter().collect::<Vec<_>>();
    let lens = Lens::new(&game, kind, vec![percent], &[], &inputs);
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: &game, assumptions: &assumptions };
    let market = Market { prices: HashMap::new(), chaos_per_hour: 0.0, base: 5.0 };
    let craft = Craft::new(&ctx, &lens, &[], &market, vec![product], 0);
    let exact = solve(&craft, &[(1.0, Node::Market)], 200_000).unwrap();
    assert_eq!(exact.cost, 20.0);
    let followed = craft.follow(&exact, 1_000, 1);
    assert_eq!(followed.lost, 0);
    assert!((followed.mean - exact.cost).abs() < 4.0 * followed.error);
}

struct RareRecombination(Recombinator);

impl Operation for RareRecombination {
    fn name(&self) -> String {
        self.0.name()
    }

    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        item.rarity() == Rarity::Rare
    }

    fn steps(&self) -> &[Step] {
        self.0.steps()
    }

    fn cost(&self) -> Cost {
        self.0.cost()
    }
}

#[test]
fn recombination_cannot_finish_before_its_bonus_roll() {
    let game = game();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let guide: Guide = toml::from_str(&std::fs::read_to_string(root.join("fixtures/guides/cheap-phys-weapon.toml")).unwrap()).unwrap();
    let kind = Kind { base: game.base(&guide.base_name()).unwrap(), ilvl: 83, influences: Influences::default() };
    let mut goal = guide.target("flat").unwrap();
    goal.max_mods = Some(1);
    let target = Target::compile(&game, &goal).unwrap();
    let mut item = target.witness(&game, kind).unwrap();
    item.rarity = Rarity::Rare;
    let partner = Arc::new(Stock::new("empty", 0.0, Item::new(kind)));
    let op = RareRecombination(Recombinator::new("Two Hand Axe", "one", partner));
    let ops: Vec<&dyn Operation> = vec![&op];
    let assumptions = Assumptions { recombine_bonus: 1.0, ..Assumptions::default() };
    let ctx = Ctx { game: &game, assumptions: &assumptions };
    let market =
        Market { prices: HashMap::from([("weapon-recombinator".into(), 1.0), ("chaos".into(), 1.0)]), chaos_per_hour: 0.0, base: 1.0 };
    let lens = Lens::new(&game, kind, vec![target], &ops, &[&item]);
    let chance: f64 = lens.run(&ctx, op.steps(), &lens.project(&item)).iter().filter(|(_, s)| lens.holds(0, s)).map(|(p, _)| p).sum();
    assert!(chance > 0.0 && chance < 0.5);
    let craft = Craft::new(&ctx, &lens, &ops, &market, vec![Arc::new(Stock::new("base", 1.0, item))], 0);
    let solved = solve(&craft, &[(1.0, Node::Market)], 1_000).unwrap();
    assert!((solved.cost - 2.0 / chance).abs() < 1e-8 * solved.cost);
}

#[test]
fn a_transmutation_needs_scouring_before_it_can_retry() {
    let game = game();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let guide: Guide = toml::from_str(&std::fs::read_to_string(root.join("fixtures/guides/cheap-phys-weapon.toml")).unwrap()).unwrap();
    let kind = Kind { base: game.base(&guide.base_name()).unwrap(), ilvl: 83, influences: Influences::default() };
    let target = Target::compile(&game, &guide.target("flat").unwrap()).unwrap();
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: &game, assumptions: &assumptions };
    let market = Market { prices: HashMap::from([("transmute".into(), 1.0), ("scour".into(), 1.0)]), chaos_per_hour: 0.0, base: 5.0 };
    let transmute = parse(&game, "Two Hand Axe", "transmutation", &[]).unwrap();
    let scour = parse(&game, "Two Hand Axe", "scouring", &[]).unwrap();
    let ops: Vec<&dyn Operation> = vec![transmute.as_ref(), scour.as_ref()];
    let lens = Lens::new(&game, kind, vec![target], &ops, &[]);
    let item = Item::new(kind);
    let chance: f64 =
        lens.run(&ctx, transmute.steps(), &lens.project(&item)).iter().filter(|(_, s)| lens.holds(0, s)).map(|(p, _)| p).sum();
    let craft = Craft::new(&ctx, &lens, &ops, &market, vec![Arc::new(Stock::new("base", 5.0, item))], 0);
    let solved = solve(&craft, &[(1.0, Node::Market)], 1_000).unwrap();
    let expected = 5.0 + (1.0 + (1.0 - chance)) / chance;
    assert!((solved.cost - expected).abs() < 1e-8 * expected);
}

#[test]
fn retry_bounds_include_the_finishing_craft() {
    let game = game();
    let goal = toml::from_str(
        r#"
base = "Vaal Axe"
rarity = "magic"
mods = [{ text = "Prefixes Cannot Be Changed", source = "crafted" }]
"#,
    )
    .unwrap();
    let kind = Kind { base: game.base("Vaal Axe").unwrap(), ilvl: 83, influences: Influences::default() };
    let target = Target::compile(&game, &goal).unwrap();
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: &game, assumptions: &assumptions };
    let market = Market {
        prices: HashMap::from([
            ("transmute".into(), 1.0),
            ("alt".into(), 1.0),
            ("divine".into(), 500.0),
            ("chaos".into(), 1_000_000.0),
            ("fracturing-orb".into(), 1_000_000.0),
        ]),
        chaos_per_hour: 0.0,
        base: 5.0,
    };
    let operations: Vec<_> = ["transmutation", "alteration", "chaos", "fracturing", "bench:prefixes-cannot-be-changed"]
        .into_iter()
        .map(|name| parse(&game, "Two Hand Axe", name, &[]).unwrap())
        .collect();
    let ops: Vec<&dyn Operation> = operations.iter().map(|o| o.as_ref()).collect();
    let lens = Lens::new(&game, kind, vec![target], &ops, &[]);
    let item = Item::new(kind);
    let chance: f64 = lens
        .run(&ctx, ops[0].steps(), &lens.project(&item))
        .iter()
        .filter(|(_, s)| lens.view(s).count(Side::Suffix) == 0)
        .map(|(p, _)| p)
        .sum();
    let finish = market.price(&ops[4].cost()).unwrap();
    let craft = Craft::new(&ctx, &lens, &ops, &market, vec![Arc::new(Stock::new("base", 5.0, item))], 0);
    let solved = solve(&craft, &[(1.0, Node::Market)], 32).unwrap();
    let expected = 5.0 + 1.0 / chance + finish;
    assert!((solved.cost - expected).abs() < 1e-8 * expected);
}
