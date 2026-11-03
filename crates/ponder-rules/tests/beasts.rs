use std::path::PathBuf;
use std::sync::OnceLock;

use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind, ModId, Side};
use ponder_rules::goal::{Goal, Influence, Rarity, Source};
use ponder_rules::item::{Affix, Item};
use ponder_rules::lens::Lens;
use ponder_rules::ops::{Ctx, Operation, Step, parse, run};
use ponder_rules::rng::Rng;
use ponder_rules::target::{Need, Target};

const BASE: &str = "Fingerless Silk Gloves";

fn game() -> &'static Game {
    static GAME: OnceLock<Game> = OnceLock::new();
    GAME.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/repoe");
        Game::new(Repoe::load(&root).unwrap())
    })
}

fn kind() -> Kind {
    Kind { base: game().base(BASE).unwrap(), ilvl: 86, influences: Influences::of(&[Influence::Warlord]) }
}

fn op(name: &str) -> Box<dyn Operation> {
    parse(game(), "Gloves", name, &[]).unwrap()
}

fn shares_group(a: ModId, b: ModId) -> bool {
    game().def(a).groups.iter().any(|g| game().def(b).groups.contains(g))
}

/// Warlord glove mods on `side`, best tier of each family first, none sharing a group.
fn warlord(side: Side) -> Vec<ModId> {
    let mut mods: Vec<ModId> = game()
        .pool(kind())
        .mods
        .iter()
        .map(|m| m.0)
        .filter(|&m| game().def(m).source == Source::Influence && game().def(m).side == side && game().def(m).level <= 86)
        .collect();
    mods.sort_by_key(|&m| (std::cmp::Reverse(game().def(m).level), game().def(m).key.clone()));
    let mut out: Vec<ModId> = Vec::new();
    for m in mods {
        if !out.iter().any(|&o| shares_group(m, o)) {
            out.push(m);
        }
    }
    out
}

fn item(rarity: Rarity, mods: &[ModId]) -> Item {
    let mut item = Item::new(kind());
    item.rarity = rarity;
    item.affixes = mods.iter().map(|&id| Affix { id, crafted: game().def(id).source == Source::Crafted, fractured: false }).collect();
    item
}

fn target(needs: &[ModId]) -> Target {
    let mut target = Target::compile(game(), &Goal { base: Some(BASE.into()), ..Goal::default() }).unwrap();
    target.needs = needs.iter().map(|&m| Need { ok: [m].into(), count: 1, fractured: false }).collect();
    target
}

/// The exact chance that `steps` (of `ops`, the first applying to `item`) on `item` meet
/// `goal`, checked against sampling.
fn chance(ops: &[&dyn Operation], steps: &[Step], item: &Item, goal: Target) -> f64 {
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: game(), assumptions: &assumptions };
    assert!(ops[0].applies(&ctx, &(game(), item)));
    let lens = Lens::new(game(), kind(), vec![goal.clone()], ops, &[item]);
    let out = lens.run(&ctx, steps, &lens.project(item));
    assert!((out.iter().map(|o| o.0).sum::<f64>() - 1.0).abs() < 1e-12);
    let expected: f64 = out.iter().filter(|(_, s)| lens.holds(0, s)).map(|(p, _)| p).sum();
    let mut rng = Rng::new(5);
    let mut hits = 0;
    for _ in 0..20_000 {
        let mut sampled = item.clone();
        run(&ctx, steps, &mut sampled, &mut rng);
        hits += usize::from(goal.holds(game(), &sampled));
    }
    assert!((hits as f64 / 20_000.0 - expected).abs() < 0.015, "sampled {hits}/20000 vs exact {expected}");
    expected
}

#[test]
fn dominance_elevates_the_best_tier_and_removes_another_influenced_mod() {
    let dominance = op("dominance");
    let suffixes = warlord(Side::Suffix);
    let prefixes = warlord(Side::Prefix);
    let mods = [suffixes[0], prefixes[0], prefixes[1]];
    let elevated = game().upgrade(kind(), suffixes[0]);
    assert_eq!(game().def(elevated).source, Source::Elevated);
    assert_eq!(game().def(elevated).influence, Some(Influence::Warlord));
    let open = item(Rarity::Rare, &mods);
    let p = chance(&[dominance.as_ref()], dominance.steps(), &open, target(&[elevated]));
    assert!((p - 1.0 / 3.0).abs() < 1e-9, "removal keeps it 2/3 of the time, then the upgrade takes it half: {p}");
    let mut locked = open.clone();
    let lock = game().metamods[1];
    assert_eq!(game().def(lock).side, Side::Prefix, "suffixes cannot be changed is a prefix");
    locked.affixes.push(Affix { id: lock, crafted: true, fractured: false });
    let p = chance(&[dominance.as_ref()], dominance.steps(), &locked, target(&[elevated]));
    assert!((p - 0.5).abs() < 1e-9, "the lock keeps the suffix from removal, not from the upgrade: {p}");
}

#[test]
fn dominance_moves_lower_tiers_up_one() {
    let dominance = op("dominance");
    let family: Vec<ModId> = game()
        .pool(kind())
        .tiers
        .values()
        .find_map(|tiers| {
            let ids: Vec<ModId> =
                tiers.iter().map(|t| t.0).filter(|&m| game().def(m).source == Source::Influence && game().def(m).level <= 86).collect();
            (ids.len() >= 3).then_some(ids)
        })
        .expect("an influenced family with three tiers");
    let low = family[2];
    assert_eq!(game().upgrade(kind(), low), family[1]);
    let side = if game().def(low).side == Side::Prefix { Side::Suffix } else { Side::Prefix };
    let other = warlord(side)[0];
    let p = chance(&[dominance.as_ref()], dominance.steps(), &item(Rarity::Rare, &[low, other]), target(&[family[1]]));
    assert!((p - 0.5).abs() < 1e-9, "{p}");
}

#[test]
fn the_matron_adds_an_eligible_meta_modifier() {
    let matron = op("beast:wild-bristle-matron");
    let suffixes = warlord(Side::Suffix);
    let [prefix_lock, suffix_lock] = [game().metamods[0], game().metamods[1]];
    let full = item(Rarity::Rare, &suffixes[..3]);
    assert!(chance(&[matron.as_ref()], matron.steps(), &full, target(&[suffix_lock])) > 0.999);
    let open = item(Rarity::Rare, &suffixes[..1]);
    let p = chance(&[matron.as_ref()], matron.steps(), &open, target(&[prefix_lock]));
    assert!((p - 0.2).abs() < 1e-9, "{p}");
}

#[test]
fn matrons_until_the_lock_cost_one_per_meta_modifier_that_fits() {
    let suffixes = warlord(Side::Suffix);
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: game(), assumptions: &assumptions };
    let full = item(Rarity::Rare, &suffixes[..3]);
    let open = item(Rarity::Rare, &suffixes[..1]);
    let sure = op("beast:wild-bristle-matron:suffixes:1");
    assert!(sure.applies(&ctx, &(game(), &full)));
    assert!(!sure.applies(&ctx, &(game(), &open)));
    let five = op("beast:wild-bristle-matron:prefixes:5");
    assert!(five.applies(&ctx, &(game(), &open)));
    assert!(!op("beast:wild-bristle-matron:prefixes:4").applies(&ctx, &(game(), &open)));
    assert_eq!(five.cost().items, vec![("wild-bristle-matron".to_string(), 5.0), ("scour".to_string(), 4.0)]);
    let p = chance(&[five.as_ref()], five.steps(), &open, target(&[game().metamods[0]]));
    assert!(p > 0.999, "{p}");
}

#[test]
fn an_imprint_restores_the_item_once() {
    let imprint = op("imprint");
    let restore = op("restore");
    let alteration = op("alteration");
    let magic = item(Rarity::Magic, &warlord(Side::Suffix)[..1]);
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: game(), assumptions: &assumptions };
    assert!(!restore.applies(&ctx, &(game(), &magic)));
    let steps: Vec<Step> = [imprint.as_ref(), alteration.as_ref(), restore.as_ref()].iter().flat_map(|o| o.steps()).cloned().collect();
    let kept = magic.affixes[0].id;
    let p = chance(&[imprint.as_ref(), alteration.as_ref(), restore.as_ref()], &steps, &magic, target(&[kept]));
    assert!(p > 0.999, "{p}");
    let mut restored = magic.clone();
    run(&ctx, &steps, &mut restored, &mut Rng::new(1));
    assert_eq!(restored, magic);
}

#[test]
fn the_influence_reforge_guarantees_an_influenced_mod() {
    let reforge = op("harvest:reforge-influence");
    let rare = item(Rarity::Rare, &[]);
    let needs: Vec<ModId> = game()
        .pool(kind())
        .mods
        .iter()
        .map(|m| m.0)
        .filter(|&m| game().def(m).source == Source::Influence && game().def(m).level <= 86)
        .collect();
    let mut goal = target(&[]);
    goal.needs = vec![Need { ok: needs.into_iter().collect(), count: 1, fractured: false }];
    assert!(chance(&[reforge.as_ref()], reforge.steps(), &rare, goal) > 0.999);
}

#[test]
fn a_lock_lasts_until_the_next_currency() {
    let lock = op("beast:lock");
    let alteration = op("alteration");
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: game(), assumptions: &assumptions };
    let mut magic = item(Rarity::Magic, &warlord(Side::Suffix)[..1]);
    assert!(!lock.applies(&ctx, &(game(), &item(Rarity::Rare, &[]))));
    assert!(lock.applies(&ctx, &(game(), &magic)));
    let lens = Lens::new(game(), kind(), vec![target(&[])], &[lock.as_ref(), alteration.as_ref()], &[&magic]);
    let locked = lens.run(&ctx, lock.steps(), &lens.project(&magic));
    assert!(locked.len() == 1 && locked[0].1.foreseen);
    assert!(lens.run(&ctx, alteration.steps(), &locked[0].1).iter().all(|(_, s)| !s.foreseen));
    run(&ctx, lock.steps(), &mut magic, &mut Rng::new(1));
    assert!(magic.foreseen && !lock.applies(&ctx, &(game(), &magic)));
    run(&ctx, alteration.steps(), &mut magic, &mut Rng::new(1));
    assert!(!magic.foreseen);
}
