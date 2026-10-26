use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use ponder_plan::market::Market;
use ponder_plan::session::{Error, Method, Request, Session};
use ponder_plan::supply::Stock;
use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind};
use ponder_rules::goal::{Goal, Rarity};
use ponder_rules::guide::Guide;
use ponder_rules::item::Item;
use ponder_rules::ops::currency::Currency;
use ponder_rules::ops::{Ctx, Operation, catalogue};
use ponder_rules::target::Target;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn game() -> Game {
    Game::new(Repoe::load(&root().join("fixtures/repoe")).unwrap())
}

fn request(game: &Game, price: f64) -> Request {
    let kind = Kind { base: game.base("Vaal Axe").unwrap(), ilvl: 83, influences: Influences::default() };
    let goal = Target::compile(game, &Goal { base: Some("Vaal Axe".into()), rarity: Some(Rarity::Magic), ..Goal::default() }).unwrap();
    Request {
        kind,
        goal,
        methods: vec![Method::Named("transmutation".into())],
        starts: vec![Arc::new(Stock::new("base", price, Item::new(kind)))],
    }
}

#[test]
fn equal_requests_share_a_plan_and_prices_remain_exact() {
    let game = game();
    let assumptions = Assumptions::default();
    let market = Market { prices: HashMap::from([("transmute".into(), 1.0)]), chaos_per_hour: 0.0, base: 5.0 };
    let mut session = Session::new(Ctx { game: &game, assumptions: &assumptions }, &market);
    let first = session.solve(request(&game, 5.0), 200_000).unwrap();
    let same = session.solve(request(&game, 5.0), 200_000).unwrap();
    assert!(Arc::ptr_eq(&first, &same));
    assert_eq!(first.solution.cost, 6.0);
    let different = session.solve(request(&game, 5.0f64.next_up()), 200_000).unwrap();
    assert!(!Arc::ptr_eq(&first, &different));
    assert_eq!(different.solution.cost, 5.0f64.next_up() + 1.0);
    let followed = same.craft().follow(&same.solution, 10, 1);
    assert_eq!(followed.lost, 0);
    assert_eq!(followed.mean, 6.0);
    assert!(matches!(session.solve(request(&game, 5.0), 1), Err(Error::Solve(ponder_solve::Error::TooManyStates(1)))));
}

#[test]
fn the_existing_state_budget_bounds_cached_plans() {
    let game = game();
    let assumptions = Assumptions::default();
    let market = Market { prices: HashMap::from([("transmute".into(), 1.0)]), chaos_per_hour: 0.0, base: 5.0 };
    let mut session = Session::new(Ctx { game: &game, assumptions: &assumptions }, &market);
    let first = session.solve(request(&game, 5.0), 200_000).unwrap();
    let limit = 2 * (first.solution.states.len() + 1);
    let second = session.solve(request(&game, 6.0), limit).unwrap();
    assert!(Arc::ptr_eq(&first, &session.solve(request(&game, 5.0), limit).unwrap()));
    session.solve(request(&game, 7.0), limit).unwrap();
    assert!(Arc::ptr_eq(&first, &session.solve(request(&game, 5.0), limit).unwrap()));
    assert!(!Arc::ptr_eq(&second, &session.solve(request(&game, 6.0), limit).unwrap()));
}

#[test]
fn methods_and_unobserved_item_details_do_not_alias() {
    let game = game();
    let assumptions = Assumptions::default();
    let market = Market { prices: HashMap::from([("transmute".into(), 1.0), ("alt".into(), 1.0)]), chaos_per_hour: 0.0, base: 5.0 };
    let mut session = Session::new(Ctx { game: &game, assumptions: &assumptions }, &market);
    let first = session.solve(request(&game, 5.0), 200_000).unwrap();
    let mut extra_method = request(&game, 5.0);
    extra_method.methods.push(Method::Named("alteration".into()));
    let other = session.solve(extra_method, 200_000).unwrap();
    assert!(!Arc::ptr_eq(&first, &other));
    let mut quality = request(&game, 5.0);
    let mut item = Item::new(quality.kind);
    item.quality = 20;
    quality.starts = vec![Arc::new(Stock::new("base", 5.0, item))];
    let other = session.solve(quality, 200_000).unwrap();
    assert!(!Arc::ptr_eq(&first, &other));
    let followed = other.craft().follow(&other.solution, 10, 1);
    assert!(followed.items.iter().all(|i| i.quality == 20));
}

#[test]
fn independently_compiled_mod_requirements_share_a_plan() {
    let game = game();
    let assumptions = Assumptions::default();
    let market = Market { prices: HashMap::from([("transmute".into(), 1.0)]), chaos_per_hour: 0.0, base: 5.0 };
    let mut session = Session::new(Ctx { game: &game, assumptions: &assumptions }, &market);
    let guide: Guide = toml::from_str(&std::fs::read_to_string(root().join("fixtures/guides/cheap-phys-weapon.toml")).unwrap()).unwrap();
    let make = || {
        let mut request = request(&game, 5.0);
        request.goal = Target::compile(&game, &guide.target("flat").unwrap()).unwrap();
        request
    };
    let first = session.solve(make(), 200_000).unwrap();
    let second = session.solve(make(), 200_000).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn recombination_reuses_the_policy_with_its_real_partner_items() {
    let game = game();
    let assumptions = Assumptions { recombine_bonus: 0.0, ..Assumptions::default() };
    let market =
        Market { prices: HashMap::from([("weapon-recombinator".into(), 1.0), ("chaos".into(), 1.0)]), chaos_per_hour: 0.0, base: 5.0 };
    let mut session = Session::new(Ctx { game: &game, assumptions: &assumptions }, &market);
    let guide: Guide = toml::from_str(&std::fs::read_to_string(root().join("fixtures/guides/cheap-phys-weapon.toml")).unwrap()).unwrap();
    let make = |price: f64| {
        let mut request = request(&game, 5.0);
        request.goal = Target::compile(&game, &guide.target("flat").unwrap()).unwrap();
        let partner = request.goal.witness(&game, request.kind).unwrap();
        request.methods = vec![Method::Recombine("a+b".into(), Arc::new(Stock::new("donor", price, partner)))];
        request
    };
    let first = session.solve(make(3.0), 200_000).unwrap();
    let second = session.solve(make(3.0), 200_000).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert!((first.solution.cost - 11.0).abs() < 1e-9);
    let different = session.solve(make(4.0), 200_000).unwrap();
    assert!(!Arc::ptr_eq(&first, &different));
    assert!((different.solution.cost - 12.5).abs() < 1e-9);
    let followed = second.craft().follow(&second.solution, 500, 1);
    assert_eq!(followed.lost, 0);
    assert!((followed.mean - second.solution.cost).abs() < 4.0 * followed.error);
}

#[test]
#[ignore]
fn thousand_axe_donors() {
    let game = game();
    let assumptions = Assumptions::default();
    let market = Market::load(&root().join("fixtures/prices.json.gz")).unwrap();
    let guide: Guide = toml::from_str(&std::fs::read_to_string(root().join("fixtures/guides/cheap-phys-weapon.toml")).unwrap()).unwrap();
    let target = Target::compile(&game, &guide.target("flat").unwrap()).unwrap();
    let wanted = target.needs.iter().flat_map(|n| n.ok.iter().copied()).collect();
    let class = &game.bases[target.base.unwrap().0 as usize].class;
    let methods: Vec<_> = Currency::ALL
        .iter()
        .map(|c| Method::Named(c.name()))
        .chain(std::iter::once(Method::Named("bench:remove-crafted".into())))
        .chain(catalogue(&game, class, &wanted).iter().map(|o| Method::Named(o.name())))
        .collect();
    let make = |i: usize| {
        let mut request = request(&game, 5.0 + i as f64 / 1000.0);
        request.goal = target.clone();
        request.methods = methods.clone();
        request
    };
    let mut session = Session::new(Ctx { game: &game, assumptions: &assumptions }, &market);
    let clock = Instant::now();
    let mut cold = Vec::new();
    for i in 0..1000 {
        cold.push(session.solve(make(i), 200_000).unwrap());
    }
    let cold_seconds = clock.elapsed().as_secs_f64();
    let clock = Instant::now();
    for (i, plan) in cold.iter().enumerate() {
        assert!(Arc::ptr_eq(plan, &session.solve(make(i), 200_000).unwrap()));
    }
    let warm_seconds = clock.elapsed().as_secs_f64();
    println!(
        "1000 distinct donor requests: cold {cold_seconds:.3}s, warm {warm_seconds:.3}s; {} explored, {} retained policy states",
        cold.iter().map(|p| p.solution.explored).sum::<usize>(),
        cold.iter().map(|p| p.solution.states.len()).sum::<usize>()
    );
    let followed = cold[0].craft().follow(&cold[0].solution, 500, 1_000_000);
    assert_eq!(followed.lost, 0);
    assert!((followed.mean - cold[0].solution.cost).abs() < 4.0 * followed.error);
}
