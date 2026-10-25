//! Solver unit tests on toy pools (no game data needed).

use std::collections::BTreeSet;

use super::*;

const FREE: Settings = Settings { chaos_per_hour: 0.0 }; // tests check currency maths; time costs get their own

fn prices() -> Prices {
    [
        ("transmute", 0.1),
        ("alt", 1.0),
        ("aug", 0.2),
        ("regal", 3.0),
        ("exalted", 50.0),
        ("annul", 20.0),
        ("alch", 0.5),
        ("chaos", 2.0),
        ("scour", 1.0),
        ("lifeforce", 0.1),
        ("fossil", 5.0),
    ]
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
        Action::new("transmute", ActionKind::Transmute, &[("transmute", 1.0)], &[NORMAL]),
        Action::new("alt", ActionKind::Alt, &[("alt", 1.0)], &[MAGIC]),
        Action::new("aug", ActionKind::Aug, &[("aug", 1.0)], &[MAGIC]),
        Action::new("regal", ActionKind::Regal, &[("regal", 1.0)], &[MAGIC]),
        Action::new("exalt", ActionKind::Exalt, &[("exalted", 1.0)], &[RARE]),
        Action::new("annul", ActionKind::Annul, &[("annul", 1.0)], &[RARE]),
        Action::new("alch", ActionKind::RerollRare, &[("alch", 1.0)], &[NORMAL]),
        Action::new("chaos", ActionKind::Reforge, &[("chaos", 1.0)], &[RARE]),
        Action::new("scour+alch", ActionKind::RerollRare, &[("scour", 1.0), ("alch", 1.0)], &[RARE]),
        Action::new("scour+transmute", ActionKind::RerollMagic, &[("scour", 1.0), ("transmute", 1.0)], &[RARE]),
    ]
}

fn eldritch() -> Vec<Action> {
    vec![
        Action::new("eldritch exalt (prefix)", ActionKind::ExaltPrefix, &[("eldritch-exalted-orb", 1.0)], &[RARE]),
        Action::new("eldritch exalt (suffix)", ActionKind::ExaltSuffix, &[("eldritch-exalted-orb", 1.0)], &[RARE]),
        Action::new("eldritch annul (prefix)", ActionKind::AnnulPrefix, &[("eldritch-orb-of-annulment", 1.0)], &[RARE]),
        Action::new("eldritch annul (suffix)", ActionKind::AnnulSuffix, &[("eldritch-orb-of-annulment", 1.0)], &[RARE]),
    ]
}

fn toy_with(extra: Vec<Action>, profiles: &[(&str, Vec<(String, f64)>)], fixed: &[&str], targets: &[&str]) -> Problem {
    let pool = pool();
    let mut actions = basic();
    actions.extend(extra);
    let mut p = ProblemSpec::new(pool.clone(), targets.iter().map(|t| target(&pool, t)).collect(), targets.len(), actions, Kind::Jewel);
    p.fixed = pool.iter().filter(|m| fixed.contains(&m.id.as_str())).cloned().collect();
    for (name, w) in profiles {
        std::sync::Arc::make_mut(&mut p.profiles)
            .insert(name.to_string(), w.iter().cloned().collect::<std::collections::BTreeMap<_, _>>().into());
    }
    p.compile()
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

fn solved(p: &Problem, prices: &Prices) -> Plan {
    solve(p, prices, &FREE).unwrap()
}

#[test]
fn rolls_are_distributions() {
    let p = toy();
    for rarity in [MAGIC, RARE] {
        let out = p
            .roll(&Action::new(
                "roll",
                if rarity == MAGIC { ActionKind::RerollMagic } else { ActionKind::RerollRare },
                &[],
                &[NORMAL, RARE],
            ))
            .unwrap();
        approx(out.iter().map(|o| o.0).sum(), 1.0, 1e-9);
    }
    assert!(
        p.roll(&Action::new("roll", ActionKind::RerollMagic, &[], &[NORMAL, RARE]))
            .unwrap()
            .iter()
            .all(|(_, s)| p.count(s, Side::Prefix) <= 1)
    );
}

#[test]
fn solver_matches_simulation() {
    let p = toy();
    let plan = solved(&p, &prices());
    approx(simulated(&p, &plan, &prices(), 4000, 1), plan.cost, 0.08);
}

#[test]
fn harvest_and_fossil_match_simulation() {
    let harvest = Action::new("harvest fire", ActionKind::RerollRare, &[("lifeforce", 30.0)], &[RARE]).with_first_profile("fire");
    let fossil = Action::new("fire fossil", ActionKind::RerollRare, &[("fossil", 1.0)], &[NORMAL, RARE]).with_profile("fossil");
    let fossil_weights =
        pool().iter().map(|m| (m.id.clone(), m.weight * if m.tags.iter().any(|t| t == "fire") { 10.0 } else { 1.0 })).collect();
    let p = toy_with(vec![harvest, fossil], &[("fire", vec![("A".into(), 100.0)]), ("fossil", fossil_weights)], &[], &["A", "B"]);
    let plan = solved(&p, &prices());
    approx(simulated(&p, &plan, &prices(), 4000, 3), plan.cost, 0.08);
}

#[test]
fn policy_beats_alt_spam() {
    let p = toy();
    let plan = solved(&p, &prices());
    let policy = plan.policy.keys().map(|s| (*s, ["transmute", "alt", "scour+transmute"][s.rarity as usize].into())).collect();
    let alt_only = Plan { policy, ..Plan::default() };
    assert!(plan.cost < simulated(&p, &alt_only, &prices(), 2000, 0));
}

#[test]
fn magic_with_one_target_augments() {
    let p = toy();
    let plan = solved(&p, &prices());
    assert_eq!(plan.policy[&st(MAGIC, &[GOOD, MISSING], &[])].as_ref(), "aug");
}

#[test]
fn fractured_mod_survives_and_lowers_cost() {
    let plain = solved(&toy(), &prices()).cost;
    let fractured = toy_with(vec![], &[], &["A"], &["A", "B"]);
    assert_eq!(fractured.start(), st(NORMAL, &[GOOD, MISSING], &[]));
    assert!(
        fractured
            .roll(&Action::new("roll", ActionKind::RerollRare, &[], &[NORMAL, RARE]))
            .unwrap()
            .iter()
            .all(|(_, s)| s.status[0] == GOOD)
    );
    assert!(solved(&fractured, &prices()).cost < plain);
}

#[test]
fn annul_never_removes_fractured() {
    let p = toy_with(vec![], &[], &["A"], &["A", "B"]);
    let s = st(RARE, &[GOOD, MISSING], &[0, 0, 0, 1]);
    assert!(p.annul(&s, None).iter().all(|(_, n)| n.status[0] == GOOD));
}

#[test]
fn unpriced_actions_are_dropped_not_free() {
    let mystery = Action::new("mystery", ActionKind::RerollRare, &[("unknown-orb", 1.0)], &[NORMAL, RARE]);
    let p = toy_with(vec![mystery], &[], &[], &["A", "B"]);
    let plan = solved(&p, &prices());
    assert!(plan.unpriced.contains(&"mystery".to_string()) && !plan.usage.contains_key("mystery"));
}

/// Bench a metamod, take it off, scour a locked item.
fn metamods() -> Vec<Action> {
    vec![
        Action::new("lock prefixes", ActionKind::MetaLockPrefixes, &[("divine", 2.0)], &[RARE]),
        Action::new("lock suffixes", ActionKind::MetaLockSuffixes, &[("divine", 2.0)], &[RARE]),
        Action::new("no attack mods", ActionKind::MetaNoAttack, &[("divine", 1.0)], &[RARE]),
        Action::new("remove crafted", ActionKind::RemoveCrafted, &[("scour", 1.0)], &[RARE]),
        Action::new("scour", ActionKind::Scour, &[("scour", 1.0)], &[RARE]),
    ]
}

/// Put `name` into the policy wherever `when` holds and it can be used, so a test can check
/// the solver's exact value for a policy that uses it against a replay.
fn force(p: &Problem, plan: &mut Plan, name: &str, when: impl Fn(&State) -> bool) {
    let a = p.actions.iter().find(|a| a.name == name).unwrap().clone();
    for s in plan.order.clone() {
        if when(&s) && !p.outcomes(&s, &a).is_empty() {
            plan.policy.insert(s, name.into());
        }
    }
}

#[test]
fn a_metamod_takes_a_slot_and_must_come_off() {
    let p = toy_with(metamods(), &[], &[], &["A", "B"]);
    let lock = p.actions.iter().find(|a| a.name == "lock prefixes").unwrap().clone();
    let s = st(RARE, &[GOOD, MISSING], &[0, 1, 0, 1]);
    let out = p.outcomes(&s, &lock);
    assert_eq!(out[0].1, s.with_meta(META_LOCK_PREFIXES));
    assert_eq!(p.count(&out[0].1, Side::Suffix), 2); // a suffix
    assert!(p.outcomes(&st(RARE, &[GOOD, MISSING], &[0, 0, 0, 2]), &lock).is_empty()); // no suffix room
    assert!(p.outcomes(&out[0].1, &lock).is_empty()); // one at a time
    assert!(!p.done(&st(RARE, &[GOOD, GOOD], &[]).with_meta(META_NO_ATTACK)));
    let scour = p.actions.iter().find(|a| a.name == "scour").unwrap().clone();
    assert_eq!(p.outcomes(&out[0].1, &scour)[0].1, st(RARE, &[GOOD, MISSING], &[0, 1, 0, 0]));
    assert!(p.outcomes(&s, &scour).is_empty()); // only on a locked item
    let mut prices = prices();
    prices.insert("divine".into(), 0.5);
    let mut plan = solved(&p, &prices);
    force(&p, &mut plan, "lock prefixes", |s| s.status[1] == MISSING && s.junk[3] != 0 && s.meta == 0);
    force(&p, &mut plan, "scour", |s| s.meta == META_LOCK_PREFIXES && s.junk[3] != 0);
    assert!(plan.policy.values().any(|a| a.as_ref() == "scour"));
    let exact = evaluate(&p, &plan.policy, &prices, &FREE).unwrap();
    assert!(exact >= plan.cost * (1.0 - 1e-9)); // nothing beats the solver's own plan
    approx(simulated(&p, &plan, &prices, 4000, 4), exact, 0.08);
}

#[test]
fn essences_and_fossils_cannot_go_on_with_a_metamod() {
    let essence = Action::new("essence", ActionKind::RerollRare, &[("alch", 1.0)], &[RARE]).with_forced("A");
    let p = toy_with(vec![essence.clone()], &[], &[], &["A", "B"]);
    let s = st(RARE, &[MISSING, MISSING], &[0, 1, 0, 0]);
    assert!(!p.outcomes(&s, &essence).is_empty());
    assert!(p.outcomes(&s.with_meta(META_LOCK_PREFIXES), &essence).is_empty());
}

#[test]
fn annul_keeps_a_locked_side_and_can_take_the_metamod() {
    let p = toy_with(metamods(), &[], &[], &["A", "B"]);
    let s = st(RARE, &[GOOD, GOOD], &[0, 1, 0, 0]).with_meta(META_LOCK_PREFIXES);
    let out = p.annul(&s, None);
    approx(out.iter().map(|o| o.0).sum(), 1.0, 1e-9);
    assert_eq!(out.len(), 2); // B or the metamod: the prefixes are locked
    assert!(out.iter().all(|(_, n)| n.status[0] == GOOD && n.junk[1] == 1));
    assert!(out.iter().any(|(_, n)| n.meta == 0 && n.status[1] == GOOD));
}

fn attack_toy(extra: Vec<Action>) -> Problem {
    // A is an attack mod; x1/x2 are attack junk, y is not.
    let mut pool = pool();
    for m in pool.iter_mut().filter(|m| ["A", "x1", "x2"].contains(&m.id.as_str())) {
        m.tags.push("attack".into());
    }
    let mut actions = basic();
    actions.extend(extra);
    let targets = ["C", "B"].iter().map(|t| target(&pool, t)).collect();
    ProblemSpec::new(pool, targets, 2, actions, Kind::Gear).compile()
}

#[test]
fn cannot_roll_attack_keeps_attack_mods_off_and_on() {
    let p = attack_toy(metamods());
    let exalt = p.actions.iter().find(|a| a.name == "exalt").unwrap().clone();
    let s = st(RARE, &[GOOD, MISSING], &[1, 0, 0, 0]).with_meta(META_NO_ATTACK); // A on it, as prefix junk
    for (_, n) in p.outcomes(&s, &exalt).iter() {
        assert!(n.junk[0] == 1 && n.junk[1] == 0); // no attack prefix (x) rolls; A is junk here
    }
    // Annul can't take x (all attack); C, B and the metamod can go.
    let s = st(RARE, &[GOOD, GOOD], &[0, 1, 0, 0]).with_meta(META_NO_ATTACK);
    assert!(p.annul(&s, None).iter().all(|(_, n)| n.junk[1] == 1));
    let mut prices = gear_prices();
    prices.insert("divine".into(), 1.0);
    let mut plan = solved(&p, &prices);
    force(&p, &mut plan, "no attack mods", |s| s.meta == 0 && s.rarity == RARE && s.status[0] == GOOD);
    force(&p, &mut plan, "exalt", |s| s.meta == META_NO_ATTACK && s.status[1] == MISSING);
    force(&p, &mut plan, "annul", |s| s.meta == META_NO_ATTACK && s.status[1] == BLOCKED);
    force(&p, &mut plan, "remove crafted", |s| s.meta == META_NO_ATTACK && s.status[..2] == [GOOD, GOOD]);
    let exact = evaluate(&p, &plan.policy, &prices, &FREE).unwrap();
    approx(simulated(&p, &plan, &prices, 4000, 9), exact, 0.08);
}

#[test]
fn harvest_add_remove_adds_a_tagged_mod_and_takes_another() {
    let cold: Vec<(String, f64)> = vec![("B".into(), 100.0), ("y".into(), 500.0)];
    let augment = Action::new("harvest add/remove cold", ActionKind::Augment, &[("lifeforce", 100.0)], &[RARE]).with_profile("cold");
    let p = toy_with(vec![augment.clone()], &[("cold", cold)], &[], &["A", "B"]);
    let s = st(RARE, &[GOOD, MISSING], &[0, 1, 0, 0]);
    let out = p.outcomes(&s, &augment);
    approx(out.iter().map(|o| o.0).sum(), 1.0, 1e-9);
    assert!(out.iter().all(|(_, n)| p.count(n, Side::Suffix) == 1 && p.count(n, Side::Prefix) == 1)); // one suffix in, one prefix out
    let plan = solved(&p, &prices());
    approx(simulated(&p, &plan, &prices(), 4000, 10), plan.cost, 0.08);
}

#[test]
fn reserved_bench_slot_must_stay_open() {
    let mut p = toy_with(vec![], &[], &[], &["A"]);
    let mut spec = p.to_spec();
    spec.reserve.insert(Side::Prefix, 1);
    p = spec.compile();
    assert!(!p.done(&st(RARE, &[GOOD], &[0, 1, 0, 0]))); // jewel: two prefixes, none left
    assert!(p.done(&st(RARE, &[GOOD], &[0, 0, 0, 2])));
    let reserved = solved(&p, &prices()).cost;
    assert!(reserved > solved(&toy_with(vec![], &[], &[], &["A"]), &prices()).cost);
}

fn gear_toy(extra: Vec<Action>, targets: &[&str], filler_need: usize) -> Problem {
    let mut pool = pool();
    pool.push(Mod::new("r1", Side::Suffix, 200.0, &["r1"], false, "fire res", &[]));
    pool.push(Mod::new("r2", Side::Suffix, 200.0, &["r2"], false, "cold res", &[]));
    let mut actions = basic();
    actions.extend(extra);
    let mut p = ProblemSpec::new(pool.clone(), targets.iter().map(|t| target(&pool, t)).collect(), targets.len(), actions, Kind::Gear);
    p.filler = ["r1", "r2"].into_iter().map(String::from).collect();
    p.filler_need = filler_need;
    p.compile()
}

fn gear_prices() -> Prices {
    let mut p = prices();
    p.extend([("eldritch-exalted-orb".to_string(), 6.0), ("eldritch-orb-of-annulment".to_string(), 15.0), ("divine".to_string(), 30.0)]);
    p
}

#[test]
fn filler_counts_any_of_its_mods() {
    let p = gear_toy(vec![], &["A", "C"], 1);
    assert_eq!(p.classify("r2"), (-1, 5));
    assert!(!p.done(&st(RARE, &[GOOD, GOOD], &[0, 1, 0, 1, 0, 0])));
    assert!(p.done(&st(RARE, &[GOOD, GOOD], &[0, 0, 0, 0, 0, 1])));
    let plan = solved(&p, &gear_prices());
    approx(simulated(&p, &plan, &gear_prices(), 4000, 5), plan.cost, 0.08);
}

#[test]
fn eldritch_side_currencies_match_simulation() {
    let p = gear_toy(eldritch(), &["A", "C"], 1);
    let plan = solved(&p, &gear_prices());
    assert!(plan.cost < solved(&gear_toy(vec![], &["A", "C"], 1), &gear_prices()).cost); // cheap side-only slams help
    assert!(plan.usage.iter().any(|(a, n)| a.contains("eldritch") && *n > 0.0));
    approx(simulated(&p, &plan, &gear_prices(), 4000, 6), plan.cost, 0.08);
}

#[test]
fn a_locked_reforge_keeps_its_side_and_can_fill_it() {
    let p = gear_toy(metamods(), &["A", "C"], 2);
    let chaos = p.actions.iter().find(|a| a.name == "chaos").unwrap().clone();
    let s = st(RARE, &[GOOD, GOOD], &[0, 0, 0, 1, 0, 1]).with_meta(META_LOCK_PREFIXES);
    let out = p.outcomes(&s, &chaos);
    approx(out.iter().map(|o| o.0).sum(), 1.0, 1e-9);
    assert!(out.iter().all(|(_, n)| n.status[..2] == [GOOD, GOOD] && n.meta == 0));
    assert!(out.iter().any(|(_, n)| p.count(n, Side::Prefix) == 3)); // a new prefix on the locked side
    assert!(out.iter().any(|(_, n)| p.count(n, Side::Suffix) == 3)); // the metamod's slot is free again
    // Without the lock, chaos rerolls everything.
    assert!(p.outcomes(&st(RARE, &[GOOD, GOOD], &[0, 0, 0, 1, 0, 1]), &chaos).iter().any(|(_, n)| n.status[0] != GOOD));
    let mut prices = gear_prices();
    prices.insert("divine".into(), 1.0);
    let mut plan = solved(&p, &prices);
    force(&p, &mut plan, "lock prefixes", |s| s.meta == 0 && s.status[..2] == [GOOD, GOOD]);
    force(&p, &mut plan, "chaos", |s| s.meta == META_LOCK_PREFIXES);
    let exact = evaluate(&p, &plan.policy, &prices, &FREE).unwrap();
    approx(simulated(&p, &plan, &prices, 4000, 7), exact, 0.08);
}

#[test]
fn clicking_time_is_charged() {
    // Charging for time makes plans dearer, and `currency` is what's left once the time is taken out.
    let free = solved(&toy(), &prices());
    assert!(free.hours > 0.0 && (free.currency(&FREE) - free.cost).abs() < 1e-9);
    let slow = Settings { chaos_per_hour: 3600.0 };
    let paid = solve(&toy(), &prices(), &slow).unwrap();
    assert!(paid.cost > free.cost);
    assert!((paid.cost - paid.currency(&slow) - paid.hours * 3600.0).abs() < 1e-6);
}

#[test]
fn iteration_limits_never_return_values_for_another_policy() {
    assert_eq!(
        solve_with(&toy(), &prices(), &FREE, SolveOptions { max_rounds: 0, ..Default::default() }).unwrap_err(),
        SolveError::IterationLimit(0)
    );
    let mut limited = false;
    for rounds in [1, 2, 60] {
        let p = toy();
        match solve_with(&p, &prices(), &FREE, SolveOptions { max_rounds: rounds, ..Default::default() }) {
            Ok(plan) => approx(evaluate(&p, &plan.policy, &prices(), &FREE).unwrap(), plan.cost, 1e-9),
            Err(SolveError::IterationLimit(n)) => {
                assert_eq!(n, rounds);
                limited = true;
            }
            Err(e) => panic!("{e}"),
        }
    }
    assert!(limited);
}

#[test]
fn self_fracture_needs_four_mods_and_pays_for_misses() {
    // Fracturing hits the wanted mod 1 time in k (k >= 4 mods); misses start over.
    let pool = pool();
    let p = ProblemSpec::new(pool.clone(), vec![target(&pool, "A"), target(&pool, "B")], 2, basic(), Kind::Gear).compile();
    let mut prices = prices();
    prices.insert(FRACTURE_ORB.into(), 100.0);
    let f = self_fracture(&p, 0, &prices, &FREE).unwrap();
    assert!(f.max_mods >= 4);
    assert!((1.0 / 6.0..=0.25).contains(&f.fracture_chance));
    let a = p.actions.iter().find(|a| a.name == f.action).unwrap();
    assert!((f.cost - (step_cost(a, &prices, &FREE) / f.hit + 100.0) / f.fracture_chance).abs() < 1e-9);
    let jewel = self_fracture(&toy(), 0, &prices, &FREE).unwrap(); // a jewel only with all 4 mods
    assert_eq!((jewel.max_mods, jewel.fracture_chance), (4, 0.25));
}

#[test]
fn steps_follow_the_policy_flow() {
    // Steps start at the normal base, every listed exit points at a step or finishes, and the chances add up.
    let p = toy();
    let plan = solved(&p, &prices());
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

#[test]
fn finishing_a_half_done_item() {
    // An item with A already on starts part way: the from-scratch plan already prices its state,
    // and solving from it gives the same number; a junk-filled one is never worse than scouring it.
    let p = toy();
    let plan = solved(&p, &prices());
    let pool = pool();
    let (a, y) = (&pool[0], &pool[5]);
    let half = p.state_of(RARE, [a, y]);
    assert_eq!(half.status[..2], [GOOD, MISSING]);
    assert_eq!(half.junk[3], 1);
    let mut p2 = toy().to_spec();
    p2.begin = Some(half);
    let p2 = p2.compile();
    let begun = solved(&p2, &prices());
    assert!(begun.cost <= plan.cost + prices()["scour"] + 1e-9);
    if plan.values.contains_key(&half) {
        approx(plan.to_finish(&p, &half).unwrap(), begun.cost, 1e-6);
    }
    let mut cheap = prices();
    cheap.insert("exalted".into(), 0.5); // so a clean rare with A is worth exalting, not scouring
    let mut p3 = toy().to_spec();
    p3.begin = Some(p.state_of(RARE, [a]));
    let p3 = p3.compile();
    let clean = solved(&p3, &cheap);
    assert!(clean.cost < solved(&toy(), &cheap).cost);
    assert_eq!(steps(&p3, &clean, 0.02)[0].have, vec!["A".to_string()]); // the recipe starts where the item is
    let crafted = Mod::new("bench", Side::Suffix, 0.0, &["B"], false, "crafted B-group", &[]); // outside the pool: blocks B
    assert_eq!(p.state_of(RARE, [&crafted]).status[..2], [MISSING, BLOCKED]);
}

#[test]
fn unreachable_targets_are_an_error() {
    // Only an exalt, which needs a rare to start from: nothing makes the base rare.
    let pool = pool();
    let actions = vec![Action::new("exalt", ActionKind::Exalt, &[("exalted", 1.0)], &[RARE])];
    let p = ProblemSpec::new(pool.clone(), vec![target(&pool, "A")], 1, actions, Kind::Jewel).compile();
    assert_eq!(solve(&p, &prices(), &FREE).unwrap_err(), SolveError::Unreachable);
}

#[test]
fn describe_names_what_is_on_the_item() {
    let p = toy();
    assert_eq!(describe(&p, &st(RARE, &[GOOD, BLOCKED], &[0, 1, 0, 0])), "rare: A + B (blocked) + 1 other prefix");
    assert_eq!(describe(&p, &st(NORMAL, &[], &[])), "normal: nothing useful");
}

#[test]
fn a_fractured_mod_blocks_its_group() {
    // A fractured big "L" (heaviest prefix group) can't roll again: its tiers must not count as junk.
    use Side::*;
    let pool = vec![
        Mod::new("L1", Prefix, 1000.0, &["L"], false, "L1", &[]),
        Mod::new("L2", Prefix, 1000.0, &["L"], false, "L2", &[]),
        Mod::new("A", Prefix, 100.0, &["A"], false, "A", &[]),
        Mod::new("p1", Prefix, 100.0, &["p1"], false, "p1", &[]),
        Mod::new("p2", Prefix, 100.0, &["p2"], false, "p2", &[]),
        Mod::new("B", Suffix, 100.0, &["B"], false, "B", &[]),
        Mod::new("s1", Suffix, 300.0, &["s1"], false, "s1", &[]),
    ];
    let mut p = ProblemSpec::new(pool.clone(), vec![target(&pool, "A"), target(&pool, "B")], 2, basic(), Kind::Jewel);
    p.fixed = vec![Mod { weight: 0.0, ..pool[0].clone() }];
    let p = p.compile();
    let plan = solved(&p, &prices());
    approx(simulated(&p, &plan, &prices(), 6000, 7), plan.cost, 0.05);
}

#[test]
fn an_essence_clashing_with_a_fractured_mod_cannot_be_used() {
    // x1 is fractured; an essence forcing x2 (same group) has no outcome, one forcing y still rolls.
    let clash = Action::new("x essence", ActionKind::RerollRare, &[("chaos", 1.0)], &[NORMAL, RARE]).with_forced("x2");
    let fine = Action::new("y essence", ActionKind::RerollRare, &[("chaos", 1.0)], &[NORMAL, RARE]).with_forced("y");
    let p = toy_with(vec![clash.clone(), fine.clone()], &[], &["x1"], &["A", "B"]);
    assert!(p.outcomes(&p.start(), &clash).is_empty());
    assert!(!p.outcomes(&p.start(), &fine).is_empty());
}

#[test]
fn a_full_item_needs_every_slot_taken() {
    // Both targets on a magic item would do; wanting four mods means any two more on a rare.
    let mut p = toy();
    let mut spec = p.to_spec();
    spec.min_mods = 4;
    p = spec.compile();
    assert!(!p.done(&st(RARE, &[GOOD, GOOD], &[])));
    assert!(p.done(&st(RARE, &[GOOD, GOOD], &[1, 0, 0, 1])));
    let plain = solved(&toy(), &prices());
    let full = solved(&p, &prices());
    assert!(full.cost > plain.cost);
    approx(simulated(&p, &full, &prices(), 4000, 5), full.cost, 0.08);
}

#[test]
fn harvest_removal_does_not_protect_attack_modifiers() {
    let augment = Action::new("harvest cold", ActionKind::Augment, &[("chaos", 1.0)], &[RARE]).with_profile("cold");
    let mut p = toy_with(vec![augment.clone()], &[("cold", vec![("B".into(), 100.0)])], &[], &["A", "B"]);
    let mut spec = p.to_spec();
    std::sync::Arc::make_mut(&mut spec.pool)[0].tags.push("attack".into());
    p = spec.compile();
    let s = st(RARE, &[GOOD, MISSING], &[]).with_meta(META_NO_ATTACK);
    let out = p.outcomes(&s, &augment);
    let removed: f64 = out.iter().filter(|(_, n)| n.status[0] == MISSING).map(|(p, _)| p).sum();
    approx(removed, 0.5, 1e-12);
    assert!(out.iter().all(|(_, n)| n.status[1] == GOOD));
}

#[test]
fn harvest_first_modifier_respects_cannot_roll() {
    let harvest = Action::new("harvest fire", ActionKind::Reforge, &[("chaos", 1.0)], &[RARE]).with_first_profile("fire");
    let mut p = toy_with(vec![harvest.clone()], &[("fire", vec![("A".into(), 100.0)])], &[], &["A", "B"]);
    let mut spec = p.to_spec();
    std::sync::Arc::make_mut(&mut spec.pool)[0].tags.push("attack".into());
    p = spec.compile();
    assert!(!p.outcomes(&st(RARE, &[], &[]), &harvest).is_empty());
    assert!(p.outcomes(&st(RARE, &[], &[]).with_meta(META_NO_ATTACK), &harvest).is_empty());
}

#[test]
fn bench_conflicts_must_be_removed_before_finishing() {
    let mut p = toy();
    let mut spec = p.to_spec();
    spec.bench.push(Mod::new("bench x", Side::Prefix, 0.0, &["x"], false, "bench x", &[]));
    spec.reserve.insert(Side::Prefix, 1);
    spec.kind = Kind::Gear;
    p = spec.compile();
    let mods = pool();
    let blocked = p.state_of(RARE, [&mods[0], &mods[1], &mods[3]]);
    assert!(!p.done(&blocked));
    assert_eq!(p.count(&blocked, Side::Prefix), 2);
    assert!(p.annul(&blocked, None).iter().any(|(_, s)| p.done(s)));
    let mut prices = prices();
    prices.insert("annul".into(), 0.01);
    let mut spec = p.to_spec();
    spec.begin = Some(blocked);
    p = spec.compile();
    let plan = solved(&p, &prices);
    assert!(plan.cost > 0.0);
    assert_eq!(plan.policy[&blocked].as_ref(), "annul");
}

#[test]
fn search_limits_report_incomplete_search() {
    for limits in [
        SolveLimits { states: 1, ..Default::default() },
        SolveLimits { transitions: 1, ..Default::default() },
        SolveLimits { state_actions: 1, ..Default::default() },
    ] {
        let error = solve_with(&toy(), &prices(), &FREE, SolveOptions { limits, ..Default::default() }).unwrap_err();
        assert!(matches!(error, SolveError::ResourceLimit { .. }));
        assert!(error.to_string().contains("search incomplete"));
    }
}

#[test]
fn solving_with_new_prices_preserves_the_catalogue_and_reuses_rolls() {
    let mods = pool();
    let mut actions = vec![
        Action::new("first", ActionKind::RerollRare, &[("first", 1.0)], &[NORMAL, RARE]),
        Action::new("second", ActionKind::RerollRare, &[("second", 1.0)], &[NORMAL, RARE]),
    ];
    actions.push(Action::new("unpriced", ActionKind::RerollRare, &[("missing", 1.0)], &[NORMAL, RARE]).with_profile("unavailable"));
    let p = ProblemSpec::new(mods.clone(), vec![target(&mods, "A")], 1, actions, Kind::Gear).compile();
    let original = serde_json::to_vec(&p).unwrap();
    let first = solve(&p, &[("first".into(), 1.0), ("second".into(), 10.0)].into(), &FREE).unwrap();
    let replay = simulate(&p, &first, &[("first".into(), 1.0), ("second".into(), 10.0)].into(), &FREE, 3000, 7, 100_000);
    approx(replay, first.cost, 0.06);
    let roll = p.roll(&p.actions[0]).unwrap();
    let second = solve(&p, &[("first".into(), 10.0), ("second".into(), 1.0)].into(), &FREE).unwrap();
    assert_eq!(serde_json::to_vec(&p).unwrap(), original);
    assert!(first.policy.values().all(|a| a.as_ref() == "first"));
    assert!(second.policy.values().all(|a| a.as_ref() == "second"));
    approx(first.cost, second.cost, 1e-10);
    assert!(std::sync::Arc::ptr_eq(&roll, &p.clone().roll(&p.actions[1]).unwrap()));
    let mut changed = p.to_spec();
    std::sync::Arc::make_mut(&mut changed.pool)[0].weight *= 10.0;
    let changed = changed.compile();
    let cost = solve(&changed, &[("first".into(), 1.0)].into(), &FREE).unwrap().cost;
    assert!(cost < first.cost);
    assert_eq!(serde_json::to_vec(&p).unwrap(), original);
}

#[test]
fn action_timing_is_explicit_and_independent_of_display_names() {
    let raw =
        serde_json::json!({"name":"any label", "kind":"reroll_rare", "cost":[["scour",1.0],["dense-fossil",1.0]], "from_rarity":[RARE]});
    assert!(serde_json::from_value::<Action>(raw.clone()).is_err());
    let mut action = Action::new("any label", ActionKind::RerollRare, &[("scour", 1.0), ("dense-fossil", 1.0)], &[RARE]);
    assert_eq!(action.seconds, 5.0);
    action.name = "harvest something else".into();
    assert_eq!(action.seconds, 5.0);
    let restored: Action = serde_json::from_slice(&serde_json::to_vec(&action.with_seconds(8.0)).unwrap()).unwrap();
    assert_eq!(restored.seconds, 8.0);
    let mut unknown = raw;
    unknown["kind"] = "typo".into();
    assert!(serde_json::from_value::<Action>(unknown).is_err());
}

#[test]
fn fracture_rolls_are_shared_across_targets_and_ignore_unrollable_veil_groups() {
    let base = toy();
    let mut table = prices();
    table.insert("fracturing-orb".into(), 100.0);
    let expected: Vec<_> = (0..base.targets.len()).map(|i| self_fracture(&base, i, &table, &FREE)).collect();
    let mut spec = base.to_spec();
    let extra = Mod::new("unrolled veil", Side::Prefix, 1.0, &["unused", "other"], false, "unrolled", &[]);
    spec.unveils.push(extra.clone());
    spec.extra_mods.insert(extra.id.clone(), extra);
    let problem = spec.compile();
    for (i, expected) in expected.into_iter().enumerate() {
        let actual = self_fracture(&problem, i, &table, &FREE);
        assert_eq!(actual, expected);
        let action = problem.actions.iter().find(|a| problem.roll_key(a).is_some()).unwrap();
        let first = problem.roll(action).unwrap();
        let second = problem.roll(action).unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &second));
    }
}

#[test]
fn solve_caches_share_exact_rolls_but_release_state_specific_transitions() {
    let problem = toy();
    let action = problem.actions.iter().find(|a| problem.roll_key(a).is_some()).unwrap().clone();
    let original = problem.roll(&action).unwrap();
    let locked = State::new(RARE, &[GOOD], &[]).with_meta(META_LOCK_PREFIXES);
    let reforge = Action::new("chaos", ActionKind::Reforge, &[], &[RARE]);
    let transient = problem.outcomes(&locked, &reforge);
    let weak = std::sync::Arc::downgrade(&transient);
    let fresh = problem.fresh_transition_cache();
    assert!(std::ptr::eq(problem.mods(), fresh.mods()));
    assert_eq!(problem.fingerprint(), fresh.fingerprint());
    let rebuilt = fresh.roll(&action).unwrap();
    assert_eq!(&original[..], &rebuilt[..]);
    assert!(std::sync::Arc::ptr_eq(&original, &rebuilt));
    let rebuilt_transient = fresh.outcomes(&locked, &reforge);
    assert_eq!(&transient[..], &rebuilt_transient[..]);
    assert!(!std::sync::Arc::ptr_eq(&transient, &rebuilt_transient));
    drop(transient);
    drop(problem);
    assert!(weak.upgrade().is_none());
}

#[test]
fn unsuccessful_exact_refinement_does_not_request_an_identical_graph() {
    let action = Action::new("alch", ActionKind::RerollRare, &[], &[NORMAL]);
    for (classes, kind) in [(256, Kind::Jewel), (60, Kind::Gear)] {
        let pool = (0..classes)
            .map(|i| {
                let side = if i % 2 == 0 { Side::Prefix } else { Side::Suffix };
                Mod::new(&format!("p{i}"), side, i as f64 + 1.0, &[&format!("p{i}")], false, "", &[])
            })
            .collect();
        let problem = ProblemSpec::new(pool, vec![], 0, vec![action.clone()], kind).compile();
        let before = problem.outcomes(&problem.start(), &action);
        assert!(!problem.refine(&action));
        assert_eq!(before, problem.outcomes(&problem.start(), &action));
        assert!(!problem.refine(&action));
    }
}
