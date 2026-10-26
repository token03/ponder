//! Solver states: items reduced to what the goals and operations can tell apart (SPEC §5.3).
//!
//! A lens is built from the targets and operations of one problem. Mods fall into classes:
//! - tracked: mods sharing a group with anything a target asks for, an operation forces or an
//!   unveil prefers, split by which needs they serve and which tags operations read;
//! - fixed: forced mods that never roll (essence-only, crafted) and veiled placeholders;
//! - untracked: everything else, counted per side, origin and read tags. Their effect on later
//!   draws uses the class's average group weight, the one estimate the lens makes.
//!
//! Classes also split by origin (natural, an influence's, fossil-only, unveiled), so every draw
//! pool gives each class one weight. [`Lens::run`] interprets an operation's steps on states
//! exactly, merging equal states after every step. Unveil offers are drawn by class, removing a
//! class's average mod weight per offer (exact for single-mod classes).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::game::{Game, GroupId, Influences, Kind, ModId, Side, TagId};
use crate::goal::{Influence, Rarity, Source};
use crate::item::Item;
use crate::ops::{
    Ctx, Operation, Pick, Pool, Sides, Step, TagFilter, UNVEIL_OFFERS, View, count_table, pool_weight, resolve, unveil_weight,
};
use crate::target::{Target, matches};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct State {
    pub rarity: Rarity,
    pub influences: u8,
    pub eldritch: Option<Side>,
    /// Per class: normal affixes, then (after `classes`) fractured ones.
    pub counts: Box<[u8]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Origin {
    /// Natural, or needing this influence on the item.
    Natural(Option<Influence>),
    Fixed,
}

#[derive(Debug)]
struct Class {
    side: Side,
    origin: Origin,
    /// Distinct mod groups (for the average-weight estimate) and veiled mods (for unveil offers).
    groups: usize,
    veiled: usize,
    tracked: bool,
    crafted: bool,
    veil: bool,
    /// Bit per read tag.
    tags: u32,
    /// Bit per (target, need) the class's mods serve.
    serves: u128,
    /// Unveil preference: 1 is kept first, 0 is not preferred.
    rank: u8,
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
    /// Class weights per draw (pool and tag filter), and for unveil offers.
    pools: Vec<((Pool, TagFilter), Vec<f64>)>,
    unveil: Vec<f64>,
    targets: Vec<(Target, usize)>,
    /// Class of the prefix lock and suffix lock metamods; of any metamod; of the attack and
    /// caster bans with the tag bit each removes.
    lock: [Option<usize>; 2],
    metamods: Vec<usize>,
    bans: Vec<(usize, u32)>,
}

type Key = (Side, Option<GroupId>, u128, u32, Origin, Option<ModId>, u8);
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

        // What the operations read and add.
        let steps: Vec<&Step> = ops.iter().flat_map(|op| op.steps()).collect();
        let mut picks: Vec<&Pick> = Vec::new();
        let mut filters: Vec<TagFilter> = Vec::new();
        let mut forced: BTreeSet<ModId> = BTreeSet::new();
        let mut prefs: Vec<&Vec<Vec<ModId>>> = Vec::new();
        let mut influences = kind.influences.0 | targets.iter().fold(0, |b, t| b | t.influences.0);
        for step in &steps {
            match step {
                Step::Draw(p) | Step::Fill(p) => picks.push(p),
                Step::Swap { add, remove } => {
                    picks.push(add);
                    filters.push(*remove);
                }
                Step::Remove { tag, .. } => filters.push(*tag),
                Step::Force(m) => {
                    forced.insert(*m);
                }
                Step::Craft(ms) => forced.extend(ms.iter().copied()),
                Step::Unveil(p) => prefs.push(p),
                Step::Influence(i) => influences |= 1 << *i as u8,
                Step::Veil(_) => forced.extend(game.veils),
                _ => {}
            }
        }
        let kind = Kind { influences: Influences(influences), ..kind };
        let bans: Vec<(ModId, TagId)> = [(2, "attack"), (3, "caster")]
            .into_iter()
            .filter_map(|(i, t)| Some((*game.metamods.get(i)?, game.tag_id(t)?)))
            .filter(|(m, _)| forced.contains(m))
            .collect();
        let mut read: BTreeSet<TagId> = BTreeSet::new();
        // Tagged draws need their tag's mods apart: a mod present blocks its group, and the
        // average-group estimate cannot see which part of a mixed class the group held.
        read.extend(picks.iter().filter_map(|p| p.tag.map(|t| t.0)));
        read.extend(filters.iter().filter_map(|f| f.map(|t| t.0)));
        read.extend(bans.iter().map(|b| b.1));
        let read_tags: Vec<TagId> = read.into_iter().collect();
        assert!(read_tags.len() <= 32, "at most 32 read tags per lens");
        let tag_bits =
            |m: ModId| read_tags.iter().enumerate().filter(|(_, t)| game.def(m).tags.contains(t)).fold(0u32, |b, (i, _)| b | 1 << i);
        let rank_of =
            |m: ModId| -> u8 { prefs.iter().filter_map(|p| p.iter().position(|list| list.contains(&m))).min().map_or(0, |r| r as u8 + 1) };
        let mut pools: Vec<(Pool, TagFilter)> = Vec::new();
        for p in &picks {
            if !pools.contains(&(p.pool.clone(), p.tag)) {
                pools.push((p.pool.clone(), p.tag));
            }
        }
        let added: BTreeSet<ModId> = pools
            .iter()
            .filter_map(|p| if let (Pool::Natural(Some(b)), _) = p { Some(b.added.iter().copied()) } else { None })
            .flatten()
            .collect();

        let pool = game.pool(kind);
        let rolls: Vec<(ModId, f64)> = pool
            .mods
            .iter()
            .filter(|(m, _)| {
                let def = game.def(*m);
                def.level <= kind.ilvl as u32
                    && match def.source {
                        Source::Natural | Source::Influence => true,
                        Source::Fossil => added.contains(m),
                        Source::Veiled => !prefs.is_empty() && !game.veils.contains(m),
                        _ => false,
                    }
            })
            .copied()
            .collect();
        let roll_ids: BTreeSet<ModId> = rolls.iter().map(|p| p.0).collect();
        let pref_mods = prefs.iter().flat_map(|p| p.iter().flatten().copied());
        let wanted = targets.iter().flat_map(|t| t.needs.iter().flat_map(|n| n.ok.iter().copied()));
        let tracked_groups: BTreeSet<GroupId> =
            wanted.chain(forced.iter().copied()).chain(pref_mods).flat_map(|m| game.def(m).groups.iter().copied()).collect();

        let mut keys: BTreeMap<Key, Vec<ModId>> = BTreeMap::new();
        for &(m, _) in &rolls {
            let def = game.def(m);
            let group = def.groups.iter().copied().find(|g| tracked_groups.contains(g));
            // Fossil and veiled mods differ from natural ones only in their weights.
            let origin = Origin::Natural(def.influence.filter(|_| def.source == Source::Influence));
            let rank = if def.source == Source::Veiled { rank_of(m) } else { 0 };
            keys.entry((def.side, group, serves(m), tag_bits(m), origin, None, rank)).or_default().push(m);
        }
        for &m in forced.iter().filter(|m| !roll_ids.contains(m)) {
            keys.insert((game.def(m).side, None, serves(m), tag_bits(m), Origin::Fixed, Some(m), 0), vec![m]);
        }

        let mut classes = Vec::new();
        let mut class_of = HashMap::new();
        let mut group_classes: HashMap<GroupId, Vec<usize>> = HashMap::new();
        for ((side, group, serves, tags, origin, fixed, rank), mods) in keys {
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
                origin,
                groups: groups.len().max(1),
                veiled: mods.iter().filter(|m| game.def(**m).source == Source::Veiled).count(),
                tracked: group.is_some() || fixed.is_some(),
                crafted: fixed.is_some_and(|m| game.def(m).source == Source::Crafted),
                veil: fixed.is_some_and(|m| game.veils.contains(&m)),
                tags,
                serves,
                rank,
                conflicts: Vec::new(),
            });
        }
        let mut members: Vec<Vec<ModId>> = vec![Vec::new(); classes.len()];
        for (m, c) in &class_of {
            members[*c].push(*m);
        }
        for (i, c) in classes.iter_mut().enumerate() {
            if c.tracked {
                let mut conflicts: BTreeSet<usize> = members[i]
                    .iter()
                    .flat_map(|m| game.def(*m).groups.iter())
                    .filter_map(|g| group_classes.get(g))
                    .flatten()
                    .copied()
                    .collect();
                conflicts.insert(i);
                c.conflicts = conflicts.into_iter().collect();
            }
        }
        let weight_of: HashMap<ModId, f64> = rolls.iter().copied().collect();
        let class_weights = |f: &dyn Fn(ModId, f64) -> f64| -> Vec<f64> {
            members.iter().map(|ms| ms.iter().map(|m| weight_of.get(m).map_or(0.0, |w| f(*m, *w))).sum()).collect()
        };
        let pools = pools.into_iter().map(|(p, tag)| {
            let passes = |m: ModId| tag.is_none_or(|(t, want)| game.def(m).tags.contains(&t) == want);
            let w = class_weights(&|m, w| if passes(m) { pool_weight(game.def(m), m, w, &p) } else { 0.0 });
            ((p, tag), w)
        });
        let pools = pools.collect();
        let unveil = class_weights(&|m, w| unveil_weight(game, game.def(m), m, w));
        let class_for = |key: Option<&ModId>| key.and_then(|m| class_of.get(m).copied());
        let lock = [class_for(game.metamods.first()), class_for(game.metamods.get(1))];
        let metamods = game.metamods.iter().filter_map(|m| class_of.get(m).copied()).collect();
        let bans = bans.iter().filter_map(|(m, t)| Some((*class_of.get(m)?, 1u32 << read_tags.iter().position(|x| x == t)?))).collect();
        let targets = targets.into_iter().zip(offsets).collect();
        let class_name = game.bases[kind.base.0 as usize].class.clone();
        Lens { game, kind, class_name, classes, class_of, read_tags, pools, unveil, targets, lock, metamods, bans }
    }

    pub fn classes(&self) -> usize {
        self.classes.len()
    }

    /// One class for diagnostics: its fields and a few of its mods.
    pub fn describe(&self, c: usize) -> String {
        let mods: Vec<&str> = self.class_of.iter().filter(|(_, k)| **k == c).map(|(m, _)| self.game.def(*m).key.as_str()).take(3).collect();
        format!("{:?} {mods:?}", self.classes[c])
    }

    /// The state of a real item of this lens's base.
    pub fn project(&self, item: &Item) -> State {
        let n = self.classes.len();
        let mut counts = vec![0u8; 2 * n];
        for a in &item.affixes {
            let c = *self.class_of.get(&a.id).unwrap_or_else(|| panic!("{} is outside the lens", self.game.def(a.id).key));
            counts[c + if a.fractured { n } else { 0 }] += 1;
        }
        State { rarity: item.rarity, influences: item.kind.influences.0, eldritch: item.eldritch, counts: counts.into() }
    }

    pub fn view<'a>(&'a self, state: &'a State) -> StateView<'a> {
        StateView { lens: self, state }
    }

    /// Does target `t` (in construction order) hold?
    pub fn holds(&self, t: usize, state: &State) -> bool {
        let (target, offset) = &self.targets[t];
        let v = self.view(state);
        if self.kind.ilvl < target.ilvl
            || state.influences & target.influences.0 != target.influences.0
            || !target.shape(state.rarity, Side::BOTH.map(|s| v.count(s)), Side::BOTH.map(|s| v.open(s)))
        {
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
            if let Step::Fill(pick) = step {
                // One draw at a time, merging in between: at most six affixes.
                for _ in 0..6 {
                    dist = merge(dist, &|s, total, emit| {
                        if (self.view(&s).total() as u8) < total { self.draw(s, total, pick, emit) } else { emit(1.0, s, total) }
                    });
                }
            } else {
                dist = merge(dist, &|s, total, emit| self.step(ctx, step, s, total, emit));
            }
        }
        let mut out: Vec<(f64, State)> = dist.into_iter().map(|(p, s, _)| (p, s)).collect();
        out.sort_by(|a, b| a.1.cmp(&b.1));
        out
    }

    fn step(&self, ctx: &Ctx, step: &Step, mut s: State, total: u8, emit: &mut dyn FnMut(f64, State, u8)) {
        let n = self.classes.len();
        match step {
            Step::Clear { sides, respect_lock } => {
                let v = self.view(&s);
                let allowed = resolve(*sides, s.eldritch);
                let locked = if *respect_lock { v.locked() } else { None };
                for c in 0..n {
                    let side = self.classes[c].side;
                    if allowed[side as usize] && locked != Some(side) {
                        s.counts[c] = 0;
                    }
                }
                emit(1.0, s, total);
            }
            Step::Rarity(r) => {
                s.rarity = *r;
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
                s.counts[self.class_of[m]] += 1;
                emit(1.0, s, total);
            }
            Step::Craft(mods) => {
                if let Some(m) = mods.iter().find(|m| self.view(&s).fits(**m)) {
                    s.counts[self.class_of[m]] += 1;
                }
                emit(1.0, s, total);
            }
            Step::Draw(pick) => self.draw(s, total, pick, emit),
            Step::Fill(_) => unreachable!("run expands Fill"),
            Step::Remove { sides, tag, respect_lock } => {
                let options = self.removable(&s, *sides, *tag, *respect_lock);
                self.remove(s, total, &options, emit);
            }
            Step::Swap { add, remove } => {
                let options = self.options(&s, add);
                let all: f64 = options.iter().map(|o| o.1).sum();
                if all <= 0.0 {
                    return emit(1.0, s, total);
                }
                for (c, w) in options {
                    let mut t = s.clone();
                    let mut others = self.removable(&t, Sides::Both, *remove, true);
                    t.counts[c] += 1;
                    for o in others.iter_mut() {
                        if o.0 == c {
                            o.1 = t.counts[c] - 1;
                        }
                    }
                    let all_others: f64 = others.iter().map(|o| o.1 as f64).sum();
                    if all_others == 0.0 {
                        emit(w / all, t, total);
                        continue;
                    }
                    for &(o, k) in &others {
                        if k > 0 {
                            let mut u = t.clone();
                            u.counts[o] -= 1;
                            emit(w / all * k as f64 / all_others, u, total);
                        }
                    }
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
            Step::Influence(i) => {
                s.influences |= 1 << *i as u8;
                emit(1.0, s, total);
            }
            Step::Eldritch(side) => {
                s.eldritch = *side;
                emit(1.0, s, total);
            }
            Step::Fracture => {
                let options: Vec<usize> =
                    (0..n).filter(|&c| s.counts[c] > 0 && !self.classes[c].crafted && !self.classes[c].veil).collect();
                let all: f64 = options.iter().map(|&c| s.counts[c] as f64).sum();
                if all == 0.0 {
                    return emit(1.0, s, total);
                }
                for c in options {
                    let mut t = s.clone();
                    t.counts[c] -= 1;
                    t.counts[c + n] += 1;
                    emit(s.counts[c] as f64 / all, t, total);
                }
            }
            Step::Veil(sides) => {
                let v = self.view(&s);
                let allowed = resolve(*sides, s.eldritch);
                let options: Vec<usize> = Side::BOTH
                    .into_iter()
                    .filter(|x| allowed[*x as usize] && v.open(*x) > 0 && v.fits(self.game.veils[*x as usize]))
                    .map(|x| self.class_of[&self.game.veils[x as usize]])
                    .collect();
                if options.is_empty() {
                    return emit(1.0, s, total);
                }
                for &c in &options {
                    let mut t = s.clone();
                    t.counts[c] += 1;
                    emit(1.0 / options.len() as f64, t, total);
                }
            }
            Step::Unveil(_) => self.unveil(s, total, emit),
        }
    }

    /// (class, removable count) for a removal.
    fn removable(&self, s: &State, sides: Sides, tag: TagFilter, respect_lock: bool) -> Vec<(usize, u8)> {
        let v = self.view(s);
        let allowed = resolve(sides, s.eldritch);
        let locked = if respect_lock { v.locked() } else { None };
        let bit = tag.map(|(t, want)| (self.tag_bit(t), want));
        (0..self.classes.len())
            .filter(|&c| {
                let class = &self.classes[c];
                s.counts[c] > 0
                    && allowed[class.side as usize]
                    && locked != Some(class.side)
                    && bit.is_none_or(|(b, want)| (class.tags & b != 0) == want)
            })
            .map(|c| (c, s.counts[c]))
            .collect()
    }

    fn remove(&self, s: State, total: u8, options: &[(usize, u8)], emit: &mut dyn FnMut(f64, State, u8)) {
        let all: f64 = options.iter().map(|o| o.1 as f64).sum();
        if all == 0.0 {
            return emit(1.0, s, total);
        }
        for &(c, k) in options {
            let mut t = s.clone();
            t.counts[c] -= 1;
            emit(k as f64 / all, t, total);
        }
    }

    fn tag_bit(&self, t: TagId) -> u32 {
        1 << self.read_tags.iter().position(|x| *x == t).expect("tag read by an operation")
    }

    /// One weighted draw; the state is unchanged when nothing fits.
    fn draw(&self, s: State, total: u8, pick: &Pick, emit: &mut dyn FnMut(f64, State, u8)) {
        let options = self.options(&s, pick);
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

    /// Weight a class still has in a pool given what the item holds: zero when its group is
    /// taken, reduced by the average group share for untracked classes.
    fn available(&self, s: &State, c: usize, w: f64) -> f64 {
        let class = &self.classes[c];
        let n = self.classes.len();
        let present = |o: usize| s.counts[o] + s.counts[o + n];
        if class.tracked {
            if class.conflicts.iter().any(|&o| present(o) > 0) { 0.0 } else { w }
        } else {
            w * (1.0 - present(c) as f64 / class.groups as f64).max(0.0)
        }
    }

    /// Classes a draw can land in, with their available weight.
    fn options(&self, s: &State, pick: &Pick) -> Vec<(usize, f64)> {
        let v = self.view(s);
        let weights = &self.pools.iter().find(|((p, t), _)| *p == pick.pool && *t == pick.tag).expect("pool read by an operation").1;
        let allowed = resolve(pick.sides, s.eldritch);
        let open = Side::BOTH.map(|x| allowed[x as usize] && v.open(x) > 0);
        let banned = self.bans.iter().filter(|(c, _)| v.present(*c)).fold(0u32, |b, (_, bit)| b | bit);
        (0..self.classes.len())
            .filter_map(|c| {
                let class = &self.classes[c];
                if weights[c] <= 0.0 || !open[class.side as usize] || class.tags & banned != 0 {
                    return None;
                }
                if let Origin::Natural(Some(i)) = class.origin
                    && s.influences & 1 << i as u8 == 0
                {
                    return None;
                }
                let w = self.available(s, c, weights[c]);
                (w > 0.0).then_some((c, w))
            })
            .collect()
    }

    /// Replace the veiled mod with the kept offer.
    fn unveil(&self, mut s: State, total: u8, emit: &mut dyn FnMut(f64, State, u8)) {
        let n = self.classes.len();
        let Some(veil) = (0..n).find(|&c| self.classes[c].veil && s.counts[c] > 0) else { return emit(1.0, s, total) };
        let side = self.classes[veil].side;
        s.counts[veil] -= 1;
        let offers: Vec<(usize, f64, f64)> = (0..n)
            .filter(|&c| self.classes[c].side == side && self.unveil[c] > 0.0)
            .map(|c| (c, self.available(&s, c, self.unveil[c]), self.unveil[c] / self.classes[c].veiled as f64))
            .filter(|o| o.1 > 0.0)
            .collect();
        if offers.is_empty() {
            return emit(1.0, s, total);
        }
        // Offers in order, each without the mod before it: (prob, weights left, first, best).
        let mut kept: BTreeMap<usize, f64> = BTreeMap::new();
        fn walk(
            lens: &Lens,
            offers: &[(usize, f64, f64)],
            left: &mut Vec<f64>,
            (p, first, best, k): (f64, Option<usize>, Option<usize>, usize),
            kept: &mut BTreeMap<usize, f64>,
        ) {
            let all: f64 = left.iter().sum();
            if k == UNVEIL_OFFERS || all <= 1e-12 {
                if let Some(f) = first {
                    *kept.entry(best.unwrap_or(f)).or_default() += p;
                }
                return;
            }
            for i in 0..offers.len() {
                if left[i] <= 1e-12 {
                    continue;
                }
                let (c, _, mean) = offers[i];
                let q = left[i] / all;
                let rank = lens.classes[c].rank;
                let better = rank > 0 && best.is_none_or(|b| rank < lens.classes[b].rank);
                let saved = left[i];
                left[i] = (left[i] - mean).max(0.0);
                walk(lens, offers, left, (p * q, first.or(Some(c)), if better { Some(c) } else { best }, k + 1), kept);
                left[i] = saved;
            }
        }
        let mut left: Vec<f64> = offers.iter().map(|o| o.1).collect();
        walk(self, &offers, &mut left, (1.0, None, None, 0), &mut kept);
        for (c, p) in kept {
            let mut t = s.clone();
            t.counts[c] += 1;
            emit(p, t, total);
        }
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
    fn fractured(&self) -> usize {
        let n = self.lens.classes.len();
        self.state.counts[n..].iter().map(|&c| c as usize).sum()
    }
    fn has(&self, m: ModId) -> bool {
        self.lens.class_of.get(&m).is_some_and(|&c| self.present(c))
    }
    fn removable(&self, respect_lock: bool) -> usize {
        self.lens.removable(self.state, Sides::Both, None, respect_lock).iter().map(|o| o.1 as usize).sum()
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
    fn influenced(&self) -> bool {
        self.state.influences != 0
    }
    fn eldritch(&self) -> Option<Side> {
        self.state.eldritch
    }
    fn veiled(&self) -> bool {
        (0..self.lens.classes.len()).any(|c| self.lens.classes[c].veil && self.present(c))
    }
}
