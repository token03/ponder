use std::cell::OnceCell;

use faer::Mat;
use faer::linalg::solvers::Solve;
use faer::sparse::linalg::solvers::{Lu, SymbolicLu};
use faer::sparse::{SparseColMat, Triplet};

use crate::ilu::Ilu;
use crate::policy::Reduction;

const ITERATIVE_STEPS: usize = 256;

pub(super) struct SparseRows {
    offsets: Vec<usize>,
    entries: Vec<(usize, f64)>,
}

impl SparseRows {
    pub(super) fn new(n: usize) -> Self {
        let mut offsets = Vec::with_capacity(n + 1);
        offsets.push(0);
        Self { offsets, entries: Vec::new() }
    }

    pub(super) fn push(&mut self, column: usize, value: f64) {
        self.entries.push((column, value));
    }

    pub(super) fn finish_row(&mut self) {
        self.offsets.push(self.entries.len());
    }

    pub(super) fn row(&self, k: usize) -> &[(usize, f64)] {
        &self.entries[self.offsets[k]..self.offsets[k + 1]]
    }

    pub(super) fn rows(&self) -> impl DoubleEndedIterator<Item = &[(usize, f64)]> + ExactSizeIterator {
        self.offsets.windows(2).map(|range| &self.entries[range[0]..range[1]])
    }
}

pub(super) struct SparseMatrix {
    rows: SparseRows,
    diagonal: Vec<f64>,
}

impl SparseMatrix {
    pub(super) fn new(n: usize) -> Self {
        Self { rows: SparseRows::new(n), diagonal: Vec::with_capacity(n) }
    }

    pub(super) fn push(&mut self, column: usize, probability: f64) {
        self.rows.push(column, probability);
    }

    pub(super) fn finish_row(&mut self) {
        let k = self.diagonal.len();
        self.rows.finish_row();
        let stay: f64 = self.rows.row(k).iter().filter(|e| e.0 == k).map(|e| e.1).sum();
        self.diagonal.push(1.0 - stay);
    }

    pub(super) fn len(&self) -> usize {
        self.diagonal.len()
    }

    pub(super) fn rows(&self) -> impl DoubleEndedIterator<Item = &[(usize, f64)]> + ExactSizeIterator {
        self.rows.rows()
    }

    fn apply(&self, x: &[f64], out: &mut [f64]) {
        for (k, row) in self.rows().enumerate() {
            out[k] = x[k] - row.iter().map(|&(j, p)| p * x[j]).sum::<f64>();
        }
    }
}

struct Workspace {
    r: Vec<f64>,
    rhat: Vec<f64>,
    v: Vec<f64>,
    p: Vec<f64>,
    y: Vec<f64>,
    s: Vec<f64>,
    z: Vec<f64>,
    t: Vec<f64>,
}

impl Workspace {
    fn new(n: usize) -> Self {
        Self {
            r: vec![0.0; n],
            rhat: vec![0.0; n],
            v: vec![0.0; n],
            p: vec![0.0; n],
            y: vec![0.0; n],
            s: vec![0.0; n],
            z: vec![0.0; n],
            t: vec![0.0; n],
        }
    }
}

pub(super) struct SparseSystem<'a> {
    matrix: &'a SparseMatrix,
    factor: OnceCell<Option<Ilu>>,
    direct: OnceCell<Option<DirectSystem>>,
    direct_fallback: bool,
    workspace: Workspace,
}

impl<'a> SparseSystem<'a> {
    pub(super) fn new(matrix: &'a SparseMatrix) -> Self {
        Self { matrix, factor: OnceCell::new(), direct: OnceCell::new(), direct_fallback: false, workspace: Workspace::new(matrix.len()) }
    }

    pub(super) fn hybrid(matrix: &'a SparseMatrix) -> Self {
        Self { direct_fallback: true, ..Self::new(matrix) }
    }

    pub(super) fn solve(&mut self, rhs: &[f64], initial: Vec<f64>) -> Option<Vec<f64>> {
        let guess = self.bicgstab(rhs, initial);
        self.gauss_seidel(rhs, guess)
    }

    pub(super) fn gauss_seidel(&self, rhs: &[f64], mut x: Vec<f64>) -> Option<Vec<f64>> {
        if self.matrix.diagonal.iter().any(|d| *d < 1e-14) {
            return None;
        }
        for iteration in 0..1_000_000 {
            if iteration == ITERATIVE_STEPS
                && self.direct_fallback
                && let Some(solved) = direct_solve(self.matrix, &self.direct, rhs)
            {
                return Some(solved);
            }
            let mut moved = 0.0f64;
            for (k, row) in self.matrix.rows().enumerate() {
                let inflow: f64 = row.iter().filter(|e| e.0 != k).map(|&(j, p)| p * x[j]).sum();
                let v = (rhs[k] + inflow) / self.matrix.diagonal[k];
                moved = moved.max((v - x[k]).abs() / v.abs().max(1.0));
                x[k] = v;
            }
            if moved < 1e-12 {
                return Some(x);
            }
        }
        None
    }

    pub(super) fn bicgstab(&mut self, rhs: &[f64], mut x: Vec<f64>) -> Vec<f64> {
        let n = self.matrix.len();
        let Workspace { r, rhat, v, p, y, s, z, t } = &mut self.workspace;
        self.matrix.apply(&x, v);
        for k in 0..n {
            r[k] = rhs[k] - v[k];
        }
        rhat.copy_from_slice(r);
        v.fill(0.0);
        p.fill(0.0);
        let (mut rho, mut alpha, mut omega) = (1.0, 1.0, 1.0);
        let mut best = x.clone();
        let mut least = norm(r);
        let rhs_norm = norm(rhs);
        for iteration in 0..10_000 {
            let scale = norm(&x).max(rhs_norm).max(1.0);
            if least <= 1e-13 * scale {
                break;
            }
            if iteration == ITERATIVE_STEPS
                && self.direct_fallback
                && let Some(solved) = direct_solve(self.matrix, &self.direct, rhs)
            {
                return solved;
            }
            let rho_next = dot(rhat, r);
            let beta = rho_next / rho * (alpha / omega);
            rho = rho_next;
            for k in 0..n {
                p[k] = r[k] + beta * (p[k] - omega * v[k]);
            }
            precondition(self.matrix, &self.factor, p, y);
            self.matrix.apply(y, v);
            alpha = rho / dot(rhat, v);
            for k in 0..n {
                s[k] = r[k] - alpha * v[k];
            }
            if norm(s) <= 1e-13 * scale && s.iter().all(|v| v.is_finite()) {
                for k in 0..n {
                    z[k] = x[k] + alpha * y[k];
                }
                if z.iter().all(|v| v.is_finite()) {
                    best.copy_from_slice(z);
                }
                break;
            }
            precondition(self.matrix, &self.factor, s, z);
            self.matrix.apply(z, t);
            omega = dot(t, s) / dot(t, t);
            for k in 0..n {
                x[k] = x[k] + alpha * y[k] + omega * z[k];
                r[k] = s[k] - omega * t[k];
            }
            let size = norm(r);
            if !size.is_finite() || !omega.is_finite() || omega == 0.0 || rho == 0.0 {
                break;
            }
            if size < least {
                least = size;
                best.copy_from_slice(&x);
            }
        }
        best
    }
}

pub(super) struct DirectSystem {
    outcomes: Vec<Vec<(f64, usize)>>,
    reduction: Reduction,
    factor: Lu<usize, f64>,
}

pub(super) fn direct_solve(matrix: &SparseMatrix, cached: &OnceCell<Option<DirectSystem>>, rhs: &[f64]) -> Option<Vec<f64>> {
    let direct = cached
        .get_or_init(|| {
            let outcomes: Vec<_> = matrix.rows().map(|row| row.iter().map(|&(j, p)| (p, j)).collect()).collect();
            let reduction = Reduction::new(&outcomes);
            let n = reduction.rows.len();
            let mut entries = Vec::new();
            for (i, row) in reduction.rows.iter().enumerate() {
                entries.push(Triplet::new(i, i, 1.0));
                entries.extend(row.iter().map(|&(p, j)| Triplet::new(i, j, -p)));
            }
            let matrix = SparseColMat::try_new_from_triplets(n, n, &entries).ok()?;
            let symbolic = SymbolicLu::try_new(matrix.symbolic()).ok()?;
            let factor = Lu::try_new_with_symbolic(symbolic, matrix.as_ref()).ok()?;
            Some(DirectSystem { outcomes, reduction, factor })
        })
        .as_ref()?;
    let (offset, reduced_rhs) = direct.reduction.right_hand_sides(&direct.outcomes, &[rhs.to_vec()]);
    let n = direct.reduction.rows.len();
    let mut values = Mat::from_fn(n, 1, |i, _| reduced_rhs[0][i]);
    direct.factor.solve_in_place(values.as_mut());
    let values = vec![(0..n).map(|i| values[(i, 0)]).collect()];
    let mut values = direct.reduction.expand(offset, values);
    let values = values.remove(0);
    values.iter().all(|v| v.is_finite()).then_some(values)
}

fn precondition(matrix: &SparseMatrix, factor: &OnceCell<Option<Ilu>>, rhs: &[f64], out: &mut [f64]) {
    if let Some(factor) = factor.get_or_init(|| Ilu::new(matrix)) {
        factor.apply(rhs, out);
        return;
    }
    for (k, row) in matrix.rows().enumerate() {
        let inflow: f64 = row.iter().filter(|e| e.0 < k).map(|&(j, p)| p * out[j]).sum();
        out[k] = (rhs[k] + inflow) / matrix.diagonal[k];
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn norm(a: &[f64]) -> f64 {
    a.iter().fold(0.0f64, |m, v| m.max(v.abs()))
}

pub(super) fn solve_dense(n: usize, a: &mut [f64], rhs: &mut [&mut Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    for k in 0..n {
        let p = (k..n).max_by(|&i, &j| a[i * n + k].abs().total_cmp(&a[j * n + k].abs()))?;
        if a[p * n + k].abs() < 1e-14 {
            return None;
        }
        if p != k {
            for c in 0..n {
                a.swap(k * n + c, p * n + c);
            }
            for b in rhs.iter_mut() {
                b.swap(k, p);
            }
        }
        for i in k + 1..n {
            let f = a[i * n + k] / a[k * n + k];
            if f != 0.0 {
                for c in k..n {
                    a[i * n + c] -= f * a[k * n + c];
                }
                for b in rhs.iter_mut() {
                    b[i] -= f * b[k];
                }
            }
        }
    }
    let mut out = Vec::with_capacity(rhs.len());
    for b in rhs.iter() {
        let mut x = vec![0.0; n];
        for k in (0..n).rev() {
            let s: f64 = (k + 1..n).map(|c| a[k * n + c] * x[c]).sum();
            x[k] = (b[k] - s) / a[k * n + k];
        }
        out.push(x);
    }
    Some(out)
}
