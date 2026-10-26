use ponder_core::{
    search::{self, Decision, Model, Options},
    *,
};

struct RetryModel {
    actions: Vec<Decision>,
    interval: bool,
    refinable: bool,
}

fn state(id: u8) -> State {
    State::new(NORMAL, &[id], &[])
}

struct FinishingChoices {
    actions: [Decision; 2],
}

impl Model for FinishingChoices {
    fn origin(&self) -> State {
        state(0)
    }
    fn is_goal(&self, state: &State) -> bool {
        *state == self::state(1)
    }
    fn actions(&self) -> &[Decision] {
        &self.actions
    }
    fn dominant_action(&self, _state: &State) -> Option<usize> {
        Some(0)
    }
    fn outcomes(&self, _state: &State, _action: usize, buffer: &mut TransitionBuffer) -> TransitionResult {
        buffer.clear();
        buffer.push(1.0, (1.0, 1.0), state(1));
        TransitionResult::Buffered
    }
}

#[test]
fn search_dominance_does_not_change_which_policies_can_be_evaluated() {
    let mut model = FinishingChoices { actions: [Decision { id: 0, cost: 0.0 }, Decision { id: 1, cost: 7.0 }] };
    assert_eq!(search::solve(&mut model, Options::default()).unwrap().cost, 0.0);
    let policy = StateMap::from_iter([(state(0), 1)]);
    assert_eq!(search::evaluate(&model, &policy, Options::default()).unwrap(), 7.0);
}

impl Model for RetryModel {
    fn origin(&self) -> State {
        state(0)
    }
    fn is_goal(&self, s: &State) -> bool {
        *s == state(1)
    }
    fn actions(&self) -> &[Decision] {
        &self.actions
    }

    fn measurements(&self, _state: &State, action: usize) -> Option<[f64; 4]> {
        let cost = self.actions.iter().find(|decision| decision.id == action)?.cost;
        Some([cost, cost, 1.0, 1.0 / 3600.0])
    }

    fn outcomes(&self, s: &State, action: usize, buffer: &mut TransitionBuffer) -> TransitionResult {
        buffer.clear();
        if *s != state(0) {
            return TransitionResult::Empty;
        }
        match action {
            10 if self.interval => {
                buffer.push(0.4, (0.2, 0.8), state(1));
                buffer.push(0.6, (0.2, 0.8), state(0));
            }
            10 => {
                buffer.push(0.5, (0.5, 0.5), state(1));
                buffer.push(0.5, (0.5, 0.5), state(0));
            }
            20 => buffer.push(1.0, (1.0, 1.0), state(1)),
            30 => {
                buffer.push(0.1, (0.1, 0.1), state(1));
                buffer.push(0.9, (0.9, 0.9), state(2));
            }
            _ => return TransitionResult::Empty,
        }
        TransitionResult::Buffered
    }

    fn refine(&mut self, candidates: &[(State, usize)]) -> bool {
        if self.interval && self.refinable && candidates.iter().any(|(_, a)| *a == 10) {
            self.interval = false;
            true
        } else {
            false
        }
    }
}

#[test]
fn search_handles_retries_and_dead_ends_without_a_crafting_catalogue() {
    let mut model = RetryModel {
        actions: vec![Decision { id: 10, cost: 1.0 }, Decision { id: 20, cost: 3.0 }, Decision { id: 30, cost: 0.1 }],
        interval: false,
        refinable: false,
    };
    let solution = search::solve(&mut model, Options::default()).unwrap();
    assert_eq!(solution.cost, 2.0);
    assert!(!solution.approximate);
    assert_eq!(solution.policy, vec![10]);
    assert_eq!(solution.states, vec![state(0)]);
    let risk = solution.risk.unwrap();
    assert!((risk.currency.mean - 2.0).abs() < 1e-10);
    assert!((risk.currency.variance - 2.0).abs() < 1e-10);
    assert!((risk.operations.variance - 2.0).abs() < 1e-10);
    assert!((risk.hours.variance - 2.0 / 3600.0f64.powi(2)).abs() < 1e-16);
    assert!((risk.currency.exceedance_bound(4.0).unwrap() - 1.0 / 3.0).abs() < 1e-10);
    let policy = solution.states.iter().copied().zip(solution.policy).collect();
    assert_eq!(search::evaluate(&model, &policy, Options::default()).unwrap(), 2.0);
}

#[test]
fn lumped_chances_are_flagged_and_refined_where_the_plan_uses_them() {
    let mut model =
        RetryModel { actions: vec![Decision { id: 10, cost: 1.0 }, Decision { id: 20, cost: 3.0 }], interval: true, refinable: false };
    let lumped = search::solve(&mut model, Options::default()).unwrap();
    assert!((lumped.cost - 2.5).abs() < 1e-10);
    assert!(lumped.approximate);
    model.refinable = true;
    let refined = search::solve(&mut model, Options::default()).unwrap();
    assert_eq!(refined.cost, 2.0);
    assert!(!refined.approximate);
    assert!(!model.interval);
}

struct LongRetryModel {
    actions: Vec<Decision>,
    retry: f64,
    range: Option<(f64, f64)>,
}

impl Model for LongRetryModel {
    fn origin(&self) -> State {
        state(0)
    }

    fn is_goal(&self, s: &State) -> bool {
        *s == state(96)
    }

    fn actions(&self) -> &[Decision] {
        &self.actions
    }

    fn outcomes(&self, s: &State, action: usize, buffer: &mut TransitionBuffer) -> TransitionResult {
        buffer.clear();
        match action {
            0 => buffer.push(1.0, (1.0, 1.0), state(96)),
            1 => {
                let (lo, hi) = self.range.unwrap_or((self.retry, self.retry));
                buffer.push(self.retry, (lo, hi), *s);
                buffer.push(1.0 - self.retry, (1.0 - hi, 1.0 - lo), state(96));
            }
            2 => buffer.push(1.0, (1.0, 1.0), state(s.status[0] + 1)),
            3 => buffer.push(1.0, (1.0, 1.0), *s),
            _ => unreachable!(),
        }
        TransitionResult::Buffered
    }
}

#[test]
fn sparse_policy_iteration_matches_closed_form_with_slow_retries_zero_cost_cycles_and_credits() {
    for (retry, cost) in [(0.99, 0.02), (0.5, -0.1)] {
        for backend in [Backend::Dense, Backend::Split] {
            let mut model = LongRetryModel {
                actions: vec![
                    Decision { id: 0, cost: 3.0 },
                    Decision { id: 1, cost },
                    Decision { id: 2, cost: 0.25 },
                    Decision { id: 3, cost: 0.0 },
                ],
                retry,
                range: None,
            };
            let solution = search::solve(&mut model, Options { backend, ..Default::default() }).unwrap();
            for (s, value) in solution.states.iter().zip(&solution.values) {
                let expected = (cost / (1.0 - retry)).min(0.25 * (96 - s.status[0]) as f64);
                assert!((value - expected).abs() < 1e-10, "{backend:?} {s:?}: {value} versus {expected}");
            }
            assert!(!solution.policy.contains(&3));
            let spent: f64 =
                solution.policy.iter().zip(&solution.visits).map(|(&action, visits)| model.actions[action].cost * visits).sum();
            assert!((solution.cost - spent).abs() < 1e-10);
        }
    }
}

#[test]
fn following_a_zero_cost_closed_cycle_is_singular_in_both_backends() {
    let model = LongRetryModel { actions: vec![Decision { id: 3, cost: 0.0 }], retry: 0.0, range: None };
    let policy = [(state(0), 3)].into_iter().collect();
    for backend in [Backend::Dense, Backend::Split] {
        assert!(matches!(search::evaluate(&model, &policy, Options { backend, ..Default::default() }), Err(SolveError::Singular(_))));
    }
}

struct SharedRetryModel {
    actions: Vec<Decision>,
    outcomes: Vec<std::sync::Arc<Distribution>>,
}

impl Model for SharedRetryModel {
    fn origin(&self) -> State {
        state(0)
    }

    fn is_goal(&self, s: &State) -> bool {
        *s == state(32)
    }

    fn actions(&self) -> &[Decision] {
        &self.actions
    }

    fn outcomes(&self, s: &State, action: usize, _: &mut TransitionBuffer) -> TransitionResult {
        if *s == state(33) { TransitionResult::Empty } else { TransitionResult::Shared(self.outcomes[action].clone()) }
    }
}

#[test]
fn shared_retry_distributions_preserve_reachability_and_exclude_dead_ends() {
    let safe = (0..=32).map(|i| (1.0 / 33.0, state(i))).collect();
    let mut model = SharedRetryModel {
        actions: vec![Decision { id: 0, cost: 1.0 }, Decision { id: 1, cost: 0.01 }],
        outcomes: vec![
            std::sync::Arc::new(Distribution::Exact(safe)),
            std::sync::Arc::new(Distribution::Exact(vec![(0.5, state(32)), (0.5, state(33))])),
        ],
    };
    let solution = search::solve(&mut model, Options::default()).unwrap();
    assert!((solution.cost - 33.0).abs() < 1e-10);
    assert_eq!(solution.states.len(), 32);
    assert!(solution.policy.iter().all(|&action| action == 0));
    assert!(solution.values.iter().all(|&value| (value - 33.0).abs() < 1e-10));
}

struct UnusedUncertainty {
    actions: Vec<Decision>,
}

impl Model for UnusedUncertainty {
    fn origin(&self) -> State {
        state(0)
    }
    fn is_goal(&self, s: &State) -> bool {
        *s == state(32)
    }
    fn actions(&self) -> &[Decision] {
        &self.actions
    }
    fn outcomes(&self, s: &State, action: usize, buffer: &mut TransitionBuffer) -> TransitionResult {
        buffer.clear();
        match (s.status[0], action) {
            (0, 0) => buffer.push(1.0, (1.0, 1.0), state(32)),
            (0, 1) => buffer.push(1.0, (1.0, 1.0), state(1)),
            (1, 2) => {
                buffer.push(0.5, (0.0, 1.0), state(1));
                buffer.push(0.5, (0.0, 1.0), state(32));
            }
            _ => return TransitionResult::Empty,
        }
        TransitionResult::Buffered
    }
}

#[test]
fn lumped_chances_in_an_unused_route_leave_the_plan_exact() {
    let mut model =
        UnusedUncertainty { actions: vec![Decision { id: 0, cost: 1.0 }, Decision { id: 1, cost: 10.0 }, Decision { id: 2, cost: 1.0 }] };
    let plan = search::solve(&mut model, Options::default()).unwrap();
    assert_eq!(plan.cost, 1.0);
    assert!(!plan.approximate);
    assert_eq!(plan.states.len(), 2);
}

struct CompetingRefinements {
    actions: Vec<Decision>,
    refined: [bool; 3],
    passes: usize,
}

impl Model for CompetingRefinements {
    fn origin(&self) -> State {
        state(0)
    }
    fn is_goal(&self, s: &State) -> bool {
        *s == state(32)
    }
    fn actions(&self) -> &[Decision] {
        &self.actions
    }
    fn outcomes(&self, s: &State, action: usize, buffer: &mut TransitionBuffer) -> TransitionResult {
        buffer.clear();
        if *s != state(0) {
            return TransitionResult::Empty;
        }
        if action == 20 {
            buffer.push(1.0, (1.0, 1.0), state(32));
        } else if self.refined[action - 10] {
            buffer.push(0.01, (0.01, 0.01), state(32));
            buffer.push(0.99, (0.99, 0.99), state(0));
        } else {
            buffer.push(0.1, (0.01, 1.0), state(32));
            buffer.push(0.9, (0.0, 0.99), state(0));
        }
        TransitionResult::Buffered
    }
    fn refine(&mut self, candidates: &[(State, usize)]) -> bool {
        let mut changed = false;
        for &(_, action) in candidates {
            if (10..13).contains(&action) && !self.refined[action - 10] {
                self.refined[action - 10] = true;
                changed = true;
            }
        }
        if changed {
            self.passes += 1;
        }
        changed
    }
}

#[test]
fn lumped_actions_the_plan_does_not_use_are_never_refined() {
    let mut model = CompetingRefinements {
        actions: vec![
            Decision { id: 10, cost: 1.0 },
            Decision { id: 11, cost: 1.1 },
            Decision { id: 12, cost: 1.2 },
            Decision { id: 20, cost: 3.0 },
        ],
        refined: [false; 3],
        passes: 0,
    };
    let plan = search::solve(&mut model, Options::default()).unwrap();
    assert_eq!(model.passes, 0);
    assert_eq!(model.refined, [false; 3]);
    assert_eq!(plan.cost, 3.0);
    assert!(!plan.approximate);
}
