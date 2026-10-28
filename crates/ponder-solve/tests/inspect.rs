use ponder_solve::inspect::{Options, inspect};
use ponder_solve::{Model, solve, solve_everywhere};

struct Retry {
    restart: bool,
    missing: bool,
}

impl Model for Retry {
    type State = u8;

    fn is_goal(&self, s: &u8) -> bool {
        *s == 1
    }

    fn is_restart(&self, s: &u8) -> bool {
        self.restart && *s == 2
    }

    fn actions(&self, _: &u8) -> Vec<(usize, f64)> {
        vec![(0, 2.0)]
    }

    fn outcomes(&self, _: &u8, _: usize) -> Vec<(f64, u8)> {
        if self.missing {
            return vec![(1.0, 9)];
        }
        vec![(0.5, 1), (0.5, if self.restart { 2 } else { 0 })]
    }
}

fn uniform(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut x = *seed;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((x ^ (x >> 31)) >> 11) as f64 / (1u64 << 53) as f64
}

#[test]
fn measures_retry_visits_without_turning_restarts_into_a_giant_component() {
    for restart in [false, true] {
        let model = Retry { restart, missing: false };
        let solution = solve(&model, &[(1.0, 0)], 10).unwrap();
        let mut seed = 17;
        let report = inspect(&model, &solution, &[(1.0, 0)], Options { runs: 20_000, max_steps: 100 }, || uniform(&mut seed)).unwrap();
        assert_eq!(report.finished, report.runs);
        assert!((report.mean_cost - solution.cost).abs() < 0.1, "{}", report.mean_cost);
        assert!((report.cost_stddev - 8.0f64.sqrt()).abs() < 0.15);
        let start = &report.states[solution.index[&0]];
        assert!(start.reachable);
        assert_eq!(start.runs_reaching, report.runs);
        assert!((start.visits as f64 / report.runs as f64 - 2.0).abs() < 0.05);
        assert_eq!(report.loops.len(), usize::from(!restart));
        assert!(start.residual.abs() < 1e-10);
    }
}

#[test]
fn incomplete_runs_are_reported_and_not_counted_as_successes() {
    let model = Retry { restart: false, missing: false };
    let solution = solve(&model, &[(1.0, 0)], 10).unwrap();
    let report = inspect(&model, &solution, &[(1.0, 0)], Options { runs: 3, max_steps: 2 }, || 0.9).unwrap();
    assert_eq!(report.finished, 0);
    assert_eq!(report.states[solution.index[&0]].visits, 9);
}

#[test]
fn missing_successors_are_errors_instead_of_free_finishes() {
    let mut model = Retry { restart: false, missing: false };
    let solution = solve(&model, &[(1.0, 0)], 10).unwrap();
    model.missing = true;
    let error = inspect(&model, &solution, &[(1.0, 0)], Options::default(), || 0.5).unwrap_err();
    assert!(error.contains("missing from policy"));
}

#[test]
fn invalid_start_distribution_is_rejected() {
    let model = Retry { restart: false, missing: false };
    let solution = solve(&model, &[(1.0, 0)], 10).unwrap();
    assert!(inspect(&model, &solution, &[(0.5, 0)], Options::default(), || 0.5).is_err());
}

#[test]
fn off_policy_dead_ends_can_be_retained_without_breaking_inspection() {
    struct DeadEnd;

    impl Model for DeadEnd {
        type State = u8;

        fn is_goal(&self, s: &u8) -> bool {
            *s == 1
        }

        fn actions(&self, s: &u8) -> Vec<(usize, f64)> {
            if *s == 0 { vec![(0, 2.0), (1, 1.0)] } else { Vec::new() }
        }

        fn outcomes(&self, _: &u8, action: usize) -> Vec<(f64, u8)> {
            vec![(1.0, if action == 0 { 1 } else { 2 })]
        }
    }

    let solution = solve_everywhere(&DeadEnd, &[(1.0, 0)], 10).unwrap();
    let report = inspect(&DeadEnd, &solution, &[(1.0, 0)], Options::default(), || 0.5).unwrap();
    assert!(!report.states[solution.index[&2]].reachable);
    assert_eq!(report.finished, report.runs);
    assert_eq!(report.mean_cost, 2.0);
}
