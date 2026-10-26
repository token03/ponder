//! Solver states: items reduced to what the goals and operations can tell apart (SPEC §5.3).
//!
//! A lens is built from the targets and operations of one problem. Mods fall into classes:
//! - tracked: mods sharing a group with anything a target asks for or an operation forces,
//!   split by which needs they serve and which tags operations read;
//! - fixed: forced mods that never roll (essence-only, crafted), one class each;
//! - untracked: everything else, counted per side and read tags. Their effect on later draws
//!   uses the class's average group weight, the one estimate the lens makes.
//!
//! [`Lens::run`] interprets an operation's steps on states exactly, merging equal states after
//! every step.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::game::{Game, GroupId, Kind, ModId, Side, TagId};
use crate::goal::{Rarity, Source};
use crate::item::Item;
use crate::ops::{Ctx, Operation, Step, View, count_table};
use crate::target::{Target, matches};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct State {
    pub rarity: Rarity,
    /// Per class: normal affixes, then (after `classes`) fractured ones.
    pub counts: Box<[u8]>,
}

#[derive(Debug)]
struct Class {
    side: Side,
    /// Total spawn weight of its natural mods; 0 for fixed classes.
    weight: f64,
    /// Distinct mod groups in an untracked class (for the average-weight estimate).
    groups: usize,
    tracked: bool,
    crafted: bool,
    /// Bit per read tag.
    tags: u32,
    /// Bit per (target, need) the class's mods serve.
    serves: u128,
    /// Classes that block this one when present (same group), itself included.
    conflicts: Vec<usize>,
}

pub struct Lens<'g> {
    game: &'g Game,
    pub kind: Kind,
    class_name: String,
    classes: Vec<Class>,
    class_of: HashMap<ModId, usize>,
    read_tags: Vec<TagId>,
    targets: Vec<(Target, usize)>,
    /// Class of the prefix lock and suffix lock metamods, and of any metamod.
    lock: [Option<usize>; 2],
    metamods: Vec<usize>,
}

type Key = (Side, Option<GroupId>, u128, u32, Option<ModId>);
/// Expands one (state, rolled count) into weighted successors.
type Expand<'a> = dyn Fn(State, u8, &mut dyn FnMut(f64, State, u8)) + 'a;

impl<'g> Lens<'g> {
    pub fn new(game: &'g Game, kind: Kind, targets: Vec<Target>, ops: &[&dyn Operation]) -> Lens<'g> {
        let mut offsets = Vec::new();
        let mut bits = 0;
        for t in &targets {
            offsets.push(bits);
            bits += t.needs.len();
        }
        assert!(bits <= 128, "at most 128 needs per lens");
        let serves = |m: ModId| -> u128 {
            let mut s = 0u128;
            for (t, off) in targets.iter().zip(&offsets) {
                for (i, need) in t.needs.iter().enumerate() {
                    if need.ok.contains(&m) {
                        s |= 1 << (off + i);
                    }
                }
            }
            s
        };
        let steps = ops.iter().flat_map(|op| op.steps().iter().copied());
        let read_tags: Vec<TagId> = steps
            .clone()
            .filter_map(|s| if let Step::Draw { tag, .. } = s { tag } else { None })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let forced: BTreeSet<ModId> = steps.filter_map(|s| if let Step::Force(m) = s { Some(m) } else { None }).collect();
        let tag_bits =
            |m: ModId| read_tags.iter().enumerate().filter(|(_, t)| game.def(m).tags.contains(t)).fold(0u32, |b, (i, _)| b | 1 << i);

        let pool = game.pool(kind);
        let natural: Vec<(ModId, f64)> = pool.mods.iter().filter(|(m, _)| game.def(*m).level <= kind.ilvl as u32).copied().collect();
        let natural_ids: BTreeSet<ModId> = natural.iter().map(|p| p.0).collect();
        let wanted = targets.iter().flat_map(|t| t.needs.iter().flat_map(|n| n.ok.iter().copied()));
        let tracked_groups: BTreeSet<GroupId> =
            wanted.chain(forced.iter().copied()).flat_map(|m| game.def(m).groups.iter().copied()).collect();

        let mut keys: BTreeMap<Key, (Vec<ModId>, f64)> = BTreeMap::new();
        for &(m, w) in &natural {
            let def = game.def(m);
            let group = def.groups.iter().copied().find(|g| tracked_groups.contains(g));
            let entry = keys.entry((def.side, group, serves(m), tag_bits(m), None)).or_default();
            entry.0.push(m);
            entry.1 += w;
        }
        for &m in forced.iter().filter(|m| !natural_ids.contains(m)) {
            keys.insert((game.def(m).side, None, serves(m), 0, Some(m)), (vec![m], 0.0));
        }

        let mut classes = Vec::new();
        let mut class_of = HashMap::new();
        let mut group_classes: HashMap<GroupId, Vec<usize>> = HashMap::new();
        for ((side, group, serves, tags, fixed), (mods, weight)) in keys {
            let idx = classes.len();
            let groups: BTreeSet<GroupId> = mods.iter().flat_map(|m| game.def(*m).groups.iter().copied()).collect();
            for g in groups.iter().filter(|g| tracked_groups.contains(g)) {
                group_classes.entry(*g).or_default().push(idx);
            }
            for &m in &mods {
                class_of.insert(m, idx);
            }
            classes.push(Class {
                side,
                weight,
                groups: groups.len().max(1),
                tracked: group.is_some() || fixed.is_some(),
                crafted: fixed.is_some_and(|m| game.def(m).source == Source::Crafted),
                tags,
                serves,
                conflicts: Vec::new(),
            });
        }
        for (i, c) in classes.iter_mut().enumerate() {
            if c.tracked {
                let mine: Vec<ModId> = class_of.iter().filter(|(_, ci)| **ci == i).map(|(m, _)| *m).collect();
                let mut conflicts: BTreeSet<usize> =
                    mine.iter().flat_map(|m| game.def(*m).groups.iter()).filter_map(|g| group_classes.get(g)).flatten().copied().collect();
                conflicts.insert(i);
                c.conflicts = conflicts.into_iter().collect();
            }
        }
        let class_for = |key: Option<&ModId>| key.and_then(|m| class_of.get(m).copied());
        let lock = [class_for(game.metamods.first()), class_for(game.metamods.get(1))];
        let metamods = game.metamods.iter().filter_map(|m| class_of.get(m).copied()).collect();
        let targets = targets.into_iter().zip(offsets).collect();
        let class_name = game.bases[kind.base.0 as usize].class.clone();
        Lens { game, kind, class_name, classes, class_of, read_tags, targets, lock, metamods }
    }

    pub fn classes(&self) -> usize {
        self.classes.len()
    }

    /// The state of a real item of this lens's kind.
    pub fn project(&self, item: &Item) -> State {
        let n = self.classes.len();
        let mut counts = vec![0u8; 2 * n];
        for a in &item.affixes {
            let c = *self.class_of.get(&a.id).unwrap_or_else(|| panic!("{} is outside the lens", self.game.def(a.id).key));
            counts[c + if a.fractured { n } else { 0 }] += 1;
        }
        State { rarity: item.rarity, counts: counts.into() }
    }

    pub fn view<'a>(&'a self, state: &'a State) -> StateView<'a> {
        StateView { lens: self, state }
    }

    /// Does target `t` (in construction order) hold?
    pub fn holds(&self, t: usize, state: &State) -> bool {
        let (target, offset) = &self.targets[t];
        let v = self.view(state);
        if self.kind.ilvl < target.ilvl || !target.shape(state.rarity, Side::BOTH.map(|s| v.count(s)), Side::BOTH.map(|s| v.open(s))) {
            return false;
        }
        let n = self.classes.len();
        let affixes: Vec<(usize, bool)> = (0..2 * n).flat_map(|i| std::iter::repeat_n((i % n, i >= n), state.counts[i] as usize)).collect();
        matches(&target.needs, affixes.len(), &|need, i| {
            let (class, fractured) = affixes[i];
            self.classes[class].serves >> (offset + need) & 1 == 1 && (!target.needs[need].fractured || fractured)
        })
    }

    /// The exact outcome distribution of `steps` on `state`.
    pub fn run(&self, ctx: &Ctx, steps: &[Step], state: &State) -> Vec<(f64, State)> {
        let mut dist: Vec<(f64, State, u8)> = vec![(1.0, state.clone(), 0)];
        let merge = |dist: Vec<(f64, State, u8)>, f: &Expand| {
            let mut next: HashMap<(State, u8), f64> = HashMap::new();
            for (p, s, total) in dist {
                f(s, total, &mut |q, s2, t2| *next.entry((s2, t2)).or_default() += p * q);
            }
            next.into_iter().map(|((s, t), p)| (p, s, t)).collect::<Vec<_>>()
        };
        for step in steps {
            if *step == Step::Fill {
                // One draw at a time, merging in between: at most six affixes.
                for _ in 0..6 {
                    dist = merge(dist, &|s, total, emit| {
                        if (self.view(&s).total() as u8) < total { self.draw(s, total, None, None, emit) } else { emit(1.0, s, total) }
                    });
                }
            } else {
                dist = merge(dist, &|s, total, emit| self.step(ctx, *step, s, total, emit));
            }
        }
        let mut out: Vec<(f64, State)> = dist.into_iter().map(|(p, s, _)| (p, s)).collect();
        out.sort_by(|a, b| a.1.cmp(&b.1));
        out
    }

    fn step(&self, ctx: &Ctx, step: Step, mut s: State, total: u8, emit: &mut dyn FnMut(f64, State, u8)) {
        let n = self.classes.len();
        match step {
            Step::Clear { keep_locked } => {
                let locked = if keep_locked { self.view(&s).locked() } else { None };
                for c in 0..n {
                    if locked != Some(self.classes[c].side) {
                        s.counts[c] = 0;
                    }
                }
                emit(1.0, s, total);
            }
            Step::Rarity(r) => {
                s.rarity = r;
                emit(1.0, s, total);
            }
            Step::Settle => {
                let v = self.view(&s);
                let most = Side::BOTH.iter().map(|x| v.count(*x)).max().unwrap_or(0);
                s.rarity = match most {
                    0 => Rarity::Normal,
                    1 => Rarity::Magic,
                    _ => Rarity::Rare,
                };
                emit(1.0, s, total);
            }
            Step::RollCount => {
                let table = count_table(ctx, &self.class_name, s.rarity);
                let sum: f64 = table.iter().map(|e| e.1).sum();
                for &(count, w) in table {
                    emit(w / sum, s.clone(), count);
                }
            }
            Step::Force(m) => {
                s.counts[self.class_of[&m]] += 1;
                emit(1.0, s, total);
            }
            Step::Draw { side, tag } => self.draw(s, total, side, tag, emit),
            Step::Fill => unreachable!("run expands Fill"),
            Step::Remove { respect_lock } => {
                let locked = if respect_lock { self.view(&s).locked() } else { None };
                let options: Vec<usize> = (0..n).filter(|&c| s.counts[c] > 0 && locked != Some(self.classes[c].side)).collect();
                let all: f64 = options.iter().map(|&c| s.counts[c] as f64).sum();
                if all == 0.0 {
                    return emit(1.0, s, total);
                }
                for c in options {
                    let mut t = s.clone();
                    t.counts[c] -= 1;
                    emit(s.counts[c] as f64 / all, t, total);
                }
            }
            Step::RemoveCrafted => {
                for c in 0..n {
                    if self.classes[c].crafted {
                        s.counts[c] = 0;
                    }
                }
                emit(1.0, s, total);
            }
        }
    }

    /// One weighted draw; the state is unchanged when nothing fits.
    fn draw(&self, s: State, total: u8, side: Option<Side>, tag: Option<TagId>, emit: &mut dyn FnMut(f64, State, u8)) {
        let options = self.options(&s, side, tag);
        let all: f64 = options.iter().map(|o| o.1).sum();
        if all <= 0.0 {
            return emit(1.0, s, total);
        }
        for (c, w) in options {
            let mut t = s.clone();
            t.counts[c] += 1;
            emit(w / all, t, total);
        }
    }

    /// Classes a draw can land in, with their available weight.
    fn options(&self, s: &State, side: Option<Side>, tag: Option<TagId>) -> Vec<(usize, f64)> {
        let v = self.view(s);
        let tag_bit = tag.map(|t| 1u32 << self.read_tags.iter().position(|x| *x == t).expect("tag read by an operation"));
        let open = Side::BOTH.map(|x| v.open(x) > 0);
        let n = self.classes.len();
        let present = |c: usize| s.counts[c] + s.counts[c + n];
        (0..n)
            .filter_map(|c| {
                let class = &self.classes[c];
                if class.weight <= 0.0 || !open[class.side as usize] || side.is_some_and(|x| x != class.side) {
                    return None;
                }
                if tag_bit.is_some_and(|b| class.tags & b == 0) {
                    return None;
                }
                let w = if class.tracked {
                    if class.conflicts.iter().any(|&o| present(o) > 0) { 0.0 } else { class.weight }
                } else {
                    class.weight * (1.0 - present(c) as f64 / class.groups as f64).max(0.0)
                };
                (w > 0.0).then_some((c, w))
            })
            .collect()
    }
}

/// A solver state seen through the [`View`] operations ask.
pub struct StateView<'a> {
    lens: &'a Lens<'a>,
    state: &'a State,
}

impl StateView<'_> {
    fn total(&self) -> usize {
        self.state.counts.iter().map(|&c| c as usize).sum()
    }

    fn present(&self, c: usize) -> bool {
        let n = self.lens.classes.len();
        self.state.counts[c] + self.state.counts[c + n] > 0
    }
}

impl View for StateView<'_> {
    fn rarity(&self) -> Rarity {
        self.state.rarity
    }
    fn count(&self, side: Side) -> usize {
        let n = self.lens.classes.len();
        (0..2 * n).filter(|&i| self.lens.classes[i % n].side == side).map(|i| self.state.counts[i] as usize).sum()
    }
    fn limit(&self) -> usize {
        let jewel = self.lens.class_name.contains("Jewel");
        match self.state.rarity {
            Rarity::Normal => 0,
            Rarity::Magic => 1,
            Rarity::Rare if jewel => 2,
            Rarity::Rare => 3,
        }
    }
    fn crafted(&self) -> usize {
        (0..self.lens.classes.len()).filter(|&c| self.lens.classes[c].crafted).map(|c| self.state.counts[c] as usize).sum()
    }
    fn has(&self, m: ModId) -> bool {
        self.lens.class_of.get(&m).is_some_and(|&c| self.present(c))
    }
    fn removable(&self, respect_lock: bool) -> usize {
        let locked = if respect_lock { self.locked() } else { None };
        (0..self.lens.classes.len()).filter(|&c| locked != Some(self.lens.classes[c].side)).map(|c| self.state.counts[c] as usize).sum()
    }
    fn fits(&self, m: ModId) -> bool {
        let Some(&c) = self.lens.class_of.get(&m) else { return false };
        let class = &self.lens.classes[c];
        self.open(class.side) > 0 && !class.conflicts.iter().any(|&o| self.present(o))
    }
    fn fixed_blocks(&self, m: ModId) -> bool {
        let n = self.lens.classes.len();
        let Some(&c) = self.lens.class_of.get(&m) else { return false };
        self.lens.classes[c].conflicts.iter().any(|&o| self.state.counts[o + n] > 0)
    }
    fn locked(&self) -> Option<Side> {
        match self.lens.lock {
            [Some(c), _] if self.present(c) => Some(Side::Prefix),
            [_, Some(c)] if self.present(c) => Some(Side::Suffix),
            _ => None,
        }
    }
    fn has_metamod(&self) -> bool {
        self.lens.metamods.iter().any(|&c| self.present(c))
    }
}
