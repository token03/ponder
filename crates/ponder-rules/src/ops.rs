//! The operation contract and the primitives mechanics are built from (SPEC §6).
//!
//! A mechanic is a type implementing [`Operation`]. Its randomness comes only from the
//! primitives here, so every mechanic draws mods the same way (SPEC §4.3–4.4).

pub mod currency;

use crate::assumptions::Assumptions;
use crate::game::{Game, ModId, Side};
use crate::goal::Rarity;
use crate::item::{Affix, Item};
use crate::rng::Rng;

pub struct Ctx<'a> {
    pub game: &'a Game,
    pub assumptions: &'a Assumptions,
}

pub trait Operation: Send + Sync {
    fn name(&self) -> String;
    /// May the operation be used on this item?
    fn applies(&self, ctx: &Ctx, item: &Item) -> bool;
    /// One random outcome. Only called when `applies` holds.
    fn apply(&self, ctx: &Ctx, item: &mut Item, rng: &mut Rng);
}

/// A sample of `op` on a copy of `item`, or `None` when it does not apply.
pub fn sample(op: &dyn Operation, ctx: &Ctx, item: &Item, rng: &mut Rng) -> Option<Item> {
    op.applies(ctx, item).then(|| {
        let mut next = item.clone();
        op.apply(ctx, &mut next, rng);
        next
    })
}

/// Natural mods that could be added to `item` now, with their spawn weights. `side` restricts
/// the draw to one side.
pub fn eligible(ctx: &Ctx, item: &Item, side: Option<Side>) -> Vec<(ModId, f64)> {
    let game = ctx.game;
    let open = Side::BOTH.map(|s| item.open(game, s) > 0);
    game.pool(item.kind)
        .mods
        .iter()
        .filter(|(m, _)| {
            let def = game.def(*m);
            def.level <= item.kind.ilvl as u32 && open[def.side as usize] && side.is_none_or(|s| s == def.side) && !item.blocks(game, *m)
        })
        .copied()
        .collect()
}

/// Add one weighted random mod. False when nothing can be added.
pub fn add(ctx: &Ctx, item: &mut Item, rng: &mut Rng, side: Option<Side>) -> bool {
    let pool = eligible(ctx, item, side);
    match rng.pick(pool.iter().map(|p| p.1)) {
        Some(i) => {
            item.affixes.push(Affix { id: pool[i].0, crafted: false, fractured: false });
            true
        }
        None => false,
    }
}

/// Remove one uniformly random affix among those `keep` does not protect. Fractured mods are
/// never removed (SPEC §4.5).
pub fn remove(item: &mut Item, rng: &mut Rng, keep: impl Fn(&Affix) -> bool) -> bool {
    let removable: Vec<usize> = (0..item.affixes.len()).filter(|&i| !item.affixes[i].fractured && !keep(&item.affixes[i])).collect();
    if removable.is_empty() {
        return false;
    }
    let pick = removable[(rng.next_u64() % removable.len() as u64) as usize];
    item.affixes.remove(pick);
    true
}

/// Remove every affix `keep` does not protect, and every non-fractured one.
pub fn clear(item: &mut Item, keep: impl Fn(&Affix) -> bool) {
    item.affixes.retain(|a| a.fractured || keep(a));
}

/// Add random mods until the item carries `total` mods or nothing more fits.
pub fn fill(ctx: &Ctx, item: &mut Item, rng: &mut Rng, total: usize) {
    while item.affixes.len() < total && add(ctx, item, rng, None) {}
}

/// The total mod count a newly magic or rare item rolls.
pub fn roll_count(ctx: &Ctx, item: &Item, rarity: Rarity, rng: &mut Rng) -> usize {
    let a = ctx.assumptions;
    let jewel = ctx.game.bases[item.kind.base.0 as usize].class.contains("Jewel");
    let table = match rarity {
        Rarity::Magic => &a.magic_mods,
        Rarity::Rare if jewel => &a.rare_mods_jewel,
        _ => &a.rare_mods,
    };
    rng.pick(table.iter().map(|e| e.1)).map_or(0, |i| table[i].0 as usize)
}
