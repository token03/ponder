use ponder_solve::policy::PolicySystem;

#[test]
fn policy_values_and_visits_agree_on_expected_reward() {
    let policy = PolicySystem::new(vec![vec![(0.5, 0), (0.25, 1)], vec![(0.5, 0)]]).unwrap();
    let rewards = vec![vec![1.0, 2.0], vec![-1.0, -2.0]];
    let values = policy.values(&rewards).unwrap();
    let visits = policy.visits(&[1.0, 0.0]).unwrap();
    assert!((visits[0] - 8.0 / 3.0).abs() < 1e-12);
    assert!((visits[1] - 2.0 / 3.0).abs() < 1e-12);
    for (reward, value) in rewards.iter().zip(values) {
        let expected: f64 = visits.iter().zip(reward).map(|(n, r)| n * r).sum();
        assert!((value[0] - expected).abs() < 1e-12);
    }
}

#[test]
fn deterministic_retry_cycle_preserves_costs_and_occupancy() {
    let n = 129;
    let outcomes = (0..n).map(|s| vec![(if s + 1 == n { 0.25 } else { 1.0 }, (s + 1) % n)]).collect();
    let policy = PolicySystem::new(outcomes).unwrap();
    let values = policy.values(&[vec![1.0; n]]).unwrap();
    let mut start = vec![0.0; n];
    start[0] = 1.0;
    let visits = policy.visits(&start).unwrap();
    assert!((values[0][0] - n as f64 / 0.75).abs() < 1e-8);
    assert!(visits.iter().all(|n| (n - 1.0 / 0.75).abs() < 1e-9));
}

#[test]
fn sparse_policy_evaluation_agrees_with_its_transposed_system() {
    let n = 129;
    let outcomes = (0..n).map(|s| vec![(0.5, (s + 1) % n), (0.25, (s + n - 1) % n)]).collect();
    let policy = PolicySystem::new(outcomes).unwrap();
    let values = policy.values(&[vec![1.0; n]]).unwrap();
    let mut start = vec![0.0; n];
    start[0] = 1.0;
    let visits = policy.visits(&start).unwrap();
    assert!(values[0].iter().all(|v| (v - 4.0).abs() < 1e-9));
    assert!((visits.iter().sum::<f64>() - 4.0).abs() < 1e-9);
}

#[test]
fn invalid_transitions_and_closed_policies_have_no_evaluation() {
    assert!(PolicySystem::new(vec![vec![(1.0, 1)]]).is_none());
    assert!(PolicySystem::new(vec![vec![(1.1, 0)]]).is_none());
    assert!(PolicySystem::new(vec![vec![(f64::NAN, 0)]]).is_none());
    let closed = PolicySystem::new(vec![vec![(1.0, 1)], vec![(1.0, 0)]]).unwrap();
    assert!(closed.values(&[vec![1.0, 1.0]]).is_none());
    assert!(closed.visits(&[1.0, 0.0]).is_none());
}

#[test]
fn reduced_chains_preserve_intermediate_rewards_and_initial_mass() {
    let policy = PolicySystem::new(vec![vec![(0.5, 1)], vec![(1.0, 2)], vec![]]).unwrap();
    let values = policy.values(&[vec![1.0, 2.0, 3.0]]).unwrap();
    assert_eq!(values[0], vec![3.5, 5.0, 3.0]);
    assert_eq!(policy.visits(&[1.0, 1.0, 0.0]).unwrap(), vec![1.0, 1.5, 1.5]);
}
