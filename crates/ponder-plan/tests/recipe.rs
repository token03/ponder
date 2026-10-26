use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use ponder_plan::craft::{Craft, Node};
use ponder_plan::market::Market;
use ponder_plan::recipe::{Following, Position, Recipe};
use ponder_plan::supply::{Product, Stock};
use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind};
use ponder_rules::guide::Guide;
use ponder_rules::item::Item;
use ponder_rules::lens::Lens;
use ponder_rules::ops::{Ctx, Operation, Supply, parse};
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
    let kind = Kind { base: game.base(&guide.base).unwrap(), ilvl: 86, influences: Influences::default() };
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
    let kind = Kind { base: game.base(&guide.base).unwrap(), ilvl: 86, influences: Influences::default() };
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
