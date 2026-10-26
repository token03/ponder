//! The operation contract (SPEC §6).
//!
//! A mechanic is a type implementing [`Operation`]: when it applies, what it costs, and the
//! primitive [`Step`]s it performs. Steps are data, so the same mechanic is sampled on real
//! items ([`run`]) and enumerated exactly on solver states (`lens`) without a second copy of
//! its logic. All randomness is in `Draw`, `Fill`, `Remove` and `RollCount` (SPEC §4.3–4.4).

pub mod bench;
pub mod currency;
pub mod essence;
pub mod harvest;

use crate::assumptions::Assumptions;
use crate::game::{Game, ModId, Side, TagId};
use crate::goal::Rarity;
use crate::item::{Affix, Item};
use crate::rng::Rng;

pub struct Ctx<'a> {
    pub game: &'a Game,
    pub assumptions: &'a Assumptions,
}

/// One primitive. Fractured mods survive every step (SPEC §4.5); a "Cannot Be Changed"
/// metamod protects its side from `Clear` and `Remove` (SPEC §4.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    /// Remove every mod except fractured ones and, when `keep_locked`, the locked side.
    Clear {
        keep_locked: bool,
    },
    Rarity(Rarity),
    /// Rarity from what is left: no mods normal, at most one per side magic, else rare.
    Settle,
    /// Choose the total mod count of the item's rarity (SPEC §10).
    RollCount,
    /// Add this mod (crafted or forced by an essence).
    Force(ModId),
    /// Add one weighted natural mod, optionally only on one side or carrying a tag.
    Draw {
        side: Option<Side>,
        tag: Option<TagId>,
    },
    /// Draw until the item has the rolled count, or nothing fits.
    Fill,
    /// Remove one random mod the lock (when `respect_lock`) and fractures allow.
    Remove {
        respect_lock: bool,
    },
    RemoveCrafted,
}

/// What an operation's `applies` may ask, answered by real items and solver states alike.
pub trait View {
    fn rarity(&self) -> Rarity;
    fn count(&self, side: Side) -> usize;
    fn limit(&self) -> usize;
    fn crafted(&self) -> usize;
    fn has(&self, m: ModId) -> bool;
    /// Mods `Remove` could take.
    fn removable(&self, respect_lock: bool) -> usize;
    /// Could `m` be added now: its side has room and no mod shares its group?
    fn fits(&self, m: ModId) -> bool;
    /// Does a mod no reset removes (fractured) share a group with `m`?
    fn fixed_blocks(&self, m: ModId) -> bool;
    /// The side a "Cannot Be Changed" metamod protects.
    fn locked(&self) -> Option<Side>;
    /// Any bench metamod on the item.
    fn has_metamod(&self) -> bool;

    fn open(&self, side: Side) -> usize {
        self.limit().saturating_sub(self.count(side))
    }
}

/// Price ids and amounts, plus clicking time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cost {
    pub items: Vec<(String, f64)>,
    pub seconds: f64,
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
        match *step {
            Step::Clear { keep_locked } => {
                let locked = if keep_locked { item.locked(game) } else { None };
                item.affixes.retain(|a| a.fractured || locked == Some(game.def(a.id).side));
            }
            Step::Rarity(r) => item.rarity = r,
            Step::Settle => {
                let most = Side::BOTH.iter().map(|s| item.count(game, *s)).max().unwrap_or(0);
                item.rarity = match most {
                    0 => Rarity::Normal,
                    1 => Rarity::Magic,
                    _ => Rarity::Rare,
                };
            }
            Step::RollCount => total = roll_count(ctx, item, rng),
            Step::Force(m) => {
                item.affixes.push(Affix { id: m, crafted: game.def(m).source == crate::goal::Source::Crafted, fractured: false })
            }
            Step::Draw { side, tag } => {
                draw(ctx, item, rng, side, tag);
            }
            Step::Fill => while item.affixes.len() < total && draw(ctx, item, rng, None, None) {},
            Step::Remove { respect_lock } => {
                let locked = if respect_lock { item.locked(game) } else { None };
                let removable: Vec<usize> = (0..item.affixes.len())
                    .filter(|&i| !item.affixes[i].fractured && locked != Some(game.def(item.affixes[i].id).side))
                    .collect();
                if !removable.is_empty() {
                    item.affixes.remove(removable[(rng.next_u64() % removable.len() as u64) as usize]);
                }
            }
            Step::RemoveCrafted => item.affixes.retain(|a| !a.crafted),
        }
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

/// Natural mods that could be added to `item` now, with their spawn weights.
pub fn eligible(ctx: &Ctx, item: &Item, side: Option<Side>, tag: Option<TagId>) -> Vec<(ModId, f64)> {
    let game = ctx.game;
    let open = Side::BOTH.map(|s| item.open(game, s) > 0);
    game.pool(item.kind)
        .mods
        .iter()
        .filter(|(m, _)| {
            let def = game.def(*m);
            def.level <= item.kind.ilvl as u32
                && open[def.side as usize]
                && side.is_none_or(|s| s == def.side)
                && tag.is_none_or(|t| def.tags.contains(&t))
                && !item.blocks(game, *m)
        })
        .copied()
        .collect()
}

fn draw(ctx: &Ctx, item: &mut Item, rng: &mut Rng, side: Option<Side>, tag: Option<TagId>) -> bool {
    let pool = eligible(ctx, item, side, tag);
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
    fn has(&self, m: ModId) -> bool {
        self.1.affixes.iter().any(|a| a.id == m)
    }
    fn removable(&self, respect_lock: bool) -> usize {
        let locked = if respect_lock { self.1.locked(self.0) } else { None };
        self.1.affixes.iter().filter(|a| !a.fractured && locked != Some(self.0.def(a.id).side)).count()
    }
    fn fits(&self, m: ModId) -> bool {
        self.1.open(self.0, self.0.def(m).side) > 0 && !self.1.blocks(self.0, m)
    }
    fn fixed_blocks(&self, m: ModId) -> bool {
        let groups = &self.0.def(m).groups;
        self.1.affixes.iter().any(|a| a.fractured && self.0.def(a.id).groups.iter().any(|g| groups.contains(g)))
    }
    fn locked(&self) -> Option<Side> {
        self.1.locked(self.0)
    }
    fn has_metamod(&self) -> bool {
        self.1.affixes.iter().any(|a| self.0.metamods.contains(&a.id))
    }
}

/// The operation a guide names (`annulment`, `essence:deafening-woe`, `bench:remove-crafted`,
/// `harvest:reforge-critical`) for an item class, if it is implemented.
pub fn parse(game: &Game, class: &str, op: &str) -> Option<Box<dyn Operation>> {
    if let Some(c) = currency::Currency::parse(op) {
        return Some(Box::new(c));
    }
    let (mechanic, arg) = op.split_once(':')?;
    match mechanic {
        "essence" => essence::Essence::new(game, arg, class).map(|e| Box::new(e) as Box<dyn Operation>),
        "bench" => bench::Bench::new(game, arg).map(|b| Box::new(b) as Box<dyn Operation>),
        "harvest" => harvest::Reforge::new(game, arg).map(|r| Box::new(r) as Box<dyn Operation>),
        _ => None,
    }
}
