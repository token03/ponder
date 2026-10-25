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
    assert_eq!(solution.bounds, Bounds::exact(2.0));
    assert_eq!(solution.policy, vec![10]);
    assert_eq!(solution.states, vec![state(0)]);
    let policy = solution.states.iter().copied().zip(solution.policy).collect();
    assert_eq!(search::evaluate(&model, &policy, Options::default()).unwrap(), 2.0);
}

#[test]
fn interval_evaluation_and_refinement_share_the_same_model_contract() {
    let mut model =
        RetryModel { actions: vec![Decision { id: 10, cost: 1.0 }, Decision { id: 20, cost: 3.0 }], interval: true, refinable: false };
    let bounded = search::solve(&mut model, Options::default()).unwrap();
    assert!((bounded.cost - 2.5).abs() < 1e-10);
    assert!((bounded.bounds.lower - 1.25).abs() < 1e-10);
    assert!((bounded.bounds.upper - 5.0).abs() < 1e-10);
    model.actions[0].cost = 2.0;
    let fixed = search::solve(&mut model, Options::default()).unwrap();
    assert_eq!(fixed.bounds.upper, 3.0);
    model.actions[0].cost = 1.0;
    model.refinable = true;
    let refined = search::solve(&mut model, Options::default()).unwrap();
    assert_eq!(refined.cost, 2.0);
    assert_eq!(refined.bounds, Bounds::exact(2.0));
    assert!(!model.interval);
}

#[test]
fn competitive_search_requires_a_certificate_and_keeps_equal_cost_plans() {
    let mut model = RetryModel { actions: vec![Decision { id: 10, cost: 1.0 }], interval: true, refinable: false };
    let Err(error) = search::solve(&mut model, Options { cost_ceiling: Some(1.0), ..Default::default() }) else {
        panic!("the certified lower bound exceeds the ceiling");
    };
    assert!(matches!(error, SolveError::Dominated { lower_bound, ceiling: 1.0 } if (lower_bound - 1.25).abs() < 1e-10));
    assert!(model.interval);

    model.refinable = false;
    let uncertain = search::solve(&mut model, Options { cost_ceiling: Some(2.0), ..Default::default() }).unwrap();
    assert!((uncertain.cost - 2.5).abs() < 1e-10);
    assert!(uncertain.bounds.lower < 2.0);

    model.refinable = true;
    let tied = search::solve(&mut model, Options { cost_ceiling: Some(2.0), ..Default::default() }).unwrap();
    assert_eq!(tied.cost, 2.0);
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

#[test]
fn interval_certificates_match_closed_form_for_large_positive_and_negative_cost_models() {
    for (retry, cost, range, lower, upper) in
        [(0.99, 0.02, (0.97, 0.995), 0.02 / 0.03, 0.02 / 0.005), (0.5, -0.1, (0.25, 0.75), -0.1 / 0.25, -0.1 / 0.75)]
    {
        for backend in [Backend::Dense, Backend::Split] {
            let mut model = LongRetryModel {
                actions: vec![
                    Decision { id: 0, cost: 3.0 },
                    Decision { id: 1, cost },
                    Decision { id: 2, cost: 0.25 },
                    Decision { id: 3, cost: 0.0 },
                ],
                retry,
                range: Some(range),
            };
            let solution = search::solve(&mut model, Options { backend, ..Default::default() }).unwrap();
            assert!((solution.cost - cost / (1.0 - retry)).abs() < 1e-10);
            assert!((solution.bounds.lower - lower).abs() < 1e-10, "{backend:?}: {:?}", solution.bounds);
            assert!((solution.bounds.upper - upper).abs() < 1e-10, "{backend:?}: {:?}", solution.bounds);
        }
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

#[test]
fn interval_policies_detect_a_closed_adversarial_cycle_and_a_forced_escape() {
    let outcomes = vec![(0.25, state(0)), (0.25, state(1)), (0.5, state(32))];
    let mut model = SharedRetryModel {
        actions: vec![Decision { id: 0, cost: 1.0 }],
        outcomes: vec![std::sync::Arc::new(Distribution::Bounded {
            outcomes: outcomes.clone(),
            bounds: vec![(0.0, 0.5), (0.0, 0.5), (0.0, 1.0)],
        })],
    };
    let unbounded = search::solve(&mut model, Options::default()).unwrap();
    assert!((unbounded.cost - 2.0).abs() < 1e-10);
    assert_eq!(unbounded.bounds.upper, f64::INFINITY);
    assert!((unbounded.bounds.lower - 1.0).abs() < 1e-10);
    model.outcomes[0] = std::sync::Arc::new(Distribution::Bounded { outcomes, bounds: vec![(0.0, 0.5), (0.0, 0.5), (0.1, 1.0)] });
    let bounded = search::solve(&mut model, Options::default()).unwrap();
    assert!((bounded.cost - 2.0).abs() < 1e-10);
    assert!((bounded.bounds.upper - 10.0).abs() < 1e-10);
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
fn uncertainty_in_an_unused_route_does_not_invalidate_the_selected_policy_bound() {
    let mut model =
        UnusedUncertainty { actions: vec![Decision { id: 0, cost: 1.0 }, Decision { id: 1, cost: 10.0 }, Decision { id: 2, cost: 1.0 }] };
    let plan = search::solve(&mut model, Options::default()).unwrap();
    assert_eq!(plan.cost, 1.0);
    assert_eq!(plan.bounds, Bounds::exact(1.0));
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
fn competing_uncertain_actions_are_refined_together_before_rebuilding_the_graph() {
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
    assert_eq!(model.passes, 1);
    assert_eq!(model.refined, [true; 3]);
    assert_eq!(plan.cost, 3.0);
    assert_eq!(plan.bounds, Bounds::exact(3.0));
}
