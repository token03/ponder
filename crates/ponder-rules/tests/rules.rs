//! SPEC §4 checked on random walks: every operation, many items, every step.

use std::path::PathBuf;
use std::sync::OnceLock;

use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind, Side};
use ponder_rules::goal::{Rarity, Source};
use ponder_rules::item::{Affix, Item};
use ponder_rules::ops::currency::Currency;
use ponder_rules::ops::{Ctx, Operation, eligible};
use ponder_rules::rng::Rng;

fn game() -> &'static Game {
    static GAME: OnceLock<Game> = OnceLock::new();
    GAME.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/repoe");
        Game::new(Repoe::load(&dir).unwrap())
    })
}

const BASES: [&str; 6] = ["Titan Gauntlets", "Focused Amulet", "Convoking Wand", "Vaal Regalia", "Spine Bow", "Cobalt Jewel"];

fn kinds(game: &Game) -> impl Iterator<Item = Kind> + '_ {
    BASES.iter().flat_map(move |b| {
        let base = game.base(b).unwrap_or_else(|| panic!("{b}"));
        [60, 86].map(|ilvl| Kind { base, ilvl, influences: Influences::default() })
    })
}

/// Everything SPEC §4.1–4.3 and §4.5 say about one item.
fn check(game: &Game, item: &Item, fractured: &[Affix]) -> Result<(), String> {
    for side in Side::BOTH {
        if item.count(game, side) > item.limit(game).max(fractured.len()) {
            return Err(format!("{side:?} over the limit"));
        }
    }
    for (i, a) in item.affixes.iter().enumerate() {
        let def = game.def(a.id);
        if item.affixes[..i].iter().any(|b| game.def(b.id).groups.iter().any(|g| def.groups.contains(g))) {
            return Err(format!("{} shares a group", def.key));
        }
        if !a.fractured && !a.crafted {
            let pool = game.pool(item.kind);
            if !matches!(def.source, Source::Natural | Source::Influence) || !pool.mods.iter().any(|(m, _)| *m == a.id) {
                return Err(format!("{} cannot roll here", def.key));
            }
            if def.level > item.kind.ilvl as u32 {
                return Err(format!("{} above item level", def.key));
            }
        }
    }
    if fractured.iter().any(|f| !item.affixes.contains(f)) {
        return Err("lost a fractured mod".into());
    }
    if item.rarity == Rarity::Normal && item.affixes.iter().any(|a| !a.fractured) {
        return Err("normal item with mods".into());
    }
    Ok(())
}

#[test]
fn random_walks_obey_the_rules() {
    let game = game();
    let assumptions = Assumptions::default();
    let ctx = Ctx { game, assumptions: &assumptions };
    let mut rng = Rng::new(7);
    for kind in kinds(game) {
        for walk in 0..200 {
            let mut item = Item::new(kind);
            let mut fractured = Vec::new();
            if walk % 2 == 1 {
                // Start from a rare with one fractured mod.
                item.rarity = Rarity::Rare;
                let pool = eligible(&ctx, &item, None);
                let a = Affix { id: pool[walk % pool.len()].0, crafted: false, fractured: true };
                item.affixes.push(a);
                fractured.push(a);
            }
            for step in 0..40 {
                let usable: Vec<Currency> = Currency::ALL.into_iter().filter(|c| c.applies(&ctx, &item)).collect();
                let op = usable[(rng.next_u64() % usable.len() as u64) as usize];
                op.apply(&ctx, &mut item, &mut rng);
                if let Err(e) = check(game, &item, &fractured) {
                    panic!(
                        "{} ilvl {} walk {walk} step {step} after {}: {e}\n{item:?}",
                        game.bases[kind.base.0 as usize].name,
                        kind.ilvl,
                        op.name()
                    );
                }
            }
        }
    }
}

#[test]
fn chaos_mod_counts_follow_the_assumption() {
    let game = game();
    let assumptions = Assumptions::default();
    let ctx = Ctx { game, assumptions: &assumptions };
    let kind = Kind { base: game.base("Titan Gauntlets").unwrap(), ilvl: 86, influences: Influences::default() };
    let mut item = Item::new(kind);
    let mut rng = Rng::new(11);
    Currency::Alchemy.apply(&ctx, &mut item, &mut rng);
    let mut counts = [0usize; 7];
    let n = 20_000;
    for _ in 0..n {
        Currency::Chaos.apply(&ctx, &mut item, &mut rng);
        counts[item.affixes.len()] += 1;
    }
    let total: f64 = assumptions.rare_mods.iter().map(|e| e.1).sum();
    for (count, weight) in &assumptions.rare_mods {
        let seen = counts[*count as usize] as f64 / n as f64;
        assert!((seen - weight / total).abs() < 0.015, "{count} mods: {seen} vs {}", weight / total);
    }
}
