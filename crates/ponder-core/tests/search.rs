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
    model.refinable = true;
    let refined = search::solve(&mut model, Options::default()).unwrap();
    assert_eq!(refined.cost, 2.0);
    assert_eq!(refined.bounds, Bounds::exact(2.0));
    assert!(!model.interval);
}
