use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SideState {
    pub state: State,
    pub locked: bool,
}

pub struct SideRelaxation {
    pub side: Side,
    on_side: Vec<bool>,
    needs: Vec<(Vec<usize>, u8, bool)>,
    lock_mod: Option<ModId>,
    lock_class: Option<usize>,
    contexts: Vec<Vec<usize>>,
}

impl SideRelaxation {
    pub fn new(lens: &Lens<'_>, goal: usize) -> Option<Self> {
        let target = &lens.targets[goal];
        let (side, needs) = Side::BOTH
            .map(|side| {
                let needs: Vec<_> = target
                    .target
                    .needs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !target.classes[*i].is_empty() && target.classes[*i].iter().all(|&c| lens.classes[c].side == side))
                    .map(|(i, need)| (target.classes[i].clone(), need.count, need.fractured))
                    .collect();
                (side, needs)
            })
            .into_iter()
            .max_by_key(|(_, needs)| needs.iter().map(|(_, count, _)| *count as usize).sum::<usize>())?;
        let total: usize = needs.iter().map(|(_, n, _)| *n as usize).sum();
        if !(2..=3).contains(&total) {
            return None;
        }
        let mut blockers = BTreeMap::<(Vec<usize>, u32), usize>::new();
        for (c, class) in lens.classes.iter().enumerate().filter(|(_, c)| c.side != side) {
            let conflicts: Vec<_> =
                lens.classes.iter().enumerate().filter(|(_, d)| d.side == side && d.conflicts.contains(&c)).map(|(d, _)| d).collect();
            let banned = lens.bans.iter().filter(|(b, _)| *b == c).fold(0, |bits, (_, tags)| bits | tags);
            if !conflicts.is_empty() || banned != 0 {
                if class.veil {
                    return None;
                }
                blockers.entry((conflicts, banned)).or_insert(c);
            }
        }
        let blockers: Vec<_> = blockers.into_values().collect();
        let mut contexts = vec![Vec::new()];
        for (i, &a) in blockers.iter().enumerate() {
            contexts.push(vec![a]);
            for (j, &b) in blockers.iter().enumerate().skip(i + 1) {
                contexts.push(vec![a, b]);
                for &c in &blockers[j + 1..] {
                    contexts.push(vec![a, b, c]);
                }
            }
        }
        let state = State { rarity: Rarity::Rare, influences: 0, eldritch: None, counts: [0; 2 * CLASSES] };
        if lens.view(&state).limit() != 3 {
            return None;
        }
        let on_side = lens.classes.iter().map(|c| c.side == side).collect();
        Some(Self {
            side,
            on_side,
            needs,
            lock_mod: lens.game.metamods.get(side as usize).copied(),
            lock_class: lens.lock[side as usize],
            contexts,
        })
    }

    pub fn project(&self, state: &State) -> SideState {
        let n = self.on_side.len();
        let locked = self.lock_class.is_some_and(|c| state.counts[c] + state.counts[c + n] > 0);
        let mut state = *state;
        for (c, &keep) in self.on_side.iter().enumerate() {
            if !keep {
                state.counts[c] = 0;
                state.counts[c + n] = 0;
            }
        }
        SideState { state, locked }
    }

    pub fn holds(&self, state: SideState) -> bool {
        let n = self.on_side.len();
        self.needs.iter().all(|(classes, count, fractured)| {
            classes
                .iter()
                .map(|&c| state.state.counts[c + n] as usize + if *fractured { 0 } else { state.state.counts[c] as usize })
                .sum::<usize>()
                >= *count as usize
        })
    }

    pub fn required(&self) -> usize {
        self.needs.iter().map(|(_, n, _)| *n as usize).sum()
    }

    pub fn contexts(&self) -> usize {
        self.contexts.len()
    }

    fn context(&self, state: SideState, context: usize) -> State {
        let mut raw = state.state;
        for &c in &self.contexts[context] {
            raw.counts[c] = 1;
        }
        raw
    }

    pub fn count(&self, state: SideState) -> usize {
        state.state.counts.iter().map(|&n| n as usize).sum()
    }

    pub fn fractured(&self, state: SideState) -> usize {
        state.state.counts[self.on_side.len()..].iter().map(|&n| n as usize).sum()
    }

    pub fn erase(&self, mut state: SideState, c: usize) -> SideState {
        state.state.counts[c] -= 1;
        state
    }

    pub fn unfix(&self, mut state: SideState, c: usize) -> SideState {
        let n = self.on_side.len();
        state.state.counts[c + n] -= 1;
        state.state.counts[c] += 1;
        state
    }

    pub fn clear(&self, mut state: SideState, sides: Sides, respect_lock: bool) -> SideState {
        if resolve(sides, state.state.eldritch)[self.side as usize] && !(respect_lock && state.locked) {
            state.state.counts[..self.on_side.len()].fill(0);
        }
        state
    }

    pub fn force(&self, lens: &Lens<'_>, mut state: SideState, m: ModId) -> SideState {
        let c = lens.class_of[&m];
        if self.on_side[c] && self.count(state) < if state.state.rarity == Rarity::Rare { 3 } else { 1 } {
            state.state.counts[c] += 1;
        }
        state.locked |= self.lock_mod == Some(m);
        state
    }

    pub fn craft(&self, lens: &Lens<'_>, state: SideState, mods: &[ModId]) -> Vec<SideState> {
        let mut out = vec![state];
        for &m in mods {
            let next = self.force(lens, state, m);
            if !out.contains(&next) {
                out.push(next);
            }
        }
        out
    }

    pub fn draw(&self, lens: &Lens<'_>, ctx: &Ctx, state: SideState, pick: &Pick, context: usize) -> Vec<(f64, SideState)> {
        if !resolve(pick.sides, state.state.eldritch)[self.side as usize] {
            return vec![(1.0, state)];
        }
        let pick = Pick { sides: Sides::Only(self.side), ..pick.clone() };
        lens.run(ctx, &[Step::Draw(pick)], &self.context(state, context))
            .into_iter()
            .map(|(p, s)| (p, SideState { state: self.project(&s).state, ..state }))
            .collect()
    }

    pub fn veil(&self, lens: &Lens<'_>, ctx: &Ctx, state: SideState, sides: Sides) -> Vec<(f64, SideState)> {
        if !resolve(sides, state.state.eldritch)[self.side as usize] {
            return vec![(1.0, state)];
        }
        lens.run(ctx, &[Step::Veil(Sides::Only(self.side))], &state.state)
            .into_iter()
            .map(|(p, s)| (p, SideState { state: s, ..state }))
            .collect()
    }

    pub fn run(&self, lens: &Lens<'_>, ctx: &Ctx, state: SideState, step: &Step, context: usize) -> Vec<(f64, SideState)> {
        lens.run(ctx, std::slice::from_ref(step), &self.context(state, context))
            .into_iter()
            .map(|(p, s)| (p, SideState { state: self.project(&s).state, ..state }))
            .collect()
    }

    pub fn fracture(&self, lens: &Lens<'_>, ctx: &Ctx, state: SideState) -> Vec<(f64, SideState)> {
        lens.run(ctx, &[Step::Fracture], &state.state).into_iter().map(|(p, s)| (p, SideState { state: s, ..state })).collect()
    }

    pub fn pool(&self, first: SideState, second: SideState, heir: SideState) -> SideState {
        let n = self.on_side.len();
        let mut state = heir.state;
        state.rarity = Rarity::Rare;
        for c in 0..n {
            state.counts[c] = first.state.counts[c] + second.state.counts[c];
        }
        SideState { state, locked: first.locked || second.locked }
    }

    pub fn sample(&self, lens: &Lens<'_>, ctx: &Ctx, pool: SideState, excluded: bool) -> Vec<(f64, SideState)> {
        let affixes: Vec<_> = pool.state.counts.iter().copied().enumerate().filter(|(_, n)| *n > 0).collect();
        lens.kept(ctx, &affixes, 3, excluded, pool.state.influences)
            .iter()
            .map(|(p, kept)| {
                let mut state = pool.state;
                state.counts.fill(0);
                for (&(c, _), &count) in affixes.iter().zip(kept) {
                    state.counts[c] = count;
                }
                (*p, SideState { state, ..pool })
            })
            .collect()
    }
}
