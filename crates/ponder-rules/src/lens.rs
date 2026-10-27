//! Solver states: items reduced to what the goals and operations can tell apart (SPEC §5.3).
//!
//! A lens is built from the targets and operations of one problem. Mods fall into classes:
//! - tracked: mods sharing a group with anything a target asks for, an operation forces or an
//!   unveil prefers, split by which needs they serve and which tags operations read;
//! - fixed: crafted mods and veiled placeholders;
//! - untracked: everything else, counted per side, origin and read tags. A class holding `k`
//!   mods keeps the weight expected after `k` weighted draws without replacement over its groups
//!   (each as likely as it spawns), the one estimate the lens makes.
//!
//! Classes also split by origin (natural, an influence's, fossil-only, unveiled), so every draw
//! pool gives each class one weight. [`Lens::run`] interprets an operation's steps on states
//! exactly, merging equal states after every step. Unveil offers are drawn by class, removing a
//! class's average mod weight per offer (exact for single-mod classes).

use rustc_hash::FxHashMap as HashMap;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use crate::game::{Game, GroupId, Influences, Kind, ModId, Side, TagId};
use crate::goal::{Influence, Rarity, Source};
use crate::item::Item;
use crate::ops::{
    Ctx, Operation, Pick, Pool, Sides, Step, SupplyRef, TagFilter, UNVEIL_OFFERS, View, count_table, pool_weight, resolve, unveil_weight,
};
use crate::target::{Target, matches};

/// Most classes a lens has: states hold their counts inline, so making one allocates nothing.
pub const CLASSES: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct State {
    pub rarity: Rarity,
    pub influences: u8,
    pub eldritch: Option<Side>,
    /// Per class: normal affixes, then (after the lens's classes) fractured ones.
    pub counts: [u8; 2 * CLASSES],
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
    /// Veiled mods (for unveil offers).
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

struct TargetLens {
    target: Target,
    offset: usize,
    classes: Vec<Vec<usize>>,
    distinct: bool,
}

pub struct Lens<'g> {
    game: &'g Game,
    pub kind: Kind,
    class_name: String,
    classes: Vec<Class>,
    class_of: HashMap<ModId, usize>,
    read_tags: Vec<TagId>,
    /// Class weights per draw (pool and tag filter), and for unveil offers: indexed by how many
    /// of the class's mods the item holds (see [`Weights`]).
    pools: Vec<((Pool, TagFilter), Vec<Weights>)>,
    unveil: Vec<Weights>,
    /// Per untracked class, the chance two of its rolled mods share a group (recombination).
    collide: Vec<f64>,
    targets: Vec<TargetLens>,
    /// Class of the prefix lock and suffix lock metamods; of any metamod; of the attack and
    /// caster bans with the tag bit each removes.
    lock: [Option<usize>; 2],
    metamods: Vec<usize>,
    bans: Vec<(usize, u32)>,
    partners: Vec<(SupplyRef, Vec<(f64, State)>)>,
    samples: Mutex<SampleCache>,
    /// What the lens was built from, to build its relaxation.
    source: (Vec<Step>, Vec<Item>),
}

/// How a lens's states look through its relaxation (see [`Lens::relaxed`]).
pub struct Relaxation {
    class: Vec<usize>,
    classes: usize,
}

impl Relaxation {
    pub fn state(&self, s: &State) -> State {
        let n = self.class.len();
        let mut counts = [0u8; 2 * CLASSES];
        for (c, &r) in self.class.iter().enumerate() {
            counts[r] += s.counts[c];
            counts[r + self.classes] += s.counts[c + n];
        }
        State { counts, ..*s }
    }
}

static ANY: Pick = Pick::ANY;

/// (side, tracked groups, needs served, read tags, origin, fixed mod, unveil rank, pools rolling it,
/// relaxed blocker).
type Key = (Side, Vec<GroupId>, u128, u32, Origin, Option<ModId>, u8, u32, bool);
type Samples = Arc<[(f64, Vec<u8>)]>;
type SampleKey = (Vec<(usize, u8)>, Vec<(usize, u64)>);
type SampleCache = HashMap<SampleKey, Samples>;
/// Expands one (state, rolled count) into weighted successors.
type Expand<'a> = dyn Fn(State, u8, &mut dyn FnMut(f64, State, u8)) + 'a;

impl<'g> Lens<'g> {
    pub fn new(game: &'g Game, kind: Kind, targets: Vec<Target>, ops: &[&dyn Operation], inputs: &[&Item]) -> Lens<'g> {
        let steps = ops.iter().flat_map(|op| op.steps()).cloned().collect();
        Lens::build(game, kind, targets, (steps, inputs.iter().map(|i| (*i).clone()).collect()), false)
    }

    /// The same problem coarser: a blocker (a mod sharing a group with a need without serving
    /// it) no longer blocks the need, though a need present still keeps it out, only the needs'
    /// groups are tracked, and junk ignores the pools that roll it and its unveil rank. Its costs are close lower
    /// bounds on this lens's, and its states far fewer. Also how this lens's states look
    /// through it.
    pub fn relaxed(&self) -> (Lens<'g>, Relaxation) {
        let targets = self.targets.iter().map(|t| t.target.clone()).collect();
        let relaxed = Lens::build(self.game, self.kind, targets, self.source.clone(), true);
        let mut class = vec![usize::MAX; self.classes.len()];
        for (m, &c) in &self.class_of {
            class[c] = relaxed.class_of[m];
        }
        let classes = relaxed.classes.len();
        (relaxed, Relaxation { class, classes })
    }

    fn build(game: &'g Game, kind: Kind, targets: Vec<Target>, source: (Vec<Step>, Vec<Item>), relaxed: bool) -> Lens<'g> {
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
        let steps: Vec<&Step> = source.0.iter().collect();
        let mut partners = Vec::new();
        for step in &steps {
            if let Step::Recombine(p) = step
                && !partners.contains(p)
            {
                partners.push(p.clone());
            }
        }
        let inputs: Vec<&Item> = source.1.iter().chain(partners.iter().flat_map(|p| p.0.items())).collect();
        let mut picks: Vec<&Pick> = Vec::new();
        let mut filters: Vec<TagFilter> = Vec::new();
        let mut forced: BTreeSet<ModId> = BTreeSet::new();
        let mut prefs: Vec<&Vec<Vec<ModId>>> = Vec::new();
        let mut influences = kind.influences.0 | targets.iter().fold(0, |b, t| b | t.influences.0);
        influences |= inputs.iter().fold(0, |b, i| b | i.kind.influences.0);
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
                // The bonus mod, like an exalt.
                Step::RecombineBonus => picks.push(&ANY),
                _ => {}
            }
        }
        let kind = Kind { influences: Influences(influences), ..kind };
        let bans: Vec<(ModId, TagId)> = [(2, "attack"), (3, "caster")]
            .into_iter()
            .filter_map(|(i, t)| Some((*game.metamods.get(i)?, game.tag_id(t)?)))
            .filter(|(m, _)| forced.contains(m))
            .collect();
        // Tags read off the mods an item holds (tagged removals, bans) keep those mods apart; a
        // tagged draw only weighs each mod by its own tags.
        let mut read: BTreeSet<TagId> = BTreeSet::new();
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
        forced.extend(inputs.iter().flat_map(|i| &i.affixes).map(|a| a.id).filter(|m| !roll_ids.contains(m)));
        let pref_mods = prefs.iter().flat_map(|p| p.iter().flatten().copied());
        let wanted = targets.iter().flat_map(|t| t.needs.iter().flat_map(|n| n.ok.iter().copied()));
        let tracked_groups: BTreeSet<GroupId> = if relaxed {
            wanted.flat_map(|m| game.def(m).groups.iter().copied()).collect()
        } else {
            wanted.chain(forced.iter().copied()).chain(pref_mods).flat_map(|m| game.def(m).groups.iter().copied()).collect()
        };
        // A blocker in the relaxation is still kept out by a need present but no longer blocks one.
        let blocker = |m: ModId| relaxed && serves(m) == 0 && game.def(m).groups.iter().any(|g| tracked_groups.contains(g));
        let tracked = |m: ModId| -> Vec<GroupId> { game.def(m).groups.iter().copied().filter(|g| tracked_groups.contains(g)).collect() };

        let mut keys: BTreeMap<Key, Vec<ModId>> = BTreeMap::new();
        for &(m, w) in &rolls {
            let def = game.def(m);
            // All tracked groups, not just the first: a hybrid's conflicts are not its parts'.
            let group = tracked(m);
            // Fossil and veiled mods differ from natural ones only in their weights.
            let origin = Origin::Natural(def.influence.filter(|_| def.source == Source::Influence));
            let rank = if def.source == Source::Veiled { rank_of(m) } else { 0 };
            // Junk splits by the pools that can roll it: a mod a pool cannot roll (an earlier
            // fossil's) must not count as one of that pool's draws. Draw tags only weigh it.
            // The relaxation merges junk the pools roll differently and its unveil ranks.
            let junk = group.is_empty();
            let rolled_by = if junk && !relaxed {
                pools.iter().enumerate().filter(|(_, p)| pool_weight(def, m, w, &p.0) > 0.0).fold(0u32, |b, (i, _)| b | 1 << i)
            } else {
                0
            };
            let rank = if relaxed && junk { 0 } else { rank };
            keys.entry((def.side, group, serves(m), tag_bits(m), origin, None, rank, rolled_by, blocker(m))).or_default().push(m);
        }
        for &m in forced.iter().filter(|m| !roll_ids.contains(m)) {
            let def = game.def(m);
            let fixed = (def.source == Source::Crafted || game.veils.contains(&m)).then_some(m);
            let group = tracked(m);
            let origin = fixed.map_or(Origin::Natural(def.influence), |_| Origin::Fixed);
            keys.entry((def.side, group, serves(m), tag_bits(m), origin, fixed, 0, 0, blocker(m))).or_default().push(m);
        }

        assert!(keys.len() <= CLASSES, "at most {CLASSES} classes per lens");
        let mut classes = Vec::new();
        let mut class_of = HashMap::default();
        let mut group_classes: HashMap<GroupId, Vec<usize>> = HashMap::default();
        let mut blockers = Vec::new();
        for ((side, group, serves, tags, origin, fixed, rank, _, blocks), mods) in keys {
            let idx = classes.len();
            for g in group.iter().filter(|_| !blocks) {
                group_classes.entry(*g).or_default().push(idx);
            }
            for &m in &mods {
                class_of.insert(m, idx);
            }
            if blocks {
                blockers.push(idx);
            }
            classes.push(Class {
                side,
                origin,
                veiled: mods.iter().filter(|m| game.def(**m).source == Source::Veiled).count(),
                tracked: (!group.is_empty() && !blocks) || fixed.is_some(),
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
            if c.tracked || blockers.contains(&i) {
                let mut conflicts: BTreeSet<usize> = members[i]
                    .iter()
                    .flat_map(|m| game.def(*m).groups.iter())
                    .filter_map(|g| group_classes.get(g))
                    .flatten()
                    .copied()
                    .collect();
                if c.tracked {
                    conflicts.insert(i);
                }
                c.conflicts = conflicts.into_iter().collect();
            }
        }
        let weight_of: HashMap<ModId, f64> = rolls.iter().copied().collect();
        let in_pool = |m: ModId, w: f64, (p, tag): &(Pool, TagFilter)| {
            if tag.is_none_or(|(t, want)| game.def(m).tags.contains(&t) == want) { pool_weight(game.def(m), m, w, p) } else { 0.0 }
        };
        let class_weights = |f: &dyn Fn(ModId, f64) -> f64| -> Vec<Weights> {
            members
                .iter()
                .map(|ms| {
                    let mut groups: BTreeMap<Option<GroupId>, (f64, f64)> = BTreeMap::new();
                    for m in ms {
                        let (h, w) = weight_of.get(m).map_or((0.0, 0.0), |&w| (w, f(*m, w)));
                        let g = groups.entry(game.def(*m).groups.first().copied()).or_default();
                        (g.0, g.1) = (g.0 + h, g.1 + w);
                    }
                    remaining(&groups.into_values().filter(|g| g.0 > 0.0 || g.1 > 0.0).collect::<Vec<_>>())
                })
                .collect()
        };
        let pools: Vec<_> = pools.iter().map(|p| (p.clone(), class_weights(&|m, w| in_pool(m, w, p)))).collect();
        let unveil = class_weights(&|m, w| unveil_weight(game, game.def(m), m, w));
        // Two rolled mods of an untracked class share a group with chance sum(w_g^2) / (sum w)^2.
        let collide = members
            .iter()
            .zip(&classes)
            .map(|(ms, c)| {
                let mut groups: BTreeMap<Option<GroupId>, f64> = BTreeMap::new();
                for m in ms {
                    *groups.entry(game.def(*m).groups.first().copied()).or_default() += weight_of.get(m).copied().unwrap_or(0.0);
                }
                let all: f64 = groups.values().sum();
                if c.tracked || all <= 0.0 { 0.0 } else { groups.values().map(|w| w * w).sum::<f64>() / (all * all) }
            })
            .collect();
        let class_for = |key: Option<&ModId>| key.and_then(|m| class_of.get(m).copied());
        let lock = [class_for(game.metamods.first()), class_for(game.metamods.get(1))];
        let metamods = game.metamods.iter().filter_map(|m| class_of.get(m).copied()).collect();
        let bans = bans.iter().filter_map(|(m, t)| Some((*class_of.get(m)?, 1u32 << read_tags.iter().position(|x| x == t)?))).collect();
        let targets = targets
            .into_iter()
            .zip(offsets)
            .map(|(target, offset)| {
                let mut used = BTreeSet::new();
                let mut distinct = true;
                let relevant = (0..target.needs.len())
                    .map(|i| {
                        (0..classes.len())
                            .filter(|&c| classes[c].serves & (1 << (offset + i)) != 0)
                            .inspect(|&c| {
                                distinct &= used.insert(c);
                            })
                            .collect()
                    })
                    .collect();
                TargetLens { target, offset, classes: relevant, distinct }
            })
            .collect();
        let class_name = game.bases[kind.base.0 as usize].class.clone();
        let mut lens = Lens {
            game,
            kind,
            class_name,
            classes,
            class_of,
            read_tags,
            pools,
            unveil,
            collide,
            targets,
            lock,
            metamods,
            bans,
            partners: Vec::new(),
            samples: Mutex::default(),
            source: (Vec::new(), Vec::new()),
        };
        lens.partners = partners
            .into_iter()
            .map(|p| {
                let states = lens.project_items(p.0.items());
                (p, states)
            })
            .collect();
        lens.source = source;
        lens
    }

    pub fn is_tracked(&self, c: usize) -> bool {
        self.classes[c].tracked
    }

    pub fn classes(&self) -> usize {
        self.classes.len()
    }

    pub fn clear_cache(&mut self) {
        *self.samples.get_mut().unwrap() = HashMap::default();
    }

    /// One class for diagnostics: its fields and a few of its mods.
    pub fn describe(&self, c: usize) -> String {
        let mods: Vec<&str> = self.class_of.iter().filter(|(_, k)| **k == c).map(|(m, _)| self.game.def(*m).key.as_str()).take(3).collect();
        format!("{:?} {mods:?}", self.classes[c])
    }

    /// The state of a real item of this lens's base.
    pub fn project(&self, item: &Item) -> State {
        let n = self.classes.len();
        let mut counts = [0u8; 2 * CLASSES];
        for a in &item.affixes {
            let c = *self.class_of.get(&a.id).unwrap_or_else(|| panic!("{} is outside the lens", self.game.def(a.id).key));
            counts[c + if a.fractured { n } else { 0 }] += 1;
        }
        State { rarity: item.rarity, influences: item.kind.influences.0, eldritch: item.eldritch, counts }
    }

    pub fn project_items(&self, items: &[Item]) -> Vec<(f64, State)> {
        let mut counts = BTreeMap::<State, usize>::new();
        for item in items {
            *counts.entry(self.project(item)).or_default() += 1;
        }
        counts.into_iter().map(|(s, n)| (n as f64 / items.len() as f64, s)).collect()
    }

    /// Whether two step lists do the same to every state: equal but for forcing different
    /// mods of one class (essences of different tiers that serve the same needs).
    pub fn same_effect(&self, a: &[Step], b: &[Step]) -> bool {
        // Identical steps can still differ in where they apply (augmentation and exalted both
        // add a mod), so only different forced mods of one class count.
        a != b
            && a.len() == b.len()
            && a.iter().zip(b).all(|(x, y)| match (x, y) {
                (Step::Force(m), Step::Force(n)) => self.class_of.get(m) == self.class_of.get(n),
                _ => x == y,
            })
    }

    pub fn view<'a>(&'a self, state: &'a State) -> StateView<'a> {
        let n = self.classes.len();
        let (mut counts, mut crafted, mut fractured) = ([0; 2], 0, 0);
        for (c, class) in self.classes.iter().enumerate() {
            counts[class.side as usize] += (state.counts[c] + state.counts[c + n]) as usize;
            crafted += if class.crafted { state.counts[c] as usize } else { 0 };
            fractured += state.counts[c + n] as usize;
        }
        StateView { lens: self, state, counts, crafted, fractured }
    }

    /// Does the item hold nothing but what some need asks for?
    pub fn clean(&self, state: &State) -> bool {
        let n = self.classes.len();
        (0..n).all(|c| self.classes[c].serves != 0 || state.counts[c] + state.counts[c + n] == 0)
    }

    /// Does target `t` (in construction order) hold?
    pub fn holds(&self, t: usize, state: &State) -> bool {
        let predicate = &self.targets[t];
        let TargetLens { target, offset, .. } = predicate;
        let v = self.view(state);
        if self.kind.ilvl < target.ilvl
            || state.influences & target.influences.0 != target.influences.0
            || !target.shape(state.rarity, Side::BOTH.map(|s| v.count(s)), Side::BOTH.map(|s| v.open(s)))
        {
            return false;
        }
        let n = self.classes.len();
        if target.needs.iter().zip(&predicate.classes).any(|(need, classes)| {
            classes.iter().map(|&c| state.counts[c + n] as usize + if need.fractured { 0 } else { state.counts[c] as usize }).sum::<usize>()
                < need.count as usize
        }) {
            return false;
        }
        if predicate.distinct {
            return true;
        }
        let affixes: Vec<(usize, bool)> = (0..2 * n).flat_map(|i| std::iter::repeat_n((i % n, i >= n), state.counts[i] as usize)).collect();
        matches(&target.needs, affixes.len(), &|need, i| {
            let (class, fractured) = affixes[i];
            self.classes[class].serves >> (offset + need) & 1 == 1 && (!target.needs[need].fractured || fractured)
        })
    }

    pub fn viable(&self, t: usize, state: &State) -> bool {
        if !self.partners.is_empty() {
            return true;
        }
        let n = self.classes.len();
        let fixed: Vec<_> = (0..n).filter(|&c| state.counts[c + n] > 0).collect();
        if fixed.is_empty() {
            return true;
        }
        let TargetLens { target, offset, .. } = &self.targets[t];
        target.needs.iter().enumerate().all(|(i, need)| {
            let serves = |c: usize| self.classes[c].serves & (1 << (offset + i)) != 0;
            let have: usize = fixed.iter().filter(|&&c| serves(c)).map(|&c| state.counts[c + n] as usize).sum();
            have >= need.count as usize
                || (!need.fractured && (0..n).any(|c| serves(c) && !fixed.iter().any(|f| self.classes[c].conflicts.contains(f))))
        })
    }

    /// The exact outcome distribution of `steps` on `state`.
    pub fn run(&self, ctx: &Ctx, steps: &[Step], state: &State) -> Vec<(f64, State)> {
        let mut dist: Vec<(f64, State, u8)> = vec![(1.0, *state, 0)];
        let merge = |dist: Vec<(f64, State, u8)>, f: &Expand| {
            let mut next: HashMap<(State, u8), f64> = HashMap::default();
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
        out.sort_by_key(|a| a.1);
        out.dedup_by(|a, b| {
            if a.1 != b.1 {
                return false;
            }
            b.0 += a.0;
            true
        });
        out
    }

    fn step(&self, ctx: &Ctx, step: &Step, mut s: State, total: u8, emit: &mut dyn FnMut(f64, State, u8)) {
        let n = self.classes.len();
        match step {
            Step::Clear { sides, respect_lock } => {
                let v = self.view(&s);
                let allowed = resolve(*sides, s.eldritch);
                let locked = if *respect_lock { v.locked() } else { [false; 2] };
                for c in 0..n {
                    let side = self.classes[c].side;
                    if allowed[side as usize] && !locked[side as usize] {
                        s.counts[c] = 0;
                    }
                }
                emit(1.0, s, total);
            }
            Step::Rarity(r) => {
                s.rarity = *r;
                emit(1.0, s, total);
            }
            Step::Settle => emit(1.0, self.settled(s), total),
            Step::RollCount => {
                let table = count_table(ctx, &self.class_name, s.rarity);
                let sum: f64 = table.iter().map(|e| e.1).sum();
                for &(count, w) in table {
                    emit(w / sum, s, count);
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
                    let mut t = s;
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
                            let mut u = t;
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
                let options: Vec<usize> = (0..n).filter(|&c| s.counts[c] > 0 && !self.classes[c].veil).collect();
                let all: f64 = options.iter().map(|&c| s.counts[c] as f64).sum();
                if all == 0.0 {
                    return emit(1.0, s, total);
                }
                for c in options {
                    let mut t = s;
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
                    let mut t = s;
                    t.counts[c] += 1;
                    emit(1.0 / options.len() as f64, t, total);
                }
            }
            Step::Unveil(_) => self.unveil(s, total, emit),
            Step::Recombine(p) => {
                let partner = &self.partners.iter().find(|(supply, _)| supply == p).expect("partner read by an operation").1;
                self.recombine(ctx, &s, total, partner, emit);
            }
            Step::RecombineBonus => {
                let rare = State { rarity: Rarity::Rare, ..s };
                let limit = self.view(&rare).limit();
                let v = self.view(&s);
                let (prefixes, suffixes) = (v.count(Side::Prefix), v.count(Side::Suffix));
                if prefixes + suffixes >= 2 * limit {
                    return emit(1.0, s, total);
                }
                let bonus = ctx.assumptions.recombine_bonus;
                emit(1.0 - bonus, s, total);
                let sides = crate::ops::bonus_sides(s.rarity, prefixes, suffixes, limit);
                self.draw(rare, total, &Pick { sides, ..Pick::ANY }, &mut |p, t, total| emit(bonus * p, self.settled(t), total));
            }
        }
    }

    /// (class, removable count) for a removal.
    fn removable(&self, s: &State, sides: Sides, tag: TagFilter, respect_lock: bool) -> Vec<(usize, u8)> {
        let v = self.view(s);
        let allowed = resolve(sides, s.eldritch);
        let locked = if respect_lock { v.locked() } else { [false; 2] };
        let bit = tag.map(|(t, want)| (self.tag_bit(t), want));
        (0..self.classes.len())
            .filter(|&c| {
                let class = &self.classes[c];
                s.counts[c] > 0
                    && allowed[class.side as usize]
                    && !locked[class.side as usize]
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
            let mut t = s;
            t.counts[c] -= 1;
            emit(k as f64 / all, t, total);
        }
    }

    fn tag_bit(&self, t: TagId) -> u32 {
        1 << self.read_tags.iter().position(|x| *x == t).expect("tag read by an operation")
    }

    /// One weighted draw; the state is unchanged when nothing fits.
    /// Rarity from what is left: no mods normal, at most one per side magic, else rare.
    fn settled(&self, mut s: State) -> State {
        let v = self.view(&s);
        let most = Side::BOTH.iter().map(|x| v.count(*x)).max().unwrap_or(0);
        s.rarity = match most {
            0 => Rarity::Normal,
            1 => Rarity::Magic,
            _ => Rarity::Rare,
        };
        s
    }

    /// A recombination with each partner state: per heir (whose influences and Eldritch
    /// implicits the result takes), each side keeps a uniform sample of the pooled affixes,
    /// dropping any that conflicts with one kept.
    fn recombine(&self, ctx: &Ctx, s: &State, total: u8, partner: &[(f64, State)], emit: &mut dyn FnMut(f64, State, u8)) {
        let n = self.classes.len();
        let rare = State { rarity: Rarity::Rare, ..*s };
        let limit = self.view(&rare).limit();
        for (q, o) in partner {
            let heirs = if (s.influences, s.eldritch) == (o.influences, o.eldritch) { vec![s] } else { vec![s, o] };
            for heir in &heirs {
                let q = q / heirs.len() as f64;
                let allowed = |c: usize| match self.classes[c % n].origin {
                    Origin::Natural(Some(i)) => heir.influences & 1 << i as u8 != 0,
                    _ => true,
                };
                let sides = Side::BOTH.map(|side| {
                    let pool: Vec<(usize, u8)> = (0..2 * n)
                        .filter(|&c| self.classes[c % n].side == side && allowed(c))
                        .map(|c| (c, s.counts[c] + o.counts[c]))
                        .filter(|e| e.1 > 0)
                        .collect();
                    let kept = self.kept(ctx, &pool, limit);
                    (pool, kept)
                });
                for (p, prefix) in sides[0].1.iter() {
                    for (r, suffix) in sides[1].1.iter() {
                        let mut counts = [0; 2 * CLASSES];
                        for ((pool, _), chosen) in sides.iter().zip([prefix, suffix]) {
                            for (&(c, _), &count) in pool.iter().zip(chosen) {
                                counts[c] = count;
                            }
                        }
                        let t = self.settled(State { rarity: heir.rarity, influences: heir.influences, eldritch: heir.eldritch, counts });
                        emit(q * p * r, t, total);
                    }
                }
            }
        }
    }

    fn kept(&self, ctx: &Ctx, pool: &[(usize, u8)], limit: usize) -> Samples {
        let size = pool.iter().map(|e| e.1 as usize).sum();
        let table = crate::ops::recombine_kept(ctx, size);
        let key = (pool.to_vec(), table.iter().map(|&(k, w)| ((k as usize).min(limit), w.to_bits())).collect());
        if let Some(out) = self.samples.lock().unwrap().get(&key) {
            return out.clone();
        }
        let all: f64 = table.iter().map(|e| e.1).sum();
        let mut mixed = HashMap::<Vec<u8>, f64>::default();
        for &(k, w) in table {
            for (p, chosen) in self.sample(pool, (k as usize).min(limit)) {
                *mixed.entry(chosen).or_default() += w / all * p;
            }
        }
        let mut out: Vec<_> = mixed.into_iter().map(|(s, p)| (p, s)).collect();
        out.sort_by(|a, b| a.1.cmp(&b.1));
        let out: Samples = out.into();
        self.samples.lock().unwrap().insert(key, out.clone());
        out
    }

    /// Distinct affixes drawn uniformly from `pool` ((class, count) entries) until `keep` are
    /// kept or none are left; one conflicting with a kept affix is discarded (for untracked
    /// classes, sharing a group with each kept one of its class by chance). Per outcome, how
    /// many of each entry were kept.
    fn sample(&self, pool: &[(usize, u8)], keep: usize) -> Vec<(f64, Vec<u8>)> {
        let n = self.classes.len();
        let conflicts = |a: usize, b: usize| {
            let (a, b) = (a % n, b % n);
            self.classes[a].conflicts.contains(&b) || (a == b && self.classes[a].origin == Origin::Fixed)
        };
        let mut open: Vec<(f64, Vec<u8>, Vec<u8>)> = vec![(1.0, pool.iter().map(|e| e.1).collect(), vec![0; pool.len()])];
        let mut done: HashMap<Vec<u8>, f64> = HashMap::default();
        while !open.is_empty() {
            let mut next: HashMap<(Vec<u8>, Vec<u8>), f64> = HashMap::default();
            for (p, left, kept) in open {
                let remaining: u32 = left.iter().map(|&x| x as u32).sum();
                if remaining == 0 || kept.iter().map(|&x| x as usize).sum::<usize>() >= keep {
                    *done.entry(kept).or_default() += p;
                    continue;
                }
                for e in 0..pool.len() {
                    if left[e] == 0 {
                        continue;
                    }
                    let mut left = left.clone();
                    let r = p * left[e] as f64 / remaining as f64;
                    left[e] -= 1;
                    if (0..pool.len()).any(|f| kept[f] > 0 && conflicts(pool[e].0, pool[f].0)) {
                        *next.entry((left, kept.clone())).or_default() += r;
                        continue;
                    }
                    let c = pool[e].0 % n;
                    let same: i32 = (0..pool.len()).filter(|&f| pool[f].0 % n == c).map(|f| kept[f] as i32).sum();
                    let clash = 1.0 - (1.0 - self.collide[c]).powi(same);
                    let mut grown = kept.clone();
                    grown[e] += 1;
                    *next.entry((left.clone(), grown)).or_default() += r * (1.0 - clash);
                    if clash > 0.0 {
                        *next.entry((left, kept.clone())).or_default() += r * clash;
                    }
                }
            }
            open = next.into_iter().map(|((l, k), p)| (p, l, k)).collect();
        }
        let mut out: Vec<_> = done.into_iter().map(|(k, p)| (p, k)).collect();
        out.sort_by(|a, b| a.1.cmp(&b.1));
        out
    }

    fn draw(&self, s: State, total: u8, pick: &Pick, emit: &mut dyn FnMut(f64, State, u8)) {
        let options = self.options(&s, pick);
        let all: f64 = options.iter().map(|o| o.1).sum();
        if all <= 0.0 {
            return emit(1.0, s, total);
        }
        for (c, w) in options {
            let mut t = s;
            t.counts[c] += 1;
            emit(w / all, t, total);
        }
    }

    /// Weight a class still has in a pool given what the item holds: zero when its group is
    /// taken; for untracked classes, what is expected to remain after the mods it holds.
    fn available(&self, s: &State, c: usize, w: &Weights) -> f64 {
        let class = &self.classes[c];
        let n = self.classes.len();
        let present = |o: usize| s.counts[o] + s.counts[o + n];
        if class.conflicts.iter().any(|&o| present(o) > 0) {
            0.0
        } else if class.tracked {
            w[0]
        } else {
            w[(present(c) as usize).min(w.len() - 1)]
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
                if weights[c][0] <= 0.0 || !open[class.side as usize] || class.tags & banned != 0 {
                    return None;
                }
                if let Origin::Natural(Some(i)) = class.origin
                    && s.influences & 1 << i as u8 == 0
                {
                    return None;
                }
                let w = self.available(s, c, &weights[c]);
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
            .filter(|&c| self.classes[c].side == side && self.unveil[c][0] > 0.0)
            .map(|c| (c, self.available(&s, c, &self.unveil[c]), self.unveil[c][0] / self.classes[c].veiled as f64))
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
            let mut t = s;
            t.counts[c] += 1;
            emit(p, t, total);
        }
    }
}

/// A solver state seen through the [`View`] operations ask.
pub struct StateView<'a> {
    lens: &'a Lens<'a>,
    state: &'a State,
    counts: [usize; 2],
    crafted: usize,
    fractured: usize,
}

impl StateView<'_> {
    fn total(&self) -> usize {
        self.counts.iter().sum()
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
        self.counts[side as usize]
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
        self.crafted
    }
    fn fractured(&self) -> usize {
        self.fractured
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
    fn locked(&self) -> [bool; 2] {
        self.lens.lock.map(|c| c.is_some_and(|c| self.present(c)))
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

/// A class's weight in one pool while the item holds 0, 1, 2 or 3 of its mods.
type Weights = [f64; 4];

/// Per group (spawn weight, weight in a pool): expected pool weight left once the item holds `k`
/// of the class's mods, drawn without replacement by spawn weight, for `k` = 0..=3. A tagged draw
/// keeps its weight when the mods held are mostly untagged.
fn remaining(groups: &[(f64, f64)]) -> Weights {
    fn go(groups: &[(f64, f64)], taken: &mut Vec<usize>, k: usize) -> f64 {
        let left =
            |f: fn(&(f64, f64)) -> f64| groups.iter().enumerate().filter(|(i, _)| !taken.contains(i)).map(|(_, g)| f(g)).sum::<f64>();
        let (held, weight) = (left(|g| g.0), left(|g| g.1));
        if k == 0 || held <= 0.0 {
            return weight;
        }
        let mut sum = 0.0;
        for i in 0..groups.len() {
            if !taken.contains(&i) {
                taken.push(i);
                sum += groups[i].0 / held * go(groups, taken, k - 1);
                taken.pop();
            }
        }
        sum
    }
    std::array::from_fn(|k| go(groups, &mut Vec::new(), k))
}
