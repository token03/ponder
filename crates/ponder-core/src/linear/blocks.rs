use super::*;

pub(super) struct BlockLu {
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

    pub(super) fn new(n: usize, entries: &[Triplet<usize, usize, f64>], cache: &mut FactorizationCache) -> Option<Self> {
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

    pub(super) fn solve_transpose(&self, rhs: &mut Mat<f64>) {
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

    pub(super) fn solve(&self, rhs: &mut Mat<f64>) {
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
