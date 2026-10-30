//! candidate: archetype helpers on small made-up inputs.

use ponder::candidate::{Archetype, Counter};
use ponder_game::mods::IdMods;

#[path = "../../ponder-game/tests/common/mod.rs"]
mod common;

#[test]
fn mined_goals_keep_an_observed_combination_and_allow_the_solver_to_choose_fracturing() {
    let db = common::db();
    let item = db.match_ids(
        "Bone Ring",
        &IdMods { explicit: vec!["IncreasedLife7".into()], fractured: vec!["IncreasedEnergyShield10".into()], ..Default::default() },
        Some(84),
        &Default::default(),
        "RARE",
        "",
        &Default::default(),
    );
    assert!(item.mods.iter().any(|m| m.fractured()));
    let keys = ponder::candidate::stat_keys(&item).into_iter().map(str::to_owned).collect();
    let mut archetype = Archetype::new("Ring", keys, 1, 1);
    archetype.examples.push(std::sync::Arc::new(item.clone()));
    let goal = ponder::candidate::representative(db, &archetype).unwrap();
    assert_eq!(goal.mods.iter().map(|m| &m.mod_id).collect::<Vec<_>>(), item.mods.iter().map(|m| &m.mod_id).collect::<Vec<_>>());
    assert!(goal.mods.iter().all(|m| !m.fractured()));
    assert!(item.mods.iter().any(|m| m.fractured()));
    assert!(goal.finish.is_none());
}

#[test]
fn niche_and_usual_tier() {
    let mut a = Archetype::new("g", vec!["x".into()], 25, 40);
    a.lift = 1.0;
    assert_eq!(a.niche(), 0.0);
    a.lift = std::f64::consts::E;
    assert!((a.niche() - 5.0).abs() < 1e-12);
    for t in [1, 1, 2, 3] {
        a.tiers.entry("x".into()).or_default().add(t);
    }
    assert_eq!(a.usual_tier("x", 0.5), Some(1));
    assert_eq!(a.usual_tier("x", 0.75), Some(2));
    assert_eq!(a.usual_tier("x", 1.0), Some(3));
    assert_eq!(a.usual_tier("y", 0.5), None);
}

#[test]
fn counter_most_common_keeps_insertion_order_on_ties() {
    let mut c = Counter::new();
    for k in ["b", "a", "c", "a", "c"] {
        c.add(k.to_string());
    }
    c.add("d".to_string());
    assert_eq!(c.most_common().iter().map(|(k, n)| (k.as_str(), *n)).collect::<Vec<_>>(), [("a", 2), ("c", 2), ("b", 1), ("d", 1)]);
}
