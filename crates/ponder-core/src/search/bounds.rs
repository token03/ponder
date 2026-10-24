use super::*;

impl Graph {
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

    /// The most `policy` can cost when every chance may be anywhere within its bounds (nature
    /// choosing the worst, by policy iteration on nature's side). None if it may never finish,
    /// or nature's iteration doesn't settle (then no upper bound is certain).
    pub(super) fn worst_case(&self, policy: &[usize], start: &[f64], backend: Backend, max_rounds: usize) -> Option<Vec<f64>> {
        let mut value = start.to_vec();
        let mut prob: Option<Vec<f64>> = None;
        for _ in 0..max_rounds {
            let next = self.extreme(&value, true);
            if prob.as_ref() == Some(&next) {
                return Some(value);
            }
            value = self.values_with(policy, &next, backend)?;
            prob = Some(next);
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
    /// The lower bound is 0 if its iteration doesn't settle.
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
    ) -> (Bounds, Option<PolicyVisits>) {
        let upper = self.worst_case(policy, value, backend, max_rounds).map_or(f64::INFINITY, |w| w[start]);
        let (mut lower, mut kind) = (0.0, None);
        let (mut pol, mut v) = (policy.to_vec(), value.to_vec());
        for _ in 0..max_rounds {
            let prob = self.extreme(&v, false);
            let better = self.improve(&self.q_with(&v, &prob), &pol);
            let Some(lu) = self.factor_with(&better, &prob, backend) else { break };
            let next = self.evaluate(&lu, &better);
            if !next.iter().all(|x| x.is_finite() && *x >= -1e-9) {
                break;
            }
            let settled = better == pol && next.iter().zip(&v).all(|(a, b)| (a - b).abs() <= 1e-9 * a.abs().max(1.0));
            pol = better;
            v = next;
            if settled {
                lower = v[start];
                kind = Some((pol, self.visits(&lu, start)));
                break;
            }
        }
        (Bounds { lower: lower.min(value[start]), upper: upper.max(value[start]) }, kind)
    }
}
