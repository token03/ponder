use super::*;

pub(super) fn classify_in(targets: &[Target], filler: &BTreeSet<String>, m: &Mod) -> (i32, u8) {
    for (i, t) in targets.iter().enumerate() {
        if t.good.contains(&m.id) {
            return (i as i32, GOOD);
        }
        if m.groups.iter().any(|g| t.groups.contains(g)) {
            return (i as i32, BLOCKED);
        }
    }
    if filler.contains(&m.id) {
        return (-1, if m.side == Side::Prefix { 4 } else { 5 });
    }
    (-1, (if m.side == Side::Prefix { 0 } else { 2 }) + if m.notable { 0 } else { 1 })
}

impl Problem {
    pub fn limit(&self) -> usize {
        self.side_limit(RARE, Side::Prefix).max(self.side_limit(RARE, Side::Suffix))
    }

    pub fn rare_counts(&self) -> Vec<(usize, f64)> {
        crate::mechanics::ItemRules::new(self.kind, self.notable_cap, self.affix_limits).counts(RARE)
    }

    pub fn side_limit(&self, rarity: u8, side: Side) -> usize {
        crate::mechanics::ItemRules::new(self.kind, self.notable_cap, self.affix_limits).limit(rarity, side)
    }

    /// A fresh base (with its fractured mods).
    pub fn start(&self) -> State {
        let b = self.c().base;
        State { rarity: NORMAL, ..b }
    }

    /// Where this craft starts: `begin`, else a fresh base.
    pub fn origin(&self) -> State {
        self.begin.unwrap_or_else(|| self.start())
    }

    /// (target index, GOOD or BLOCKED), or (-1, junk slot).
    pub fn classify(&self, mod_id: &str) -> (i32, u8) {
        let m = self.mods().get(mod_id).unwrap_or_else(|| panic!("unknown mod {mod_id:?}"));
        self.classify_mod(m)
    }

    /// The state of a real item with these mods (fractured ones included, as in `fixed`).
    pub fn state_of<'a>(&self, rarity: u8, mods: impl IntoIterator<Item = &'a Mod>) -> State {
        let mut s = State::new(rarity, &[], &[]);
        for m in mods {
            let (i, v) = self.classify_mod(m);
            if i >= 0 {
                let st = &mut s.status[i as usize];
                *st = if *st != MISSING { *st } else { v };
            } else {
                s.junk[v as usize] += 1;
            }
        }
        s
    }

    pub(super) fn counts(&self, c: &Compiled, s: &State) -> [usize; 2] {
        let mut n = [0usize; 2];
        for (i, side) in c.t_side.iter().enumerate() {
            if s.status[i] != MISSING {
                n[side.index()] += 1;
            }
        }
        for (side, slots) in SIDE_SLOTS.iter().enumerate() {
            n[side] += slots.iter().map(|&k| s.junk[k] as usize).sum::<usize>();
        }
        if let Some(side) = meta_side(s.meta) {
            n[side.index()] += 1;
        }
        if let Some(side) = Self::pending_side(s) {
            n[side.index()] += 1;
        }
        n
    }

    pub fn count(&self, s: &State, side: Side) -> usize {
        self.counts(self.c(), s)[side.index()]
    }

    pub(super) fn notables_in(&self, c: &Compiled, s: &State) -> usize {
        let on = c.t_notable.iter().enumerate().filter(|(i, n)| **n && s.status[*i] != MISSING).count();
        on + s.junk[0] as usize + s.junk[2] as usize
    }

    pub fn total(&self, s: &State) -> usize {
        let n = self.counts(self.c(), s);
        n[0] + n[1]
    }

    pub fn done(&self, s: &State) -> bool {
        if s.unveil != 0
            || matches!(s.veiled, VEILED_PREFIX | VEILED_SUFFIX)
            || s.meta != 0
            || s.status[self.targets.len()..self.targets.len() + self.bench.len()].iter().any(|&v| v != MISSING)
        {
            return false; // the bench has to take it off
        }
        let c = self.c();
        let good = c.t_side.iter().enumerate().filter(|(i, _)| s.status[*i] == GOOD).count();
        if good < self.need || s.junk[4] as usize + (s.junk[5] as usize) < self.filler_need {
            return false;
        }
        if let Some((p, q)) = self.filler_sides
            && ((s.junk[4] as usize) < p || (s.junk[5] as usize) < q)
        {
            return false;
        }
        if let Some((p, q)) = self.junk_cap
            && (s.junk[0] as usize + s.junk[1] as usize > p || s.junk[2] as usize + s.junk[3] as usize > q)
        {
            return false;
        }
        let counts = self.counts(c, s);
        if counts[0] + counts[1] + self.reserve.values().sum::<usize>() < self.min_mods {
            return false;
        }
        self.reserve.iter().all(|(side, n)| counts[side.index()] + n <= self.side_limit(s.rarity, *side))
    }
}

impl ProblemSpec {
    /// `classify` for any mod, also one outside the pool (a crafted or influence mod on a bought item).
    pub fn classify_mod(&self, m: &Mod) -> (i32, u8) {
        let found = classify_in(&self.targets, &self.filler, m);
        if found.0 >= 0 {
            return found;
        }
        self.bench
            .iter()
            .chain(&self.blockers)
            .position(|b| m.groups.iter().any(|g| b.groups.contains(g)))
            .map_or(found, |i| ((self.targets.len() + i) as i32, BLOCKED))
    }
}
