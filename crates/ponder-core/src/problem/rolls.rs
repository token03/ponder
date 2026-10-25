use super::*;

/// What a reroll's outcome depends on. A forced (essence) mod matters through what it counts
/// as and, when junk, the groups it keeps out of the roll.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RollKey {
    Magic(String),
    /// profile, first profile, and a forced mod's class (with its groups when it is junk: they leave the roll)
    Rare(String, Option<String>, Option<((i32, u8), Vec<String>)>),
}

/// Junk groups pooled for a roll: `n` groups with the same weight per slot on the first draw
/// (`f`) and after it (`m`). Groups of equal weights are interchangeable.
struct Class {
    n: usize,
    f: [f64; JUNK_SLOTS],
    m: [f64; JUNK_SLOTS],
    /// the side its slots are on, if one
    side: Option<Side>,
}

#[derive(PartialEq, Eq, Hash)]
struct RollShape {
    state: State,
    placed: usize,
    restricted: bool,
    limits: [usize; 2],
    notable_cap: usize,
    target_sides: Vec<Side>,
    target_notables: Vec<bool>,
    depths: Vec<bool>,
    first_targets: Vec<u8>,
    main_targets: Vec<u8>,
    classes: Vec<(usize, u8, u8, Option<Side>)>,
}

impl RollShape {
    fn new(problem: &Problem, state: State, placed: usize, restricted: bool, first: &Lumped, main: &Lumped, classes: &[Class]) -> Self {
        let support = |weights: &[f64]| weights.iter().enumerate().fold(0, |bits, (i, &weight)| bits | u8::from(weight > 0.0) << i);
        let counts = problem.counts_for(state.rarity);
        let top = counts.iter().map(|c| c.0).max().unwrap_or(0);
        let mut shapes: Vec<_> = classes.iter().map(|class| (class.n.min(6), support(&class.f), support(&class.m), class.side)).collect();
        shapes.sort_unstable();
        Self {
            state,
            placed,
            restricted,
            limits: [problem.side_limit(state.rarity, Side::Prefix), problem.side_limit(state.rarity, Side::Suffix)],
            notable_cap: problem.notable_cap,
            target_sides: problem.c().t_side.clone(),
            target_notables: problem.c().t_notable.clone(),
            depths: (0..=top)
                .map(|depth| counts.iter().filter(|count| count.0 > depth).map(|count| count.1).sum::<f64>() > 1e-15)
                .collect(),
            first_targets: first.tw.iter().map(|weights| support(weights)).collect(),
            main_targets: main.tw.iter().map(|weights| support(weights)).collect(),
            classes: shapes,
        }
    }
}

fn failed_shapes() -> &'static Mutex<rustc_hash::FxHashSet<RollShape>> {
    static SHAPES: OnceLock<Mutex<rustc_hash::FxHashSet<RollShape>>> = OnceLock::new();
    SHAPES.get_or_init(Default::default)
}

/// Packs a partial roll (state, classes drawn) into one hashable word.
fn pack(s: &State, drawn: &[u8; 6]) -> u128 {
    let mut x: u128 = s.rarity as u128 | (s.meta as u128) << 2;
    for (i, &v) in s.status.iter().enumerate() {
        x |= (v as u128) << (5 + 2 * i);
    }
    for (i, &v) in s.junk.iter().enumerate() {
        x |= (v as u128 & 7) << (21 + 3 * i);
    }
    for (i, &d) in drawn.iter().enumerate() {
        x |= (d as u128) << (39 + 8 * i);
    }
    x
}

impl Problem {
    /// A fresh roll: worked out exactly (`roll_classes`) or, cheaply, on lumped states with bounds (`roll_bounded`).
    pub(super) fn roll_listed(
        &self,
        rarity: u8,
        profile: usize,
        first: Option<usize>,
        forced: Option<&str>,
        first_restriction: Option<usize>,
        exact: bool,
    ) -> Arc<Distribution> {
        let (out, bounds) = if exact {
            self.roll_classes(rarity, profile, first, forced, first_restriction)
        } else {
            self.roll_bounded(rarity, profile, first, forced, first_restriction)
        };
        Arc::new(Distribution::new(out, bounds))
    }

    /// The mod counts a fresh roll at `rarity` picks from, as (count, chance).
    pub(super) fn counts_for(&self, rarity: u8) -> Vec<(usize, f64)> {
        crate::mechanics::ItemRules::new(self.kind, self.notable_cap, self.affix_limits).counts(rarity)
    }

    /// The forced mod placed on a fresh base: the state, mods placed, and the groups it keeps out
    /// when junk. None when it clashes with a fractured mod (the essence can't be used).
    pub(super) fn forced_start<'c>(&self, c: &'c Compiled, rarity: u8, forced: Option<&str>) -> Option<(State, usize, &'c [String])> {
        let mut state = State { rarity, ..c.base };
        let mut placed = self.fixed.len();
        let mut also: &[String] = &[];
        if let Some(f) = forced {
            if self.fixed.iter().any(|m| crate::mechanics::groups_overlap(&m.groups, &c.mods[f].groups)) {
                return None;
            }
            state = self.force(&state, f);
            placed += 1;
            if self.classify(f).0 < 0 {
                also = &c.mods[f].groups;
            }
        }
        Some((state, placed, also))
    }

    /// A fresh roll, tracking which junk groups are drawn so every draw knows what is kept out.
    ///
    /// Groups of equal weights are interchangeable, so they are pooled into classes and the roll
    /// is carried through the classes drawn so far: exact. A big pool of distinct weights makes
    /// that too many partial items (`ROLL_STATES`); then the roll falls back on lumped states,
    /// with bounds (`roll_bounded`).
    pub(super) fn roll_classes(
        &self,
        rarity: u8,
        profile: usize,
        first: Option<usize>,
        forced: Option<&str>,
        first_restriction: Option<usize>,
    ) -> (Vec<Outcome>, Option<Vec<Bound>>) {
        let c = self.c();
        let Some((state, placed, also)) = self.forced_start(c, rarity, forced) else { return (Vec::new(), None) };
        let p0 = if forced.is_none() { first.unwrap_or(profile) } else { profile };
        let first_lump = first_restriction.map_or(&c.lumped[p0], |t| &c.lumped_no[t][p0]);
        let main = &c.lumped[profile];
        let mut classes: Vec<Class> = Vec::new();
        let mut drawn_groups = rustc_hash::FxHashSet::default();
        for (k, key) in c.keys.iter().enumerate() {
            let (f, m) = (first_lump.kw[k], main.kw[k]);
            if key.groups.iter().any(|g| also.contains(g)) || f.iter().chain(&m).all(|w| *w <= 0.0) {
                continue;
            }
            if key.groups.iter().any(|group| !drawn_groups.insert(group.as_str())) {
                return self.roll_bounded(rarity, profile, first, forced, first_restriction);
            }
            match classes.iter_mut().find(|x| x.f == f && x.m == m) {
                Some(x) => x.n += 1,
                None => {
                    let on = |side: Side| side.slots().iter().any(|&j| f[j] > 0.0 || m[j] > 0.0);
                    let side = match (on(Side::Prefix), on(Side::Suffix)) {
                        (true, false) => Some(Side::Prefix),
                        (false, true) => Some(Side::Suffix),
                        _ => None,
                    };
                    classes.push(Class { n: 1, f, m, side });
                }
            }
        }
        let restricted = first.is_some() && forced.is_none();
        let shape = RollShape::new(self, state, placed, restricted, first_lump, main, &classes);
        if failed_shapes().lock().unwrap().contains(&shape) {
            return self.roll_bounded(rarity, profile, first, forced, first_restriction);
        }
        match self.roll_dp(c, rarity, state, placed, restricted, first_lump, main, &classes) {
            Some(out) => (out, None),
            None => {
                failed_shapes().lock().unwrap().insert(shape);
                self.roll_bounded(rarity, profile, first, forced, first_restriction)
            }
        }
    }

    /// `roll_classes` over given classes; None past `ROLL_STATES`. `restricted`: the first mod
    /// comes from a filtered pool, and the roll can't be made if that is empty.
    #[allow(clippy::too_many_arguments)]
    fn roll_dp(
        &self,
        c: &Compiled,
        rarity: u8,
        state: State,
        placed: usize,
        restricted: bool,
        first_lump: &Lumped,
        main: &Lumped,
        classes: &[Class],
    ) -> Option<Vec<Outcome>> {
        const NONE: u8 = u8::MAX;
        /// Most partial items a roll carries from one draw to the next before falling back.
        const ROLL_STATES: usize = 30_000;
        if classes.len() >= NONE as usize {
            return None;
        }
        let counts = self.counts_for(rarity);
        // A partial item: its state and the classes of the junk drawn so far, sorted.
        let mut dist: Vec<(State, [u8; 6], f64)> = vec![(state, [NONE; 6], 1.0)];
        let mut result = Dist::new();
        let mut alive = 1.0;
        let top = counts.iter().map(|c| c.0).max().unwrap_or(0);
        let mut cands: Vec<(State, [u8; 6], f64)> = Vec::new();
        let mut next: Vec<(State, [u8; 6], f64)> = Vec::new();
        let mut at: FxHashMap<u128, usize> = FxHashMap::default();
        for k in placed..=top {
            let more: f64 = counts.iter().filter(|c| c.0 > k).map(|c| c.1).sum();
            let stop = alive - more;
            for &(s, _, p) in &dist {
                result.add(s, p * stop, p * stop, p * stop);
            }
            alive = more;
            if alive <= 1e-15 {
                break;
            }
            let first_draw = k == placed;
            // The draw before the last: what it lands is projected straight into the result.
            let last = k + 1 >= top;
            let lump = if first_draw { first_lump } else { main };
            next.clear();
            at.clear();
            for &(s, drawn, p) in &dist {
                let room = self.counts(c, &s);
                let cap_hit = self.notables_in(c, &s) >= self.notable_cap;
                let mut left = [0usize; NONE as usize];
                for (j, cl) in classes.iter().enumerate() {
                    left[j] = cl.n;
                }
                for &d in drawn.iter().filter(|&&d| d != NONE) {
                    left[d as usize] -= 1;
                }
                cands.clear();
                let mut total = 0.0;
                for side in [Side::Prefix, Side::Suffix] {
                    let limit = self.side_limit(rarity, side);
                    if room[side.index()] >= limit {
                        continue;
                    }
                    // Landing here fills the side: which of its groups were drawn stops mattering.
                    let fills = room[side.index()] + 1 >= limit;
                    let mut keep = drawn;
                    if fills {
                        for x in keep.iter_mut().filter(|x| **x != NONE && classes[**x as usize].side == Some(side)) {
                            *x = NONE;
                        }
                        keep.sort_unstable();
                    }
                    for i in 0..c.t_side.len() {
                        if c.t_side[i] != side || s.status[i] != MISSING || (c.t_notable[i] && cap_hit) {
                            continue;
                        }
                        for v in [GOOD, BLOCKED] {
                            let w = lump.tw[i][v as usize - 1];
                            if w > 0.0 {
                                let mut n = s;
                                n.status[i] = v;
                                cands.push((n, keep, w));
                                total += w;
                            }
                        }
                    }
                    for (j, cl) in classes.iter().enumerate() {
                        if left[j] == 0 {
                            continue;
                        }
                        let weights = if first_draw { &cl.f } else { &cl.m };
                        for &slot in side.slots() {
                            let w = left[j] as f64 * weights[slot];
                            if w <= 0.0 || ((slot == 0 || slot == 2) && cap_hit) {
                                continue;
                            }
                            let mut n = s;
                            n.junk[slot] += 1;
                            let mut d = keep;
                            if !(fills && cl.side == Some(side)) {
                                let free = d.iter().position(|&x| x == NONE).expect("at most six mods");
                                d[free] = j as u8;
                                d.sort_unstable();
                            }
                            cands.push((n, d, w));
                            total += w;
                        }
                    }
                }
                if cands.is_empty() {
                    if first_draw && restricted {
                        return Some(Vec::new()); // the first mod's restricted pool is empty: unusable
                    }
                    cands.push((s, drawn, 1.0));
                    total = 1.0;
                }
                for &(n, d, w) in &cands {
                    let q = p * w / total;
                    if last {
                        result.add(n, q * alive, q * alive, q * alive);
                        continue;
                    }
                    match at.entry(pack(&n, &d)) {
                        std::collections::hash_map::Entry::Occupied(e) => next[*e.get()].2 += q,
                        std::collections::hash_map::Entry::Vacant(e) => {
                            e.insert(next.len());
                            next.push((n, d, q));
                            if next.len() > ROLL_STATES {
                                return None;
                            }
                        }
                    }
                }
            }
            if last {
                break;
            }
            std::mem::swap(&mut dist, &mut next);
        }
        Some(result.finish().0)
    }

    /// A fresh roll on lumped states, with bounds: what `roll_classes` falls back on for big pools.
    pub(super) fn roll_bounded(
        &self,
        rarity: u8,
        profile: usize,
        first: Option<usize>,
        forced: Option<&str>,
        first_restriction: Option<usize>,
    ) -> (Vec<Outcome>, Option<Vec<Bound>>) {
        let c = self.c();
        let counts = self.counts_for(rarity);
        let Some((state, placed, also)) = self.forced_start(c, rarity, forced) else { return (Vec::new(), None) };
        // A forced junk mod takes its group out of the roll (a target's tiers stop rolling by themselves).
        let own: Option<(Lumped, [u8; JUNK_SLOTS])> =
            (!also.is_empty()).then(|| (self.lump(c, &c.profile_names[profile], also, None), state.junk));
        let mut dist = Dist::one(state);
        let mut result = Dist::new();
        let mut alive = 1.0; // chance the roll still has more mods to add
        let (mut scratch, mut bounds) = (Vec::new(), Vec::new());
        let top = counts.iter().map(|c| c.0).max().unwrap_or(0);
        for k in placed..=top {
            let more: f64 = counts.iter().filter(|c| c.0 > k).map(|c| c.1).sum();
            let stop = alive - more;
            for &(s, p, lo, hi) in &dist.items {
                result.add(s, p * stop, lo * stop, hi * stop);
            }
            alive = more;
            if alive <= 1e-15 {
                break;
            }
            let prof = if k == placed && forced.is_none() { first.unwrap_or(profile) } else { profile };
            let initial =
                if k == placed { first_restriction.map_or(&c.lumped[prof], |tag| &c.lumped_no[tag][prof]) } else { &c.lumped[prof] };
            let (lump, floor) = own.as_ref().map_or((initial, &c.base.junk), |(l, f)| (l, f));
            let mut next = Dist::new();
            for &(s, p, lo, hi) in &dist.items {
                self.add_mod_lumped(c, &s, lump, floor, &[Side::Prefix, Side::Suffix], 0, &mut scratch, &mut bounds);
                if scratch.is_empty() {
                    if k == placed && first.is_some() && forced.is_none() {
                        return (Vec::new(), None);
                    }
                    next.add(s, p, lo, hi);
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

    pub(super) fn lock_reroll_idx(&self, s: &State, keep: Side, profile: usize, first: Option<usize>) -> Option<Arc<Distribution>> {
        let c = self.c();
        if s.rarity != RARE {
            return None;
        }
        let mut state = *s;
        state.meta = 0;
        state.veiled = if self.veiled_side(s) == Some(keep) { s.veiled } else { 0 };
        state.unveil = 0;
        for i in 0..c.t_side.len() {
            if c.t_side[i] != keep {
                state.status[i] = c.base.status[i];
            }
        }
        for slot in 0..JUNK_SLOTS {
            if !keep.slots().contains(&slot) {
                state.junk[slot] = c.base.junk[slot];
            }
        }
        // Only the kept side matters, so states that agree on it share one (identical) outcome list.
        let key = (keep.index() as u8, profile as u32, first.map_or(u32::MAX, |f| f as u32), state);
        if let Some(hit) = self.caches.locks.lock().unwrap().get(&key) {
            return Some(hit.clone());
        }
        let (out, bounds) = self.fill_roll(state, profile, first, &[Side::Prefix, Side::Suffix], &self.rare_counts());
        let made = Arc::new(Distribution::new(out, bounds));
        Some(self.caches.locks.lock().unwrap().entry(key).or_insert(made).clone())
    }

    /// What a reroll's outcome depends on; None for actions that aren't rerolls.
    pub fn roll_key(&self, a: &Action) -> Option<RollKey> {
        match a.kind.effect() {
            Effect::Roll(MAGIC) => Some(RollKey::Magic(a.profile.clone())),
            Effect::Roll(RARE) | Effect::Reforge => {
                let forced = a.forced.as_deref().filter(|f| !f.is_empty());
                let class = forced.map(|f| {
                    let class = self.classify(f);
                    (class, if class.0 < 0 { self.mods()[f].groups.clone() } else { Vec::new() })
                });
                Some(RollKey::Rare(a.profile.clone(), a.first_profile.clone(), class))
            }
            _ => None,
        }
    }

    /// A reroll's outcome distribution, computed once per `roll_key` (the actions sharing one share it); None for other actions.
    pub fn roll(&self, a: &Action) -> Option<Arc<Distribution>> {
        self.refine(a);
        self.rolled_listed(a)
    }

    pub(super) fn rolled_listed(&self, a: &Action) -> Option<Arc<Distribution>> {
        let key = self.roll_key(a)?;
        if let Some(hit) = self.caches.rolls.lock().unwrap().get(&key) {
            return Some(hit.distribution.clone());
        }
        let made = self.make_roll(a, &key, false);
        Some(
            self.caches
                .rolls
                .lock()
                .unwrap()
                .entry(key)
                .or_insert(CachedRoll { distribution: made, attempted_exact: false })
                .distribution
                .clone(),
        )
    }

    pub(super) fn make_roll(&self, a: &Action, key: &RollKey, exact: bool) -> Arc<Distribution> {
        let first = a.first_profile.as_deref().filter(|f| !f.is_empty()).map(|f| self.profile_index(f));
        let forced = a.forced.as_deref().filter(|f| !f.is_empty());
        match key {
            RollKey::Magic(_) => self.roll_listed(MAGIC, self.profile_index(&a.profile), None, None, None, exact),
            RollKey::Rare(..) => self.roll_listed(RARE, self.profile_index(&a.profile), first, forced, None, exact),
        }
    }

    pub(crate) fn refine(&self, a: &Action) -> bool {
        let Some(key) = self.roll_key(a) else { return false };
        if self.caches.rolls.lock().unwrap().get(&key).is_some_and(|x| x.attempted_exact) {
            return false;
        }
        let made = self.make_roll(a, &key, true);
        let previous = self.caches.rolls.lock().unwrap().insert(key, CachedRoll { distribution: made.clone(), attempted_exact: true });
        let mut changed = previous.is_none_or(|old| old.distribution != made);
        if let Some(first) = a.first_profile.as_deref().filter(|f| !f.is_empty()).map(|f| self.profile_index(f)) {
            let profile = self.profile_index(&a.profile);
            for tag in 0..META_TAGS.len() {
                if self.caches.restricted_rolls.lock().unwrap().contains_key(&(profile, first, tag)) {
                    let roll = self.roll_listed(RARE, profile, Some(first), None, Some(tag), true);
                    let previous = self
                        .caches
                        .restricted_rolls
                        .lock()
                        .unwrap()
                        .insert((profile, first, tag), CachedRoll { distribution: roll.clone(), attempted_exact: true });
                    changed |= previous.is_none_or(|old| old.distribution != roll);
                }
            }
        }
        changed
    }

    pub(super) fn restricted_reforge(&self, profile: usize, first: usize, tag: usize) -> Arc<Distribution> {
        let key = (profile, first, tag);
        if let Some(roll) = self.caches.restricted_rolls.lock().unwrap().get(&key) {
            return roll.distribution.clone();
        }
        let roll = self.roll_listed(RARE, profile, Some(first), None, Some(tag), false);
        self.caches
            .restricted_rolls
            .lock()
            .unwrap()
            .entry(key)
            .or_insert(CachedRoll { distribution: roll, attempted_exact: false })
            .distribution
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_failed_roll_shapes_reuse_the_same_bounded_distribution_without_disabling_smaller_exact_rolls() {
        let pool: Vec<_> = (1..=20)
            .flat_map(|weight| {
                [Side::Prefix, Side::Suffix].map(move |side| {
                    let id = format!("{}-{weight}", side.name());
                    Mod::new(&id, side, weight as f64, &[&id], false, &id, &[])
                })
            })
            .collect();
        let mut spec = ProblemSpec::new(pool, Vec::new(), 0, Vec::new(), Kind::Gear);
        spec.profiles = BTreeMap::from([
            ("scaled".into(), spec.pool.iter().map(|m| (m.id.clone(), 7.0 * m.weight)).collect::<BTreeMap<_, _>>().into()),
            (
                "narrow".into(),
                spec.pool.iter().map(|m| (m.id.clone(), if m.weight == 1.0 { 1.0 } else { 0.0 })).collect::<BTreeMap<_, _>>().into(),
            ),
        ])
        .into();
        let problem = spec.compile();
        let c = problem.c();
        let classes: Vec<_> = problem
            .pool
            .iter()
            .map(|m| {
                let mut weights = [0.0; JUNK_SLOTS];
                weights[if m.side == Side::Prefix { 1 } else { 3 }] = m.weight;
                Class { n: 1, f: weights, m: weights, side: Some(m.side) }
            })
            .collect();
        let state = State { rarity: RARE, ..problem.start() };
        let shape = RollShape::new(&problem, state, 0, false, &c.lumped[0], &c.lumped[0], &classes);
        assert!(problem.roll_dp(c, RARE, state, 0, false, &c.lumped[0], &c.lumped[0], &classes).is_none());
        let (base, bounds) = problem.roll_classes(RARE, 0, None, None, None);
        assert!(bounds.is_some());
        assert!(failed_shapes().lock().unwrap().contains(&shape));
        let scaled = problem.profile_index("scaled");
        let (scaled, scaled_bounds) = problem.roll_classes(RARE, scaled, None, None, None);
        assert!(scaled_bounds.is_some());
        assert_eq!(base.len(), scaled.len());
        for (&(left, state), &(right, other)) in base.iter().zip(&scaled) {
            assert_eq!(state, other);
            assert!((left - right).abs() < 1e-12);
        }
        let narrow = problem.profile_index("narrow");
        let (_, bounds) = problem.roll_classes(RARE, narrow, None, None, None);
        assert!(bounds.is_none());
        let mut spec = problem.to_spec();
        spec.affix_limits = Some([1, 1]);
        let (_, bounds) = spec.compile().roll_classes(RARE, 0, None, None, None);
        assert!(bounds.is_none());
    }
}
