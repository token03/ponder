pub(super) struct Ilu {
    rows: Vec<Row>,
}

struct Row {
    lower: Vec<(usize, f64)>,
    upper: Vec<(usize, f64)>,
    diagonal: f64,
}

impl Ilu {
    pub(super) fn new(probabilities: &[Vec<(usize, f64)>]) -> Option<Self> {
        let n = probabilities.len();
        let mut rows: Vec<Row> = Vec::with_capacity(n);
        let mut work = vec![0.0; n];
        let mut marked = vec![usize::MAX; n];
        for (i, source) in probabilities.iter().enumerate() {
            let mut columns: Vec<_> = source.iter().map(|&(j, _)| j).chain(std::iter::once(i)).collect();
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
            let mut lower = Vec::new();
            for j in columns.iter().copied().take_while(|&j| j < i) {
                let multiplier = work[j] / rows[j].diagonal;
                if multiplier == 0.0 {
                    continue;
                }
                lower.push((j, multiplier));
                for &(k, value) in &rows[j].upper {
                    if marked[k] == i {
                        work[k] -= multiplier * value;
                    }
                }
            }
            let diagonal = work[i];
            if !diagonal.is_finite() || diagonal < 1e-14 {
                return None;
            }
            let upper = columns.into_iter().filter(|&j| j > i && work[j] != 0.0).map(|j| (j, work[j])).collect();
            rows.push(Row { lower, upper, diagonal });
        }
        Some(Self { rows })
    }

    pub(super) fn apply(&self, rhs: &[f64]) -> Vec<f64> {
        let mut x = rhs.to_vec();
        for (i, row) in self.rows.iter().enumerate() {
            let inflow: f64 = row.lower.iter().map(|&(j, value)| value * x[j]).sum();
            x[i] -= inflow;
        }
        for (i, row) in self.rows.iter().enumerate().rev() {
            x[i] = (x[i] - row.upper.iter().map(|&(j, value)| value * x[j]).sum::<f64>()) / row.diagonal;
        }
        x
    }
}
