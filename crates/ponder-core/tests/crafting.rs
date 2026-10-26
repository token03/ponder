use std::collections::BTreeSet;

use ponder_core::*;

fn modifier(id: &str, side: Side) -> Mod {
    Mod::new(id, side, 1.0, &[id], false, id, &[])
}

fn target(id: &str, side: Side) -> Target {
    Target { name: id.into(), side, good: BTreeSet::from([id.into()]), groups: BTreeSet::from([id.into()]), notable: false }
}

fn action(kind: ActionKind) -> Action {
    Action::new("test", kind, &[], &[NORMAL, MAGIC, RARE])
}

fn problem(limits: [usize; 2]) -> Problem {
    let pool = (0..8).flat_map(|i| [modifier(&format!("p{i}"), Side::Prefix), modifier(&format!("s{i}"), Side::Suffix)]).collect();
    let mut spec = ProblemSpec::new(pool, vec![target("p0", Side::Prefix), target("s0", Side::Suffix)], 2, vec![], Kind::Gear);
    spec.affix_limits = Some(limits);
    spec.compile()
}

fn normalised(out: &Distribution) {
    assert!(!out.is_empty());
    assert!((out.iter().map(|x| x.0).sum::<f64>() - 1.0).abs() < 1e-9);
    if let Some(bounds) = out.bounds() {
        assert!(out.iter().zip(bounds).all(|(&(p, _), &(lo, hi))| lo >= 0.0 && lo <= p && p <= hi && hi <= 1.0));
    }
}

#[test]
fn donor_continuations_solve_without_a_direct_plan_and_preserve_branch_weights() {
    let pool = vec![modifier("p", Side::Prefix), modifier("s", Side::Suffix)];
    let actions = vec![Action::new("exalt", ActionKind::Exalt, &[("exalted", 1.0)], &[RARE])];
    let p = ProblemSpec::new(pool, vec![target("p", Side::Prefix), target("s", Side::Suffix)], 2, actions, Kind::Gear).compile();
    let prices = Prices::from([("exalted".into(), 1.0)]);
    let settings = Settings { chaos_per_hour: 0.0 };
    assert!(solve(&p, &prices, &settings).is_err());
    let empty = State::new(RARE, &[], &[]);
    let complete = State::new(RARE, &[GOOD, GOOD], &[]);
    let initial = std::sync::Arc::new(Distribution::Exact(vec![(0.5, empty), (0.5, complete)]));
    let plan = solve_from(&p, initial, &prices, &settings).unwrap();
    assert!((plan.cost - 1.0).abs() < 1e-9);
    assert!((plan.usage["exalt"] - 1.0).abs() < 1e-9);
    let steps = steps(&p, &plan, 0.0);
    assert!((steps[0].reached - 0.5).abs() < 1e-9);
    let partial = State::new(RARE, &[GOOD], &[]);
    let initial = std::sync::Arc::new(Distribution::Exact(vec![(1.0, partial)]));
    let followed = follow_from(&p, initial, &plan.policy, &prices, &settings).unwrap();
    assert!((followed.cost - 1.0).abs() < 1e-9);
    assert!((followed.usage["exalt"] - 1.0).abs() < 1e-9);
}

#[test]
fn an_unrollable_requirement_is_rejected_before_exploring_unrelated_states() {
    let p = ProblemSpec::new(
        vec![modifier("other", Side::Prefix)],
        vec![target("missing", Side::Prefix)],
        1,
        vec![Action::new("roll", ActionKind::RerollRare, &[], &[NORMAL, RARE])],
        Kind::Gear,
    )
    .compile();
    let options = SolveOptions { limits: SolveLimits { states: 0, ..Default::default() }, ..Default::default() };
    assert_eq!(solve_with(&p, &Prices::new(), &Settings::default(), options).unwrap_err(), SolveError::Unreachable);
    let complete = State::new(RARE, &[GOOD], &[]);
    let plan =
        solve_from(&p, std::sync::Arc::new(Distribution::Exact(vec![(1.0, complete)])), &Prices::new(), &Settings::default()).unwrap();
    assert_eq!(plan.cost, 0.0);
}

#[test]
fn a_completed_prefix_side_is_finished_without_expanding_prefix_recovery_states() {
    let mut pool =
        vec![modifier("p0", Side::Prefix), modifier("p1", Side::Prefix), modifier("p2", Side::Prefix), modifier("s0", Side::Suffix)];
    pool.push(Mod::new("junk", Side::Suffix, 9.0, &["junk"], false, "junk", &[]));
    let targets = vec![target("p0", Side::Prefix), target("p1", Side::Prefix), target("p2", Side::Prefix), target("s0", Side::Suffix)];
    let actions = vec![
        Action::new("lock", ActionKind::MetaLockPrefixes, &[("lock", 1.0)], &[RARE]),
        Action::new("chaos", ActionKind::Reforge, &[("chaos", 1.0)], &[RARE]),
    ];
    let p = ProblemSpec::new(pool, targets, 4, actions, Kind::Gear).compile();
    let seed = State::new(RARE, &[GOOD, GOOD, GOOD], &[]);
    let initial = std::sync::Arc::new(Distribution::Exact(vec![(1.0, seed)]));
    let prices = Prices::from([("lock".into(), 2.0), ("chaos".into(), 1.0)]);
    let settings = Settings { chaos_per_hour: 0.0 };
    let plan = solve_preserving_side(&p, initial, Side::Prefix, &prices, &settings).unwrap();
    let chance = (9.0 * 0.1 + 4.0) / 13.0;
    assert!((plan.cost - 3.0 / chance).abs() < 1e-8);
    assert!((plan.usage["chaos"] - 1.0 / chance).abs() < 1e-8);
    assert!(plan.policy.keys().all(|s| s.status[..3] == [GOOD; 3]));
    assert!(plan.states < 100);
}

#[test]
fn recovery_prices_failed_annuls_and_rebuilds_without_expanding_lost_core_states() {
    let mut pool =
        vec![modifier("p0", Side::Prefix), modifier("p1", Side::Prefix), modifier("p2", Side::Prefix), modifier("s0", Side::Suffix)];
    pool.extend((0..3).map(|i| Mod::new(&format!("junk{i}"), Side::Suffix, 9.0, &[&format!("junk{i}")], false, "junk", &[])));
    let targets = vec![target("p0", Side::Prefix), target("p1", Side::Prefix), target("p2", Side::Prefix), target("s0", Side::Suffix)];
    let actions = vec![
        Action::new("lock", ActionKind::MetaLockPrefixes, &[("lock", 1.0)], &[RARE]),
        Action::new("chaos", ActionKind::Reforge, &[("chaos", 1.0)], &[RARE]),
        Action::new("annul", ActionKind::Annul, &[("annul", 1.0)], &[RARE]),
    ];
    let p = ProblemSpec::new(pool, targets, 4, actions, Kind::Gear).compile();
    let seed = State::new(RARE, &[GOOD, GOOD, GOOD], &[]);
    let initial = std::sync::Arc::new(Distribution::Exact(vec![(1.0, seed)]));
    let recovery = Recovery { initial: initial.clone(), cost: 100.0, hours: 2.0, sale: None, usage: [("rebuild".into(), 1.0)].into() };
    let prices = Prices::from([("lock".into(), 2.0), ("chaos".into(), 1.0), ("annul".into(), 0.1)]);
    let settings = Settings { chaos_per_hour: 0.0 };
    assert!(solve_preserving_side(&p, initial.clone(), Side::Prefix, &prices, &settings).is_err());
    let plan = solve_with_recovery(&p, initial.clone(), Side::Prefix, recovery.clone(), &prices, &settings).unwrap();
    let misses = [27.0 / 28.0, 27.0 / 28.0 * 18.0 / 19.0, 27.0 / 28.0 * 18.0 / 19.0 * 9.0 / 10.0];
    let hit = 1.0 - (9.0 * misses[0] + 3.0 * misses[1] + misses[2]) / 13.0;
    let full_miss = misses[2] / 13.0;
    assert!((plan.cost - (3.0 + full_miss * 50.1) / hit).abs() < 1e-8);
    assert!((plan.restarts - 0.5 * full_miss / hit).abs() < 1e-8);
    assert!((plan.usage["rebuild"] - plan.restarts).abs() < 1e-8);
    assert!(plan.policy.keys().all(|s| s.status[..3] == [GOOD; 3]));
    assert!(steps(&p, &plan, 0.0).iter().flat_map(|s| &s.exits).any(|e| e.what == "rebuild the crafting core"));
    let replay = follow_with_recovery(&p, initial.clone(), &plan, Side::Prefix, recovery.clone(), &prices, &settings).unwrap();
    assert!((replay.cost - plan.cost).abs() < 1e-8);
    let mut quoted = recovery;
    quoted.sale = Some(SaleRecovery { proceeds: 119.0, cost: 120.0, hours: 0.5, usage: [("replace".into(), 1.0)].into() });
    let sale = solve_with_recovery(&p, initial.clone(), Side::Prefix, quoted.clone(), &prices, &settings).unwrap();
    assert!((sale.cost - (3.0 + full_miss) / hit).abs() < 1e-8);
    assert!((sale.sales - full_miss / hit).abs() < 1e-8);
    assert!((sale.restarts - sale.sales).abs() < 1e-8);
    assert!((sale.sale_proceeds - sale.sales * 119.0).abs() < 1e-8);
    assert!(sale.sell_states.keys().all(|s| s.status[..3] == [GOOD; 3] && p.total(s) == 6));
    assert!(!sale.usage.contains_key("annul") || sale.usage["annul"] < 1e-8);
    let replay = follow_with_recovery(&p, initial.clone(), &sale, Side::Prefix, quoted.clone(), &prices, &settings).unwrap();
    assert!((replay.cost - sale.cost).abs() < 1e-8);
    assert!((replay.sales - sale.sales).abs() < 1e-8);
    assert!(steps(&p, &sale, 0.0).iter().flat_map(|s| &s.exits).any(|e| e.what == "sell the blocked item and replace it"));
    quoted.sale.as_mut().unwrap().proceeds = 190.0;
    let credited = solve_with_recovery(&p, initial.clone(), Side::Prefix, quoted.clone(), &prices, &settings).unwrap();
    assert!((credited.cost - (3.0 - 70.0 * full_miss) / hit).abs() < 1e-8);
    assert!(credited.cost < 0.0);
    let replay = follow_with_recovery(&p, initial.clone(), &credited, Side::Prefix, quoted.clone(), &prices, &settings).unwrap();
    assert!((replay.cost - credited.cost).abs() < 1e-8);
    quoted.sale.as_mut().unwrap().proceeds = 0.0;
    let unsold = solve_with_recovery(&p, initial, Side::Prefix, quoted, &prices, &settings).unwrap();
    assert!((unsold.cost - plan.cost).abs() < 1e-8);
    assert!(unsold.sales < 1e-8);
}

#[test]
fn bench_finishing_is_terminal_and_obeys_crafting_slots_and_conflicts() {
    let pool = vec![modifier("p", Side::Prefix), modifier("junk", Side::Suffix)];
    let mut bench = Action::new("bench s", ActionKind::FinishBench, &[("bench", 1.0)], &[RARE]);
    bench.forced = Some("s".into());
    let mut spec = ProblemSpec::new(
        pool,
        vec![target("p", Side::Prefix), target("s", Side::Suffix)],
        2,
        vec![Action::new("roll", ActionKind::RerollRare, &[("chaos", 1.0)], &[NORMAL, RARE]), bench],
        Kind::Gear,
    );
    spec.extra_mods.insert("s".into(), modifier("s", Side::Suffix));
    spec.allow_terminal_bench = true;
    let p = spec.compile();
    let action = &p.actions[1];
    assert!(p.outcomes(&State::new(RARE, &[], &[]), action).is_empty());
    assert!(p.outcomes(&State::new(RARE, &[GOOD, BLOCKED], &[]), action).is_empty());
    assert!(p.outcomes(&State::new(RARE, &[GOOD], &[]).with_meta(META_NO_ATTACK), action).is_empty());
    assert!(p.outcomes(&State::new(RARE, &[GOOD], &[0, 0, 0, 3]), action).is_empty());
    assert!(p.done(&p.outcomes(&State::new(RARE, &[GOOD], &[]), action)[0].1));
    let prices = Prices::from([("chaos".into(), 1.0), ("bench".into(), 0.1)]);
    let settings = Settings { chaos_per_hour: 0.0 };
    let plan = solve(&p, &prices, &settings).unwrap();
    assert!((plan.cost - 1.1).abs() < 1e-8);
    assert!((simulate(&p, &plan, &prices, &settings, 1000, 4, 20) - 1.1).abs() < 1e-8);
}

#[test]
fn acquisitions_can_retry_their_reforge_without_charging_only_one_join() {
    let pool = vec![modifier("p", Side::Prefix), modifier("s", Side::Suffix)];
    let p = ProblemSpec::new(
        pool,
        vec![target("p", Side::Prefix), target("s", Side::Suffix), target("finish", Side::Prefix)],
        3,
        vec![],
        Kind::Gear,
    )
    .compile();
    let complete = State::new(RARE, &[GOOD, GOOD, GOOD], &[0, 0, 0, 1]);
    let blocked = State::new(RARE, &[GOOD, GOOD], &[0, 2, 0, 2]);
    let initial = std::sync::Arc::new(Distribution::Exact(vec![(0.5, complete), (0.5, blocked)]));
    let recovery = Recovery { initial: initial.clone(), cost: 10.0, hours: 0.25, sale: None, usage: [("join".into(), 1.0)].into() };
    let preserved = Preservation { targets: 0b11, filler: [0; 2] };
    let settings = Settings { chaos_per_hour: 0.0 };
    let plan = solve_after_acquisition(&p, initial.clone(), preserved, recovery.clone(), &Prices::new(), &settings).unwrap();
    assert!((plan.cost - 10.0).abs() < 1e-8);
    assert!((plan.restarts - 1.0).abs() < 1e-8);
    assert!((plan.usage["join"] - 1.0).abs() < 1e-8);
    assert!((plan.hours - 0.25).abs() < 1e-8);
    assert!((plan.to_finish(&p, &blocked).unwrap() - 20.0).abs() < 1e-8);
    let followed = follow_after_acquisition(&p, initial, &plan, preserved, recovery, &Prices::new(), &settings).unwrap();
    assert!((followed.cost - plan.cost).abs() < 1e-8);
}

#[test]
fn heist_limits_apply_to_rarity_and_every_draw() {
    for limits in [[1, 2], [2, 1], [1, 4], [2, 4], [4, 2], [4, 1], [6, 0], [0, 6]] {
        let p = problem(limits);
        let out = p.outcomes(&p.start(), &action(ActionKind::RerollRare));
        normalised(&out);
        for &(_, s) in out.iter() {
            assert!(p.count(&s, Side::Prefix) <= limits[0]);
            assert!(p.count(&s, Side::Suffix) <= limits[1]);
            assert!(p.total(&s) >= 4.min(limits.iter().sum()));
        }
        if limits == [1, 2] || limits == [2, 1] {
            let magic = p.outcomes(&p.start(), &action(ActionKind::Transmute));
            normalised(&magic);
            assert!(magic.iter().all(|(_, s)| p.total(s) == 0));
        }
    }
}

#[test]
fn composite_can_add_a_sixth_prefix_and_never_a_suffix() {
    let p = problem([6, 0]);
    let state = State::new(RARE, &[GOOD], &[0, 4]);
    let out = p.outcomes(&state, &action(ActionKind::Exalt));
    normalised(&out);
    assert!(out.iter().all(|(_, s)| p.count(s, Side::Prefix) == 6 && p.count(s, Side::Suffix) == 0));
    assert!(p.outcomes(&out[0].1, &action(ActionKind::Exalt)).is_empty());
}

#[test]
fn matron_includes_multimod_and_is_deterministic_with_full_suffixes() {
    let p = problem([3, 3]);
    let all = p.outcomes(&State::new(RARE, &[], &[]), &action(ActionKind::BristleMatron));
    normalised(&all);
    assert_eq!(all.len(), 5);
    assert!(all.iter().any(|(_, s)| s.meta == META_MULTIMOD));
    let full = State::new(RARE, &[], &[0, 0, 0, 3]);
    let out = p.outcomes(&full, &action(ActionKind::BristleMatron));
    assert_eq!(&out[..], &[(1.0, full.with_meta(META_LOCK_SUFFIXES))]);
    assert!(p.outcomes(&out[0].1, &action(ActionKind::BristleMatron)).is_empty());
}

#[test]
fn protected_cleanup_matches_its_lock_and_scour_operations() {
    let p = problem([3, 3]);
    for (side, lock) in [(Side::Prefix, ActionKind::MetaLockPrefixes), (Side::Suffix, ActionKind::MetaLockSuffixes)] {
        let start = State::new(RARE, &[GOOD, GOOD], &[0, 1, 0, 1]);
        let locked = p.outcomes(&start, &action(lock));
        assert_eq!(locked.len(), 1);
        let explicit = p.outcomes(&locked[0].1, &action(ActionKind::Scour));
        let compound = p.outcomes(&start, &action(ActionKind::ProtectedScour(side)));
        assert_eq!(compound, explicit);
        assert_eq!(compound[0].1.meta, 0);
        let full = State::new(RARE, &[GOOD, GOOD], &[0, 2, 0, 2]);
        assert!(p.outcomes(&full, &action(ActionKind::ProtectedScour(side))).is_empty());
    }
    let full_suffixes = State::new(RARE, &[GOOD, GOOD], &[0, 1, 0, 2]);
    let matron = p.outcomes(&full_suffixes, &action(ActionKind::BristleMatron));
    assert_eq!(matron.len(), 1);
    let explicit = p.outcomes(&matron[0].1, &action(ActionKind::Scour));
    assert_eq!(p.outcomes(&full_suffixes, &action(ActionKind::MatronScour)), explicit);
    assert!(p.outcomes(&State::new(RARE, &[GOOD, GOOD], &[]), &action(ActionKind::MatronScour)).is_empty());
}

#[test]
fn eldritch_chaos_keeps_only_the_opposite_side() {
    let p = problem([3, 3]);
    let state = State::new(RARE, &[GOOD, GOOD], &[0, 2, 0, 0]);
    let out = p.outcomes(&state, &action(ActionKind::ChaosPrefix));
    normalised(&out);
    assert!(out.iter().all(|(_, s)| s.status[1] == GOOD && p.count(s, Side::Suffix) == 1));
    assert!(out.iter().all(|(_, s)| p.count(s, Side::Prefix) == 3));
    let locked = state.with_meta(META_LOCK_PREFIXES);
    assert!(p.outcomes(&locked, &action(ActionKind::ChaosPrefix)).is_empty());
}

#[test]
fn beast_swap_removes_before_adding_and_ignores_removal_tag_protection() {
    let mut spec = problem([3, 3]).to_spec();
    std::sync::Arc::make_mut(&mut spec.pool).iter_mut().filter(|m| m.side == Side::Prefix).for_each(|m| m.tags.push("attack".into()));
    let p = spec.compile();
    let state = State::new(RARE, &[GOOD], &[0, 2]).with_meta(META_NO_ATTACK);
    let out = p.outcomes(&state, &action(ActionKind::BeastPrefixToSuffix));
    normalised(&out);
    assert!(out.iter().all(|(_, s)| p.count(s, Side::Prefix) == 2 && p.count(s, Side::Suffix) == 2));
    let locked = state.with_meta(META_LOCK_PREFIXES);
    let out = p.outcomes(&locked, &action(ActionKind::BeastPrefixToSuffix));
    normalised(&out);
    assert!(out.iter().all(|(_, s)| p.count(s, Side::Prefix) == 3 && p.count(s, Side::Suffix) == 2));
}

fn veiled_problem() -> Problem {
    let mut spec = ProblemSpec::new(vec![modifier("natural", Side::Prefix)], vec![target("wanted", Side::Prefix)], 1, vec![], Kind::Gear);
    spec.unveils = ["wanted", "a", "b", "c"].iter().map(|id| modifier(id, Side::Prefix)).collect();
    for m in &spec.unveils {
        spec.extra_mods.insert(m.id.clone(), m.clone());
    }
    spec.extra_mods.insert("VeiledPrefix".into(), modifier("VeiledPrefix", Side::Prefix));
    spec.extra_mods.insert("VeiledSuffix".into(), modifier("VeiledSuffix", Side::Suffix));
    spec.compile()
}

#[test]
fn veil_offers_three_without_replacement_then_allows_a_choice() {
    let p = veiled_problem();
    let state = p.state_of(RARE, [&p.pool[0]]);
    let out = p.outcomes(&state, &action(ActionKind::VeiledExalt));
    normalised(&out);
    assert!(out.is_exact());
    let hit: f64 = out.iter().filter(|(_, s)| s.unveil & 1 != 0).map(|x| x.0).sum();
    assert!((hit - 0.75).abs() < 1e-10);
    for &(_, pending) in out.iter() {
        assert_eq!(p.total(&pending), 1);
        assert!(!p.done(&pending));
        assert!(p.outcomes(&pending, &action(ActionKind::Exalt)).is_empty());
        if pending.unveil & 1 != 0 {
            let chosen = p.outcomes(&pending, &action(ActionKind::Unveil(0)));
            assert!(p.done(&chosen[0].1));
            assert_eq!(p.total(&chosen[0].1), 1);
            assert_eq!(chosen[0].1.veiled, 1);
            let removed = p.annul(&chosen[0].1, None);
            assert!(removed.iter().all(|(_, s)| s.veiled == 0 && s.status[0] == MISSING));
        }
    }
}

#[test]
fn bench_blocking_precedes_the_offer_and_respects_slots_and_existing_groups() {
    let mut spec = veiled_problem().to_spec();
    spec.blockers = vec![modifier("a", Side::Prefix)];
    let p = spec.compile();
    let state = p.state_of(RARE, [&p.pool[0]]);
    let veiled = p.outcomes(&state, &action(ActionKind::VeiledExalt));
    assert!(veiled.iter().all(|(_, s)| s.unveil == 0));
    let pending = veiled[0].1;
    assert!(p.outcomes(&pending, &action(ActionKind::Unveil(0))).is_empty());
    let normal = p.outcomes(&pending, &action(ActionKind::RevealUnveil(0)));
    let blocked = p.outcomes(&pending, &action(ActionKind::RevealUnveil(1)));
    normalised(&blocked);
    assert!(blocked.is_exact());
    assert!((normal.iter().filter(|(_, s)| s.unveil & 1 != 0).map(|x| x.0).sum::<f64>() - 0.75).abs() < 1e-9);
    assert!(blocked.iter().all(|(_, s)| s.unveil & 1 != 0));
    assert!(blocked.iter().all(|(_, s)| s.status[1] == MISSING && p.total(s) == 1));
    let full = State { junk: [0, 2, 0, 0, 0, 0], ..pending };
    assert!(p.outcomes(&full, &action(ActionKind::RevealUnveil(1))).is_empty());
    let conflict = State { status: [MISSING, BLOCKED, 0, 0, 0, 0, 0, 0], ..pending };
    assert!(p.outcomes(&conflict, &action(ActionKind::RevealUnveil(1))).is_empty());
    assert!(p.outcomes(&blocked[0].1, &action(ActionKind::RevealUnveil(1))).is_empty());
    let locked = pending.with_meta(META_LOCK_SUFFIXES);
    assert!(p.outcomes(&locked, &action(ActionKind::RevealUnveil(1))).is_empty());
    let removed = p.outcomes(&locked, &action(ActionKind::RemoveCrafted));
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].1, pending);
    assert!(!p.outcomes(&removed[0].1, &action(ActionKind::RevealUnveil(1))).is_empty());
    assert!(!p.done(&State::acquisition()));
    assert!(!p.done(&State::recovery()));
    for state in [State::acquisition(), State::recovery()] {
        assert_eq!(serde_json::from_str::<State>(&serde_json::to_string(&state).unwrap()).unwrap(), state);
        assert!(p.outcomes(&state, &action(ActionKind::RemoveCrafted)).is_empty());
    }
}

#[test]
fn unveiled_junk_can_be_removed_and_clears_the_veil_restriction() {
    let p = veiled_problem();
    let state = State { veiled: 18, ..State::new(RARE, &[], &[0, 2]) };
    let out = p.annul(&state, None);
    normalised(&out);
    assert!(out.is_exact());
    assert!((out.iter().filter(|(_, s)| s.veiled == 0).map(|x| x.0).sum::<f64>() - 0.5).abs() < 1e-10);
}

#[test]
fn awakener_reforge_retains_two_transfers_and_fills_other_affixes() {
    let p = problem([3, 3]);
    let seed = State::new(RARE, &[GOOD, GOOD], &[]);
    let out = p.reforge_from(seed);
    normalised(&out);
    assert!(out.iter().all(|(_, s)| s.status[0] == GOOD && s.status[1] == GOOD && p.total(s) >= 4));
}

#[test]
fn veiled_policy_replays_with_real_offers_and_group_exclusion() {
    let mut spec = veiled_problem().to_spec();
    spec.actions = vec![
        Action::new("roll", ActionKind::RerollRare, &[("chaos", 1.0)], &[NORMAL, RARE]),
        Action::new("veil", ActionKind::VeiledExalt, &[("veil", 1.0)], &[RARE]),
        Action::new("choose wanted", ActionKind::Unveil(0), &[], &[RARE]).with_seconds(0.0),
        Action::new("choose other", ActionKind::Unveil(17), &[], &[RARE]).with_seconds(0.0),
    ];
    let p = spec.compile();
    let prices = [("chaos".into(), 1.0), ("veil".into(), 2.0)].into_iter().collect();
    let settings = Settings { chaos_per_hour: 0.0 };
    let plan = solve(&p, &prices, &settings).unwrap();
    assert!((plan.cost - 4.0).abs() < 1e-8);
    let replay = simulate(&p, &plan, &prices, &settings, 10000, 88, 1000);
    assert!((replay - plan.cost).abs() < 0.1);
}

#[test]
fn veiled_chaos_shares_outcomes_after_erasing_input_affixes() {
    let p = veiled_problem();
    let currency = action(ActionKind::VeiledChaos);
    let a = State::new(RARE, &[GOOD], &[0, 1, 0, 2]);
    let b = State::new(RARE, &[BLOCKED], &[0, 2, 0, 1]);
    let left = p.outcomes(&a, &currency);
    let right = p.outcomes(&b, &currency);
    normalised(&left);
    assert!(std::sync::Arc::ptr_eq(&left, &right));
    let a = a.with_meta(META_LOCK_SUFFIXES);
    let b = State::new(RARE, &[BLOCKED], &[0, 0, 0, 2]).with_meta(META_LOCK_SUFFIXES);
    let left = p.outcomes(&a, &currency);
    let right = p.outcomes(&b, &currency);
    normalised(&left);
    assert!(std::sync::Arc::ptr_eq(&left, &right));
}
