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
use faer::sparse::linalg::solvers::{Lu as SparseLu, SymbolicLu};
use faer::sparse::{SparseColMat, SymbolicSparseColMat, Triplet};
use rustc_hash::FxHashMap;

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
    b: BlockLu,
    shared: Vec<Shared>,
    w: Mat<f64>,          // B^-1 U, n x K
    m: PartialPivLu<f64>, // I - R^T W, K x K
}

struct Shared {
    r: Vec<(usize, f64)>, // the list's open entries
    rows: Vec<usize>,     // states that use it
}

#[derive(Default)]
pub(crate) struct FactorizationCache {
    symbolic: Option<(SymbolicSparseColMat<usize>, SymbolicLu<usize>)>,
    #[cfg(test)]
    analyses: usize,
}

impl FactorizationCache {
    fn sparse_lu(&mut self, matrix: &SparseColMat<usize, f64>) -> Option<SparseLu<usize, f64>> {
        let pattern = matrix.symbolic();
        let matches = self.symbolic.as_ref().is_some_and(|(previous, _)| {
            previous.nrows() == pattern.nrows()
                && previous.ncols() == pattern.ncols()
                && previous.col_ptr() == pattern.col_ptr()
                && previous.row_idx() == pattern.row_idx()
                && previous.col_nnz() == pattern.col_nnz()
        });
        if !matches {
            self.symbolic = Some((pattern.to_owned().ok()?, SymbolicLu::try_new(pattern).ok()?));
            #[cfg(test)]
            {
                self.analyses += 1;
            }
        }
        SparseLu::try_new_with_symbolic(self.symbolic.as_ref()?.1.clone(), matrix.as_ref()).ok()
    }
}

pub(crate) fn factor(sys: &System, backend: Backend, cache: &mut FactorizationCache) -> Option<Lu> {
    match backend {
        Backend::Dense => Some(dense(sys)),
        Backend::Split => split(sys, cache),
        Backend::Auto if sys.n() < SMALL => Some(dense(sys)),
        Backend::Auto => split(sys, cache),
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

fn split(sys: &System, cache: &mut FactorizationCache) -> Option<Lu> {
    let n = sys.n();
    let mut shared: Vec<Shared> = Vec::new();
    let mut at = FxHashMap::default();
    let mut uses = FxHashMap::default();
    for &list in &sys.rows {
        *uses.entry(list).or_insert(0usize) += 1;
    }
    let mut t = Vec::with_capacity(n * 6);
    for (i, &l) in sys.rows.iter().enumerate() {
        t.push(Triplet::new(i, i, 1.0));
        if uses[&l] > 1 && sys.list_ptr[l + 1] - sys.list_ptr[l] >= LONG_LIST {
            let k = *at.entry(l).or_insert_with(|| {
                shared.push(Shared { r: sys.entries(l).collect(), rows: Vec::new() });
                shared.len() - 1
            });
            shared[k].rows.push(i);
        } else {
            t.extend(sys.entries(l).map(|(j, p)| Triplet::new(i, j, -p)));
        }
    }
    let b = BlockLu::new(n, &t, cache)?;
    let k = shared.len();
    let mut w = Mat::<f64>::zeros(n, k);
    for (c, s) in shared.iter().enumerate() {
        for &i in &s.rows {
            w[(i, c)] = 1.0;
        }
    }
    b.solve(&mut w);
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
                s.b.solve(rhs);
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
                s.b.solve_transpose(rhs);
                let mut q = Mat::<f64>::from_fn(s.shared.len(), 1, |a, _| s.shared[a].rows.iter().map(|&i| rhs[(i, 0)]).sum());
                s.m.solve_transpose_in_place(q.as_mut());
                let mut h = Mat::<f64>::zeros(rhs.nrows(), 1);
                for (a, sh) in s.shared.iter().enumerate() {
                    for &(j, p) in &sh.r {
                        h[(j, 0)] += p * q[(a, 0)];
                    }
                }
                s.b.solve_transpose(&mut h);
                for i in 0..rhs.nrows() {
                    rhs[(i, 0)] += h[(i, 0)];
                }
            }
        }
    }
}

struct BlockLu {
    row_ptr: Vec<usize>,
    edges: Vec<(usize, f64)>,
    diagonal: Vec<f64>,
    blocks: Vec<Block>,
}

struct Block {
    states: Vec<usize>,
    factor: Option<BlockFactor>,
}

enum BlockFactor {
    Dense(Box<PartialPivLu<f64>>),
    Sparse(Box<SparseLu<usize, f64>>),
}

impl BlockFactor {
    fn solve(&self, rhs: &mut Mat<f64>) {
        match self {
            Self::Dense(lu) => lu.solve_in_place(rhs.as_mut()),
            Self::Sparse(lu) => lu.solve_in_place(rhs.as_mut()),
        }
    }

    fn solve_transpose(&self, rhs: &mut Mat<f64>) {
        match self {
            Self::Dense(lu) => lu.solve_transpose_in_place(rhs.as_mut()),
            Self::Sparse(lu) => lu.solve_transpose_in_place(rhs.as_mut()),
        }
    }
}

impl BlockLu {
    fn adjusted_rhs(&self, rhs: &Mat<f64>, state: usize, col: usize) -> f64 {
        rhs[(state, col)]
            + self.edges[self.row_ptr[state]..self.row_ptr[state + 1]]
                .iter()
                .map(|&(next, probability)| probability * rhs[(next, col)])
                .sum::<f64>()
    }

    fn new(n: usize, entries: &[Triplet<usize, usize, f64>], cache: &mut FactorizationCache) -> Option<Self> {
        let mut row_ptr = vec![0; n + 1];
        let mut diagonal = vec![0.0; n];
        for entry in entries {
            if entry.row == entry.col {
                diagonal[entry.row] += entry.val;
            } else if entry.val != 0.0 {
                row_ptr[entry.row + 1] += 1;
            }
        }
        for i in 0..n {
            row_ptr[i + 1] += row_ptr[i];
        }
        let mut edges = vec![(0, 0.0); row_ptr[n]];
        let mut cursor = row_ptr[..n].to_vec();
        for entry in entries.iter().filter(|entry| entry.row != entry.col && entry.val != 0.0) {
            edges[cursor[entry.row]] = (entry.col, -entry.val);
            cursor[entry.row] += 1;
        }
        let (component, groups) = components(&row_ptr, &edges);
        let mut positions = vec![0; n];
        let mut blocks = Vec::with_capacity(groups.len());
        for states in groups {
            for (position, &state) in states.iter().enumerate() {
                positions[state] = position;
            }
            let factor = if states.len() == 1 {
                if diagonal[states[0]] == 0.0 || !diagonal[states[0]].is_finite() {
                    return None;
                }
                None
            } else if states.len() < SMALL {
                let mut matrix = Mat::<f64>::zeros(states.len(), states.len());
                for &state in &states {
                    let row = positions[state];
                    matrix[(row, row)] = diagonal[state];
                    for &(next, probability) in &edges[row_ptr[state]..row_ptr[state + 1]] {
                        if component[next] == component[state] {
                            matrix[(row, positions[next])] -= probability;
                        }
                    }
                }
                let lu = matrix.partial_piv_lu();
                if (0..states.len()).any(|i| lu.U()[(i, i)] == 0.0 || !lu.U()[(i, i)].is_finite()) {
                    return None;
                }
                Some(BlockFactor::Dense(Box::new(lu)))
            } else {
                let mut entries = Vec::new();
                for &state in &states {
                    let row = positions[state];
                    entries.push(Triplet::new(row, row, diagonal[state]));
                    for &(next, probability) in &edges[row_ptr[state]..row_ptr[state + 1]] {
                        if component[next] == component[state] {
                            entries.push(Triplet::new(row, positions[next], -probability));
                        }
                    }
                }
                let matrix = SparseColMat::try_new_from_triplets(states.len(), states.len(), &entries).ok()?;
                Some(BlockFactor::Sparse(Box::new(cache.sparse_lu(&matrix)?)))
            };
            if factor.is_none()
                && let Some(Block { states: previous, factor: None }) = blocks.last_mut()
            {
                previous.extend(states);
            } else {
                blocks.push(Block { states, factor });
            }
        }
        let mut read = 0;
        let mut write = 0;
        for state in 0..n {
            let end = row_ptr[state + 1];
            row_ptr[state] = write;
            while read < end {
                let edge = edges[read];
                if component[edge.0] != component[state] {
                    edges[write] = edge;
                    write += 1;
                }
                read += 1;
            }
        }
        row_ptr[n] = write;
        edges.truncate(write);
        Some(Self { row_ptr, edges, diagonal, blocks })
    }

    fn solve_transpose(&self, rhs: &mut Mat<f64>) {
        if rhs.ncols() == 0 {
            return;
        }
        for block in &self.blocks {
            if let Some(factor) = &block.factor {
                let mut local = Mat::from_fn(block.states.len(), rhs.ncols(), |row, col| rhs[(block.states[row], col)]);
                factor.solve_transpose(&mut local);
                for (row, &state) in block.states.iter().enumerate() {
                    for col in 0..rhs.ncols() {
                        rhs[(state, col)] = local[(row, col)];
                    }
                }
            } else {
                for col in 0..rhs.ncols() {
                    for &state in &block.states {
                        rhs[(state, col)] /= self.diagonal[state];
                        for &(next, probability) in &self.edges[self.row_ptr[state]..self.row_ptr[state + 1]] {
                            rhs[(next, col)] += probability * rhs[(state, col)];
                        }
                    }
                }
                continue;
            }
            for &state in &block.states {
                for &(next, probability) in &self.edges[self.row_ptr[state]..self.row_ptr[state + 1]] {
                    for col in 0..rhs.ncols() {
                        rhs[(next, col)] += probability * rhs[(state, col)];
                    }
                }
            }
        }
    }

    fn solve(&self, rhs: &mut Mat<f64>) {
        if rhs.ncols() == 0 {
            return;
        }
        for block in self.blocks.iter().rev() {
            if let Some(factor) = &block.factor {
                let mut local = Mat::from_fn(block.states.len(), rhs.ncols(), |row, col| self.adjusted_rhs(rhs, block.states[row], col));
                factor.solve(&mut local);
                for (row, &state) in block.states.iter().enumerate() {
                    for col in 0..rhs.ncols() {
                        rhs[(state, col)] = local[(row, col)];
                    }
                }
            } else {
                for col in 0..rhs.ncols() {
                    for &state in block.states.iter().rev() {
                        rhs[(state, col)] = self.adjusted_rhs(rhs, state, col) / self.diagonal[state];
                    }
                }
            }
        }
    }
}

fn components(row_ptr: &[usize], edges: &[(usize, f64)]) -> (Vec<usize>, Vec<Vec<usize>>) {
    let n = row_ptr.len() - 1;
    let mut reverse_ptr = vec![0; n + 1];
    for &(next, _) in edges {
        reverse_ptr[next + 1] += 1;
    }
    for i in 0..n {
        reverse_ptr[i + 1] += reverse_ptr[i];
    }
    let mut reverse = vec![0; edges.len()];
    let mut cursor = reverse_ptr[..n].to_vec();
    for state in 0..n {
        for &(next, _) in &edges[row_ptr[state]..row_ptr[state + 1]] {
            reverse[cursor[next]] = state;
            cursor[next] += 1;
        }
    }
    let mut visited = vec![false; n];
    let mut order = Vec::with_capacity(n);
    let mut stack = Vec::new();
    for start in 0..n {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        stack.push((start, row_ptr[start]));
        while let Some((state, edge)) = stack.last_mut() {
            if *edge == row_ptr[*state + 1] {
                order.push(*state);
                stack.pop();
            } else {
                let next = edges[*edge].0;
                *edge += 1;
                if !visited[next] {
                    visited[next] = true;
                    stack.push((next, row_ptr[next]));
                }
            }
        }
    }
    let mut component = vec![usize::MAX; n];
    let mut groups = Vec::new();
    let mut pending = Vec::new();
    for start in order.into_iter().rev() {
        if component[start] != usize::MAX {
            continue;
        }
        let id = groups.len();
        let mut states = Vec::new();
        component[start] = id;
        pending.push(start);
        while let Some(state) = pending.pop() {
            states.push(state);
            for &previous in &reverse[reverse_ptr[state]..reverse_ptr[state + 1]] {
                if component[previous] == usize::MAX {
                    component[previous] = id;
                    pending.push(previous);
                }
            }
        }
        states.sort_unstable();
        groups.push(states);
    }
    (component, groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rayon::prelude::*;

    #[test]
    fn cyclic_blocks_with_dependencies_match_dense_solutions_for_multiple_right_hand_sides() {
        let n = 96;
        let list_ptr: Vec<_> = (0..=n).map(|i| i * 4).collect();
        let e_next: Vec<_> = (0..n)
            .flat_map(|i| {
                let group = i / 3 * 3;
                [i as i32, (group + (i + 1) % 3) as i32, if group + 3 < n { (group + 3) as i32 } else { -1 }, -1]
            })
            .collect();
        let e_prob: Vec<_> = (0..n).flat_map(|_| [0.1, 0.2, 0.3, 0.4]).collect();
        let system = System { rows: (0..n).collect(), list_ptr: &list_ptr, e_next: &e_next, e_prob: &e_prob };
        let split = factor(&system, Backend::Split, &mut FactorizationCache::default()).unwrap();
        let dense = factor(&system, Backend::Dense, &mut FactorizationCache::default()).unwrap();
        for transpose in [false, true] {
            let mut actual = Mat::from_fn(n, 4, |row, col| (row + 1 + col * 7) as f64);
            let mut expected = actual.clone();
            if transpose {
                split.solve_transpose(&mut actual);
                dense.solve_transpose(&mut expected);
            } else {
                split.solve(&mut actual);
                dense.solve(&mut expected);
            }
            for row in 0..n {
                for col in 0..4 {
                    assert!((actual[(row, col)] - expected[(row, col)]).abs() < 1e-10);
                }
            }
        }
    }

    #[test]
    fn shared_resets_across_cyclic_blocks_match_dense_values_and_visits() {
        let n = 96;
        let mut list_ptr: Vec<_> = (0..=n).map(|i| i * 4).collect();
        let mut e_next: Vec<_> = (0..n)
            .flat_map(|i| {
                let group = i / 3 * 3;
                [i as i32, (group + (i + 1) % 3) as i32, if group + 3 < n { (group + 3) as i32 } else { -1 }, -1]
            })
            .collect();
        let mut e_prob: Vec<_> = (0..n).flat_map(|_| [0.1, 0.2, 0.3, 0.4]).collect();
        for weight in [0.2, 0.3] {
            e_next.extend((0..n as i32).chain(std::iter::once(-1)));
            e_prob.extend(std::iter::repeat_n(weight / n as f64, n).chain(std::iter::once(1.0 - weight)));
            list_ptr.push(e_next.len());
        }
        let rows = (0..n)
            .map(|i| {
                if i % 7 == 0 {
                    n
                } else if i % 11 == 0 {
                    n + 1
                } else {
                    i
                }
            })
            .collect();
        let system = System { rows, list_ptr: &list_ptr, e_next: &e_next, e_prob: &e_prob };
        let split = factor(&system, Backend::Split, &mut FactorizationCache::default()).unwrap();
        let dense = factor(&system, Backend::Dense, &mut FactorizationCache::default()).unwrap();
        for transpose in [false, true] {
            let mut actual = Mat::from_fn(n, 1, |row, _| (row % 5 + 1) as f64);
            let mut expected = actual.clone();
            if transpose {
                split.solve_transpose(&mut actual);
                dense.solve_transpose(&mut expected);
            } else {
                split.solve(&mut actual);
                dense.solve(&mut expected);
            }
            for row in 0..n {
                assert!((actual[(row, 0)] - expected[(row, 0)]).abs() < 1e-10);
            }
        }
    }

    #[test]
    fn singleton_runs_between_cycles_match_dense_solutions_in_both_directions() {
        let n = 96;
        let order: Vec<_> = (0..n).map(|i| i * 37 % n).collect();
        let list_ptr: Vec<_> = (0..=n).map(|i| i * 4).collect();
        let mut e_next = vec![-1; n * 4];
        for i in 0..n {
            let state = order[i];
            e_next[state * 4] = state as i32;
            e_next[state * 4 + 1] = if i == 32 { order[30] as i32 } else { order.get(i + 1).map_or(-1, |&next| next as i32) };
            e_next[state * 4 + 2] = order.get(i + 7).map_or(-1, |&next| next as i32);
        }
        let e_prob: Vec<_> = (0..n).flat_map(|_| [0.1, 0.3, 0.2, 0.4]).collect();
        let system = System { rows: (0..n).collect(), list_ptr: &list_ptr, e_next: &e_next, e_prob: &e_prob };
        let split = factor(&system, Backend::Split, &mut FactorizationCache::default()).unwrap();
        let dense = factor(&system, Backend::Dense, &mut FactorizationCache::default()).unwrap();
        for transpose in [false, true] {
            let mut actual = Mat::from_fn(n, 8, |row, col| (row + 1 + col * 7) as f64);
            let mut expected = actual.clone();
            if transpose {
                split.solve_transpose(&mut actual);
                dense.solve_transpose(&mut expected);
            } else {
                split.solve(&mut actual);
                dense.solve(&mut expected);
            }
            for row in 0..n {
                for col in 0..8 {
                    assert!((actual[(row, col)] - expected[(row, col)]).abs() < 1e-10);
                }
            }
        }
    }

    #[test]
    fn a_singular_cycle_inside_an_otherwise_terminating_system_has_no_factorization() {
        let n = 96;
        let list_ptr: Vec<_> = (0..=n).collect();
        let e_next: Vec<_> = (0..n)
            .map(|i| match i {
                0 => 1,
                1 => 0,
                _ => -1,
            })
            .collect();
        let e_prob = vec![1.0; n];
        let system = System { rows: (0..n).collect(), list_ptr: &list_ptr, e_next: &e_next, e_prob: &e_prob };
        assert!(factor(&system, Backend::Split, &mut FactorizationCache::default()).is_none());
    }

    #[test]
    fn symbolic_analysis_reuses_patterns_and_preserves_primal_and_transpose_solutions() {
        let mut cache = FactorizationCache::default();
        let n = 194;
        let list_ptr: Vec<_> = (0..=n).map(|i| i * 3).collect();
        for (step, probability, analyses) in [(1, 0.2, 1), (1, 0.3, 1), (2, 0.3, 2), (2, 0.5, 2), (2, 0.0, 2)] {
            let e_next: Vec<_> = (0..n).flat_map(|i| [i as i32, ((i + step) % n) as i32, -1]).collect();
            let e_prob: Vec<_> = (0..n).flat_map(|_| [0.1, probability, 0.9 - probability]).collect();
            let system = System { rows: (0..n).collect(), list_ptr: &list_ptr, e_next: &e_next, e_prob: &e_prob };
            let cached = factor(&system, Backend::Split, &mut cache).unwrap();
            let dense = factor(&system, Backend::Dense, &mut FactorizationCache::default()).unwrap();
            assert_eq!(cache.analyses, analyses);
            for transpose in [false, true] {
                let mut actual = Mat::from_fn(n, 1, |i, _| (i + 1) as f64);
                let mut expected = actual.clone();
                if transpose {
                    cached.solve_transpose(&mut actual);
                    dense.solve_transpose(&mut expected);
                } else {
                    cached.solve(&mut actual);
                    dense.solve(&mut expected);
                }
                for i in 0..n {
                    assert!((actual[(i, 0)] - expected[(i, 0)]).abs() < 1e-10);
                }
            }
        }
    }

    #[test]
    fn large_policy_solves_agree_inside_single_and_multi_worker_pools() {
        for workers in [1, 2] {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap();
            pool.install(|| {
                [Backend::Dense, Backend::Split].into_par_iter().for_each(|backend| {
                    let n = 192;
                    let list_ptr = vec![0, n + 1];
                    let e_next: Vec<_> = (0..n as i32).chain([-1]).collect();
                    let e_prob = vec![1.0 / (n + 1) as f64; n + 1];
                    let system = System { rows: vec![0; n], list_ptr: &list_ptr, e_next: &e_next, e_prob: &e_prob };
                    let lu = factor(&system, backend, &mut FactorizationCache::default()).unwrap();
                    let mut costs = Mat::from_fn(n, 1, |_, _| 1.0);
                    lu.solve(&mut costs);
                    assert!((0..n).all(|i| (costs[(i, 0)] - (n + 1) as f64).abs() < 1e-8));
                    let mut visits = Mat::from_fn(n, 1, |i, _| if i == 0 { 1.0 } else { 0.0 });
                    lu.solve_transpose(&mut visits);
                    assert!((visits[(0, 0)] - 2.0).abs() < 1e-8);
                    assert!((1..n).all(|i| (visits[(i, 0)] - 1.0).abs() < 1e-8));
                });
            });
        }
    }
}
