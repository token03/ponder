use std::path::PathBuf;

use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind};
use ponder_rules::goal::{Goal, Rarity};
use ponder_rules::item::{Affix, Item};
use ponder_rules::lens::Lens;
use ponder_rules::ops::currency::Currency;
use ponder_rules::ops::{Ctx, Operation, Step, View, parse, run};
use ponder_rules::rng::Rng;
use ponder_rules::target::Target;

#[test]
fn fractured_bench_mods_keep_their_slot_and_survive_removal() {
    let game = Game::new(Repoe::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/repoe")).unwrap());
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: &game, assumptions: &assumptions };
    let kind = Kind { base: game.base("Opal Sceptre").unwrap(), ilvl: 86, influences: Influences::default() };
    let lock = parse(&game, "Sceptre", "bench:suffixes-cannot-be-changed", &[]).unwrap();
    let finish = parse(&game, "Sceptre", "bench:increased-damage-over-time", &[]).unwrap();
    let mut item = Item::new(kind);
    item.rarity = Rarity::Rare;
    let pool = game.pool(kind);
    item.affixes =
        ["+#% to Damage over Time Multiplier", "+#% to Fire Damage over Time Multiplier", "#% increased Spell Critical Strike Chance"]
            .into_iter()
            .map(|text| Affix {
                id: game.with_template(text).iter().copied().find(|m| pool.mods.iter().any(|(id, w)| id == m && *w > 0.0)).unwrap(),
                crafted: false,
                fractured: false,
            })
            .collect();
    assert!(lock.applies(&ctx, &(&game, &item)));
    run(&ctx, lock.steps(), &mut item, &mut Rng::new(0));
    let operations: Vec<&dyn Operation> = vec![&Currency::Fracturing, lock.as_ref(), finish.as_ref()];
    let target = Target::compile(&game, &Goal { base: Some("Opal Sceptre".into()), ..Goal::default() }).unwrap();
    let lens = Lens::new(&game, kind, vec![target], &operations, &[&item]);
    let state = lens.project(&item);
    let expected = lens.run(&ctx, Currency::Fracturing.steps(), &state);
    let mut fractured_lock = None;
    for seed in 0..100 {
        let mut next = item.clone();
        run(&ctx, Currency::Fracturing.steps(), &mut next, &mut Rng::new(seed));
        assert_eq!(next.crafted(), 1);
        let projected = lens.project(&next);
        assert_eq!(lens.view(&projected).crafted(), 1);
        assert!(!finish.applies(&ctx, &(&game, &next)));
        assert!(!finish.applies(&ctx, &lens.view(&projected)));
        assert!(expected.iter().any(|(p, t)| *p > 0.0 && *t == projected));
        if next.affixes.iter().any(|a| a.id == game.metamods[1] && a.fractured) {
            fractured_lock = Some(next);
        }
    }
    let mut item = fractured_lock.unwrap();
    let before = lens.project(&item);
    run(&ctx, &[Step::RemoveCrafted], &mut item, &mut Rng::new(0));
    assert_eq!(item.crafted(), 1);
    assert!(item.affixes.iter().any(|a| a.id == game.metamods[1] && a.fractured && a.crafted));
    assert_eq!(lens.run(&ctx, &[Step::RemoveCrafted], &before), vec![(1.0, lens.project(&item))]);
    run(&ctx, Currency::Scouring.steps(), &mut item, &mut Rng::new(0));
    assert_eq!(item.crafted(), 1);
    assert!(item.affixes.iter().any(|a| a.id == game.metamods[1] && a.fractured && a.crafted));
}
