use std::path::PathBuf;
use std::sync::OnceLock;

use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind};
use ponder_rules::goal::{Goal, Influence, Rarity};
use ponder_rules::item::{Affix, Item};
use ponder_rules::lens::Lens;
use ponder_rules::ops::currency::Currency;
use ponder_rules::ops::{Ctx, Operation, View, parse, run};
use ponder_rules::rng::Rng;
use ponder_rules::target::{Need, Target};

fn game() -> &'static Game {
    static GAME: OnceLock<Game> = OnceLock::new();
    GAME.get_or_init(|| Game::new(Repoe::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/repoe")).unwrap()))
}

#[test]
fn influence_slams_reject_existing_and_eldritch_influence() {
    let game = game();
    let assumptions = Assumptions::default();
    let ctx = Ctx { game, assumptions: &assumptions };
    let kind = Kind { base: game.base("Titan Gauntlets").unwrap(), ilvl: 86, influences: Influences::default() };
    let slam = parse(game, "Gloves", "influence-exalted:crusader", &[]).unwrap();
    let ember = parse(game, "Gloves", "eldritch-implicit:lesser-ember", &[]).unwrap();
    let mut item = Item::new(kind);
    run(&ctx, Currency::Alchemy.steps(), &mut item, &mut Rng::new(1));
    let target = Target::compile(game, &Goal { base: Some("Titan Gauntlets".into()), ..Goal::default() }).unwrap();
    let lens = Lens::new(game, kind, vec![target], &[slam.as_ref(), ember.as_ref(), &Currency::Alchemy], &[&item]);
    for influence in [None, Some(Influence::Crusader), Some(Influence::Hunter)] {
        item.kind.influences = influence.map_or(Influences::default(), |i| Influences::of(&[i]));
        assert_eq!(slam.applies(&ctx, &(game, &item)), influence.is_none());
        assert_eq!(slam.applies(&ctx, &lens.view(&lens.project(&item))), influence.is_none());
    }
    item.kind.influences = Influences::default();
    run(&ctx, ember.steps(), &mut item, &mut Rng::new(1));
    assert!(!slam.applies(&ctx, &(game, &item)));
    assert!(!slam.applies(&ctx, &lens.view(&lens.project(&item))));
}

#[test]
fn eldritch_implicits_only_apply_to_eligible_armour_classes() {
    let game = game();
    let assumptions = Assumptions::default();
    let ctx = Ctx { game, assumptions: &assumptions };
    for (base, allowed) in
        [("Titan Gauntlets", true), ("Giantslayer Helmet", true), ("Spine Bow", false), ("Heat-attuned Tower Shield", false)]
    {
        let kind = Kind { base: game.base(base).unwrap(), ilvl: 86, influences: Influences::default() };
        let item = Item::new(kind);
        let class = &game.bases[kind.base.0 as usize].class;
        let ember = parse(game, class, "eldritch-implicit:lesser-ember", &[]).unwrap();
        let goal = Target::compile(game, &Goal { base: Some(base.into()), ..Goal::default() }).unwrap();
        let lens = Lens::new(game, kind, vec![goal], &[ember.as_ref()], &[&item]);
        assert_eq!(ember.applies(&ctx, &(game, &item)), allowed, "{base}");
        assert_eq!(ember.applies(&ctx, &lens.view(&lens.project(&item))), allowed, "{base}");
    }
}

#[test]
fn veiled_exalted_removes_before_adding_even_on_full_items() {
    let game = game();
    let assumptions = Assumptions::default();
    let ctx = Ctx { game, assumptions: &assumptions };
    let kind = Kind { base: game.base("Opal Sceptre").unwrap(), ilvl: 86, influences: Influences::default() };
    let mut item = Item::new(kind);
    let mut rng = Rng::new(19);
    run(&ctx, Currency::Alchemy.steps(), &mut item, &mut rng);
    while item.affixes.len() != 6 {
        run(&ctx, Currency::Chaos.steps(), &mut item, &mut rng);
    }
    let orb = parse(game, "Sceptre", "veiled-exalted", &[]).unwrap();
    let goal = Target::compile(game, &Goal { base: Some("Opal Sceptre".into()), ..Goal::default() }).unwrap();
    let lens = Lens::new(game, kind, vec![goal], &[orb.as_ref(), &Currency::Alchemy], &[&item]);
    assert!(orb.applies(&ctx, &(game, &item)));
    assert!(orb.applies(&ctx, &lens.view(&lens.project(&item))));
    let outcomes = lens.run(&ctx, orb.steps(), &lens.project(&item));
    assert!((outcomes.iter().map(|r| r.0).sum::<f64>() - 1.0).abs() < 1e-12);
    for _ in 0..200 {
        let mut next = item.clone();
        run(&ctx, orb.steps(), &mut next, &mut rng);
        assert_eq!(next.affixes.len(), 6);
        assert_eq!(next.affixes.iter().filter(|a| game.veils.contains(&a.id)).count(), 1);
        assert_eq!(next.affixes.iter().filter(|a| item.affixes.contains(a)).count(), 5);
        assert!(outcomes.iter().any(|(p, s)| *p > 0.0 && *s == lens.project(&next)));
    }
}

#[test]
fn annulments_respect_attack_and_caster_protection_on_supplied_items() {
    let game = game();
    let assumptions = Assumptions::default();
    let ctx = Ctx { game, assumptions: &assumptions };
    let kind = Kind { base: game.base("Opal Sceptre").unwrap(), ilvl: 86, influences: Influences::default() };
    let pool = game.pool(kind);
    for (meta, tag) in [(2, "attack"), (3, "caster")] {
        let tag = game.tag_id(tag).unwrap();
        let id = pool.mods.iter().find(|(m, w)| *w > 0.0 && game.def(*m).tags.contains(&tag)).unwrap().0;
        let mut item = Item::new(kind);
        item.rarity = Rarity::Rare;
        item.affixes =
            vec![Affix { id, crafted: false, fractured: false }, Affix { id: game.metamods[meta], crafted: true, fractured: false }];
        let mut goal = Target::compile(game, &Goal { base: Some("Opal Sceptre".into()), ..Goal::default() }).unwrap();
        goal.needs = vec![Need { ok: [id].into(), count: 1, fractured: false }];
        let lens = Lens::new(game, kind, vec![goal], &[&Currency::Annulment], &[&item]);
        let state = lens.project(&item);
        assert_eq!(lens.view(&state).removable(true, true), 1);
        assert_eq!(lens.view(&state).removable(true, false), 2);
        assert!(lens.run(&ctx, Currency::Annulment.steps(), &state).iter().all(|(_, s)| lens.holds(0, s)));
        for seed in 0..100 {
            let mut next = item.clone();
            run(&ctx, Currency::Annulment.steps(), &mut next, &mut Rng::new(seed));
            assert_eq!(next.affixes, vec![Affix { id, crafted: false, fractured: false }]);
        }
    }
}
