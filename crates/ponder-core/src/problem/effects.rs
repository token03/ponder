use super::*;

/// An action with everything the hot loop needs resolved.
pub(crate) struct PreparedAction {
    pub kind: Effect,
    pub(super) mask: u8,
    pub(super) profile: usize,
    pub(super) first: Option<usize>,
    pub(super) roll: Option<Arc<Distribution>>,
}

impl Problem {
    /// Scour an item whose `side` is locked: only that side (and fractured mods) stay; the
    /// metamod, on the other side, goes too.
    pub(super) fn keep(&self, s: &State, side: Side) -> Vec<Outcome> {
        let c = self.c();
        let mut out = *s;
        out.rarity = RARE;
        out.meta = 0;
        for i in 0..c.t_side.len() {
            if c.t_side[i] != side {
                out.status[i] = c.base.status[i];
            }
        }
        for slot in 0..JUNK_SLOTS {
            if !side.slots().contains(&slot) {
                out.junk[slot] = c.base.junk[slot];
            }
        }
        if self.total(&out) == 0 {
            out.rarity = NORMAL;
        }
        vec![(1.0, out)]
    }

    // -- mechanics ----------------------------------------------------------
    //
    // Every transition comes with bounds on its probabilities. A state counts a slot's other
    // mods but not their groups, and the groups decide what the next mod can be: the point
    // estimate takes the expected groups, the bounds the least and most they can keep out.
    // Fresh rolls don't depend on what was on the item, and are worked out exactly.

    #[allow(clippy::too_many_arguments)]
    pub(super) fn add_mod_into(
        &self,
        c: &Compiled,
        s: &State,
        profile: usize,
        sides: &[Side],
        taken: usize,
        out: &mut Vec<Outcome>,
        bounds: &mut Vec<Bound>,
    ) {
        self.add_mod_lumped(c, s, self.lump_for(c, profile, s.meta), &c.base.junk, sides, taken, out, bounds)
    }

    /// `add_mod_into` with given weights; `floor` is the junk already counted out of them (fractured, forced).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn add_mod_lumped(
        &self,
        c: &Compiled,
        s: &State,
        lump: &Lumped,
        floor: &[u8; JUNK_SLOTS],
        sides: &[Side],
        taken: usize,
        out: &mut Vec<Outcome>,
        bounds: &mut Vec<Bound>,
    ) {
        let limit = if s.rarity == MAGIC { 1 } else { self.limit() };
        let cap_hit = self.notables_in(c, s) >= self.notable_cap;
        let counts = self.counts(c, s);
        let mut cands: Vec<(State, f64, f64, f64)> = Vec::new();
        for &side in sides {
            if counts[side.index()] + taken >= limit {
                continue;
            }
            for i in 0..c.t_side.len() {
                if c.t_side[i] != side || s.status[i] != MISSING || (c.t_notable[i] && cap_hit) {
                    continue;
                }
                for v in [GOOD, BLOCKED] {
                    let w = lump.tw[i][v as usize - 1];
                    if w > 0.0 {
                        let mut n = *s;
                        n.status[i] = v;
                        cands.push((n, w, w, w));
                    }
                }
            }
            for &slot in side.slots() {
                if (slot == 0 || slot == 2) && cap_hit {
                    continue;
                }
                let sw = &lump.junk[slot];
                let rolled = |j: usize| s.junk[j].saturating_sub(floor[j]) as usize;
                let k = rolled(slot);
                // Mods in linked slots may be of this slot's groups too.
                let (mut linked, mut kept) = (0, 0.0);
                for j in (0..JUNK_SLOTS).filter(|&j| c.linked[j][slot]) {
                    linked += rolled(j);
                    kept += rolled(j) as f64 * lump.cross[j][slot];
                }
                let w = if k >= sw.rolls { 0.0 } else { (sw.total - sw.blocked[k.min(SLOT_MAX)] - kept).max(0.0) };
                let (lo, hi) = if c.overlap {
                    (0.0, sw.total)
                } else if k >= sw.may {
                    (0.0, 0.0)
                } else {
                    ((sw.total - sw.hi[(k + linked).min(ITEM_MAX)]).max(0.0), (sw.total - sw.lo[k.min(ITEM_MAX)]).max(0.0))
                };
                if hi > 0.0 || w > 0.0 {
                    let mut n = *s;
                    n.junk[slot] += 1;
                    cands.push((n, w, lo.min(w), hi.max(w)));
                }
            }
        }
        normalise(&cands, out, bounds);
    }

    pub(super) fn force(&self, s: &State, mod_id: &str) -> State {
        let (i, v) = self.classify(mod_id);
        let mut n = *s;
        if i >= 0 {
            let st = &mut n.status[i as usize];
            *st = if *st != MISSING { *st } else { v };
        } else {
            n.junk[v as usize] += 1;
        }
        n
    }

    /// Remove one random mod that isn't fractured (eldritch annulment: from one side only).
    ///
    /// A lock keeps its side; "cannot roll" protects mods with its tag (junk by its slot's share);
    /// the metamod itself can go.
    pub fn annul(&self, s: &State, side: Option<Side>) -> Distribution {
        let (mut out, mut bounds) = (Vec::new(), Vec::new());
        self.annul_into(self.c(), s, side, &mut out, &mut bounds);
        Distribution::new(out, Some(bounds))
    }

    pub(super) fn annul_into(&self, c: &Compiled, s: &State, side: Option<Side>, out: &mut Vec<Outcome>, bounds: &mut Vec<Bound>) {
        self.remove_into(c, s, side, meta_tag(s.meta), out, bounds);
    }

    /// Under a "cannot roll" metamod (`tag`), whether a mod is protected depends on which mod it
    /// is: the point estimate goes by the usual case, the bounds by every mod it could be.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn remove_into(
        &self,
        c: &Compiled,
        s: &State,
        side: Option<Side>,
        tag: Option<usize>,
        out: &mut Vec<Outcome>,
        bounds: &mut Vec<Bound>,
    ) {
        let open = |sd: Side| crate::mechanics::removable_side(sd, side, s.meta);
        let mut cands: Vec<(State, f64, f64, f64)> = Vec::new();
        for i in 0..c.t_side.len() {
            if !open(c.t_side[i]) || s.status[i] == MISSING || c.base.status[i] != MISSING {
                continue;
            }
            let (w, lo, hi) = match tag {
                None => (1.0, 1.0, 1.0),
                Some(t) => {
                    let (tagged, untagged) = c.t_tag_range[t][i][if s.status[i] == GOOD { 0 } else { 1 }];
                    let (lo, hi) =
                        if tagged || untagged { (if tagged { 0.0 } else { 1.0 }, if untagged { 1.0 } else { 0.0 }) } else { (0.0, 1.0) };
                    let w: f64 = if c.t_tagged[t][i] { 0.0 } else { 1.0 };
                    (w.clamp(lo, hi), lo, hi)
                }
            };
            if hi > 0.0 {
                let mut n = *s;
                n.status[i] = MISSING;
                cands.push((n, w, lo, hi));
            }
        }
        for slot in 0..JUNK_SLOTS {
            let slot_side = if Side::Prefix.slots().contains(&slot) { Side::Prefix } else { Side::Suffix };
            let n = s.junk[slot] as i32 - c.base.junk[slot] as i32;
            if n <= 0 || !open(slot_side) {
                continue;
            }
            let n = n as f64;
            let (w, lo, hi) = match tag {
                None => (n, n, n),
                Some(t) => {
                    let w = n * (1.0 - c.junk_tagged[t][slot]);
                    let (mut lo, mut hi) = (0.0, n);
                    if !c.phantoms {
                        let keys = c.keys.iter().filter(|k| k.slots[slot]);
                        let (may_tag, may_not) =
                            keys.fold((0.0, 0.0), |a, k| (a.0 + k.tagged[slot][t].0 as u8 as f64, a.1 + k.tagged[slot][t].1 as u8 as f64));
                        lo = (n - may_tag).max(0.0);
                        hi = n.min(may_not);
                        if lo > hi {
                            (lo, hi) = (0.0, n); // more mods than its groups explain: assume nothing
                        }
                    }
                    (w.clamp(lo, hi), lo, hi)
                }
            };
            if hi > 0.0 {
                let mut m = *s;
                m.junk[slot] -= 1;
                cands.push((m, w, lo, hi));
            }
        }
        if let Some(ms) = meta_side(s.meta)
            && side.is_none_or(|x| x == ms)
        {
            cands.push((State { meta: 0, ..*s }, 1.0, 1.0, 1.0));
        }
        normalise(&cands, out, bounds);
    }

    pub(super) fn augment_into(&self, c: &Compiled, s: &State, profile: usize, out: &mut Vec<Outcome>, bounds: &mut Vec<Bound>) {
        let (mut adds, mut add_bounds) = (Vec::new(), Vec::new());
        self.add_mod_into(c, s, profile, &[Side::Prefix, Side::Suffix], 0, &mut adds, &mut add_bounds);
        let (mut removes, mut remove_bounds) = (Vec::new(), Vec::new());
        self.remove_into(c, s, None, None, &mut removes, &mut remove_bounds);
        out.clear();
        bounds.clear();
        if adds.is_empty() || removes.is_empty() {
            out.extend(adds);
            bounds.extend(add_bounds);
            return;
        }
        let mut d = Dist::new();
        for (&(p, n), &(pl, ph)) in adds.iter().zip(&add_bounds) {
            for &(q, r) in &removes {
                let mut x = n;
                for i in 0..MAX_TARGETS {
                    if r.status[i] != s.status[i] {
                        x.status[i] = MISSING;
                    }
                }
                for k in 0..JUNK_SLOTS {
                    x.junk[k] -= s.junk[k] - r.junk[k];
                }
                x.meta = r.meta;
                d.add(x, p * q, pl * q, ph * q);
            }
        }
        for (st, p, lo, hi) in d.items {
            out.push((p, st));
            bounds.push((lo, hi));
        }
    }

    pub(crate) fn prepare(&self, a: &Action) -> PreparedAction {
        let kind = a.kind.effect();
        let mask = a.from_rarity.iter().filter(|&&r| r < 8).fold(0u8, |m, &r| m | 1 << r);
        let mut c = PreparedAction { kind, mask, profile: 0, first: None, roll: None };
        match kind {
            Effect::Roll(_) => c.roll = self.rolled_listed(a),
            Effect::Reforge => {
                c.roll = self.rolled_listed(a);
                c.profile = self.profile_index(&a.profile);
                c.first = a.first_profile.as_deref().filter(|f| !f.is_empty()).map(|f| self.profile_index(f));
            }
            Effect::Augment => c.profile = self.profile_index(&a.profile),
            _ => {}
        }
        c
    }

    /// The outcomes of action `a` from `s`. Fresh outcomes go to `scratch`, their bounds to `bounds`.
    pub(crate) fn outcomes_into(
        &self,
        s: &State,
        a: &PreparedAction,
        scratch: &mut Vec<Outcome>,
        bounds: &mut Vec<Bound>,
    ) -> TransitionResult {
        let c = self.c();
        scratch.clear();
        bounds.clear();
        if s.rarity >= 8 || a.mask & (1 << s.rarity) == 0 {
            return TransitionResult::Empty;
        }
        let both = [Side::Prefix, Side::Suffix];
        let sure = |scratch: &mut Vec<Outcome>, bounds: &mut Vec<Bound>, n: State| {
            scratch.push((1.0, n));
            bounds.push((1.0, 1.0));
        };
        match a.kind {
            Effect::Roll(_) | Effect::Reforge => {
                let rolled = match meta_locks(s.meta) {
                    _ if matches!(a.kind, Effect::Roll(_)) && s.meta != 0 => None, // essences and fossils can't go on with a metamod
                    Some(keep) => self.lock_reroll_idx(s, keep, a.profile, a.first),
                    None => match a.first.zip(meta_tag(s.meta)) {
                        Some((first, tag)) => Some(self.restricted_reforge(a.profile, first, tag)),
                        None => a.roll.clone(),
                    },
                };
                return match rolled {
                    Some(r) if !r.is_empty() => TransitionResult::Shared(r),
                    _ => TransitionResult::Empty,
                };
            }
            Effect::Add(rarity, side) => {
                let limit = if rarity == MAGIC { 2 } else { 2 * self.limit() };
                if self.total(s) < limit {
                    self.add_mod_into(
                        c,
                        &State { rarity, ..*s },
                        0,
                        side.as_ref().map_or(both.as_slice(), std::slice::from_ref),
                        0,
                        scratch,
                        bounds,
                    );
                }
            }
            Effect::Annul(side) => self.annul_into(c, s, side, scratch, bounds),
            Effect::Augment => self.augment_into(c, s, a.profile, scratch, bounds),
            Effect::Meta(m) => {
                let side = meta_side(m).expect("a metamod has a side");
                if s.meta == 0 && self.counts(c, s)[side.index()] < self.limit() {
                    sure(scratch, bounds, s.with_meta(m));
                }
            }
            Effect::Unmeta => {
                if s.meta != 0 {
                    sure(scratch, bounds, s.with_meta(0));
                }
            }
            Effect::Scour => {
                if let Some(side) = meta_locks(s.meta) {
                    for (p, n) in self.keep(s, side) {
                        scratch.push((p, n));
                        bounds.push((p, p));
                    }
                }
            }
        }
        if scratch.is_empty() { TransitionResult::Empty } else { TransitionResult::Buffered }
    }

    pub fn outcomes(&self, s: &State, a: &Action) -> Arc<Distribution> {
        let ac = self.prepare(a);
        let (mut scratch, mut bounds) = (Vec::new(), Vec::new());
        match self.outcomes_into(s, &ac, &mut scratch, &mut bounds) {
            TransitionResult::Empty => Arc::new(Distribution::Exact(Vec::new())),
            TransitionResult::Buffered => Arc::new(Distribution::new(scratch, Some(bounds))),
            TransitionResult::Shared(r) => r,
        }
    }
}
