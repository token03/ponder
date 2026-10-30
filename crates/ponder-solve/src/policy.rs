use std::cell::OnceCell;

use crate::components;
use crate::linear::{SparseMatrix, direct_solve, solve_dense};

pub struct PolicySystem {
    outcomes: Vec<Vec<(f64, usize)>>,
    reduction: Reduction,
}

impl PolicySystem {
    pub fn new(outcomes: Vec<Vec<(f64, usize)>>) -> Option<Self> {
        let n = outcomes.len();
        if outcomes
            .iter()
            .any(|row| row.iter().any(|&(p, t)| !p.is_finite() || p < 0.0 || t >= n) || row.iter().map(|r| r.0).sum::<f64>() > 1.0 + 1e-8)
        {
            return None;
        }
        let reduction = Reduction::new(&outcomes);
        Some(Self { outcomes, reduction })
    }

    pub fn outcomes(&self, state: usize) -> &[(f64, usize)] {
        &self.outcomes[state]
    }

    pub fn values(&self, rewards: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
        let n = self.outcomes.len();
        if rewards.iter().any(|rhs| rhs.len() != n || rhs.iter().any(|v| !v.is_finite())) {
            return None;
        }
        let reduced = &self.reduction;
        let (offsets, rhs) = reduced.right_hand_sides(&self.outcomes, rewards);
        let values = solve_components(&reduced.rows, &rhs)?;
        Some(reduced.expand(offsets, values))
    }

    pub fn visits(&self, start: &[f64]) -> Option<Vec<f64>> {
        if start.len() != self.outcomes.len() || start.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return None;
        }
        let reduced = &self.reduction;
        let n = reduced.states.len();
        let mut initial = vec![0.0; n];
        for (s, &mass) in start.iter().enumerate() {
            initial[reduced.destination[s]] += mass * reduced.weight[s];
        }
        let mut transposed = vec![Vec::new(); n];
        for (s, row) in reduced.rows.iter().enumerate() {
            for &(p, t) in row {
                transposed[t].push((p, s));
            }
        }
        let values = solve_components(&transposed, &[initial])?;
        let mut visits = start.to_vec();
        for (i, &s) in reduced.states.iter().enumerate() {
            visits[s] = values[0][i];
        }
        for &s in &reduced.states {
            for &(p, t) in &self.outcomes[s] {
                if !reduced.retained[t] {
                    visits[t] += p * visits[s];
                }
            }
        }
        for &s in reduced.eliminated.iter().rev() {
            let (p, t) = self.outcomes[s][0];
            if !reduced.retained[t] {
                visits[t] += p * visits[s];
            }
        }
        visits.iter().all(|v| v.is_finite()).then_some(visits)
    }
}

pub(super) struct Reduction {
    retained: Vec<bool>,
    states: Vec<usize>,
    destination: Vec<usize>,
    weight: Vec<f64>,
    eliminated: Vec<usize>,
    pub(super) rows: Vec<Vec<(f64, usize)>>,
}

impl Reduction {
    pub(super) fn right_hand_sides(&self, outcomes: &[Vec<(f64, usize)>], rewards: &[Vec<f64>]) -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
        let mut offsets = vec![vec![0.0; outcomes.len()]; rewards.len()];
        for &s in &self.eliminated {
            let (p, t) = outcomes[s][0];
            for (offset, reward) in offsets.iter_mut().zip(rewards) {
                offset[s] = reward[s] + p * offset[t];
            }
        }
        let rhs = rewards
            .iter()
            .zip(&offsets)
            .map(|(reward, offset)| {
                self.states.iter().map(|&s| reward[s] + outcomes[s].iter().map(|&(p, t)| p * offset[t]).sum::<f64>()).collect()
            })
            .collect();
        (offsets, rhs)
    }

    pub(super) fn expand(&self, offsets: Vec<Vec<f64>>, values: Vec<Vec<f64>>) -> Vec<Vec<f64>> {
        offsets
            .into_iter()
            .zip(values)
            .map(|(offset, values)| (0..self.destination.len()).map(|s| offset[s] + self.weight[s] * values[self.destination[s]]).collect())
            .collect()
    }

    pub(super) fn new(outcomes: &[Vec<(f64, usize)>]) -> Self {
        let n = outcomes.len();
        let mut retained: Vec<bool> =
            outcomes.iter().enumerate().map(|(s, row)| row.len() != 1 || row[0].0 == 0.0 || row[0].1 == s).collect();
        let parts = components(n, |s| outcomes[s].iter().filter(|r| r.0 > 0.0).map(|r| r.1));
        for part in parts.iter() {
            if part.len() > 1 && !part.iter().any(|&s| retained[s]) {
                retained[part[0]] = true;
            }
        }
        let states: Vec<_> = (0..n).filter(|&s| retained[s]).collect();
        let mut destination = vec![usize::MAX; n];
        let mut weight = vec![0.0; n];
        for (i, &s) in states.iter().enumerate() {
            destination[s] = i;
            weight[s] = 1.0;
        }
        let mut eliminated = Vec::new();
        let mut path = Vec::new();
        for source in 0..n {
            let mut s = source;
            while destination[s] == usize::MAX {
                path.push(s);
                s = outcomes[s][0].1;
            }
            while let Some(s) = path.pop() {
                let (p, t) = outcomes[s][0];
                destination[s] = destination[t];
                weight[s] = p * weight[t];
                eliminated.push(s);
            }
        }
        let rows = states
            .iter()
            .map(|&s| {
                let mut row: Vec<(f64, usize)> = Vec::new();
                let mut positions = rustc_hash::FxHashMap::default();
                for &(p, t) in &outcomes[s] {
                    let position = *positions.entry(destination[t]).or_insert_with(|| {
                        row.push((0.0, destination[t]));
                        row.len() - 1
                    });
                    row[position].0 += p * weight[t];
                }
                row
            })
            .collect();
        Self { retained, states, destination, weight, eliminated, rows }
    }
}

fn solve_components(outcomes: &[Vec<(f64, usize)>], rewards: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = outcomes.len();
    if rewards.iter().any(|rhs| rhs.len() != n || rhs.iter().any(|v| !v.is_finite())) {
        return None;
    }
    let parts = components(n, |s| outcomes[s].iter().filter(|r| r.0 > 0.0).map(|r| r.1));
    let mut values = rewards.to_vec();
    let mut local = vec![usize::MAX; n];
    for part in parts.iter() {
        for (i, &s) in part.iter().enumerate() {
            local[s] = i;
        }
        let mut rhs: Vec<Vec<f64>> = rewards
            .iter()
            .zip(&values)
            .map(|(reward, value)| {
                part.iter()
                    .map(|&s| reward[s] + outcomes[s].iter().filter(|r| local[r.1] == usize::MAX).map(|&(p, t)| p * value[t]).sum::<f64>())
                    .collect()
            })
            .collect();
        let size = part.len();
        let solved = if size <= 128 {
            let mut matrix = vec![0.0; size * size];
            for (i, &s) in part.iter().enumerate() {
                matrix[i * size + i] = 1.0;
                for &(p, t) in &outcomes[s] {
                    if local[t] != usize::MAX {
                        matrix[i * size + local[t]] -= p;
                    }
                }
            }
            solve_dense(size, &mut matrix, &mut rhs.iter_mut().collect::<Vec<_>>())?
        } else {
            let mut matrix = SparseMatrix::new(size);
            for &s in part {
                for &(p, t) in &outcomes[s] {
                    if local[t] != usize::MAX {
                        matrix.push(local[t], p);
                    }
                }
                matrix.finish_row();
            }
            let factor = OnceCell::new();
            rhs.iter().map(|b| direct_solve(&matrix, &factor, b)).collect::<Option<Vec<_>>>()?
        };
        for (value, solved) in values.iter_mut().zip(solved) {
            for (&s, v) in part.iter().zip(solved) {
                if !v.is_finite() {
                    return None;
                }
                value[s] = v;
            }
        }
        for &s in part {
            local[s] = usize::MAX;
        }
    }
    Some(values)
}
