use std::path::PathBuf;
use std::sync::OnceLock;

use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind};
use ponder_rules::goal::{Goal, Source};
use ponder_rules::ops::essence::Essence;
use ponder_rules::target::Target;

fn game() -> &'static Game {
    static GAME: OnceLock<Game> = OnceLock::new();
    GAME.get_or_init(|| Game::new(Repoe::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/repoe")).unwrap()))
}

fn target(base: &str, requirement: &str) -> Target {
    let goal: Goal = toml::from_str(&format!("base = {base:?}\nilvl = 86\nmods = [{requirement}]\n")).unwrap();
    Target::compile(game(), &goal).unwrap()
}

#[test]
fn bench_mods_only_satisfy_explicitly_crafted_requirements() {
    let text = "#% increased Attack Speed / +# to Dexterity and Intelligence";
    let ordinary = target("Spine Bow", &format!("{{ text = {text:?}, tier = \"any\" }}"));
    let crafted = target("Spine Bow", &format!("{{ text = {text:?}, source = \"crafted\" }}"));
    let unveiled = target("Spine Bow", &format!("{{ text = {text:?}, source = \"veiled\" }}"));
    for &m in &crafted.needs[0].ok {
        assert!(!ordinary.needs[0].ok.contains(&m));
        assert!(!unveiled.needs[0].ok.contains(&m));
    }
    assert!(unveiled.needs[0].ok.iter().all(|m| game().def(*m).source == Source::Veiled));
    assert!(ordinary.needs[0].ok.iter().any(|m| game().def(*m).source == Source::Veiled));
}

#[test]
fn physical_damage_quality_accepts_natural_and_essence_alternatives() {
    let goal = target("Runic Hatchet", "{ text = \"Adds # to # Physical Damage\", tier = 2 }");
    for key in ["LocalAddedPhysicalDamage9", "LocalAddedPhysicalDamage8", "LocalAddedPhysicalDamageEssenceNew7"] {
        assert!(goal.needs[0].ok.contains(&game().mod_id(key).unwrap()), "{key}");
    }
    assert!(!goal.needs[0].ok.contains(&game().mod_id("LocalAddedPhysicalDamage1").unwrap()));
    assert!(goal.needs[0].ok.iter().all(|m| game().def(*m).source != Source::Crafted));
}

#[test]
fn tier_defaults_to_two_and_can_be_overridden() {
    let template = "Adds # to # Physical Damage";
    let default = target("Runic Hatchet", &format!("{{ text = {template:?} }}"));
    let two = target("Runic Hatchet", &format!("{{ text = {template:?}, tier = 2 }}"));
    let one = target("Runic Hatchet", &format!("{{ text = {template:?}, tier = 1 }}"));
    let three = target("Runic Hatchet", &format!("{{ text = {template:?}, tier = 3 }}"));
    let any = target("Runic Hatchet", &format!("{{ text = {template:?}, tier = \"any\" }}"));
    assert_eq!(default, two);
    assert!(one.needs[0].ok.is_subset(&two.needs[0].ok));
    assert!(two.needs[0].ok.is_subset(&three.needs[0].ok));
    assert!(three.needs[0].ok.is_subset(&any.needs[0].ok));
    assert!(one.needs[0].ok.len() < two.needs[0].ok.len());
    assert!(two.needs[0].ok.len() < three.needs[0].ok.len());
    let low = game().mod_id("LocalAddedPhysicalDamage1").unwrap();
    assert!(any.needs[0].ok.contains(&low));
    assert!(!default.needs[0].ok.contains(&low));
    for tier in ["0", "-1", "\"best\""] {
        assert!(toml::from_str::<Goal>(&format!("mods = [{{ text = {template:?}, tier = {tier} }}]")).is_err());
    }
}

#[test]
fn influence_quality_does_not_require_a_preinfluenced_start() {
    let default = target("Leather Belt", "{ text = \"#% increased Attributes\" }");
    let goal: Goal =
        toml::from_str("base = \"Leather Belt\"\nilvl = 86\ninfluences = [\"hunter\"]\nmods = [{ text = \"#% increased Attributes\" }]")
            .unwrap();
    let influenced = Target::compile(game(), &goal).unwrap();
    let hunter = |target: &Target| {
        target.needs[0]
            .ok
            .iter()
            .copied()
            .filter(|m| game().def(*m).influence == Some(ponder_rules::goal::Influence::Hunter))
            .collect::<std::collections::HashSet<_>>()
    };
    assert!(!hunter(&default).is_empty());
    assert_eq!(hunter(&default), hunter(&influenced));
    assert!(default.needs[0].ok.iter().all(|m| game().def(*m).source == Source::Influence));
}

#[test]
fn essence_only_stats_have_base_specific_quality_ranks() {
    let game = game();
    let helmet = Kind { base: game.base("Giantslayer Helmet").unwrap(), ilvl: 86, influences: Influences::default() };
    let goal = target("Giantslayer Helmet", "{ text = \"#% increased Mana Reservation Efficiency of Skills\", tier = 2 }");
    for (name, rank) in [("deafening-loathing", 1), ("shrieking-loathing", 2), ("screaming-loathing", 3)] {
        let m = Essence::new(game, name, "Helmet").unwrap().forced();
        assert_eq!(game.tier(helmet, m), Some(rank), "{name}");
        assert_eq!(goal.needs[0].ok.contains(&m), rank <= 2, "{name}");
        let claw = Kind { base: game.base("Imperial Claw").unwrap(), ..helmet };
        assert_eq!(game.tier(claw, m), None);
    }
}
