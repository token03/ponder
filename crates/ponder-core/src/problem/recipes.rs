use super::*;

impl Problem {
    pub fn reforge_from(&self, state: State) -> Arc<Distribution> {
        let (out, bounds) = self.fill_roll(state, 0, None, &[Side::Prefix, Side::Suffix], &self.rare_counts());
        Arc::new(Distribution::new(out, bounds))
    }

    pub(super) fn available_metas(&self, s: &State) -> Vec<u8> {
        if s.meta != 0 {
            return Vec::new();
        }
        [META_LOCK_PREFIXES, META_LOCK_SUFFIXES, META_NO_ATTACK, META_NO_CASTER, META_MULTIMOD]
            .into_iter()
            .filter(|&m| {
                let side = meta_side(m).unwrap();
                self.count(s, side) < self.side_limit(s.rarity, side)
            })
            .collect()
    }

    pub(super) fn remove_add(&self, c: &Compiled, s: &State, side: Side, out: &mut Vec<Outcome>, bounds: &mut Vec<Bound>) {
        let add_side = side.other();
        if self.count(s, side) == 0 || self.count(s, add_side) >= self.side_limit(s.rarity, add_side) {
            return;
        }
        let (mut removes, mut rb) = (Vec::new(), Vec::new());
        self.remove_into(c, s, Some(side), None, &mut removes, &mut rb);
        if removes.is_empty() {
            removes.push((1.0, *s));
            rb.push((1.0, 1.0));
        }
        let (mut adds, mut ab) = (Vec::new(), Vec::new());
        let mut dist = Dist::new();
        for (&(p, state), &(lo, hi)) in removes.iter().zip(&rb) {
            self.add_mod_into(c, &state, 0, &[add_side], 0, &mut adds, &mut ab);
            if adds.is_empty() {
                dist.add(state, p, lo, hi);
            }
            for (&(q, next), &(ql, qh)) in adds.iter().zip(&ab) {
                dist.add(next, p * q, lo * ql, hi * qh);
            }
        }
        for (state, p, lo, hi) in dist.items {
            out.push((p, state));
            bounds.push((lo, hi));
        }
    }

    pub(super) fn reroll_side(&self, s: &State, side: Side) -> Option<Arc<Distribution>> {
        if meta_locks(s.meta) == Some(side) {
            return None;
        }
        let mut state = self.keep(s, side.other())[0].1;
        state.rarity = RARE;
        state.meta = if meta_side(s.meta) == Some(side) { 0 } else { s.meta };
        let key = (2 + side.index() as u8, 0, u32::MAX, state);
        if let Some(hit) = self.caches.locks.lock().unwrap().get(&key) {
            return Some(hit.clone());
        }
        let result = self.fill_roll(state, 0, None, &[side], &self.rare_counts());
        let made = Arc::new(Distribution::new(result.0, result.1));
        Some(self.caches.locks.lock().unwrap().entry(key).or_insert(made).clone())
    }

    pub(super) fn fill_roll(
        &self,
        state: State,
        profile: usize,
        first: Option<usize>,
        sides: &[Side],
        counts: &[(usize, f64)],
    ) -> (Vec<Outcome>, Option<Vec<Bound>>) {
        let c = self.c();
        let placed = self.total(&state);
        let mut dist = Dist::one(state);
        let mut result = Dist::new();
        let mut alive = 1.0;
        let (mut scratch, mut bounds) = (Vec::new(), Vec::new());
        let top = counts.iter().map(|c| c.0).max().unwrap_or(0).max(placed);
        for k in placed..=top {
            let more: f64 = counts.iter().filter(|c| c.0 > k).map(|c| c.1).sum();
            let stop = alive - more;
            for &(st, p, lo, hi) in &dist.items {
                result.add(st, p * stop, lo * stop, hi * stop);
            }
            alive = more;
            if alive <= 1e-15 {
                break;
            }
            let prof = if k == placed { first.unwrap_or(profile) } else { profile };
            let mut next = Dist::new();
            for &(st, p, lo, hi) in &dist.items {
                self.add_mod_into(c, &st, prof, sides, 0, &mut scratch, &mut bounds);
                if scratch.is_empty() {
                    next.add(st, p, lo, hi);
                } else {
                    for (&(q, n), &(ql, qh)) in scratch.iter().zip(&bounds) {
                        next.add(n, p * q, lo * ql, hi * qh);
                    }
                }
            }
            dist = next;
        }
        result.finish()
    }
}
