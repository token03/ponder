use crate::linear::{SparseMatrix, SparseRows};

pub(super) struct Ilu {
    lower: SparseRows,
    upper: SparseRows,
    diagonal: Vec<f64>,
}

impl Ilu {
    pub(super) fn new(probabilities: &SparseMatrix) -> Option<Self> {
        let n = probabilities.len();
        let mut lower = SparseRows::new(n);
        let mut upper = SparseRows::new(n);
        let mut diagonal = Vec::with_capacity(n);
        let mut work = vec![0.0; n];
        let mut marked = vec![usize::MAX; n];
        let mut columns = Vec::new();
        for (i, source) in probabilities.rows().enumerate() {
            columns.clear();
            columns.extend(source.iter().map(|&(j, _)| j).chain(std::iter::once(i)));
            columns.sort_unstable();
            columns.dedup();
            for &j in &columns {
                work[j] = 0.0;
                marked[j] = i;
            }
            work[i] = 1.0;
            for &(j, p) in source {
                work[j] -= p;
            }
            for j in columns.iter().copied().take_while(|&j| j < i) {
                let multiplier = work[j] / diagonal[j];
                if multiplier == 0.0 {
                    continue;
                }
                lower.push(j, multiplier);
                for &(k, value) in upper.row(j) {
                    if marked[k] == i {
                        work[k] -= multiplier * value;
                    }
                }
            }
            let pivot = work[i];
            if !pivot.is_finite() || pivot < 1e-14 {
                return None;
            }
            for &j in columns.iter().filter(|&&j| j > i && work[j] != 0.0) {
                upper.push(j, work[j]);
            }
            lower.finish_row();
            upper.finish_row();
            diagonal.push(pivot);
        }
        Some(Self { lower, upper, diagonal })
    }

    pub(super) fn apply(&self, rhs: &[f64], x: &mut [f64]) {
        x.copy_from_slice(rhs);
        for (i, row) in self.lower.rows().enumerate() {
            let inflow: f64 = row.iter().map(|&(j, value)| value * x[j]).sum();
            x[i] -= inflow;
        }
        for (i, row) in self.upper.rows().enumerate().rev() {
            x[i] = (x[i] - row.iter().map(|&(j, value)| value * x[j]).sum::<f64>()) / self.diagonal[i];
        }
    }
}
