use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind, ModId};
use ponder_rules::goal::{Goal, Influence, Rarity, Source};
use ponder_rules::item::{Affix, Item};
use ponder_rules::lens::Lens;
use ponder_rules::ops::awakener::Awakener;
use ponder_rules::ops::{Ctx, Operation, Supply, run};
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

struct Donor(Vec<Item>);

impl Supply for Donor {
    fn name(&self) -> String {
        "donor".into()
    }
    fn price(&self) -> f64 {
        0.0
    }
    fn items(&self) -> &[Item] {
        &self.0
    }
    fn item(&self, rng: &mut Rng) -> Item {
        self.0[rng.below(self.0.len())].clone()
    }
}

fn kind(influences: &[Influence]) -> Kind {
    Kind { base: game().base(BASE).unwrap(), ilvl: 86, influences: Influences::of(influences) }
}

/// Influence mods of `influence` that roll on the gloves, by key.
fn influenced(influence: Influence) -> Vec<ModId> {
    let mut mods: Vec<ModId> = game()
        .pool(kind(&[influence]))
        .mods
        .iter()
        .map(|m| m.0)
        .filter(|&m| game().def(m).source == Source::Influence && game().def(m).influence == Some(influence))
        .collect();
    mods.sort_by_key(|&m| game().def(m).key.clone());
    mods
}

fn shares_group(a: ModId, b: ModId) -> bool {
    game().def(a).groups.iter().any(|g| game().def(b).groups.contains(g))
}

/// The first `n` of `mods` in distinct groups, none sharing one with `besides`.
fn apart(mods: &[ModId], besides: &[ModId], n: usize) -> Vec<ModId> {
    let mut out: Vec<ModId> = Vec::new();
    for &m in mods {
        if out.len() < n && !out.iter().chain(besides).any(|&o| shares_group(m, o)) {
            out.push(m);
        }
    }
    out
}

fn item(influence: Influence, mods: &[ModId]) -> Item {
    let mut item = Item::new(kind(&[influence]));
    item.rarity = Rarity::Rare;
    item.affixes = mods.iter().map(|&id| Affix { id, crafted: false, fractured: false }).collect();
    item
}

fn target(needs: &[ModId]) -> Target {
    let mut target = Target::compile(game(), &Goal { base: Some(BASE.into()), ..Goal::default() }).unwrap();
    target.needs = needs.iter().map(|&m| Need { ok: [m].into(), count: 1, fractured: false }).collect();
    target
}

/// The exact chance that awakening `receiver` with `donor` meets `goal`, checked against
/// sampling the same operation on real items.
fn chance(receiver: &Item, donor: &Item, goal: Target) -> f64 {
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: game(), assumptions: &assumptions };
    let awaken = Awakener::new("test", Arc::new(Donor(vec![donor.clone()])));
    assert!(awaken.applies(&ctx, &(game(), receiver)));
    let lens = Lens::new(game(), kind(&[]), vec![goal.clone()], &[&awaken], &[receiver]);
    let out = lens.run(&ctx, awaken.steps(), &lens.project(receiver));
    assert!((out.iter().map(|o| o.0).sum::<f64>() - 1.0).abs() < 1e-12);
    let expected: f64 = out.iter().filter(|(_, s)| lens.holds(0, s)).map(|(p, _)| p).sum();
    let mut rng = Rng::new(7);
    let mut hits = 0;
    for _ in 0..20_000 {
        let mut sampled = receiver.clone();
        run(&ctx, awaken.steps(), &mut sampled, &mut rng);
        assert_eq!(sampled.kind.influences, Influences::of(&[Influence::Elder, Influence::Shaper]));
        assert_eq!(sampled.rarity, Rarity::Rare);
        hits += usize::from(goal.holds(game(), &sampled));
    }
    assert!((hits as f64 / 20_000.0 - expected).abs() < 0.015, "sampled {hits}/20000 vs exact {expected}");
    expected
}

#[test]
fn one_influenced_mod_of_each_item_survives() {
    let elder = influenced(Influence::Elder);
    let shaper = influenced(Influence::Shaper);
    let kept = elder[0];
    let given = apart(&shaper, &[kept], 2);
    let receiver = item(Influence::Elder, &[kept]);
    let donor = item(Influence::Shaper, &given);
    let both = chance(&receiver, &donor, target(&[kept, given[0]]));
    assert!((0.5..0.6).contains(&both), "{both}: the donor gives one of its two mods");
    assert!(chance(&receiver, &donor, target(&[kept])) > 0.999);
}

#[test]
fn the_reroll_draws_from_both_influences() {
    let elder = influenced(Influence::Elder);
    let shaper = influenced(Influence::Shaper);
    let mods = apart(&shaper, &elder[..1], 2);
    let receiver = item(Influence::Elder, &elder[..1]);
    let donor = item(Influence::Shaper, &mods[..1]);
    let rolled = chance(&receiver, &donor, target(&mods[1..]));
    assert!(rolled > 0.0, "a Shaper mod the donor did not give can still roll");
}

#[test]
fn a_group_conflict_keeps_one_of_the_two() {
    let elder = influenced(Influence::Elder);
    let shaper = influenced(Influence::Shaper);
    let (kept, given) = elder
        .iter()
        .find_map(|&e| shaper.iter().find(|&&s| shares_group(e, s)).map(|&s| (e, s)))
        .expect("an Elder and a Shaper glove mod share a group");
    let receiver = item(Influence::Elder, &[kept]);
    let donor = item(Influence::Shaper, &[given]);
    let elder_kept = chance(&receiver, &donor, target(&[kept]));
    assert!((0.45..0.6).contains(&elder_kept), "{elder_kept}");
}

#[test]
fn everything_else_is_rerolled() {
    let elder = influenced(Influence::Elder);
    let shaper = influenced(Influence::Shaper);
    let natural: ModId = game()
        .pool(kind(&[]))
        .mods
        .iter()
        .map(|m| m.0)
        .find(|&m| game().def(m).source == Source::Natural && !shares_group(m, elder[0]) && !shares_group(m, shaper[0]))
        .unwrap();
    let receiver = item(Influence::Elder, &[elder[0], natural]);
    let donor = item(Influence::Shaper, &[shaper[0]]);
    let kept = chance(&receiver, &donor, target(&[natural]));
    assert!(kept < 0.2, "{kept}: the natural mod only survives by rerolling it");
}

#[test]
fn both_items_need_exactly_one_different_influence() {
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: game(), assumptions: &assumptions };
    let elder = item(Influence::Elder, &influenced(Influence::Elder)[..1]);
    let other_elder = Awakener::new("same", Arc::new(Donor(vec![elder.clone()])));
    assert!(!other_elder.applies(&ctx, &(game(), &elder)));
    let mut double = item(Influence::Shaper, &[]);
    double.kind.influences = Influences::of(&[Influence::Shaper, Influence::Hunter]);
    let double_donor = Awakener::new("double", Arc::new(Donor(vec![double])));
    assert!(!double_donor.applies(&ctx, &(game(), &elder)));
}
