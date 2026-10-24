//! The linear systems `(I - P) v = c` and `(I - P)^T x = e` of a policy.
//!
//! Dense LU is always right but cubic. Most states of a good policy reroll, and every state
//! that rerolls the same way has the same row of P, so P is a sparse matrix plus a few
//! (K, one per distinct long outcome list) dense rows shared by many states. With `B = I - P_s`
//! (the sparse rows; each shared-row state is an identity row in it) the system is
//! `B - U R^T`, with `U` marking the states of each shared list and `R` the lists: Woodbury
//! turns it into a sparse solve with B plus a dense K x K one. Two backends, same answers.

use faer::Mat;
use faer::linalg::solvers::{PartialPivLu, Solve};
use faer::sparse::linalg::solvers::Lu as SparseLu;
use faer::sparse::{SparseColMat, Triplet};

/// Which linear solver `solve` uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Backend {
    /// Split for big systems, dense for small ones.
    #[default]
    Auto,
    Dense,
    Split,
}

/// Lists at least this long are "shared": long enough that a row of them is not sparse.
const LONG_LIST: usize = 16;
/// Below this many states dense LU is as quick and simpler.
const SMALL: usize = 64;

/// The policy's chosen outcome list per open state, over the graph's entry arrays.
pub(crate) struct System<'a> {
    pub rows: Vec<usize>,
    pub list_ptr: &'a [usize],
    pub e_next: &'a [i32],
    pub e_prob: &'a [f64],
}

impl System<'_> {
    fn n(&self) -> usize {
        self.rows.len()
    }

    /// The open entries (next state, probability) of list `l`.
    fn entries(&self, l: usize) -> impl Iterator<Item = (usize, f64)> + '_ {
        (self.list_ptr[l]..self.list_ptr[l + 1]).filter(|&e| self.e_next[e] >= 0).map(|e| (self.e_next[e] as usize, self.e_prob[e]))
    }
}

pub(crate) enum Lu {
    Dense(PartialPivLu<f64>),
    Split(Box<Split>),
}

pub(crate) struct Split {
    b: SparseLu<usize, f64>,
    shared: Vec<Shared>,
    w: Mat<f64>,          // B^-1 U, n x K
    m: PartialPivLu<f64>, // I - R^T W, K x K
}

struct Shared {
    r: Vec<(usize, f64)>, // the list's open entries
    rows: Vec<usize>,     // states that use it
}

pub(crate) fn factor(sys: &System, backend: Backend) -> Option<Lu> {
    match backend {
        Backend::Dense => Some(dense(sys)),
        Backend::Split => split(sys),
        Backend::Auto if sys.n() < SMALL => Some(dense(sys)),
        Backend::Auto => split(sys),
    }
}

fn dense(sys: &System) -> Lu {
    let n = sys.n();
    let mut m = Mat::<f64>::identity(n, n);
    for (i, &l) in sys.rows.iter().enumerate() {
        for (j, p) in sys.entries(l) {
            m[(i, j)] -= p;
        }
    }
    Lu::Dense(m.partial_piv_lu())
}

fn split(sys: &System) -> Option<Lu> {
    let n = sys.n();
    let mut shared: Vec<Shared> = Vec::new();
    let mut at = std::collections::HashMap::new();
    let mut t = Vec::with_capacity(n * 6);
    for (i, &l) in sys.rows.iter().enumerate() {
        t.push(Triplet::new(i, i, 1.0));
        if sys.list_ptr[l + 1] - sys.list_ptr[l] >= LONG_LIST {
            let k = *at.entry(l).or_insert_with(|| {
                shared.push(Shared { r: sys.entries(l).collect(), rows: Vec::new() });
                shared.len() - 1
            });
            shared[k].rows.push(i);
        } else {
            t.extend(sys.entries(l).map(|(j, p)| Triplet::new(i, j, -p)));
        }
    }
    let b = SparseColMat::<usize, f64>::try_new_from_triplets(n, n, &t).ok()?.sp_lu().ok()?;
    let k = shared.len();
    let mut w = Mat::<f64>::zeros(n, k);
    for (c, s) in shared.iter().enumerate() {
        for &i in &s.rows {
            w[(i, c)] = 1.0;
        }
    }
    b.solve_in_place(w.as_mut());
    let mut m = Mat::<f64>::identity(k, k);
    for (a, s) in shared.iter().enumerate() {
        for &(j, p) in &s.r {
            for c in 0..k {
                m[(a, c)] -= p * w[(j, c)];
            }
        }
    }
    Some(Lu::Split(Box::new(Split { b, shared, w, m: m.partial_piv_lu() })))
}

impl Lu {
    /// Solve `(I - P) x = rhs` in place.
    pub fn solve(&self, rhs: &mut Mat<f64>) {
        match self {
            Lu::Dense(lu) => lu.solve_in_place(rhs.as_mut()),
            Lu::Split(s) => {
                // y = B^-1 rhs; the shared lists' expected values z = M^-1 R^T y; x = y + W z
                s.b.solve_in_place(rhs.as_mut());
                let mut z = Mat::<f64>::from_fn(s.shared.len(), 1, |a, _| s.shared[a].r.iter().map(|&(j, p)| p * rhs[(j, 0)]).sum());
                s.m.solve_in_place(z.as_mut());
                for c in 0..s.shared.len() {
                    let zc = z[(c, 0)];
                    for i in 0..rhs.nrows() {
                        rhs[(i, 0)] += s.w[(i, c)] * zc;
                    }
                }
            }
        }
    }

    /// Solve `(I - P)^T x = rhs` in place.
    pub fn solve_transpose(&self, rhs: &mut Mat<f64>) {
        match self {
            Lu::Dense(lu) => lu.solve_transpose_in_place(rhs.as_mut()),
            Lu::Split(s) => {
                // g = B^-T rhs; x = g + B^-T R M^-T U^T g
                s.b.solve_transpose_in_place(rhs.as_mut());
                let mut q = Mat::<f64>::from_fn(s.shared.len(), 1, |a, _| s.shared[a].rows.iter().map(|&i| rhs[(i, 0)]).sum());
                s.m.solve_transpose_in_place(q.as_mut());
                let mut h = Mat::<f64>::zeros(rhs.nrows(), 1);
                for (a, sh) in s.shared.iter().enumerate() {
                    for &(j, p) in &sh.r {
                        h[(j, 0)] += p * q[(a, 0)];
                    }
                }
                s.b.solve_transpose_in_place(h.as_mut());
                for i in 0..rhs.nrows() {
                    rhs[(i, 0)] += h[(i, 0)];
                }
            }
        }
    }
}
