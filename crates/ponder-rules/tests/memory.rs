use std::path::PathBuf;
use std::sync::OnceLock;

use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind, ModId};
use ponder_rules::goal::{Goal, Rarity};
use ponder_rules::item::Item;
use ponder_rules::lens::Lens;
use ponder_rules::ops::{Ctx, Operation, parse, run};
use ponder_rules::rng::Rng;
use ponder_rules::target::{Need, Target};

const BASE: &str = "Fingerless Silk Gloves";
const LIFE: &str = "+# to maximum Life";

fn game() -> &'static Game {
    static GAME: OnceLock<Game> = OnceLock::new();
    GAME.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/repoe");
        Game::new(Repoe::load(&root).unwrap())
    })
}

fn kind() -> Kind {
    Kind { base: game().base(BASE).unwrap(), ilvl: 86, influences: Influences::default() }
}

/// The life tiers the gloves roll at their item level, best first.
fn life() -> Vec<(ModId, f64)> {
    let pool = game().pool(kind());
    pool.tiers[LIFE].iter().copied().filter(|&(m, _)| game().def(m).level <= 86).collect()
}

fn op(name: &str) -> Box<dyn Operation> {
    parse(game(), "Gloves", name, &[]).unwrap()
}

fn rare(strands: u8) -> Item {
    let mut item = Item::new(kind());
    item.rarity = Rarity::Rare;
    item.strands = strands;
    item
}

/// A target asking for one of the best `tiers` life tiers.
fn top_life(tiers: usize) -> Target {
    let mut target = Target::compile(game(), &Goal { base: Some(BASE.into()), ..Goal::default() }).unwrap();
    target.needs = vec![Need { ok: life()[..tiers].iter().map(|t| t.0).collect(), count: 1, fractured: false }];
    target
}

/// The exact chance that `op` on `item` meets `goal`, checked against sampling, and the mean
/// strands sampling leaves, which the states never overstate.
fn chance(op: &dyn Operation, item: &Item, goal: Target) -> (f64, f64) {
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: game(), assumptions: &assumptions };
    let lens = Lens::new(game(), kind(), vec![goal.clone()], &[op], &[item]);
    let out = lens.run(&ctx, op.steps(), &lens.project(item));
    let expected: f64 = out.iter().filter(|(_, s)| lens.holds(0, s)).map(|(p, _)| p).sum();
    let strands: f64 = out.iter().map(|(p, s)| p * s.strands as f64).sum();
    let mut rng = Rng::new(11);
    let (mut hits, mut left) = (0, 0.0);
    for _ in 0..20_000 {
        let mut sampled = item.clone();
        run(&ctx, op.steps(), &mut sampled, &mut rng);
        hits += usize::from(goal.holds(game(), &sampled));
        left += sampled.strands as f64;
    }
    assert!((hits as f64 / 20_000.0 - expected).abs() < 0.015, "sampled {hits}/20000 vs exact {expected}");
    let left = left / 20_000.0;
    assert!(strands <= left + 0.5, "states keep {strands} strands, sampling leaves {left}");
    (expected, left)
}

#[test]
fn strands_leave_only_the_top_tiers_of_a_family() {
    let tiers = life();
    assert!(tiers.len() >= 8, "{} life tiers", tiers.len());
    let pool = game().pool(kind());
    let factor = |strands: u8, m: ModId| game().strand_factor(&pool, 86, strands, m);
    let rollable = |strands| tiers.iter().filter(|&&(m, _)| factor(strands, m) > 0.0).count();
    assert_eq!(rollable(0), tiers.len());
    let expected = (100.0 / (2.5 * 82.0 + 147.5) * (tiers.len() - 1) as f64) as usize + 1;
    assert_eq!(rollable(82), expected);
    let total = |strands| tiers.iter().map(|&(m, w)| w * factor(strands, m)).sum::<f64>();
    assert!((total(82) - total(0)).abs() < 1e-9, "the family keeps its weight");
}

#[test]
fn chaos_rolls_better_tiers_on_strands_and_uses_some_up() {
    let chaos = op("chaos");
    let (without, _) = chance(chaos.as_ref(), &rare(0), top_life(2));
    let (with, left) = chance(chaos.as_ref(), &rare(60), top_life(2));
    assert!(with > 2.0 * without, "{with} vs {without}");
    assert!((left - (60.0 - 25.0)).abs() < 0.5, "chaos uses 0 to 50 strands evenly: {left}");
}

#[test]
fn exalts_use_more_strands_on_fuller_items() {
    let exalt = op("exalted");
    let mut item = rare(40);
    item.affixes.clear();
    let (_, left) = chance(exalt.as_ref(), &item, top_life(1));
    assert!((left - (40.0 - 5.0)).abs() < 0.2, "an exalt onto a bare rare uses 0 to 10: {left}");
}

#[test]
fn remembrance_randomises_strands_on_normal_items() {
    let remembrance = op("remembrance");
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: game(), assumptions: &assumptions };
    let normal = Item::new(kind());
    assert!(remembrance.applies(&ctx, &(game(), &normal)));
    assert!(!remembrance.applies(&ctx, &(game(), &rare(0))));
    let (_, mean) = chance(remembrance.as_ref(), &normal, top_life(1));
    assert!((mean - 50.5).abs() < 0.5);
}

#[test]
fn harvest_reforges_keep_strands() {
    let reforge = op("harvest:reforge-life");
    let (with, left) = chance(reforge.as_ref(), &rare(60), top_life(1));
    let (without, _) = chance(reforge.as_ref(), &rare(0), top_life(1));
    assert_eq!(left, 60.0);
    assert!(with > without);
}
