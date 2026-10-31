use crate::linear::{SparseMatrix, SparseRows};

pub(super) struct Ilu {
    lower: SparseRows,
    upper: SparseRows,
    diagonal: Vec<f64>,
    order: Option<Vec<usize>>,
}

impl Ilu {
    pub(super) fn new(probabilities: &SparseMatrix) -> Option<Self> {
        Self::factor(probabilities, None)
    }

    pub(super) fn ordered(probabilities: &SparseMatrix) -> Option<Self> {
        let n = probabilities.len();
        let entries: Vec<_> = probabilities
            .rows()
            .enumerate()
            .flat_map(|(i, row)| row.iter().map(move |&(j, p)| faer::sparse::Triplet::new(i, j, p)))
            .collect();
        let symbolic = faer::sparse::SparseColMat::try_new_from_triplets(n, n, &entries).ok()?;
        let mut order = vec![0usize; n];
        let mut inverse = vec![0usize; n];
        let req = faer::sparse::linalg::amd::order_scratch::<usize>(n, entries.len());
        let mut buffer = faer::dyn_stack::MemBuffer::new(req);
        faer::sparse::linalg::amd::order(
            &mut order,
            &mut inverse,
            symbolic.symbolic(),
            Default::default(),
            faer::dyn_stack::MemStack::new(&mut buffer),
        )
        .ok()?;
        let source: Vec<_> = probabilities.rows().collect();
        let mut permuted = SparseMatrix::new(n);
        for &i in &order {
            for &(j, p) in source[i] {
                permuted.push(inverse[j], p);
            }
            permuted.finish_row();
        }
        Self::factor(&permuted, Some(order))
    }

    fn factor(probabilities: &SparseMatrix, order: Option<Vec<usize>>) -> Option<Self> {
        let n = probabilities.len();
        let mut lower = SparseRows::new(n);
        let mut upper = SparseRows::new(n);
        let mut diagonal = Vec::with_capacity(n);
        let mut work = vec![0.0; n];
        let mut marked = vec![usize::MAX; n];
        let mut columns = Vec::new();
        let source_rows: Vec<_> = probabilities.rows().collect();
        for (i, &source) in source_rows.iter().enumerate() {
            columns.clear();
            columns.extend(source.iter().map(|&(j, _)| j).chain(std::iter::once(i)));
            if order.is_some() {
                for &(j, _) in source.iter().filter(|&&(j, _)| j < i) {
                    columns.extend(source_rows[j].iter().filter(|&&(k, _)| k > j).map(|&(k, _)| k));
                }
            }
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
        Some(Self { lower, upper, diagonal, order })
    }

    pub(super) fn apply(&self, rhs: &[f64], x: &mut [f64]) {
        if let Some(order) = &self.order {
            for (i, row) in self.lower.rows().enumerate() {
                let inflow: f64 = row.iter().map(|&(j, value)| value * x[order[j]]).sum();
                x[order[i]] = rhs[order[i]] - inflow;
            }
            for (i, row) in self.upper.rows().enumerate().rev() {
                x[order[i]] = (x[order[i]] - row.iter().map(|&(j, value)| value * x[order[j]]).sum::<f64>()) / self.diagonal[i];
            }
            return;
        }
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
