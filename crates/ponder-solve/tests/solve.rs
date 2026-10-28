//! Small problems with known answers.

use ponder_solve::{Error, Model, ends, solve};

/// States 0..=n; state n is the goal. Each action is (cost, [(p, next)]) available in every
/// non-goal state, with `next` relative: Some(k) moves to k, None stays.
/// (cost, [(p, next)])
type Action = (f64, Vec<(f64, u32)>);

struct Chain {
    goal: u32,
    actions: Vec<Vec<Action>>,
}

impl Model for Chain {
    type State = u32;
    fn is_goal(&self, s: &u32) -> bool {
        *s == self.goal
    }
    fn actions(&self, s: &u32) -> Vec<(usize, f64)> {
        self.actions[*s as usize].iter().enumerate().map(|(i, a)| (i, a.0)).collect()
    }
    fn outcomes(&self, s: &u32, a: usize) -> Vec<(f64, u32)> {
        self.actions[*s as usize][a].1.clone()
    }
}

#[test]
fn retry_loop_costs_one_over_p() {
    let m = Chain { goal: 1, actions: vec![vec![(2.0, vec![(0.05, 1), (0.95, 0)])]] };
    let s = solve(&m, &[(1.0, 0)], 100).unwrap();
    assert!((s.cost - 40.0).abs() < 1e-9, "{}", s.cost);
}

#[test]
fn cheaper_expected_cost_wins() {
    // Action 0: cost 1, 10% success. Action 1: cost 5, 60% success. 1/0.1 = 10 > 5/0.6.
    let m = Chain { goal: 1, actions: vec![vec![(1.0, vec![(0.1, 1), (0.9, 0)]), (5.0, vec![(0.6, 1), (0.4, 0)])]] };
    let s = solve(&m, &[(1.0, 0)], 100).unwrap();
    assert!((s.cost - 5.0 / 0.6).abs() < 1e-9);
    assert_eq!(s.policy[s.index[&0]], Some(1));
}

#[test]
fn staged_craft_with_a_fall_back() {
    // 0 -> 1 with p=0.2 (cost 1). From 1: cost 3, 50% goal, 50% back to 0.
    // V1 = 3 + 0.5 V0, V0 = 1/0.2 + V1 = 5 + V1  =>  V1 = 3 + 2.5 + 0.5 V1  =>  V1 = 11, V0 = 16.
    let m = Chain { goal: 2, actions: vec![vec![(1.0, vec![(0.2, 1), (0.8, 0)])], vec![(3.0, vec![(0.5, 2), (0.5, 0)])]] };
    let s = solve(&m, &[(1.0, 0)], 100).unwrap();
    assert!((s.cost - 16.0).abs() < 1e-9, "{}", s.cost);
}

#[test]
fn dead_ends_are_avoided_or_reported() {
    // Action 0 can brick the item (state 2, no actions); action 1 is safe but dearer.
    let m = Chain { goal: 1, actions: vec![vec![(1.0, vec![(0.5, 1), (0.5, 2)]), (4.0, vec![(1.0, 1)])], vec![], vec![]] };
    let s = solve(&m, &[(1.0, 0)], 100).unwrap();
    assert_eq!(s.cost, 4.0);
    let bricked = Chain { goal: 1, actions: vec![vec![(1.0, vec![(0.5, 1), (0.5, 2)])], vec![], vec![]] };
    assert_eq!(solve(&bricked, &[(1.0, 0)], 100).unwrap_err(), Error::Unreachable);
}

#[test]
fn a_cheap_loop_without_an_exit_is_not_a_policy() {
    // States 0 and 1 swap for 0.5 forever; only state 0's dear action (100) reaches the goal.
    let m = Chain { goal: 2, actions: vec![vec![(0.5, vec![(1.0, 1)]), (100.0, vec![(1.0, 2)])], vec![(0.5, vec![(1.0, 0)])], vec![]] };
    let s = solve(&m, &[(1.0, 1)], 100).unwrap();
    assert!((s.cost - 100.5).abs() < 1e-9, "{}", s.cost);
    assert_eq!(s.policy[s.index[&0]], Some(1));
    let self_loop = Chain { goal: 1, actions: vec![vec![(0.0, vec![(1.0, 0)]), (2.0, vec![(1.0, 1)])], vec![]] };
    assert_eq!(solve(&self_loop, &[(1.0, 0)], 2).unwrap().cost, 2.0);
}

/// A chain whose state `restart` starts over.
struct Restarting {
    chain: Chain,
    restart: u32,
}

struct Free(Restarting);

impl Model for Free {
    type State = u32;

    fn is_goal(&self, s: &u32) -> bool {
        self.0.is_goal(s)
    }

    fn is_restart(&self, s: &u32) -> bool {
        self.0.is_restart(s)
    }

    fn restarts_anywhere(&self) -> bool {
        true
    }

    fn actions(&self, s: &u32) -> Vec<(usize, f64)> {
        self.0.actions(s)
    }

    fn outcomes(&self, s: &u32, a: usize) -> Vec<(f64, u32)> {
        self.0.outcomes(s, a)
    }
}

struct Deferred<M>(M);

struct Bounded<M> {
    model: M,
    values: Vec<Option<f64>>,
}

impl<M: Model<State = u32>> Model for Bounded<M> {
    type State = u32;

    fn is_goal(&self, s: &u32) -> bool {
        self.model.is_goal(s)
    }

    fn is_restart(&self, s: &u32) -> bool {
        self.model.is_restart(s)
    }

    fn restarts_anywhere(&self) -> bool {
        self.model.restarts_anywhere()
    }

    fn bound(&self, s: &u32) -> Option<f64> {
        self.values[*s as usize]
    }

    fn actions(&self, s: &u32) -> Vec<(usize, f64)> {
        self.model.actions(s)
    }

    fn outcomes(&self, s: &u32, action: usize) -> Vec<(f64, u32)> {
        self.model.outcomes(s, action)
    }
}

impl<M: Model> Model for Deferred<M> {
    type State = M::State;

    fn is_goal(&self, s: &M::State) -> bool {
        self.0.is_goal(s)
    }

    fn is_restart(&self, s: &M::State) -> bool {
        self.0.is_restart(s)
    }

    fn gives_up(&self) -> bool {
        self.0.gives_up()
    }

    fn restarts_anywhere(&self) -> bool {
        self.0.restarts_anywhere()
    }

    fn bound(&self, s: &M::State) -> Option<f64> {
        self.0.bound(s)
    }

    fn actions(&self, s: &M::State) -> Vec<(usize, f64)> {
        self.0.actions(s)
    }

    fn defers(&self, _: &M::State, action: usize) -> bool {
        action == 1
    }

    fn outcomes(&self, s: &M::State, action: usize) -> Vec<(f64, M::State)> {
        self.0.outcomes(s, action)
    }

    fn prepare_expectations<'a>(
        &'a self,
        _: &[M::State],
        value: &'a (dyn Fn(&M::State) -> f64 + Sync),
    ) -> impl Fn(&M::State, usize) -> Option<f64> + Sync + 'a {
        move |s, action| {
            let out = self.0.outcomes(s, action);
            (!out.is_empty()).then(|| out.iter().filter(|o| o.0 > 0.0).map(|(p, t)| p * value(t)).sum())
        }
    }

    fn discover_successors(&self, states: &[&M::State], action: usize, missing: &(dyn Fn(&M::State) -> bool + Sync)) -> Vec<M::State> {
        states.iter().flat_map(|s| self.0.outcomes(s, action)).filter(|(p, t)| *p > 0.0 && missing(t)).map(|(_, t)| t).collect()
    }
}

#[test]
fn deferred_actions_are_checked_before_accepting_a_policy() {
    for (price, target, expected) in [(12.0, 2, 10.0), (7.0, 2, 7.0), (1.0, 3, 2.0)] {
        let model = Deferred(Chain {
            goal: 2,
            actions: vec![
                vec![(5.0, vec![(1.0, 1)]), (price, vec![(1.0, target)])],
                vec![(5.0, vec![(1.0, 2)])],
                vec![],
                vec![(1.0, vec![(1.0, 2)])],
            ],
        });
        let solution = solve(&model, &[(1.0, 0)], 4).unwrap();
        assert_eq!(solution.cost, expected);
    }
}

#[test]
fn deferred_improvements_behind_an_alternative_route_are_considered() {
    let model = Deferred(Chain {
        goal: 3,
        actions: vec![
            vec![(1.0, vec![(1.0, 1)]), (1.0, vec![(1.0, 2)])],
            vec![(10.0, vec![(1.0, 3)])],
            vec![(100.0, vec![(1.0, 3)]), (1.0, vec![(1.0, 3)])],
            vec![],
        ],
    });
    let solution = solve(&model, &[(1.0, 0)], 4).unwrap();
    assert_eq!(solution.cost, 2.0);
}

#[test]
fn a_deferred_cycle_does_not_replace_a_proper_policy() {
    let model = Deferred(Chain {
        goal: 2,
        actions: vec![vec![(10.0, vec![(1.0, 2)]), (1.0, vec![(1.0, 1)])], vec![(1.0, vec![(1.0, 0)])], vec![]],
    });
    let solution = solve(&model, &[(1.0, 0)], 3).unwrap();
    assert_eq!(solution.cost, 10.0);
}

#[test]
fn expanding_a_deferred_exit_can_leave_an_unreachable_cycle() {
    let model =
        Deferred(Chain { goal: 2, actions: vec![vec![(1.0, vec![(1.0, 1)]), (1.0, vec![(1.0, 1)])], vec![(1.0, vec![(1.0, 0)])], vec![]] });
    assert_eq!(solve(&model, &[(1.0, 0)], 3).unwrap_err(), Error::Unreachable);
}

#[test]
fn deferred_search_matches_enumerated_policies() {
    let mut seed = 17_u64;
    let mut roll = || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        seed
    };
    for case in 0..128 {
        let free = case % 2 == 0;
        let mut actions: Vec<Vec<Action>> = (0..3)
            .map(|_| {
                (0..2)
                    .map(|_| {
                        let cost = (roll() % 19 + 1) as f64;
                        let chance = (roll() % 9 + 1) as f64 / 10.0;
                        let next = (roll() % 4) as u32;
                        (cost, vec![(chance, 3), (1.0 - chance, if next == 3 { 4 } else { next })])
                    })
                    .collect()
            })
            .collect();
        if free {
            for row in &mut actions {
                row.push((0.0, vec![(0.0, 3), (1.0, 4)]));
            }
        }
        let count = actions[0].len();
        let expected = (0..count.pow(3))
            .map(|policy| {
                let (mut state, mut cost, mut probability) = (0, 0.0, 1.0);
                let mut seen = [None; 3];
                loop {
                    if let Some((before, mass)) = seen[state] {
                        break if probability == mass { f64::INFINITY } else { before + (cost - before) / (1.0 - probability / mass) };
                    }
                    seen[state] = Some((cost, probability));
                    let action = &actions[state][policy / count.pow(state as u32) % count];
                    cost += probability * action.0;
                    probability *= action.1[1].0;
                    state = action.1[1].1 as usize;
                    if state == 4 {
                        state = 0;
                    }
                }
            })
            .fold(f64::INFINITY, f64::min);
        actions.extend([Vec::new(), Vec::new()]);
        let model = Restarting { chain: Chain { goal: 3, actions }, restart: 4 };
        let (eager, deferred) = if free {
            let model = Free(model);
            (solve(&model, &[(1.0, 0)], 5).unwrap(), solve(&Deferred(model), &[(1.0, 0)], 5).unwrap())
        } else {
            (solve(&model, &[(1.0, 0)], 5).unwrap(), solve(&Deferred(model), &[(1.0, 0)], 5).unwrap())
        };
        for solution in [eager, deferred] {
            assert!((solution.cost - expected).abs() < 1e-9 * expected, "case {case}: {} vs {expected}", solution.cost);
        }
    }
}

#[test]
fn a_deferred_retry_can_supply_the_first_feasible_policy() {
    let model = Deferred(Free(Restarting {
        chain: Chain {
            goal: 1,
            actions: vec![vec![(1.0, vec![(1.0, 0)]), (1.0, vec![(0.5, 1), (0.5, 0)]), (0.0, vec![(1.0, 2)])], vec![], vec![]],
        },
        restart: 2,
    }));
    let solution = solve(&model, &[(1.0, 0)], 3).unwrap();
    assert!((solution.cost - 2.0).abs() < 1e-10);
}

#[test]
fn global_checks_find_improvements_behind_another_route() {
    let model = Deferred(Free(Restarting {
        chain: Chain {
            goal: 3,
            actions: vec![
                vec![(1.0, vec![(1.0, 1)]), (1.0, vec![(1.0, 2)]), (0.0, vec![(1.0, 4)])],
                vec![(10.0, vec![(1.0, 3)]), (20.0, vec![(1.0, 3)]), (0.0, vec![(1.0, 4)])],
                vec![(100.0, vec![(1.0, 3)]), (1.0, vec![(1.0, 3)]), (0.0, vec![(1.0, 4)])],
                vec![],
                vec![],
            ],
        },
        restart: 4,
    }));
    let solution = solve(&model, &[(1.0, 0)], 5).unwrap();
    assert!((solution.cost - 2.0).abs() < 1e-10);
}

#[test]
fn discovering_a_hidden_successor_preserves_its_continuation_cost() {
    let model = Deferred(Free(Restarting {
        chain: Chain {
            goal: 3,
            actions: vec![
                vec![(5.0, vec![(1.0, 1)])],
                vec![(8.0, vec![(1.0, 3)]), (3.0, vec![(1.0, 2)]), (0.0, vec![(1.0, 4)])],
                vec![(4.0, vec![(1.0, 3)]), (0.0, vec![(1.0, 4)])],
                vec![],
                vec![],
            ],
        },
        restart: 4,
    }));
    let solution = solve(&model, &[(1.0, 0)], 5).unwrap();
    assert_eq!(solution.cost, 12.0);
    assert_eq!(solution.stats.discovered, 1);
}

#[test]
fn inconsistent_admissible_bounds_can_price_unvisited_successors() {
    for (finishing, bound, expected, limit) in [(7.0, 7.0, 13.0, 4), (2.0, 1.0, 10.0, 6)] {
        let model = Deferred(Bounded {
            model: Free(Restarting {
                chain: Chain {
                    goal: 4,
                    actions: vec![
                        vec![(5.0, vec![(1.0, 1)])],
                        vec![(8.0, vec![(1.0, 4)]), (3.0, vec![(1.0, 2)]), (0.0, vec![(1.0, 5)])],
                        vec![(0.0, vec![(1.0, 3)]), (0.0, vec![(1.0, 5)])],
                        vec![(finishing, vec![(1.0, 4)]), (0.0, vec![(1.0, 5)])],
                        vec![],
                        vec![],
                    ],
                },
                restart: 5,
            }),
            values: vec![None, None, Some(bound), Some(0.0), None, None],
        });
        let solution = solve(&model, &[(1.0, 0)], limit).unwrap();
        assert_eq!(solution.cost, expected);
        if limit == 4 {
            assert!(!solution.index.contains_key(&2));
            assert_eq!(solution.explored, 4);
        }
    }
}

#[test]
fn unvisited_tips_remain_admissible_boundaries() {
    let model = Bounded {
        model: Free(Restarting {
            chain: Chain {
                goal: 4,
                actions: vec![
                    vec![(5.0, vec![(1.0, 1)])],
                    vec![(8.0, vec![(1.0, 4)]), (3.0, vec![(1.0, 2)]), (0.0, vec![(1.0, 5)])],
                    vec![(0.0, vec![(1.0, 3)]), (0.0, vec![(1.0, 5)])],
                    vec![(7.0, vec![(1.0, 4)]), (0.0, vec![(1.0, 5)])],
                    vec![],
                    vec![],
                ],
            },
            restart: 5,
        }),
        values: vec![None, None, Some(7.0), Some(0.0), None, None],
    };
    let solution = solve(&model, &[(1.0, 0)], 5).unwrap();
    assert_eq!(solution.cost, 13.0);
    assert_eq!(solution.explored, 5);
    assert!(!solution.index.contains_key(&2));
}

#[test]
fn deferred_improvements_precede_ceiling_growth() {
    let model = Deferred(Free(Restarting {
        chain: Chain {
            goal: 2,
            actions: vec![
                vec![(5.0, vec![(1.0, 1)])],
                vec![(1.0, vec![(0.01, 2), (0.99, 1)]), (2.0, vec![(1.0, 2)]), (50.0, vec![(1.0, 4)]), (0.0, vec![(1.0, 3)])],
                vec![],
                vec![],
                vec![(1.0, vec![(1.0, 2)])],
            ],
        },
        restart: 3,
    }));
    let solution = solve(&model, &[(1.0, 0)], 4).unwrap();
    assert_eq!(solution.cost, 7.0);
    assert_eq!(solution.explored, 4);
}

#[test]
fn tip_expansion_uses_the_converged_restart_value() {
    let model = Bounded {
        model: Free(Restarting {
            chain: Chain {
                goal: 2,
                actions: vec![
                    vec![(1.0, vec![(0.5, 2), (0.5, 1)]), (0.0, vec![(1.0, 3)])],
                    vec![(0.0, vec![(1.0, 3)]), (1.4, vec![(1.0, 4)])],
                    vec![],
                    vec![],
                    vec![(1.0, vec![(1.0, 5)]), (0.0, vec![(1.0, 3)])],
                    vec![(1.0, vec![(1.0, 2)]), (0.0, vec![(1.0, 3)])],
                ],
            },
            restart: 3,
        }),
        values: vec![Some(1.5), Some(0.0), None, None, Some(1.0), Some(0.0)],
    };
    let solution = solve(&model, &[(1.0, 0)], 5).unwrap();
    assert_eq!(solution.cost, 2.0);
    assert_eq!(solution.explored, 5);
}

impl Model for Restarting {
    type State = u32;
    fn is_goal(&self, s: &u32) -> bool {
        self.chain.is_goal(s)
    }
    fn is_restart(&self, s: &u32) -> bool {
        *s == self.restart
    }
    fn actions(&self, s: &u32) -> Vec<(usize, f64)> {
        self.chain.actions(s)
    }
    fn outcomes(&self, s: &u32, a: usize) -> Vec<(f64, u32)> {
        self.chain.outcomes(s, a)
    }
}

#[test]
fn bricked_items_start_over() {
    // From 0: cost 1, half the time the goal (1), half a brick (2). The brick can only be
    // abandoned: cost 3 to buy afresh (state 3 restarts). V0 = 1 + 0.5 (3 + V0) = 5.
    let chain = Chain { goal: 1, actions: vec![vec![(1.0, vec![(0.5, 1), (0.5, 2)])], vec![], vec![(3.0, vec![(1.0, 3)])], vec![]] };
    let m = Restarting { chain, restart: 3 };
    let s = solve(&m, &[(1.0, 0)], 100).unwrap();
    assert!((s.cost - 5.0).abs() < 1e-9, "{}", s.cost);
}

#[test]
fn restarting_competes_with_repair() {
    // The brick (2) can also be repaired: cost 10 back to 0. Restarting (3 + V0 = 8) is cheaper,
    // until it costs more: with a restart price of 20, repair wins (V0 = 1 + 0.5 (10 + V0) = 12).
    for (price, want) in [(3.0, 5.0), (20.0, 12.0)] {
        let chain = Chain {
            goal: 1,
            actions: vec![vec![(1.0, vec![(0.5, 1), (0.5, 2)])], vec![], vec![(price, vec![(1.0, 3)]), (10.0, vec![(1.0, 0)])], vec![]],
        };
        let m = Restarting { chain, restart: 3 };
        let s = solve(&m, &[(1.0, 0)], 100).unwrap();
        assert!((s.cost - want).abs() < 1e-9, "price {price}: {}", s.cost);
    }
}

#[test]
fn a_large_retry_cycle_matches_its_geometric_cost() {
    let count = 96;
    let mut actions: Vec<_> = (0..count)
        .map(|i| {
            vec![(f64::from(i + 1), vec![(0.99, (i + 1) % count), (0.009, count + 1), (0.001, count)]), (100_000.0, vec![(1.0, count)])]
        })
        .collect();
    actions.extend([Vec::new(), Vec::new()]);
    let model = Restarting { chain: Chain { goal: count, actions }, restart: count + 1 };
    let cycle = |start| {
        (0..count).map(|k| 0.99_f64.powi(k as i32) * f64::from((start + k) % count + 1)).sum::<f64>() / (1.0 - 0.99_f64.powi(count as i32))
    };
    let expected = cycle(0) / 0.1;
    let solution = solve(&model, &[(1.0, 0)], 200).unwrap();
    assert!((solution.cost - expected).abs() < 1e-8 * expected, "{} vs {expected}", solution.cost);
    for state in 0..count {
        let value = cycle(state) + 0.9 * expected;
        assert!((solution.values[solution.index[&state]] - value).abs() < 1e-8 * value);
    }
}

#[test]
fn a_sparse_branching_policy_matches_its_known_potential() {
    let count = 257;
    let expected = |i: u32| 1000.0 + 0.01 * f64::from(i % 31);
    let mut actions: Vec<_> = (0..count)
        .map(|i| {
            let a = (i + 1) % count;
            let b = (i + 37) % count;
            let cost = expected(i) - 0.54 * expected(a) - 0.45 * expected(b) - 0.009 * expected(0);
            assert!(cost > 0.0);
            vec![(cost, vec![(0.54, a), (0.45, b), (0.009, count + 1), (0.001, count)]), (2000.0, vec![(1.0, count)])]
        })
        .collect();
    actions.extend([Vec::new(), Vec::new()]);
    let model = Restarting { chain: Chain { goal: count, actions }, restart: count + 1 };
    let solution = solve(&model, &[(1.0, 0)], 300).unwrap();
    assert!((solution.cost - expected(0)).abs() < 5e-8);
    for state in 0..count {
        assert!((solution.values[solution.index[&state]] - expected(state)).abs() < 5e-8);
    }
}

#[test]
fn a_sparse_band_policy_keeps_its_exact_geometric_value() {
    let count = 129_u32;
    let mut actions: Vec<_> =
        (0..count).map(|i| vec![(1.0, vec![(0.25, i.saturating_sub(1)), (0.25, (i + 1).min(count - 1)), (0.5, count)])]).collect();
    actions.push(Vec::new());
    let model = Chain { goal: count, actions };
    let solution = solve(&model, &[(1.0, 0)], 200).unwrap();
    assert!((solution.cost - 2.0).abs() < 1e-11);
    for state in 0..count {
        assert!((solution.values[solution.index[&state]] - 2.0).abs() < 1e-11);
    }
}

#[test]
fn only_the_finished_policy_is_retained() {
    let m = Chain { goal: 1, actions: vec![vec![(1.0, vec![(1.0, 1)]), (2.0, vec![(1.0, 2)])], vec![], vec![(100.0, vec![(1.0, 1)])]] };
    let s = solve(&m, &[(1.0, 0)], 3).unwrap();
    assert_eq!(s.cost, 1.0);
    assert_eq!(s.explored, 3);
    assert_eq!(s.actions, 3);
    assert_eq!(s.states.len(), 2);
    assert!(!s.index.contains_key(&2));
    assert_eq!(ends(&m, &s, &[(1.0, 0)]), vec![(1.0, 1)]);
    assert_eq!(solve(&m, &[(1.0, 0)], 2).unwrap_err(), Error::TooManyStates(2));
}
