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
}

/// A chain whose state `restart` starts over.
struct Restarting {
    chain: Chain,
    restart: u32,
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
