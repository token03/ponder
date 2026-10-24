use super::*;

/// The explored states as flat arrays.
///
/// Open (unfinished) states are numbered 0..n-1. Every (state, action) pair is
/// a row; pairs of one state are contiguous, in action order. A pair's
/// outcomes are a *list* of entries (next state, probability), with next = -1
/// for a finished item, whose remaining cost is 0. Rerolls give every state
/// the same list, so lists are stored once and pairs point at them: a
/// reroll's expected cost to finish is computed once, not once per state.
///
/// Each entry also has bounds on its probability (`e_lo`, `e_hi`): the true chance, for
/// whichever real item the state stands for, lies between them. Lists with exact chances
/// have both equal to `e_prob`.
pub(super) struct Graph {
    pub states: Vec<State>, // open states, in the order they were first acted on
    pub seen: usize,        // every state found, finished ones too
    pub(super) state_ptr: Vec<usize>,
    pub(super) pair_action: Vec<usize>,
    pub(super) pair_cost: Vec<f64>,
    pub(super) pair_list: Vec<usize>,
    pub(super) list_ptr: Vec<usize>, // list k's entries: list_ptr[k]..list_ptr[k+1]
    pub(super) e_next: Vec<i32>,
    pub(super) e_prob: Vec<f64>,
    pub(super) e_lo: Vec<f64>,
    pub(super) e_hi: Vec<f64>,
    /// lists whose chances are only bounded
    pub(super) loose: Vec<bool>,
}

/// Grows the entry arrays while states are discovered.
struct Builder {
    pub(super) seen: FxHashMap<State, u32>,
    pub(super) by_id: Vec<State>,
    pub(super) frontier: Vec<u32>,
    pub(super) list_ptr: Vec<usize>,
    pub(super) e_id: Vec<u32>,
    pub(super) e_prob: Vec<f64>,
    pub(super) e_lo: Vec<f64>,
    pub(super) e_hi: Vec<f64>,
    pub(super) loose: Vec<bool>,
}

impl Builder {
    pub(super) fn push_list(&mut self, out: &[Outcome], bounds: Option<&[Bound]>, limits: SolveLimits) -> Result<usize, SolveError> {
        if self.e_id.len().saturating_add(out.len()) > limits.transitions {
            return Err(SolveError::ResourceLimit { resource: "transitions", limit: limits.transitions });
        }
        let loose = bounds.is_some_and(|b| b.iter().zip(out).any(|(&(lo, hi), &(p, _))| lo != p || hi != p));
        for (k, &(p, n)) in out.iter().enumerate() {
            let next_id = self.by_id.len() as u32;
            if self.by_id.len() >= limits.states && !self.seen.contains_key(&n) {
                return Err(SolveError::ResourceLimit { resource: "states", limit: limits.states });
            }
            let id = *self.seen.entry(n).or_insert(next_id);
            if id == next_id {
                self.by_id.push(n);
                self.frontier.push(id);
            }
            let (lo, hi) = if loose { bounds.expect("loose lists have bounds")[k] } else { (p, p) };
            self.e_id.push(id);
            self.e_prob.push(p);
            self.e_lo.push(lo);
            self.e_hi.push(hi);
        }
        self.list_ptr.push(self.e_id.len());
        self.loose.push(loose);
        Ok(self.list_ptr.len() - 2)
    }
}

/// Every state reachable from the origin, with each action's outcomes; with `only`, just the
/// action it names in each state (an error if it names none there).
pub(super) fn explore(model: &impl Model, limits: SolveLimits, only: Option<&StateMap<usize>>) -> Result<Graph, SolveError> {
    if limits.states == 0 {
        return Err(SolveError::ResourceLimit { resource: "states", limit: 0 });
    }
    let origin = model.origin();
    let mut b = Builder {
        seen: FxHashMap::default(),
        by_id: vec![origin],
        frontier: vec![0],
        list_ptr: vec![0],
        e_id: Vec::new(),
        e_prob: Vec::new(),
        e_lo: Vec::new(),
        e_hi: Vec::new(),
        loose: Vec::new(),
    };
    b.seen.insert(origin, 0);
    let mut open: Vec<u32> = Vec::new();
    let (mut state_ptr, mut pair_action, mut pair_cost, mut pair_list) = (vec![0], Vec::new(), Vec::new(), Vec::new());
    // Rerolls share one outcome list across many states: scan each once.
    let mut shared: FxHashMap<usize, usize> = FxHashMap::default();
    let mut held = Vec::new();
    let mut buffer = TransitionBuffer::default();
    while let Some(id) = b.frontier.pop() {
        let s = b.by_id[id as usize];
        if model.is_goal(&s) {
            continue;
        }
        open.push(id);
        let chosen = match only {
            Some(policy) => Some(*policy.get(&s).ok_or_else(|| SolveError::MissingPolicy(format!("policy has no action for {s:?}")))?),
            None => None,
        };
        for &Decision { id: ai, cost } in model.actions() {
            if chosen.is_some_and(|c| c != ai) {
                continue;
            }
            let list = match model.outcomes(&s, ai, &mut buffer) {
                TransitionResult::Empty => continue,
                TransitionResult::Buffered => b.push_list(buffer.outcomes(), Some(buffer.bounds()), limits)?,
                TransitionResult::Shared(arc) => match shared.get(&(std::sync::Arc::as_ptr(&arc) as usize)) {
                    Some(&l) => l,
                    None => {
                        let l = b.push_list(&arc, arc.bounds(), limits)?;
                        shared.insert(std::sync::Arc::as_ptr(&arc) as usize, l);
                        held.push(arc); // keeps the address from being reused
                        l
                    }
                },
            };
            if pair_action.len() >= limits.state_actions {
                return Err(SolveError::ResourceLimit { resource: "state/action pairs", limit: limits.state_actions });
            }
            pair_action.push(ai);
            pair_cost.push(cost);
            pair_list.push(list);
        }
        if let Some(c) = chosen
            && state_ptr.last() == Some(&pair_list.len())
        {
            return Err(SolveError::MissingPolicy(format!("action {c} is not available in {s:?}")));
        }
        state_ptr.push(pair_list.len());
    }
    let mut index = vec![-1i32; b.by_id.len()];
    for (k, &id) in open.iter().enumerate() {
        index[id as usize] = k as i32;
    }
    Ok(Graph {
        states: open.iter().map(|&id| b.by_id[id as usize]).collect(),
        seen: b.by_id.len(),
        state_ptr,
        pair_action,
        pair_cost,
        pair_list,
        list_ptr: b.list_ptr,
        e_next: b.e_id.iter().map(|&id| index[id as usize]).collect(),
        e_prob: b.e_prob,
        e_lo: b.e_lo,
        e_hi: b.e_hi,
        loose: b.loose,
    })
}

impl Graph {
    pub(super) fn n(&self) -> usize {
        self.states.len()
    }

    pub(super) fn lists(&self) -> usize {
        self.list_ptr.len() - 1
    }

    /// Whether any list's chances are only bounded.
    pub(super) fn is_loose(&self) -> bool {
        self.loose.iter().any(|&l| l)
    }

    /// Expected cost to finish after taking each pair's action.
    pub(super) fn q(&self, value: &[f64]) -> Vec<f64> {
        self.q_with(value, &self.e_prob)
    }

    /// `q` with each entry's chance from `prob`.
    pub(super) fn q_with(&self, value: &[f64], prob: &[f64]) -> Vec<f64> {
        let expected: Vec<f64> = (0..self.lists())
            .map(|l| {
                (self.list_ptr[l]..self.list_ptr[l + 1])
                    .map(|e| match self.e_next[e] {
                        n if n >= 0 => prob[e] * value[n as usize],
                        _ => 0.0,
                    })
                    .sum()
            })
            .collect();
        self.pair_cost.iter().zip(&self.pair_list).map(|(c, &l)| c + expected[l]).collect()
    }

    /// Each state's cheapest pair (first in action order on ties), kept unless clearly better.
    pub(super) fn improve(&self, q: &[f64], policy: &[usize]) -> Vec<usize> {
        (0..self.n())
            .map(|i| {
                let range = self.state_ptr[i]..self.state_ptr[i + 1];
                let (mut best, mut candidate) = (f64::INFINITY, policy[i]);
                for p in range {
                    if q[p] < best {
                        best = q[p];
                        candidate = p;
                    }
                }
                let current = q[policy[i]];
                if best < current - 1e-9 * current.max(1.0) { candidate } else { policy[i] }
            })
            .collect()
    }

    /// A starting policy that surely finishes: always step toward fewer moves-to-done.
    ///
    /// Policy iteration needs one; "always reroll" isn't enough when no single
    /// reroll can hit every target (e.g. three notables need alt, regal, exalt).
    /// Returns None for states that can't finish.
    pub(super) fn proper_policy(&self) -> Vec<Option<usize>> {
        let n = self.n();
        let mut dist = vec![f64::INFINITY; n];
        let mut policy = vec![None; n];
        let mut list_min = vec![0.0; self.lists()];
        loop {
            for (l, m) in list_min.iter_mut().enumerate() {
                *m = (self.list_ptr[l]..self.list_ptr[l + 1])
                    .map(|e| {
                        if self.e_prob[e] <= 0.0 {
                            f64::INFINITY
                        } else if self.e_next[e] >= 0 {
                            dist[self.e_next[e] as usize]
                        } else {
                            0.0
                        }
                    })
                    .fold(f64::INFINITY, f64::min);
            }
            let mut improved = false;
            for i in 0..n {
                let (mut best, mut first) = (f64::INFINITY, None);
                for p in self.state_ptr[i]..self.state_ptr[i + 1] {
                    let step = list_min[self.pair_list[p]] + 1.0;
                    if step < best {
                        best = step;
                        first = Some(p);
                    }
                }
                if best < dist[i] {
                    dist[i] = best;
                    policy[i] = first;
                    improved = true;
                }
            }
            if !improved {
                return policy;
            }
        }
    }

    /// The states from which some policy finishes for sure, and only the actions that keep there.
    ///
    /// An action that may land where nothing can finish (an alchemy when only alterations are
    /// left) is a dead end to avoid, not a reason the target is out of reach (Kolobov, Mausam
    /// and Weld, "A theory of goal-oriented MDPs with dead ends", UAI 2012). Any outcome that
    /// may happen counts, even one the point estimate gives no chance. None if nothing is left.
    pub(super) fn without_dead_ends(self) -> Option<Graph> {
        let n = self.n();
        let pairs = self.pair_list.len();
        let mut valid = vec![true; pairs];
        let mut owner = vec![0usize; pairs];
        for i in 0..n {
            owner[self.state_ptr[i]..self.state_ptr[i + 1]].fill(i);
        }
        let mut alive;
        loop {
            // States that finish with some chance using the valid actions.
            alive = vec![false; n];
            loop {
                let mut grew = false;
                for p in 0..pairs {
                    let i = owner[p];
                    if alive[i] || !valid[p] {
                        continue;
                    }
                    let l = self.pair_list[p];
                    if (self.list_ptr[l]..self.list_ptr[l + 1])
                        .any(|e| self.e_prob[e] > 0.0 && (self.e_next[e] < 0 || alive[self.e_next[e] as usize]))
                    {
                        alive[i] = true;
                        grew = true;
                    }
                }
                if !grew {
                    break;
                }
            }
            let mut cut = false;
            for (ok, &l) in valid.iter_mut().zip(&self.pair_list) {
                if *ok && (self.list_ptr[l]..self.list_ptr[l + 1]).any(|e| self.e_next[e] >= 0 && !alive[self.e_next[e] as usize]) {
                    *ok = false;
                    cut = true;
                }
            }
            if !cut {
                break;
            }
        }
        if alive.iter().all(|&a| a) {
            return Some(self);
        }
        let mut index = vec![-1i32; n];
        let mut states = Vec::new();
        for i in 0..n {
            if alive[i] {
                index[i] = states.len() as i32;
                states.push(self.states[i]);
            }
        }
        if states.is_empty() {
            return None;
        }
        let (mut state_ptr, mut pair_action, mut pair_cost, mut pair_list) = (vec![0], Vec::new(), Vec::new(), Vec::new());
        for i in (0..n).filter(|&i| alive[i]) {
            for p in (self.state_ptr[i]..self.state_ptr[i + 1]).filter(|&p| valid[p]) {
                pair_action.push(self.pair_action[p]);
                pair_cost.push(self.pair_cost[p]);
                pair_list.push(self.pair_list[p]);
            }
            state_ptr.push(pair_list.len());
        }
        // Lists only cut actions used may still point at dead states; nothing reads them.
        let e_next = self.e_next.iter().map(|&x| if x >= 0 { index[x as usize].max(-1) } else { x }).collect();
        Some(Graph {
            states,
            seen: self.seen,
            state_ptr,
            pair_action,
            pair_cost,
            pair_list,
            list_ptr: self.list_ptr,
            e_next,
            e_prob: self.e_prob,
            e_lo: self.e_lo,
            e_hi: self.e_hi,
            loose: self.loose,
        })
    }

    /// I - P for the policy, factored. None if the matrix is singular.
    pub(super) fn factor(&self, policy: &[usize], backend: Backend) -> Option<Lu> {
        self.factor_with(policy, &self.e_prob, backend)
    }

    pub(super) fn factor_with(&self, policy: &[usize], prob: &[f64], backend: Backend) -> Option<Lu> {
        let rows = policy.iter().map(|&p| self.pair_list[p]).collect();
        linear::factor(&System { rows, list_ptr: &self.list_ptr, e_next: &self.e_next, e_prob: prob }, backend)
    }

    pub(super) fn evaluate(&self, lu: &Lu, policy: &[usize]) -> Vec<f64> {
        let mut rhs = Mat::<f64>::from_fn(self.n(), 1, |i, _| self.pair_cost[policy[i]]);
        lu.solve(&mut rhs);
        (0..self.n()).map(|i| rhs[(i, 0)]).collect()
    }

    /// The policy's values with chances `prob`; None if it doesn't surely finish under them.
    pub(super) fn values_with(&self, policy: &[usize], prob: &[f64], backend: Backend) -> Option<Vec<f64>> {
        let lu = self.factor_with(policy, prob, backend)?;
        let v = self.evaluate(&lu, policy);
        v.iter().all(|x| x.is_finite() && *x >= -1e-9).then_some(v)
    }

    /// Expected times each state is acted on, starting from state `start`.
    pub(super) fn visits(&self, lu: &Lu, start: usize) -> Vec<f64> {
        let mut rhs = Mat::<f64>::from_fn(self.n(), 1, |i, _| if i == start { 1.0 } else { 0.0 });
        lu.solve_transpose(&mut rhs);
        (0..self.n()).map(|i| rhs[(i, 0)]).collect()
    }
}
