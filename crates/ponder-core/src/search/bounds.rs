use super::*;

impl Graph {
    fn reachable_policy(&self, policy: &[usize], start: usize) -> (Self, Vec<usize>) {
        let mut indices = vec![-1; self.n()];
        let mut states = vec![start];
        indices[start] = 0;
        let mut scanned = vec![false; self.lists()];
        let mut cursor = 0;
        while cursor < states.len() {
            let list = self.pair_list[policy[states[cursor]]];
            cursor += 1;
            if scanned[list] {
                continue;
            }
            scanned[list] = true;
            for edge in self.list_ptr[list]..self.list_ptr[list + 1] {
                let next = self.e_next[edge];
                if next >= 0 && self.e_hi[edge] > 0.0 && indices[next as usize] < 0 {
                    indices[next as usize] = states.len() as i32;
                    states.push(next as usize);
                }
            }
        }
        let mut graph = Graph {
            factorization: Default::default(),
            states: states.iter().map(|&i| self.states[i]).collect(),
            seen: states.len(),
            state_ptr: (0..=states.len()).collect(),
            pair_action: states.iter().map(|&i| self.pair_action[policy[i]]).collect(),
            pair_cost: states.iter().map(|&i| self.pair_cost[policy[i]]).collect(),
            pair_list: Vec::new(),
            list_ptr: vec![0],
            e_next: Vec::new(),
            e_prob: Vec::new(),
            e_lo: Vec::new(),
            e_hi: Vec::new(),
            loose: Vec::new(),
        };
        let mut lists = FxHashMap::default();
        for &state in &states {
            let list = self.pair_list[policy[state]];
            let next = *lists.entry(list).or_insert_with(|| {
                let next = graph.loose.len();
                for edge in (self.list_ptr[list]..self.list_ptr[list + 1]).filter(|&e| self.e_hi[e] > 0.0) {
                    graph.e_next.push(if self.e_next[edge] < 0 { -1 } else { indices[self.e_next[edge] as usize] });
                    graph.e_prob.push(self.e_prob[edge]);
                    graph.e_lo.push(self.e_lo[edge]);
                    graph.e_hi.push(self.e_hi[edge]);
                }
                graph.list_ptr.push(graph.e_next.len());
                graph.loose.push(self.loose[list]);
                next
            });
            graph.pair_list.push(next);
        }
        (graph, states)
    }

    fn policy_signature(&self, policy: &[usize], prob: &[f64]) -> blake3::Hash {
        let mut used = vec![false; self.lists()];
        let mut hash = blake3::Hasher::new();
        for &pair in policy {
            hash.update(&pair.to_le_bytes());
            used[self.pair_list[pair]] = true;
        }
        for (list, _) in used.iter().enumerate().filter(|(_, used)| **used) {
            for p in &prob[self.list_ptr[list]..self.list_ptr[list + 1]] {
                hash.update(&p.to_le_bytes());
            }
        }
        hash.finalize()
    }

    fn has_robust_dead_end(&self, policy: &[usize]) -> bool {
        let mut rows = vec![Vec::new(); self.lists()];
        for (state, &pair) in policy.iter().enumerate() {
            rows[self.pair_list[pair]].push(state);
        }
        let mut incoming = vec![Vec::new(); self.n()];
        let mut remaining = vec![0.0; self.lists()];
        let mut escaping = vec![false; self.lists()];
        let mut pending = Vec::new();
        for (list, _) in rows.iter().enumerate().filter(|(_, rows)| !rows.is_empty()) {
            for edge in self.list_ptr[list]..self.list_ptr[list + 1] {
                if self.e_next[edge] >= 0 {
                    remaining[list] += self.e_hi[edge];
                    incoming[self.e_next[edge] as usize].push((list, edge));
                } else if self.e_lo[edge] > 0.0 {
                    escaping[list] = true;
                }
            }
            if remaining[list] < 1.0 {
                escaping[list] = true;
            }
            if escaping[list] {
                pending.push(list);
            }
        }
        let mut closed = vec![true; self.n()];
        while let Some(list) = pending.pop() {
            for &state in &rows[list] {
                if !closed[state] {
                    continue;
                }
                closed[state] = false;
                for &(predecessor, edge) in &incoming[state] {
                    if escaping[predecessor] {
                        continue;
                    }
                    remaining[predecessor] -= self.e_hi[edge];
                    if self.e_lo[edge] > 0.0 || remaining[predecessor] < 1.0 {
                        escaping[predecessor] = true;
                        pending.push(predecessor);
                    }
                }
            }
        }
        closed.into_iter().any(|state| state)
    }

    /// Chances within the bounds that make the cost to go (`value`) highest (`worst`) or lowest.
    ///
    /// Each list starts at its least chances and hands what's left of 1 to its dearest (or
    /// cheapest) outcomes first, up to their most: the extreme point of the bounds.
    pub(super) fn extreme(&self, value: &[f64], worst: bool) -> Vec<f64> {
        let mut prob = self.e_prob.clone();
        let mut order: Vec<usize> = Vec::new();
        for l in (0..self.lists()).filter(|&l| self.loose[l]) {
            let range = self.list_ptr[l]..self.list_ptr[l + 1];
            let v = |e: usize| if self.e_next[e] >= 0 { value[self.e_next[e] as usize] } else { 0.0 };
            order.clear();
            order.extend(range.clone());
            order.sort_by(|&a, &b| if worst { v(b).total_cmp(&v(a)) } else { v(a).total_cmp(&v(b)) });
            let mut left = 1.0 - range.clone().map(|e| self.e_lo[e]).sum::<f64>();
            for &e in &order {
                let more = (self.e_hi[e] - self.e_lo[e]).min(left).max(0.0);
                prob[e] = self.e_lo[e] + more;
                left -= more;
            }
        }
        prob
    }

    fn extreme_stable(&self, value: &[f64], worst: bool, previous: &[f64]) -> Vec<f64> {
        let mut next = self.extreme(value, worst);
        for list in (0..self.lists()).filter(|&list| self.loose[list]) {
            let range = self.list_ptr[list]..self.list_ptr[list + 1];
            let expected = |prob: &[f64]| {
                range.clone().filter(|&e| self.e_next[e] >= 0).map(|e| prob[e] * value[self.e_next[e] as usize]).sum::<f64>()
            };
            let (candidate, current) = (expected(&next), expected(previous));
            if if worst { candidate <= current } else { candidate >= current } {
                next[range.clone()].copy_from_slice(&previous[range]);
            }
        }
        next
    }

    /// The most `policy` can cost when every chance may be anywhere within its bounds (nature
    /// choosing the worst, by policy iteration on nature's side). None if it may never finish,
    /// or nature's iteration doesn't settle (then no upper bound is certain).
    fn worst_case(&self, policy: &[usize], start: &[f64], backend: Backend, max_rounds: usize) -> Option<Vec<f64>> {
        if self.has_robust_dead_end(policy) {
            return None;
        }
        let mut value = start.to_vec();
        let mut probabilities = self.e_prob.clone();
        let mut previous = self.policy_signature(policy, &self.e_prob);
        let mut seen = std::collections::HashSet::from([previous]);
        for _ in 0..max_rounds {
            let next = self.extreme_stable(&value, true, &probabilities);
            let signature = self.policy_signature(policy, &next);
            if previous == signature {
                return Some(value);
            }
            if !seen.insert(signature) {
                return None;
            }
            let evaluated = self.values_with(policy, &next, backend)?;
            if evaluated.iter().zip(&value).all(|(a, b)| (a - b).abs() <= 1e-9 * a.abs().max(1.0)) {
                return None;
            }
            value = evaluated;
            probabilities = next;
            previous = signature;
        }
        None
    }

    /// Certified bounds on the expected cost from the origin (`start`): the cheapest way to
    /// craft, on real items whose chances lie within the bounds, costs at least `lower`; and
    /// following `policy` costs at most `upper`.
    ///
    /// This is interval value iteration for bounded-parameter MDPs (Givan, Leach and Dean,
    /// "Bounded-parameter Markov decision processes", AIJ 2000), done by policy iteration: the
    /// lower bound lets nature pick the kindest chances as well as the crafter picking actions
    /// (one MDP over both choices), the upper one evaluates the policy against the cruellest.
    ///
    /// Also returns the lower bound's policy (what the crafter does when nature is kind) and how
    /// often it acts in each state, if it settled.
    pub(super) fn bounds(
        &self,
        policy: &[usize],
        value: &[f64],
        start: usize,
        backend: Backend,
        max_rounds: usize,
    ) -> (Bounds, Option<OptimisticPolicy>) {
        let upper = self.upper_bound(policy, value, start, backend, max_rounds);
        let (lower, kind) = self.optimistic(policy, value, start, backend, max_rounds);
        (Bounds { lower, upper }, kind)
    }

    pub(super) fn upper_bound(&self, policy: &[usize], value: &[f64], start: usize, backend: Backend, max_rounds: usize) -> f64 {
        let (reachable, indices) = self.reachable_policy(policy, start);
        let initial: Vec<_> = indices.iter().map(|&i| value[i]).collect();
        let selected: Vec<_> = (0..indices.len()).collect();
        reachable.worst_case(&selected, &initial, backend, max_rounds).map_or(f64::INFINITY, |w| w[0]).max(value[start])
    }

    pub(super) fn optimistic(
        &self,
        policy: &[usize],
        value: &[f64],
        start: usize,
        backend: Backend,
        max_rounds: usize,
    ) -> (f64, Option<OptimisticPolicy>) {
        let mut lower = if self.pair_cost.iter().any(|&cost| cost < 0.0) { f64::NEG_INFINITY } else { 0.0 };
        let mut kind = None;
        let (mut pol, mut v) = (policy.to_vec(), value.to_vec());
        let mut seen = std::collections::HashSet::new();
        let mut previous = None;
        let mut evaluated = None;
        let mut probabilities = self.e_prob.clone();
        for round in 0..=max_rounds {
            let prob = self.extreme_stable(&v, false, &probabilities);
            let better = self.improve(&self.q_with(&v, &prob), &pol);
            let signature = self.policy_signature(&better, &prob);
            if previous == Some(signature) {
                lower = v[start];
                kind = Some(OptimisticPolicy {
                    actions: pol,
                    visits: self.visits(evaluated.as_ref().expect("evaluated policy"), start),
                    values: v,
                });
                break;
            }
            if round == max_rounds || !seen.insert(signature) {
                break;
            }
            previous = Some(signature);
            drop(evaluated.take());
            let Some(lu) = self.factor_with(&better, &prob, backend) else { break };
            let next = self.evaluate(&lu, &better);
            if !next.iter().all(|x| x.is_finite()) {
                break;
            }
            let settled = better == pol && next.iter().zip(&v).all(|(a, b)| (a - b).abs() <= 1e-9 * a.abs().max(1.0));
            pol = better;
            v = next;
            probabilities = prob;
            if settled {
                lower = v[start];
                kind = Some(OptimisticPolicy { actions: pol, visits: self.visits(&lu, start), values: v });
                break;
            }
            evaluated = Some(lu);
        }
        (lower.min(value[start]), kind)
    }
}
