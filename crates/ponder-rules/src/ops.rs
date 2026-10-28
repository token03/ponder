//! The operation contract (SPEC §6).
//!
//! A mechanic is a type implementing [`Operation`]: when it applies, what it costs, and the
//! primitive [`Step`]s it performs. Steps are data, so the same mechanic is sampled on real
//! items ([`run`]) and enumerated exactly on solver states (`lens`) without a second copy of
//! its logic. All randomness is in `Draw`, `Fill`, `Remove`, `Swap`, `RollCount`, `Fracture`,
//! `Veil` and `Unveil` (SPEC §4.3–4.4).

pub mod bench;
pub mod currency;
pub mod eldritch;
pub mod essence;
pub mod fossil;
pub mod harvest;
pub mod influence;
pub mod misc;
pub mod recombinator;
pub mod veiled;

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::assumptions::Assumptions;
use crate::game::{Game, ModDef, ModId, Side, TagId};
use crate::goal::{Influence, Rarity, Source};
use crate::item::{Affix, Item};
use crate::rng::Rng;

pub struct Ctx<'a> {
    pub game: &'a Game,
    pub assumptions: &'a Assumptions,
}

/// The sides a step may touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sides {
    Both,
    Only(Side),
    /// The side the item's dominant Eldritch implicit selects; none without one.
    Eldritch,
}

/// A tag a mod must carry (`true`) or must not carry (`false`).
pub type TagFilter = Option<(TagId, bool)>;

/// Where a draw takes its mods from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Pool {
    /// Natural mods and those of the item's influences, reweighted by fossils if any.
    Natural(Option<Arc<Bias>>),
    /// Only the mods of one influence.
    Influence(Influence),
}

/// What one random addition may pick.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pick {
    pub sides: Sides,
    pub tag: TagFilter,
    pub pool: Pool,
}

impl Pick {
    pub const ANY: Pick = Pick { sides: Sides::Both, tag: None, pool: Pool::Natural(None) };
}

/// Fossil weight changes: per tag, the summed "more" and multiplied "less" factors of every
/// fossil in the set, and the fossil-only mods it adds.
#[derive(Debug)]
pub struct Bias {
    pub name: String,
    pub tags: Vec<(TagId, f64, f64)>,
    pub added: Vec<ModId>,
}

impl Bias {
    /// A mod's weight multiplier: the more factors of its tags add, the less factors multiply.
    pub fn factor(&self, def: &ModDef) -> f64 {
        let (mut more, mut less) = (0.0, 1.0);
        for (t, m, l) in &self.tags {
            if def.tags.contains(t) {
                more += m;
                less *= l;
            }
        }
        f64::max(more, 1.0) * less
    }
}

impl PartialEq for Bias {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}
impl Eq for Bias {}
impl Hash for Bias {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.name.hash(h)
    }
}

/// One primitive. Fractured mods survive every step (SPEC §4.5); a "Cannot Be Changed"
/// metamod protects its side when a step respects locks (SPEC §4.7).
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Remove the mods on these sides, except fractured ones and, when `respect_lock`, the
    /// locked side.
    Clear {
        sides: Sides,
        respect_lock: bool,
    },
    Rarity(Rarity),
    /// Rarity from what is left: no mods normal, at most one per side magic, else rare.
    Settle,
    /// Choose the total mod count of the item's rarity (SPEC §10).
    RollCount,
    /// Add this mod (forced by an essence).
    Force(ModId),
    /// Add the first of these crafted mods that fits (the bench).
    Craft(Arc<Vec<ModId>>),
    /// Add one weighted mod.
    Draw(Pick),
    /// Draw until the item has the rolled count, or nothing fits.
    Fill(Pick),
    /// Remove one random mod.
    Remove {
        sides: Sides,
        tag: TagFilter,
        respect_lock: bool,
    },
    /// Add a mod, then remove another random one (Harvest "add X, remove another").
    Swap {
        add: Pick,
        remove: TagFilter,
    },
    RemoveCrafted,
    /// Give the item an influence.
    Influence(Influence),
    /// Set the side the dominant Eldritch implicit selects.
    Eldritch(Option<Side>),
    /// Fracture one random mod, crafted ones included (not veiled).
    Fracture,
    /// Add a veiled mod on a random side with room.
    Veil(Sides),
    /// Replace the veiled mod: three offers from its side's unveil pool, keeping the most
    /// preferred one (earlier lists first), else the first offered.
    Unveil(Arc<Vec<Vec<ModId>>>),
    /// Combine with a second item (SPEC §4.9): base, influences and Eldritch implicits come
    /// from either, and each side keeps some of both items' affixes there.
    Recombine(SupplyRef),
    RecombineBonus,
}

pub trait Supply: Send + Sync {
    fn name(&self) -> String;
    /// Chaos for one.
    fn price(&self) -> f64;
    fn items(&self) -> &[Item];
    fn item(&self, rng: &mut Rng) -> Item;
}

/// A shared supply, equal only to itself.
#[derive(Clone)]
pub struct SupplyRef(pub Arc<dyn Supply>);

impl PartialEq for SupplyRef {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.0), Arc::as_ptr(&other.0))
    }
}

impl std::fmt::Debug for SupplyRef {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Supply({})", self.0.name())
    }
}

/// How many offers an unveil shows.
pub const UNVEIL_OFFERS: usize = 3;

/// What an operation's `applies` may ask, answered by real items and solver states alike.
pub trait View {
    fn rarity(&self) -> Rarity;
    fn count(&self, side: Side) -> usize;
    fn limit(&self) -> usize;
    fn crafted(&self) -> usize;
    fn fractured(&self) -> usize;
    fn has(&self, m: ModId) -> bool;
    /// Mods a `Remove` could take.
    fn removable(&self, respect_lock: bool) -> usize;
    /// Could `m` be added now: its side has room and no mod shares its group?
    fn fits(&self, m: ModId) -> bool;
    /// Does a mod no reset removes (fractured) share a group with `m`?
    fn fixed_blocks(&self, m: ModId) -> bool;
    /// The sides "Cannot Be Changed" metamods protect.
    fn locked(&self) -> [bool; 2];
    /// Any bench metamod on the item.
    fn has_metamod(&self) -> bool;
    fn influenced(&self) -> bool;
    fn eldritch(&self) -> Option<Side>;
    fn veiled(&self) -> bool;

    fn open(&self, side: Side) -> usize {
        self.limit().saturating_sub(self.count(side))
    }
    fn mods(&self) -> usize {
        self.count(Side::Prefix) + self.count(Side::Suffix)
    }
    /// Can `sides` take another mod?
    fn room(&self, sides: Sides) -> bool {
        resolve(sides, self.eldritch()).iter().zip(Side::BOTH).any(|(ok, s)| *ok && self.open(s) > 0)
    }
}

/// Which of [prefix, suffix] `sides` allows.
pub fn resolve(sides: Sides, eldritch: Option<Side>) -> [bool; 2] {
    match sides {
        Sides::Both => [true, true],
        Sides::Only(s) => [s == Side::Prefix, s == Side::Suffix],
        Sides::Eldritch => match eldritch {
            Some(s) => [s == Side::Prefix, s == Side::Suffix],
            None => [false, false],
        },
    }
}

/// Does a mod pass a tag filter?
pub fn tag_ok(def: &ModDef, tag: TagFilter) -> bool {
    tag.is_none_or(|(t, want)| def.tags.contains(&t) == want)
}

/// A mod's weight in a draw from `pool` before item-dependent checks (side room, groups,
/// item level, the item's influences): its spawn weight `w`, or 0 when the pool excludes it.
pub fn pool_weight(def: &ModDef, m: ModId, w: f64, pool: &Pool) -> f64 {
    match pool {
        Pool::Natural(bias) => {
            let added = bias.as_ref().is_some_and(|b| b.added.contains(&m));
            let ok = matches!(def.source, Source::Natural | Source::Influence) || (def.source == Source::Fossil && added);
            if !ok {
                return 0.0;
            }
            bias.as_ref().map_or(w, |b| w * b.factor(def))
        }
        Pool::Influence(i) => {
            if def.source == Source::Influence && def.influence == Some(*i) {
                w
            } else {
                0.0
            }
        }
    }
}

/// Unveiled mods (not placeholders) that can be offered on this side.
pub fn unveil_weight(game: &Game, def: &ModDef, m: ModId, w: f64) -> f64 {
    if def.source == Source::Veiled && !game.veils.contains(&m) { w } else { 0.0 }
}

/// Price ids and amounts, plus clicking time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cost {
    pub items: Vec<(String, f64)>,
    pub seconds: f64,
}

impl Cost {
    pub fn of(id: &str, amount: f64, seconds: f64) -> Cost {
        Cost { items: vec![(id.into(), amount)], seconds }
    }
}

pub trait Operation: Send + Sync {
    fn name(&self) -> String;
    fn applies(&self, ctx: &Ctx, item: &dyn View) -> bool;
    fn steps(&self) -> &[Step];
    fn cost(&self) -> Cost;
}

/// Apply `steps` to a real item, drawing from `rng`.
pub fn run(ctx: &Ctx, steps: &[Step], item: &mut Item, rng: &mut Rng) {
    let game = ctx.game;
    let mut total = 0;
    for step in steps {
        match step {
            Step::Clear { sides, respect_lock } => {
                let allowed = resolve(*sides, item.eldritch);
                let locked = if *respect_lock { item.locked(game) } else { [false; 2] };
                item.affixes.retain(|a| {
                    let side = game.def(a.id).side;
                    a.fractured || !allowed[side as usize] || locked[side as usize]
                });
            }
            Step::Rarity(r) => item.rarity = *r,
            Step::Settle => {
                let most = Side::BOTH.iter().map(|s| item.count(game, *s)).max().unwrap_or(0);
                item.rarity = match most {
                    0 => Rarity::Normal,
                    1 => Rarity::Magic,
                    _ => Rarity::Rare,
                };
            }
            Step::RollCount => total = roll_count(ctx, item, rng),
            Step::Force(m) => item.affixes.push(Affix { id: *m, crafted: game.def(*m).source == Source::Crafted, fractured: false }),
            Step::Craft(mods) => {
                if let Some(&m) = mods.iter().find(|m| (game, &*item).fits(**m)) {
                    item.affixes.push(Affix { id: m, crafted: true, fractured: false });
                }
            }
            Step::Draw(pick) => {
                draw(ctx, item, rng, pick);
            }
            Step::Fill(pick) => while item.affixes.len() < total && draw(ctx, item, rng, pick) {},
            Step::Remove { sides, tag, respect_lock } => {
                let options = removable(game, item, *sides, *tag, *respect_lock, None);
                if !options.is_empty() {
                    item.affixes.remove(options[rng.below(options.len())]);
                }
            }
            Step::Swap { add, remove } => {
                if draw(ctx, item, rng, add) {
                    let new = item.affixes.len() - 1;
                    let options = removable(game, item, Sides::Both, *remove, true, Some(new));
                    if !options.is_empty() {
                        item.affixes.remove(options[rng.below(options.len())]);
                    }
                }
            }
            Step::RemoveCrafted => item.affixes.retain(|a| !a.crafted),
            Step::Influence(i) => item.kind.influences.0 |= 1 << *i as u8,
            Step::Eldritch(side) => item.eldritch = *side,
            Step::Fracture => {
                let options: Vec<usize> = (0..item.affixes.len())
                    .filter(|&i| {
                        let a = item.affixes[i];
                        !a.fractured && !game.veils.contains(&a.id)
                    })
                    .collect();
                if !options.is_empty() {
                    // A fractured bench craft is no longer removable.
                    let a = &mut item.affixes[options[rng.below(options.len())]];
                    (a.fractured, a.crafted) = (true, false);
                }
            }
            Step::Veil(sides) => {
                let allowed = resolve(*sides, item.eldritch);
                let options: Vec<Side> = Side::BOTH
                    .into_iter()
                    .filter(|s| allowed[*s as usize] && item.open(game, *s) > 0 && !item.blocks(game, game.veils[*s as usize]))
                    .collect();
                if !options.is_empty() {
                    let side = options[rng.below(options.len())];
                    item.affixes.push(Affix { id: game.veils[side as usize], crafted: false, fractured: false });
                }
            }
            Step::Unveil(prefs) => unveil(ctx, item, rng, prefs),
            Step::Recombine(partner) => recombine(ctx, item, partner.0.item(rng), rng),
            Step::RecombineBonus => {
                let limit = Item { rarity: Rarity::Rare, ..item.clone() }.limit(game);
                if item.affixes.len() < 2 * limit && rng.unit() < ctx.assumptions.recombine_bonus {
                    let sides = bonus_sides(item.rarity, item.count(game, Side::Prefix), item.count(game, Side::Suffix), limit);
                    item.rarity = Rarity::Rare;
                    draw(ctx, item, rng, &Pick { sides, ..Pick::ANY });
                    run(ctx, &[Step::Settle], item, rng);
                }
            }
        }
    }
}

/// Indices of mods a removal may take.
fn removable(game: &Game, item: &Item, sides: Sides, tag: TagFilter, respect_lock: bool, except: Option<usize>) -> Vec<usize> {
    let allowed = resolve(sides, item.eldritch);
    let locked = if respect_lock { item.locked(game) } else { [false; 2] };
    (0..item.affixes.len())
        .filter(|&i| {
            let a = item.affixes[i];
            let def = game.def(a.id);
            Some(i) != except && !a.fractured && allowed[def.side as usize] && !locked[def.side as usize] && tag_ok(def, tag)
        })
        .collect()
}

fn unveil(ctx: &Ctx, item: &mut Item, rng: &mut Rng, prefs: &[Vec<ModId>]) {
    let game = ctx.game;
    let Some(at) = item.affixes.iter().position(|a| game.veils.contains(&a.id)) else { return };
    let side = game.def(item.affixes[at].id).side;
    item.affixes.remove(at);
    let mut pool: Vec<(ModId, f64)> = game
        .pool(item.kind)
        .mods
        .iter()
        .filter(|(m, _)| {
            let def = game.def(*m);
            def.side == side && def.level <= item.kind.ilvl as u32 && !item.blocks(game, *m)
        })
        .map(|&(m, w)| (m, unveil_weight(game, game.def(m), m, w)))
        .filter(|p| p.1 > 0.0)
        .collect();
    let mut offers = Vec::new();
    for _ in 0..UNVEIL_OFFERS {
        let Some(i) = rng.pick(pool.iter().map(|p| p.1)) else { break };
        offers.push(pool.remove(i).0);
    }
    let rank = |m: ModId| prefs.iter().position(|p| p.contains(&m)).unwrap_or(usize::MAX);
    if let Some(&best) = offers.iter().min_by_key(|m| rank(**m)) {
        let chosen = if rank(best) == usize::MAX { offers[0] } else { best };
        item.affixes.push(Affix { id: chosen, crafted: false, fractured: false });
    }
}

fn recombine(ctx: &Ctx, item: &mut Item, other: Item, rng: &mut Rng) {
    let game = ctx.game;
    let (a, b) = (item.kind.ilvl as u16, other.kind.ilvl as u16);
    let ilvl = ((a + b) / 2 + 2).min(a.max(b)) as u8;
    let other_heir = rng.below(2) == 1;
    let pool: Vec<Affix> = item
        .affixes
        .drain(..)
        .filter(|a| !a.fractured || !other_heir)
        .chain(other.affixes.iter().copied().filter(|a| !a.fractured || other_heir))
        .collect();
    if other_heir {
        (item.kind, item.quality, item.eldritch) = (other.kind, other.quality, other.eldritch);
    }
    item.kind.ilvl = ilvl;
    let native = game.pool(item.kind);
    item.rarity = Rarity::Rare;
    let limit = item.limit(game);
    let order = if rng.below(2) == 0 { Side::BOTH } else { [Side::Suffix, Side::Prefix] };
    let mut exclusive_kept = false;
    for side in order {
        let mut here: Vec<Affix> = pool.iter().copied().filter(|a| game.def(a.id).side == side).collect();
        let exclusive: Vec<Affix> = here.iter().copied().filter(|a| game.def(a.id).recombine_exclusive).collect();
        here.retain(|a| !game.def(a.id).recombine_exclusive);
        if !exclusive.is_empty() {
            here.push(exclusive[rng.below(exclusive.len())]);
        }
        let table = recombine_kept(ctx, here.len());
        let keep = rng.pick(table.iter().map(|e| e.1)).map_or(0, |i| table[i].0 as usize).min(limit);
        let mut kept = 0;
        while kept < keep && !here.is_empty() {
            // A mod sharing a group with one already kept is discarded.
            let a = here.remove(rng.below(here.len()));
            let def = game.def(a.id);
            let eligible = !matches!(def.source, Source::Natural | Source::Influence) || native.mods.iter().any(|m| m.0 == a.id);
            let exclusive = def.recombine_exclusive;
            if eligible && !(exclusive && exclusive_kept) && !item.blocks(game, a.id) {
                item.affixes.push(a);
                exclusive_kept |= exclusive;
                kept += 1;
            }
        }
    }
    run(ctx, &[Step::Settle], item, rng);
}

/// The kept-count table for a pool of `n` affixes on one side.
pub fn recombine_kept<'a>(ctx: &'a Ctx, n: usize) -> &'a [(u8, f64)] {
    let rows = &ctx.assumptions.recombine_kept;
    &rows[n.min(rows.len() - 1)]
}

/// Where a recombination's bonus mod may go: a magic result fills an empty side, a rare one
/// the side that has room.
pub fn bonus_sides(rarity: Rarity, prefixes: usize, suffixes: usize, limit: usize) -> Sides {
    match rarity {
        Rarity::Magic if prefixes == 0 => Sides::Only(Side::Prefix),
        Rarity::Magic if suffixes == 0 => Sides::Only(Side::Suffix),
        _ if prefixes >= limit => Sides::Only(Side::Suffix),
        _ if suffixes >= limit => Sides::Only(Side::Prefix),
        _ => Sides::Both,
    }
}

/// A sample of `op` on a copy of `item`, or `None` when it does not apply.
pub fn sample(op: &dyn Operation, ctx: &Ctx, item: &Item, rng: &mut Rng) -> Option<Item> {
    let view = (ctx.game, item);
    op.applies(ctx, &view).then(|| {
        let mut next = item.clone();
        run(ctx, op.steps(), &mut next, rng);
        next
    })
}

/// Mods `pick` could add to `item` now, with their weights.
pub fn eligible(ctx: &Ctx, item: &Item, pick: &Pick) -> Vec<(ModId, f64)> {
    let game = ctx.game;
    let allowed = resolve(pick.sides, item.eldritch);
    let open = Side::BOTH.map(|s| allowed[s as usize] && item.open(game, s) > 0);
    let banned = banned_tags(game, |m| item.affixes.iter().any(|a| a.id == m));
    game.pool(item.kind)
        .mods
        .iter()
        .filter_map(|&(m, w)| {
            let def = game.def(m);
            let ok = def.level <= item.kind.ilvl as u32
                && open[def.side as usize]
                && tag_ok(def, pick.tag)
                && !banned.iter().any(|t| def.tags.contains(t))
                && !item.blocks(game, m);
            let w = if ok { pool_weight(def, m, w, &pick.pool) } else { 0.0 };
            (w > 0.0).then_some((m, w))
        })
        .collect()
}

/// Tags the "Cannot roll Attack/Caster Modifiers" metamods present (per `has`) rule out.
pub fn banned_tags(game: &Game, has: impl Fn(ModId) -> bool) -> Vec<TagId> {
    [(2, "attack"), (3, "caster")]
        .into_iter()
        .filter(|(i, _)| game.metamods.get(*i).is_some_and(|m| has(*m)))
        .filter_map(|(_, t)| game.tag_id(t))
        .collect()
}

fn draw(ctx: &Ctx, item: &mut Item, rng: &mut Rng, pick: &Pick) -> bool {
    let pool = eligible(ctx, item, pick);
    match rng.pick(pool.iter().map(|p| p.1)) {
        Some(i) => {
            item.affixes.push(Affix { id: pool[i].0, crafted: false, fractured: false });
            true
        }
        None => false,
    }
}

/// The count table for an item's rarity.
pub fn count_table<'a>(ctx: &'a Ctx, class: &str, rarity: Rarity) -> &'a [(u8, f64)] {
    let a = ctx.assumptions;
    match rarity {
        Rarity::Normal => &[],
        Rarity::Magic => &a.magic_mods,
        Rarity::Rare if class.contains("Jewel") => &a.rare_mods_jewel,
        Rarity::Rare => &a.rare_mods,
    }
}

fn roll_count(ctx: &Ctx, item: &Item, rng: &mut Rng) -> usize {
    let table = count_table(ctx, &ctx.game.bases[item.kind.base.0 as usize].class, item.rarity);
    rng.pick(table.iter().map(|e| e.1)).map_or(0, |i| table[i].0 as usize)
}

impl View for (&Game, &Item) {
    fn rarity(&self) -> Rarity {
        self.1.rarity
    }
    fn count(&self, side: Side) -> usize {
        self.1.count(self.0, side)
    }
    fn limit(&self) -> usize {
        self.1.limit(self.0)
    }
    fn crafted(&self) -> usize {
        self.1.crafted()
    }
    fn fractured(&self) -> usize {
        self.1.affixes.iter().filter(|a| a.fractured).count()
    }
    fn has(&self, m: ModId) -> bool {
        self.1.affixes.iter().any(|a| a.id == m)
    }
    fn removable(&self, respect_lock: bool) -> usize {
        removable(self.0, self.1, Sides::Both, None, respect_lock, None).len()
    }
    fn fits(&self, m: ModId) -> bool {
        self.1.open(self.0, self.0.def(m).side) > 0 && !self.1.blocks(self.0, m)
    }
    fn fixed_blocks(&self, m: ModId) -> bool {
        let groups = &self.0.def(m).groups;
        self.1.affixes.iter().any(|a| a.fractured && self.0.def(a.id).groups.iter().any(|g| groups.contains(g)))
    }
    fn locked(&self) -> [bool; 2] {
        self.1.locked(self.0)
    }
    fn has_metamod(&self) -> bool {
        self.1.affixes.iter().any(|a| self.0.metamods.contains(&a.id))
    }
    fn influenced(&self) -> bool {
        self.1.kind.influences.0 != 0
    }
    fn eldritch(&self) -> Option<Side> {
        self.1.eldritch
    }
    fn veiled(&self) -> bool {
        self.1.affixes.iter().any(|a| self.0.veils.contains(&a.id))
    }
}

/// What a craft may use besides the basic currency without anyone naming it: every essence
/// that forces a mod in `wanted` on this item class, and every Harvest reforge whose tag a mod
/// in `wanted` that rolls naturally carries.
pub fn catalogue(game: &Game, class: &str, wanted: &std::collections::HashSet<ModId>) -> Vec<Box<dyn Operation>> {
    let essences = essence::Essence::all(game, class).into_iter().filter(|e| wanted.contains(&e.forced()));
    let rolls = |m: ModId| game.def(m).source == Source::Natural;
    let reforges =
        harvest::reforges(game).into_iter().filter(|(_, tag)| wanted.iter().any(|&m| rolls(m) && game.def(m).tags.contains(tag)));
    essences.map(|e| Box::new(e) as Box<dyn Operation>).chain(reforges.map(|r| r.0)).collect()
}

/// The operation a guide names (`annulment`, `essence:deafening-woe`, `bench:remove-crafted`,
/// `harvest:reforge-critical`, ...) for an item class, if it is implemented. Bench "any" crafts
/// avoid the groups in `avoid` (what the goals want).
pub fn parse(game: &Game, class: &str, op: &str, avoid: &[crate::game::GroupId]) -> Option<Box<dyn Operation>> {
    fn boxed<O: Operation + 'static>(o: Option<O>) -> Option<Box<dyn Operation>> {
        o.map(|o| Box::new(o) as Box<dyn Operation>)
    }
    if let Some(c) = currency::Currency::parse(op) {
        return Some(Box::new(c));
    }
    if let Some(o) = misc::parse(op) {
        return Some(o);
    }
    let (mechanic, arg) = op.split_once(':')?;
    match mechanic {
        "essence" => boxed(essence::Essence::new(game, arg, class)),
        "bench" => boxed(bench::Bench::new(game, arg, class, avoid)),
        "harvest" => harvest::parse(game, arg),
        "fossil" => boxed(fossil::Fossils::new(game, arg, class)),
        "influence-exalted" => boxed(influence::Exalt::new(arg, false)),
        "influence-orb" => boxed(influence::Exalt::new(arg, true)),
        "eldritch-implicit" => boxed(eldritch::Implicit::new(arg)),
        "unveil" => boxed(veiled::Unveil::new(game, arg)),
        _ => None,
    }
}
