//! Solver unit tests on toy pools (ported from tests/test_craft.py; no game data needed).

use std::collections::BTreeSet;

use super::*;

const FREE: Settings = Settings { chaos_per_hour: 0.0 }; // tests check currency maths; time costs get their own

fn prices() -> Prices {
    [("transmute", 0.1), ("alt", 1.0), ("aug", 0.2), ("regal", 3.0), ("exalted", 50.0), ("annul", 20.0), ("alch", 0.5), ("chaos", 2.0), ("scour", 1.0), ("lifeforce", 0.1), ("fossil", 5.0)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

fn pool() -> Vec<Mod> {
    use Side::*;
    vec![
        Mod::new("A", Prefix, 100.0, &["A"], true, "A", &["fire"]),
        Mod::new("B", Suffix, 100.0, &["B"], true, "B", &["cold"]),
        Mod::new("C", Prefix, 100.0, &["C"], true, "C", &[]),
        Mod::new("x1", Prefix, 300.0, &["x"], false, "x1", &[]),
        Mod::new("x2", Prefix, 300.0, &["x"], false, "x2", &[]), // same group as x1
        Mod::new("y", Suffix, 500.0, &["y"], false, "y", &[]),
    ]
}

fn target(pool: &[Mod], id: &str) -> Target {
    let m = pool.iter().find(|m| m.id == id).unwrap();
    Target {
        name: m.label.clone(),
        side: m.side,
        good: BTreeSet::from([m.id.clone()]),
        groups: m.groups.iter().cloned().collect(),
        notable: m.notable,
    }
}

/// The actions every item has (actions.BASIC).
fn basic() -> Vec<Action> {
    vec![
        Action::new("transmute", "transmute", &[("transmute", 1.0)], &[NORMAL]),
        Action::new("alt", "alt", &[("alt", 1.0)], &[MAGIC]),
        Action::new("aug", "aug", &[("aug", 1.0)], &[MAGIC]),
        Action::new("regal", "regal", &[("regal", 1.0)], &[MAGIC]),
        Action::new("exalt", "exalt", &[("exalted", 1.0)], &[RARE]),
        Action::new("annul", "annul", &[("annul", 1.0)], &[RARE]),
        Action::new("alch", "reroll_rare", &[("alch", 1.0)], &[NORMAL]),
        Action::new("chaos", "reroll_rare", &[("chaos", 1.0)], &[RARE]),
        Action::new("scour+alch", "reroll_rare", &[("scour", 1.0), ("alch", 1.0)], &[RARE]),
        Action::new("scour+transmute", "reroll_magic", &[("scour", 1.0), ("transmute", 1.0)], &[RARE]),
    ]
}

fn eldritch() -> Vec<Action> {
    vec![
        Action::new("eldritch exalt (prefix)", "exalt_prefix", &[("eldritch-exalted-orb", 1.0)], &[RARE]),
        Action::new("eldritch exalt (suffix)", "exalt_suffix", &[("eldritch-exalted-orb", 1.0)], &[RARE]),
        Action::new("eldritch annul (prefix)", "annul_prefix", &[("eldritch-orb-of-annulment", 1.0)], &[RARE]),
        Action::new("eldritch annul (suffix)", "annul_suffix", &[("eldritch-orb-of-annulment", 1.0)], &[RARE]),
    ]
}

fn toy_with(extra: Vec<Action>, profiles: &[(&str, Vec<(String, f64)>)], fixed: &[&str], targets: &[&str]) -> Problem {
    let pool = pool();
    let mut actions = basic();
    actions.extend(extra);
    let mut p = Problem::new(pool.clone(), targets.iter().map(|t| target(&pool, t)).collect(), targets.len(), actions, Kind::Jewel);
    p.fixed = pool.iter().filter(|m| fixed.contains(&m.id.as_str())).cloned().collect();
    for (name, w) in profiles {
        p.profiles.insert(name.to_string(), w.iter().cloned().collect());
    }
    p
}

fn toy() -> Problem {
    toy_with(vec![], &[], &[], &["A", "B"])
}

fn st(rarity: u8, status: &[u8], junk: &[u8]) -> State {
    State::new(rarity, status, junk)
}

fn approx(a: f64, b: f64, rel: f64) {
    assert!((a - b).abs() <= rel * a.abs().max(b.abs()).max(1e-12), "{a} vs {b}");
}

fn simulated(p: &Problem, plan: &Plan, prices: &Prices, runs: usize, seed: u64) -> f64 {
    simulate(p, plan, prices, &FREE, runs, seed, 100_000)
}

fn solved(p: &mut Problem, prices: &Prices) -> Plan {
    solve(p, prices, &FREE).unwrap()
}

#[test]
fn rolls_are_distributions() {
    let p = toy();
    for rarity in [MAGIC, RARE] {
        let out = p.roll(rarity, "base", None, None);
        approx(out.iter().map(|o| o.0).sum(), 1.0, 1e-9);
    }
    assert!(p.roll(MAGIC, "base", None, None).iter().all(|(_, s)| p.count(s, Side::Prefix) <= 1));
}

#[test]
fn solver_matches_simulation() {
    let mut p = toy();
    let plan = solved(&mut p, &prices());
    approx(simulated(&p, &plan, &prices(), 4000, 1), plan.cost, 0.08);
}

#[test]
fn harvest_and_fossil_match_simulation() {
    let harvest = Action::new("harvest fire", "reroll_rare", &[("lifeforce", 30.0)], &[RARE]).with_first_profile("fire");
    let fossil = Action::new("fire fossil", "reroll_rare", &[("fossil", 1.0)], &[NORMAL, RARE]).with_profile("fossil");
    let fossil_weights = pool().iter().map(|m| (m.id.clone(), m.weight * if m.tags.iter().any(|t| t == "fire") { 10.0 } else { 1.0 })).collect();
    let mut p = toy_with(vec![harvest, fossil], &[("fire", vec![("A".into(), 100.0)]), ("fossil", fossil_weights)], &[], &["A", "B"]);
    let plan = solved(&mut p, &prices());
    approx(simulated(&p, &plan, &prices(), 4000, 3), plan.cost, 0.08);
}

#[test]
fn policy_beats_alt_spam() {
    let mut p = toy();
    let plan = solved(&mut p, &prices());
    let policy = plan.policy.keys().map(|s| (*s, ["transmute", "alt", "scour+transmute"][s.rarity as usize].to_string())).collect();
    let alt_only = Plan { policy, ..Plan::default() };
    assert!(plan.cost < simulated(&p, &alt_only, &prices(), 2000, 0));
}

#[test]
fn magic_with_one_target_augments() {
    let mut p = toy();
    let plan = solved(&mut p, &prices());
    assert_eq!(plan.policy[&st(MAGIC, &[GOOD, MISSING], &[])], "aug");
}

#[test]
fn fractured_mod_survives_and_lowers_cost() {
    let plain = solved(&mut toy(), &prices()).cost;
    let mut fractured = toy_with(vec![], &[], &["A"], &["A", "B"]);
    assert_eq!(fractured.start(), st(NORMAL, &[GOOD, MISSING], &[]));
    assert!(fractured.roll(RARE, "base", None, None).iter().all(|(_, s)| s.status[0] == GOOD));
    assert!(solved(&mut fractured, &prices()).cost < plain);
}

#[test]
fn annul_never_removes_fractured() {
    let p = toy_with(vec![], &[], &["A"], &["A", "B"]);
    let s = st(RARE, &[GOOD, MISSING], &[0, 0, 0, 1]);
    assert!(p.annul(&s, None).iter().all(|(_, n)| n.status[0] == GOOD));
}

#[test]
fn unpriced_actions_are_dropped_not_free() {
    let mystery = Action::new("mystery", "reroll_rare", &[("unknown-orb", 1.0)], &[NORMAL, RARE]);
    let mut p = toy_with(vec![mystery], &[], &[], &["A", "B"]);
    let plan = solved(&mut p, &prices());
    assert!(plan.unpriced.contains(&"mystery".to_string()) && !plan.usage.contains_key("mystery"));
}

#[test]
fn metamod_keeps_one_side_and_matches_simulation() {
    let lock = Action::new("lock prefixes+scour", "keep_prefixes", &[("divine", 1.0), ("scour", 1.0)], &[RARE]);
    let mut p = toy_with(vec![lock.clone()], &[], &[], &["A", "B"]);
    let out = p.outcomes(&st(RARE, &[GOOD, MISSING], &[0, 1, 0, 1]), &lock);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].1, st(RARE, &[GOOD, MISSING], &[0, 1, 0, 0]));
    assert!(p.outcomes(&st(RARE, &[GOOD, MISSING], &[0, 0, 0, 2]), &lock).is_empty()); // no suffix room for the metamod
    // The toy pool is too small for the lock to pay off, so force it wherever it applies
    // and check the solver's exact value for that policy against a replay.
    let mut prices = prices();
    prices.insert("divine".into(), 0.5);
    let mut plan = solved(&mut p, &prices);
    for s in plan.order.clone() {
        if !p.outcomes(&s, &lock).is_empty() && s.status[1] == MISSING && s.junk[3] != 0 {
            plan.policy.insert(s, lock.name.clone());
        }
    }
    let exact = evaluate(&p, &plan.policy, &prices, &FREE).unwrap();
    assert!(exact > plan.cost); // the solver was right to skip it here
    approx(simulated(&p, &plan, &prices, 4000, 4), exact, 0.08);
}

#[test]
fn reserved_bench_slot_must_stay_open() {
    let mut p = toy_with(vec![], &[], &[], &["A"]);
    p.reserve.insert(Side::Prefix, 1);
    assert!(!p.done(&st(RARE, &[GOOD], &[0, 1, 0, 0]))); // jewel: two prefixes, none left
    assert!(p.done(&st(RARE, &[GOOD], &[0, 0, 0, 2])));
    let reserved = solved(&mut p, &prices()).cost;
    assert!(reserved > solved(&mut toy_with(vec![], &[], &[], &["A"]), &prices()).cost);
}

fn gear_toy(extra: Vec<Action>, targets: &[&str], filler_need: usize) -> Problem {
    let mut pool = pool();
    pool.push(Mod::new("r1", Side::Suffix, 200.0, &["r1"], false, "fire res", &[]));
    pool.push(Mod::new("r2", Side::Suffix, 200.0, &["r2"], false, "cold res", &[]));
    let mut actions = basic();
    actions.extend(extra);
    let mut p = Problem::new(pool.clone(), targets.iter().map(|t| target(&pool, t)).collect(), targets.len(), actions, Kind::Gear);
    p.filler = ["r1", "r2"].into_iter().map(String::from).collect();
    p.filler_need = filler_need;
    p
}

fn gear_prices() -> Prices {
    let mut p = prices();
    p.extend([("eldritch-exalted-orb".to_string(), 6.0), ("eldritch-orb-of-annulment".to_string(), 15.0), ("divine".to_string(), 30.0)]);
    p
}

#[test]
fn filler_counts_any_of_its_mods() {
    let mut p = gear_toy(vec![], &["A", "C"], 1);
    assert_eq!(p.classify("r2"), (-1, 5));
    assert!(!p.done(&st(RARE, &[GOOD, GOOD], &[0, 1, 0, 1, 0, 0])));
    assert!(p.done(&st(RARE, &[GOOD, GOOD], &[0, 0, 0, 0, 0, 1])));
    let plan = solved(&mut p, &gear_prices());
    approx(simulated(&p, &plan, &gear_prices(), 4000, 5), plan.cost, 0.08);
}

#[test]
fn eldritch_side_currencies_match_simulation() {
    let mut p = gear_toy(eldritch(), &["A", "C"], 1);
    let plan = solved(&mut p, &gear_prices());
    assert!(plan.cost < solved(&mut gear_toy(vec![], &["A", "C"], 1), &gear_prices()).cost); // cheap side-only slams help
    assert!(plan.usage.iter().any(|(a, n)| a.contains("eldritch") && *n > 0.0));
    approx(simulated(&p, &plan, &gear_prices(), 4000, 6), plan.cost, 0.08);
}

#[test]
fn lock_and_reroll_keeps_one_side_and_matches_simulation() {
    let lock = Action::new("lock prefixes+chaos", "lockreroll_prefixes", &[("divine", 1.0), ("chaos", 1.0)], &[RARE]);
    let mut p = gear_toy(vec![lock.clone()], &["A", "C"], 2);
    let out = p.outcomes(&st(RARE, &[GOOD, GOOD], &[0, 0, 0, 1, 0, 1]), &lock);
    approx(out.iter().map(|o| o.0).sum(), 1.0, 1e-9);
    assert!(out.iter().all(|(_, n)| n.status[..2] == [GOOD, GOOD] && p.count(n, Side::Suffix) <= 2)); // metamod holds a slot
    // Force the lock wherever it applies and the prefixes are done, then replay.
    let mut plan = solved(&mut p, &gear_prices());
    for s in plan.order.clone() {
        if !p.outcomes(&s, &lock).is_empty() && s.status[..2] == [GOOD, GOOD] {
            plan.policy.insert(s, lock.name.clone());
        }
    }
    let exact = evaluate(&p, &plan.policy, &gear_prices(), &FREE).unwrap();
    approx(simulated(&p, &plan, &gear_prices(), 4000, 7), exact, 0.08);
}

#[test]
fn clicking_time_is_charged() {
    // Charging for time makes plans dearer, and `currency` is what's left once the time is taken out.
    let free = solved(&mut toy(), &prices());
    assert!(free.hours > 0.0 && (free.currency(&FREE) - free.cost).abs() < 1e-9);
    let slow = Settings { chaos_per_hour: 3600.0 };
    let paid = solve(&mut toy(), &prices(), &slow).unwrap();
    assert!(paid.cost > free.cost);
    assert!((paid.cost - paid.currency(&slow) - paid.hours * 3600.0).abs() < 1e-6);
}

#[test]
fn self_fracture_needs_four_mods_and_pays_for_misses() {
    // Fracturing hits the wanted mod 1 time in k (k >= 4 mods); misses start over.
    let pool = pool();
    let p = Problem::new(pool.clone(), vec![target(&pool, "A"), target(&pool, "B")], 2, basic(), Kind::Gear);
    let mut prices = prices();
    prices.insert(FRACTURE_ORB.into(), 100.0);
    let f = self_fracture(&p, 0, &prices, &FREE).unwrap();
    assert!(f.max_mods >= 4);
    assert!((1.0 / 6.0..=0.25).contains(&f.fracture_chance));
    let a = p.actions.iter().find(|a| a.name == f.action).unwrap();
    assert!((f.cost - (step_cost(a, &prices, &FREE) / f.hit + 100.0) / f.fracture_chance).abs() < 1e-9);
    assert!(self_fracture(&toy(), 0, &prices, &FREE).is_none()); // jewels can't be fractured
}

#[test]
fn steps_follow_the_policy_flow() {
    // Steps start at the normal base, every listed exit points at a step or finishes, and the chances add up.
    let mut p = toy();
    let plan = solved(&mut p, &prices());
    let list = steps(&p, &plan, 0.0);
    assert_eq!(list[0].rarity, NORMAL);
    approx(list[0].reached, 1.0, 1e-9);
    let numbers: Vec<usize> = list.iter().map(|s| s.number).collect();
    for s in &list {
        assert!((s.exits.iter().map(|x| x.chance).sum::<f64>() - 1.0).abs() < 1e-6);
        assert!(s.exits.iter().all(|x| x.kind == "done" || numbers.contains(&x.to.unwrap())));
    }
    approx(list.iter().map(|s| s.uses).sum(), plan.usage.values().sum(), 1e-9);
    assert!(list.iter().any(|s| s.exits.iter().any(|x| x.kind == "done")));
}

fn sale(credit: f64) -> Action {
    let need = BTreeSet::from([0]);
    Action::new("sell as A", "sell", &[], &[MAGIC, RARE]).with_sale(credit, Sale { need, prefix: 0, suffix: 0, any: 0, spare: BTreeSet::new() })
}

#[test]
fn selling_a_brick_as_a_smaller_product_lowers_the_cost() {
    // With A alone sellable for a share of what it costs, misses on B are cheaper; a sale worth
    // more than making the item again would loop for profit, so the solver plans without it.
    let plain = solved(&mut toy(), &prices());
    let only_a = solved(&mut toy_with(vec![], &[], &[], &["A"]), &prices());
    let mut p = toy_with(vec![sale(0.7 * only_a.cost)], &[], &[], &["A", "B"]);
    let selling = solved(&mut p, &prices());
    assert!(selling.cost < plain.cost && selling.usage["sell as A"] > 0.0);
    approx(simulated(&p, &selling, &prices(), 3000, 0), selling.cost, 0.1);
    let mut greedy = toy_with(vec![sale(100.0 * only_a.cost)], &[], &[], &["A", "B"]);
    approx(solved(&mut greedy, &prices()).cost, plain.cost, 1e-9);
    assert!(greedy.actions.iter().all(|a| a.kind != "sell"));
}

#[test]
fn finishing_a_half_done_item() {
    // An item with A already on starts part way: the from-scratch plan already prices its state,
    // and solving from it gives the same number; a junk-filled one is never worse than scouring it.
    let mut p = toy();
    let plan = solved(&mut p, &prices());
    let pool = pool();
    let (a, y) = (&pool[0], &pool[5]);
    let half = p.state_of(RARE, [a, y]);
    assert_eq!(half.status[..2], [GOOD, MISSING]);
    assert_eq!(half.junk[3], 1);
    let mut p2 = toy();
    p2.begin = Some(half);
    let begun = solved(&mut p2, &prices());
    assert!(begun.cost <= plan.cost + prices()["scour"] + 1e-9);
    if plan.values.contains_key(&half) {
        approx(plan.to_finish(&p, &half).unwrap(), begun.cost, 1e-6);
    }
    let mut cheap = prices();
    cheap.insert("exalted".into(), 0.5); // so a clean rare with A is worth exalting, not scouring
    let mut p3 = toy();
    p3.begin = Some(p.state_of(RARE, [a]));
    let clean = solved(&mut p3, &cheap);
    assert!(clean.cost < solved(&mut toy(), &cheap).cost);
    assert_eq!(steps(&p3, &clean, 0.02)[0].have, vec!["A".to_string()]); // the recipe starts where the item is
    let crafted = Mod::new("bench", Side::Suffix, 0.0, &["B"], false, "crafted B-group", &[]); // outside the pool: blocks B
    assert_eq!(p.state_of(RARE, [&crafted]).status[..2], [MISSING, BLOCKED]);
}

#[test]
fn unreachable_targets_are_an_error() {
    // Only an exalt, which needs a rare to start from: nothing makes the base rare.
    let pool = pool();
    let actions = vec![Action::new("exalt", "exalt", &[("exalted", 1.0)], &[RARE])];
    let mut p = Problem::new(pool.clone(), vec![target(&pool, "A")], 1, actions, Kind::Jewel);
    assert_eq!(solve(&mut p, &prices(), &FREE).unwrap_err(), SolveError::Unreachable);
}

#[test]
fn describe_names_what_is_on_the_item() {
    let p = toy();
    assert_eq!(describe(&p, &st(RARE, &[GOOD, BLOCKED], &[0, 1, 0, 0])), "rare: A + B (blocked) + 1 other prefix");
    assert_eq!(describe(&p, &st(NORMAL, &[], &[])), "normal: nothing useful");
}
