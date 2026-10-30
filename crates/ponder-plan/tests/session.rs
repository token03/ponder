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
use ponder_rules::goal::{Goal, ModReq, Rarity, Source};
use ponder_rules::guide::Guide;
use ponder_rules::item::{Affix, Item};
use ponder_rules::ops::currency::Currency;
use ponder_rules::ops::{Ctx, Operation, Step, catalogue, parse};
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
fn unsupported_base_implicits_are_rejected_before_planning() {
    let game = game();
    let assumptions = Assumptions::default();
    let market = Market { prices: HashMap::new(), chaos_per_hour: 0.0, base: 5.0 };
    let mut session = Session::new(Ctx { game: &game, assumptions: &assumptions }, &market);
    for base in ["Focused Amulet", "Simplex Amulet", "Cogwork Ring"] {
        let mut request = request(&game, 5.0);
        request.kind.base = game.base(base).unwrap();
        assert!(matches!(session.solve(request, 200_000), Err(Error::Unsupported(_))), "{base}");
    }
}

#[test]
fn unrepresented_input_contexts_are_rejected_before_planning() {
    let game = game();
    let assumptions = Assumptions::default();
    let market = Market { prices: HashMap::new(), chaos_per_hour: 0.0, base: 5.0 };
    let mut session = Session::new(Ctx { game: &game, assumptions: &assumptions }, &market);
    let mut other_goal = request(&game, 5.0);
    other_goal.goal.base = Some(game.base("Runic Hatchet").unwrap());
    assert!(matches!(session.solve(other_goal, 200_000), Err(Error::Unsupported(_))));
    for donor in [false, true] {
        let mut request = request(&game, 5.0);
        let mut item = Item::new(request.kind);
        item.kind.ilvl -= 1;
        let supply = Arc::new(Stock::new("lower level", 5.0, item));
        if donor {
            request.methods = vec![Method::Recombine("a+b".into(), supply)];
        } else {
            request.starts = vec![supply];
        }
        assert!(matches!(session.solve(request, 200_000), Err(Error::Unsupported(_))));
    }
    let mut quality = request(&game, 5.0);
    quality.goal.quality = 20;
    assert!(matches!(session.solve(quality, 200_000), Err(Error::Unsupported(_))));
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
    assert_eq!(first.policy.cost, 6.0);
    let different = session.solve(request(&game, 5.0f64.next_up()), 200_000).unwrap();
    assert!(!Arc::ptr_eq(&first, &different));
    assert_eq!(different.policy.cost, 5.0f64.next_up() + 1.0);
    let followed = same.craft().follow(&same.policy, 10, 1);
    assert_eq!(followed.lost, 0);
    assert_eq!(followed.mean, 6.0);
    assert!(matches!(session.solve(request(&game, 5.0), 1), Err(Error::Solve(ponder_solve::Error::TooManyStates(1)))));
}

#[test]
fn following_preserves_action_counts_and_distinguishes_truncated_runs() {
    let game = game();
    let assumptions = Assumptions::default();
    let market = Market { prices: HashMap::from([("transmute".into(), 1.0)]), chaos_per_hour: 0.0, base: 5.0 };
    let mut session = Session::new(Ctx { game: &game, assumptions: &assumptions }, &market);
    let mut plan = session.solve(request(&game, 5.0), 200_000).unwrap();
    {
        let craft = plan.craft();
        let followed = craft.follow(&plan.policy, 10, 1);
        assert_eq!(followed.lost, 0);
        assert_eq!(followed.truncated, 0);
        assert_eq!(followed.mean, 6.0);
        assert_eq!(followed.error, 0.0);
        assert_eq!(followed.uses, vec![("transmutation".into(), 1.0), ("buy:base".into(), 1.0)]);
        assert_eq!(followed.example, vec![("buy:base".into(), 1), ("transmutation".into(), 1)]);
        assert_eq!(followed.items.len(), 10);
        let limited = craft.follow_with_limit(&plan.policy, 10, 1, 1);
        assert_eq!(limited.lost, 10);
        assert_eq!(limited.truncated, 10);
        assert_eq!(limited.mean, 5.0);
        assert_eq!(limited.uses, vec![("buy:base".into(), 1.0)]);
        assert_eq!(limited.example, vec![("buy:base".into(), 1)]);
        assert!(limited.items.is_empty());
    }
    drop(session);
    Arc::get_mut(&mut plan).unwrap().policy.policy.fill(None);
    let unseen = plan.craft().follow_with_limit(&plan.policy, 10, 1, 1);
    assert_eq!(unseen.lost, 10);
    assert_eq!(unseen.truncated, 0);
    assert_eq!(unseen.mean, 0.0);
    assert!(unseen.uses.is_empty());
    assert!(unseen.example.is_empty());
}

#[test]
fn unavailable_modifiers_are_rejected_but_starts_and_donors_remain_usable() {
    let game = game();
    let assumptions = Assumptions::default();
    let market = Market::load(&root().join("fixtures/prices.json.gz")).unwrap();
    let kind = Kind { base: game.base("Hubris Circlet").unwrap(), ilvl: 84, influences: Influences::default() };
    let goal = Target::compile(
        &game,
        &Goal {
            base: Some("Hubris Circlet".into()),
            ilvl: Some(84),
            rarity: Some(Rarity::Rare),
            mods: vec![ModReq {
                text: vec!["+# to maximum Life".into()],
                tier: None,
                count: 1,
                fractured: false,
                source: Some(Source::Crafted),
            }],
            ..Goal::default()
        },
    )
    .unwrap();
    let mut session = Session::new(Ctx { game: &game, assumptions: &assumptions }, &market);
    let mut request =
        Request { kind, goal: goal.clone(), methods: Vec::new(), starts: vec![Arc::new(Stock::new("base", 5.0, Item::new(kind)))] };
    assert!(matches!(session.solve(request.clone(), 1), Err(Error::Solve(ponder_solve::Error::Unreachable))));
    request.methods = vec![Method::Named("alchemy".into()), Method::Named("bench:life".into())];
    assert!(session.solve(request.clone(), 2_000).unwrap().policy.cost > 5.0);
    let bench = parse(&game, "Helmet", "bench:life", &[]).unwrap();
    let Step::Craft(mods) = &bench.steps()[0] else { unreachable!() };
    let mut finished = Item::new(kind);
    finished.rarity = Rarity::Rare;
    finished.affixes.push(Affix { id: mods[0], crafted: true, fractured: false });
    request.starts = vec![Arc::new(Stock::new("finished", 5.0, finished.clone()))];
    request.methods.clear();
    assert_eq!(session.solve(request.clone(), 20).unwrap().policy.cost, 5.0);
    let mut start = Item::new(kind);
    start.rarity = Rarity::Rare;
    request.starts = vec![Arc::new(Stock::new("base", 5.0, start))];
    request.methods = vec![Method::Named("regal".into()), Method::Recombine("donor".into(), Arc::new(Stock::new("donor", 10.0, finished)))];
    assert!(session.solve(request, 2_000).unwrap().policy.cost.is_finite());
}

#[test]
fn the_existing_state_budget_bounds_cached_plans() {
    let game = game();
    let assumptions = Assumptions::default();
    let market = Market { prices: HashMap::from([("transmute".into(), 1.0)]), chaos_per_hour: 0.0, base: 5.0 };
    let mut session = Session::new(Ctx { game: &game, assumptions: &assumptions }, &market);
    let first = session.solve(request(&game, 5.0), 200_000).unwrap();
    let limit = 2 * (first.policy.states.len() + 1);
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
    let followed = other.craft().follow(&other.policy, 10, 1);
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
        let mut base = Item::new(request.kind);
        base.rarity = Rarity::Rare;
        request.starts = vec![Arc::new(Stock::new("base", 5.0, base))];
        request.methods = vec![Method::Recombine("a+b".into(), Arc::new(Stock::new("donor", price, partner)))];
        request
    };
    let first = session.solve(make(3.0), 200_000).unwrap();
    let second = session.solve(make(3.0), 200_000).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert!((first.policy.cost - 9.0 / 0.59).abs() < 1e-9);
    let different = session.solve(make(4.0), 200_000).unwrap();
    assert!(!Arc::ptr_eq(&first, &different));
    assert!((different.policy.cost - 10.0 / 0.59).abs() < 1e-9);
    let followed = second.craft().follow(&second.policy, 500, 1);
    assert_eq!(followed.lost, 0);
    assert!((followed.mean - second.policy.cost).abs() < 4.0 * followed.error);
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
        cold.iter().map(|p| p.policy.explored).sum::<usize>(),
        cold.iter().map(|p| p.policy.states.len()).sum::<usize>()
    );
    let followed = cold[0].craft().follow(&cold[0].policy, 500, 1_000_000);
    assert_eq!(followed.lost, 0);
    assert!((followed.mean - cold[0].policy.cost).abs() < 4.0 * followed.error);
}
