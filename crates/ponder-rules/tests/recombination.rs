use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind, ModId, Side};
use ponder_rules::goal::{Goal, Influence, Rarity, Source};
use ponder_rules::item::{Affix, Item};
use ponder_rules::lens::Lens;
use ponder_rules::ops::currency::Currency;
use ponder_rules::ops::recombinator::Recombinator;
use ponder_rules::ops::{Ctx, Operation, Supply, parse, run};
use ponder_rules::rng::Rng;
use ponder_rules::target::{Need, Target};

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

fn kind() -> Kind {
    Kind { base: game().base("Spine Bow").unwrap(), ilvl: 82, influences: Influences::default() }
}

fn item(mods: &[&str]) -> Item {
    let mut item = Item::new(kind());
    item.rarity = Rarity::Rare;
    item.affixes = mods
        .iter()
        .map(|name| {
            let id = game().mod_id(name).unwrap();
            Affix { id, crafted: game().def(id).source == Source::Crafted, fractured: false }
        })
        .collect();
    item
}

fn target(mods: &[ModId], fractured: bool) -> Target {
    let mut target = Target::compile(game(), &Goal { base: Some("Spine Bow".into()), ..Goal::default() }).unwrap();
    target.needs = mods.iter().map(|&m| Need { ok: [m].into(), count: 1, fractured }).collect();
    target
}

fn check_prepared_transitions(lens: &Lens<'_>, ctx: &Ctx, op: &dyn Operation, states: &[ponder_rules::lens::State]) {
    let all: std::collections::BTreeSet<_> =
        states.iter().flat_map(|s| lens.run(ctx, op.steps(), s)).filter(|o| o.0 > 0.0).map(|o| o.1).collect();
    let expected = lens.prepare_expectations(ctx, states, |_| 0.0);
    for merged in [false, true] {
        let index: std::collections::HashMap<_, _> =
            all.iter().enumerate().map(|(i, &s)| (s, if merged && lens.holds(0, &s) { 0 } else { i + 1 })).collect();
        let prepared = lens.prepare_transitions(ctx, states, |s| index.get(s).copied());
        for s in states {
            let linked = prepared(s, op.steps());
            assert_eq!(linked.is_some(), expected(s, op.steps()).is_some());
            let Some(linked) = linked else { continue };
            let mut explicit = std::collections::BTreeMap::<usize, f64>::new();
            for (p, t) in lens.run(ctx, op.steps(), s).into_iter().filter(|o| o.0 > 0.0) {
                *explicit.entry(index[&t]).or_default() += p;
            }
            let linked: std::collections::BTreeMap<_, _> = linked.iter().map(|&(p, t)| (t, p)).collect();
            assert_eq!(linked.len(), explicit.len());
            for (t, p) in explicit {
                assert!((linked[&t] - p).abs() < 1e-12, "{} vs {p}", linked[&t]);
            }
        }
    }
    let missing = lens.prepare_transitions(ctx, states, |_| None);
    for s in states {
        assert!(missing(s, op.steps()).is_none());
    }
}

fn chance(first: &Item, second: &Item, target: Target) -> f64 {
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: game(), assumptions: &assumptions };
    let recombine = Recombinator::new("Bow", "test", Arc::new(Donor(vec![second.clone()])));
    let lens = Lens::new(game(), kind(), vec![target.clone()], &[&recombine], &[first, second]);
    let out = lens.run(&ctx, recombine.steps(), &lens.project(first));
    assert!((out.iter().map(|o| o.0).sum::<f64>() - 1.0).abs() < 1e-12);
    let value = |s: &ponder_rules::lens::State| {
        s.counts.iter().enumerate().map(|(c, &n)| (c + 1) as f64 * (n as f64).powi(2)).sum::<f64>()
            + s.influences as f64
            + s.rarity as u8 as f64
    };
    let weighted: f64 = out.iter().map(|(p, s)| p * value(s)).sum();
    let mut streamed = 0.0;
    lens.for_each_outcome(&ctx, recombine.steps(), &lens.project(first), &mut |p, s| streamed += p * value(&s));
    assert!((streamed - weighted).abs() < 1e-10 * weighted.abs().max(1.0));
    let states = [lens.project(first), lens.project(second)];
    check_prepared_transitions(&lens, &ctx, &recombine, &states);
    let evaluate = lens.prepare_expectations(&ctx, &states, value);
    if states.iter().all(|s| evaluate(s, recombine.steps()).is_some()) {
        let expected: std::collections::BTreeSet<_> =
            states.iter().flat_map(|s| lens.run(&ctx, recombine.steps(), s)).filter(|o| o.0 > 0.0).map(|o| o.1).collect();
        assert_eq!(
            lens.recombination_successors(&ctx, &states, recombine.steps()).into_iter().collect::<std::collections::BTreeSet<_>>(),
            expected
        );
    }
    for s in [first, second] {
        let state = lens.project(s);
        let expected: f64 = lens.run(&ctx, recombine.steps(), &state).iter().map(|(p, t)| p * value(t)).sum();
        for _ in 0..2 {
            if let Some(prepared) = evaluate(&state, recombine.steps()) {
                assert!((prepared - expected).abs() < 1e-10 * expected.abs().max(1.0));
            }
        }
    }
    let expected: f64 = out.iter().filter(|(_, s)| lens.holds(0, s)).map(|(p, _)| p).sum();
    let mut rng = Rng::new(42);
    let mut hits = 0;
    for _ in 0..20_000 {
        let mut sampled = first.clone();
        run(&ctx, recombine.steps(), &mut sampled, &mut rng);
        assert!(sampled.affixes.iter().filter(|a| game().def(a.id).recombine_exclusive).count() <= 1);
        assert!(sampled.affixes.iter().all(|a| first.affixes.contains(a) || second.affixes.contains(a)));
        hits += usize::from(target.holds(game(), &sampled));
    }
    assert!((hits as f64 / 20_000.0 - expected).abs() < 0.02, "sampled {hits}/20000 vs {expected}");
    expected
}

const FIRE: &str = "LocalAddedFireDamageTwoHand9";
const COLD: &str = "LocalAddedColdDamageTwoHand9";
const LIGHTNING: &str = "LocalAddedLightningDamageTwoHand9";
const ESSENCE_FIRE: &str = "LocalAddedFireDamageTwoHandEssence7";
const ESSENCE_LIGHTNING: &str = "LocalAddedLightningDamageTwoHandEssence7";
const PREFIX_LOCK: &str = "StrMasterItemGenerationCannotChangePrefixes";
const SUFFIX_LOCK: &str = "DexMasterItemGenerationCannotChangeSuffixes";

#[test]
fn prepared_expectations_preserve_donor_correlation() {
    let first = item(&[COLD, "LocalIncreasedAccuracyNew5"]);
    let good = item(&[FIRE, "Dexterity6"]);
    let bad = first.clone();
    let wanted = target(&good.affixes.iter().map(|a| a.id).collect::<Vec<_>>(), false);
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: game(), assumptions: &assumptions };
    let recombine = Recombinator::new("Bow", "mixed", Arc::new(Donor(vec![good.clone(), bad.clone()])));
    let lens = Lens::new(game(), kind(), vec![wanted], &[&recombine], &[&first, &good, &bad]);
    let states = [lens.project(&first)];
    check_prepared_transitions(&lens, &ctx, &recombine, &states);
    let value = |s: &ponder_rules::lens::State| f64::from(lens.holds(0, s));
    let evaluate = lens.prepare_expectations(&ctx, &states, value);
    let expected = evaluate(&states[0], recombine.steps()).unwrap();
    assert!((expected - 2.0 / 9.0).abs() < 1e-12, "{expected}");
    let explicit: f64 = lens.run(&ctx, recombine.steps(), &states[0]).iter().map(|(p, s)| p * value(s)).sum();
    assert!((expected - explicit).abs() < 1e-12);
    let expected: std::collections::BTreeSet<_> =
        lens.run(&ctx, recombine.steps(), &states[0]).into_iter().filter(|o| o.0 > 0.0).map(|o| o.1).collect();
    assert_eq!(
        lens.recombination_successors(&ctx, &states, recombine.steps()).into_iter().collect::<std::collections::BTreeSet<_>>(),
        expected
    );
}

#[test]
fn successor_support_preserves_input_pairs_and_fractured_heirs() {
    let mut first = item(&[FIRE, "Dexterity6"]);
    first.affixes[0].fractured = true;
    let second = item(&[COLD, "LocalCriticalStrikeChance3"]);
    let donor = item(&[]);
    let wanted = target(&first.affixes.iter().chain(&second.affixes).map(|a| a.id).collect::<Vec<_>>(), false);
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: game(), assumptions: &assumptions };
    let recombine = Recombinator::new("Bow", "pairs", Arc::new(Donor(vec![donor.clone()])));
    let lens = Lens::new(game(), kind(), vec![wanted], &[&recombine], &[&first, &second, &donor]);
    let states = [lens.project(&first), lens.project(&second)];
    check_prepared_transitions(&lens, &ctx, &recombine, &states);
    let expected: std::collections::BTreeSet<_> =
        states.iter().flat_map(|s| lens.run(&ctx, recombine.steps(), s)).filter(|o| o.0 > 0.0).map(|o| o.1).collect();
    assert_eq!(
        lens.recombination_successors(&ctx, &states, recombine.steps()).into_iter().collect::<std::collections::BTreeSet<_>>(),
        expected
    );
}

#[test]
fn exclusive_modifiers_cannot_coexist_on_either_side() {
    for second in [ESSENCE_LIGHTNING, PREFIX_LOCK, SUFFIX_LOCK] {
        let first = item(&[ESSENCE_FIRE]);
        let second = item(&[second]);
        let wanted = target(&[first.affixes[0].id, second.affixes[0].id], false);
        assert_eq!(chance(&first, &second, wanted), 0.0);
    }
}

#[test]
fn clean_natural_prefix_combinations_match_the_count_table() {
    for (first, second, expected) in
        [(vec![FIRE], vec![COLD], 1.0 / 3.0), (vec![FIRE], vec![COLD, LIGHTNING], 0.1), (vec![FIRE, COLD], vec![COLD, LIGHTNING], 0.3)]
    {
        let first = item(&first);
        let second = item(&second);
        let ids: std::collections::BTreeSet<_> = first.affixes.iter().chain(&second.affixes).map(|a| a.id).collect();
        let observed = chance(&first, &second, target(&ids.into_iter().collect::<Vec<_>>(), false));
        assert!((observed - expected).abs() < 1e-12, "{observed} vs {expected}");
    }
}

#[test]
fn four_desired_affixes_depend_on_their_prefix_suffix_split() {
    let first = item(&[FIRE, "LocalCriticalStrikeChance3"]);
    let second = item(&[COLD, "Dexterity6"]);
    let ids: Vec<_> = first.affixes.iter().chain(&second.affixes).map(|a| a.id).collect();
    assert!((chance(&first, &second, target(&ids, false)) - 1.0 / 9.0).abs() < 1e-12);
    let first = item(&[FIRE, COLD]);
    let second = item(&[LIGHTNING, "LocalIncreasedPhysicalDamagePercent5"]);
    let ids: Vec<_> = first.affixes.iter().chain(&second.affixes).map(|a| a.id).collect();
    assert_eq!(chance(&first, &second, target(&ids, false)), 0.0);
}

#[test]
fn recombination_requires_two_magic_or_rare_items() {
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: game(), assumptions: &assumptions };
    let rare = item(&[FIRE]);
    let normal = Item::new(kind());
    let recombine = Recombinator::new("Bow", "rare", Arc::new(Donor(vec![rare.clone()])));
    assert!(recombine.applies(&ctx, &(game(), &rare)));
    assert!(!recombine.applies(&ctx, &(game(), &normal)));
    let recombine = Recombinator::new("Bow", "normal", Arc::new(Donor(vec![normal])));
    assert!(!recombine.applies(&ctx, &(game(), &rare)));
}

#[test]
fn ordinary_essence_forced_prefixes_can_keep_one_exclusive_craft() {
    let first = item(&[FIRE, "JunMaster2LocalCriticalStrikeChanceStrengthIntelligence2___"]);
    let second = item(&[COLD, LIGHTNING]);
    assert!(!game().def(first.affixes[0].id).recombine_exclusive);
    assert!(!game().def(second.affixes[1].id).recombine_exclusive);
    assert!(game().def(first.affixes[1].id).recombine_exclusive);
    let wanted = first.affixes.iter().chain(&second.affixes).map(|a| a.id).collect::<Vec<_>>();
    assert!((chance(&first, &second, target(&wanted, false)) - 0.059).abs() < 1e-12);
}

#[test]
fn exclusive_padding_counts_as_one_candidate() {
    let first = item(&[ESSENCE_FIRE]);
    let second = item(&[ESSENCE_LIGHTNING]);
    let wanted = target(&[first.affixes[0].id], false);
    assert!((chance(&first, &second, wanted) - 0.59 / 2.0).abs() < 1e-12);
}

#[test]
fn ordinary_crafted_families_do_not_compete_with_essence_exclusives() {
    let first = item(&[ESSENCE_FIRE]);
    let second = item(&["EinharMasterLocalAccuracyRating1"]);
    assert!(!game().def(second.affixes[0].id).recombine_exclusive);
    assert!((chance(&first, &second, target(&[first.affixes[0].id, second.affixes[0].id], false)) - 0.59 * 0.59).abs() < 1e-12);
}

#[test]
fn fractured_modifiers_only_survive_on_their_own_heir() {
    let mut first = item(&[FIRE]);
    let mut second = item(&[COLD]);
    first.affixes[0].fractured = true;
    second.affixes[0].fractured = true;
    assert_eq!(chance(&first, &second, target(&[first.affixes[0].id, second.affixes[0].id], true)), 0.0);
    assert!((chance(&first, &second, target(&[first.affixes[0].id], true)) - 0.295).abs() < 1e-12);
    second.affixes[0].fractured = false;
    assert!((chance(&first, &second, target(&[first.affixes[0].id], true)) - 1.0 / 3.0).abs() < 1e-12);
}

#[test]
fn ineligible_influence_modifiers_count_before_selection() {
    let first = item(&[FIRE]);
    let influenced = Kind { influences: Influences::of(&[Influence::Elder]), ..kind() };
    let modifier = game()
        .pool(influenced)
        .mods
        .iter()
        .find(|&&(m, _)| {
            let def = game().def(m);
            def.influence == Some(Influence::Elder)
                && def.side == Side::Prefix
                && !def.groups.iter().any(|g| game().def(first.affixes[0].id).groups.contains(g))
        })
        .unwrap()
        .0;
    let mut second = Item::new(influenced);
    second.rarity = Rarity::Rare;
    second.affixes.push(Affix { id: modifier, crafted: false, fractured: false });
    assert!((chance(&first, &second, target(&[first.affixes[0].id], false)) - 5.0 / 6.0).abs() < 1e-12);
}

#[test]
fn fracturing_ignores_both_metamod_locks() {
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: game(), assumptions: &assumptions };
    for lock in [PREFIX_LOCK, SUFFIX_LOCK] {
        let original = item(&[FIRE, LIGHTNING, "LocalCriticalStrikeChance3", lock]);
        let targets: Vec<_> = original.affixes.iter().map(|a| target(&[a.id], true)).collect();
        let lens = Lens::new(game(), kind(), targets.clone(), &[&Currency::Fracturing], &[&original]);
        let out = lens.run(&ctx, Currency::Fracturing.steps(), &lens.project(&original));
        let mut counts = [0usize; 4];
        let mut rng = Rng::new(42);
        for _ in 0..20_000 {
            let mut sampled = original.clone();
            run(&ctx, Currency::Fracturing.steps(), &mut sampled, &mut rng);
            for (i, target) in targets.iter().enumerate() {
                counts[i] += usize::from(target.holds(game(), &sampled));
            }
        }
        for (i, count) in counts.into_iter().enumerate() {
            let expected: f64 = out.iter().filter(|(_, s)| lens.holds(i, s)).map(|(p, _)| p).sum();
            assert!((expected - 0.25).abs() < 1e-12);
            assert!((count as f64 / 20_000.0 - expected).abs() < 0.02);
        }
    }
}

#[test]
fn influence_cannot_be_added_to_a_fractured_item() {
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: game(), assumptions: &assumptions };
    let mut original = item(&[FIRE]);
    let elder = parse(game(), "Bow", "influence-exalted:elder", &[]).unwrap();
    assert!(elder.applies(&ctx, &(game(), &original)));
    original.affixes[0].fractured = true;
    assert!(!elder.applies(&ctx, &(game(), &original)));
}

#[test]
fn recombination_preserves_observable_essence_exclusivity_in_the_lens() {
    let first = item(&[FIRE]);
    let second = item(&[ESSENCE_FIRE]);
    let donor = Arc::new(Donor(vec![item(&[PREFIX_LOCK])]));
    let recombine = Recombinator::new("Bow", "test", donor);
    let fire = Target::compile(
        game(),
        &Goal {
            base: Some("Spine Bow".into()),
            mods: vec![ponder_rules::goal::ModReq {
                text: vec!["Adds # to # Fire Damage".into()],
                tier: None,
                count: 1,
                fractured: false,
                source: None,
            }],
            ..Goal::default()
        },
    )
    .unwrap();
    let lens = Lens::new(game(), kind(), vec![fire], &[&recombine], &[&first, &second]);
    assert_ne!(lens.project(&first), lens.project(&second));
}
