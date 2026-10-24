//! Problem building: the pieces of the crafting, pipeline and recombinator tests that touch
//! actions, gear and cluster.

mod common;
use common::*;

use ponder_core::ActionKind;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use ponder::gear::{self, BuildOpts, Pricing};
use ponder::{actions, cluster};
use ponder_core::*;
use ponder_game::mods::IdMods;
use serde_json::{Value, json};


// -- helpers ----------------------------------------------------------------------------

fn table() -> &'static Prices {
    static T: OnceLock<Prices> = OnceLock::new();
    T.get_or_init(|| serde_json::from_value(fixture("prices.json.gz")).unwrap())
}

const SETTINGS: Settings = Settings {
    chaos_per_hour: 1500.0,
};

fn pricing() -> Pricing<'static> {
    Pricing::new(table(), &SETTINGS)
}


// -- crafting ---------------------------------------------------------------------------

fn craft_prices() -> Prices {
    [
        ("transmute", 0.1),
        ("alt", 1.0),
        ("aug", 0.2),
        ("regal", 3.0),
        ("exalted", 50.0),
        ("annul", 20.0),
        ("alch", 0.5),
        ("chaos", 2.0),
        ("scour", 1.0),
        ("lifeforce", 0.1),
        ("fossil", 5.0),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), *v))
    .collect()
}

const FREE: Settings = Settings {
    chaos_per_hour: 0.0,
};

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn cluster_two_prefix_notables_need_a_rare() {
    common::setup();
    let prices = craft_prices();
    let (problem, plan) = cluster::cost(
        "medium",
        "area damage",
        &names(&["Vast Power", "Assert Dominance"]),
        Pricing::new(&prices, &FREE),
        None,
        84,
        &[],
    )
    .unwrap();
    assert!(problem.targets.iter().all(|t| t.side == Side::Prefix));
    // a magic jewel holds one prefix, so every plan must make the item rare
    assert!(
        ["regal", "alch", "chaos", "scour+alch"]
            .iter()
            .any(|a| plan.usage.get(*a).is_some_and(|u| *u > 0.0))
    );
    let simulated = simulate(&problem, &plan, &prices, &FREE, 4000, 2, 100_000);
    assert!(
        (simulated - plan.cost).abs() <= 0.15 * plan.cost,
        "{simulated} vs {}",
        plan.cost
    );
}

#[test]
fn cluster_notable_cap() {
    common::setup();
    let err = cluster::cost(
        "medium",
        "area damage",
        &names(&["Vast Power", "Assert Dominance", "Magnifier"]),
        Pricing::new(&craft_prices(), &FREE),
        None,
        84,
        &[],
    );
    assert!(err.unwrap_err().to_string().contains("at most 2 notables"));
}

#[test]
fn gear_targets_respect_tier_and_essences() {
    common::setup();
    let item = ponder_game::item::parse_item(
        "Rarity: RARE\nX\nTwo-Toned Boots\nItem Level: 84\nImplicits: 0\n+117 to maximum Life\n30% increased Movement Speed\n",
    )
    .unwrap();
    let m = db().match_item(&item);
    let gp = gear::build(
        db(),
        &m,
        Pricing::new(&craft_prices(), &FREE),
        BuildOpts::default(),
    )
    .unwrap();
    let life = gp
        .problem
        .targets
        .iter()
        .find(|t| t.name.contains("Life"))
        .unwrap();
    assert!(
        life.good
            .iter()
            .all(|id| db().mods[id.as_str()].required_level >= 54)
    ); // T1 only on boots
    assert!(
        gp.problem
            .actions
            .iter()
            .any(|a| a.name.contains("essence"))
    );
    let problem = gp.problem.clone();
    let plan = solve(&problem, &craft_prices(), &FREE).unwrap(); // essences unpriced here, so left out
    assert!(!plan.unpriced.is_empty() && plan.cost > 0.0);
}

// -- pipeline ---------------------------------------------------------------------------

#[test]
fn best_base_of_a_real_item() {
    common::setup();
    let path = repo_root().join("fixtures/real/occultist_winter_orb.txt");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let build = ponder_game::pob::load_text(&text).unwrap();
    let gloves = build
        .items
        .iter()
        .filter_map(|i| ponder_game::item::parse_item(&i.text).ok())
        .find(|i| i.base == "Warlock Gloves" && i.rarity == "RARE")
        .unwrap();
    let m = db().match_item(&gloves);
    let grp = gear::group(db(), &m);
    assert_eq!(grp.label, "Gloves (ES)");
    assert_eq!(grp.best(), "Warlock Gloves");
    assert!(grp.bases.len() > 5);
    let gp = gear::build(
        db(),
        &m,
        Pricing::new(&craft_prices(), &FREE),
        BuildOpts::default(),
    )
    .unwrap();
    assert!(!gp.problem.targets.is_empty());
}

#[test]
fn finished_item_from_a_ladder_archetype() {
    // archetypes become finished items: belts worn as life + armour + 3 suffixes
    // cost as that, not as the mods everyone shares.
    common::setup();
    let ids = IdMods {
        explicit: names(&["IncreasedLife6", "IncreasedPhysicalDamageReductionRating5"]),
        ..Default::default()
    };
    let mut m = db().match_ids(
        "Heavy Belt",
        &ids,
        None,
        &BTreeSet::new(),
        "RARE",
        "",
        &Default::default(),
    );
    let order = [
        "ChaosResistance",
        "FireResistance",
        "ColdResistance",
        "LightningResistance",
        "Strength",
    ];
    let families: serde_json::Map<String, Value> = order
        .iter()
        .map(|f| (f.to_string(), json!(if *f == "Strength" { 5 } else { 3 })))
        .collect();
    let sides = (2 - m.prefixes().count(), 3 - m.suffixes().count());
    m.finish = Some(json!({"families": families, "sides": [sides.0, sides.1], "order": order}));
    let gp = gear::build(
        db(),
        &m,
        Pricing::new(table(), &SETTINGS),
        BuildOpts {
            item_level: Some(84),
            ..Default::default()
        },
    )
    .unwrap();
    let p = &gp.problem;
    assert_eq!(p.targets.len() + p.filler_need + gp.bench.len(), 5); // a full item
    assert!(!gp.bench.is_empty() && !p.reserve.is_empty()); // one open suffix is a bench resistance
    assert!(
        p.filler
            .iter()
            .any(|x| db().mods[x.as_str()].type_ == "ColdResistance")
    );
    assert!(
        p.filler
            .iter()
            .any(|x| db().mods[x.as_str()].type_ == "Strength")
    );
}

// -- recombinators ----------------------------------------------------------------------

#[test]
fn problem_on_a_plain_base() {
    common::setup();
    let base = db().find_base("Warlock Gloves").unwrap().to_string();
    let tags = db().item_tags(&base, &BTreeSet::new());
    let pool = gear::pool(db(), &base, &tags, 84);
    assert!(!pool.is_empty());
    let target = |family: &str, name: &str| {
        let ids: BTreeSet<String> = pool
            .iter()
            .filter(|m| {
                db().mods[m.id.as_str()].type_ == family
                    && db().tier(&m.id, &tags).unwrap_or(99) <= 2
            })
            .map(|m| m.id.clone())
            .collect();
        let side = pool.iter().find(|m| ids.contains(&m.id)).unwrap().side;
        let groups = pool
            .iter()
            .filter(|m| ids.contains(&m.id))
            .flat_map(|m| m.groups.iter().cloned())
            .collect();
        Target {
            name: name.into(),
            side,
            good: ids,
            groups,
            notable: false,
        }
    };
    let targets = vec![
        target("IncreasedLife", "life"),
        target("Intelligence", "int"),
    ];
    let p = gear::problem_spec_on(
        db(),
        &base,
        targets,
        84,
        &BTreeSet::new(),
        Some((0, 0)),
        pricing(),
    );
    assert_eq!(p.junk_cap, Some((0, 0)));
    assert_eq!((p.need, p.kind, p.pool.len()), (2, Kind::Gear, pool.len()));
    assert!(p.actions.iter().any(|a| a.name.contains("fossil")));
    // Donors are clean one- or two-mod items: no metamods or Harvest add/remove.
    assert!(!p.actions.iter().any(|a| matches!(a.kind.effect(), ponder_core::Effect::Meta(_)) || a.kind == ActionKind::Augment));
    assert!(p.actions.iter().any(|a| a.name.starts_with("eldritch")));
    // Influenced bases lose the eldritch currencies.
    let shaped = gear::problem_spec_on(
        db(),
        &base,
        Vec::new(),
        84,
        &BTreeSet::from(["shaper".to_string()]),
        None,
        pricing(),
    );
    assert!(
        !shaped
            .actions
            .iter()
            .any(|a| a.name.starts_with("eldritch"))
    );
}

// -- smaller facts -----------------------------------------------------------------------

#[test]
fn spawn_tags_and_enchants() {
    common::setup();
    assert!(gear::spawn_tags().contains("ring") && gear::spawn_tags().contains("default"));
    let e = cluster::find_enchant("large", "minion damage").unwrap();
    assert_eq!(e.short, "Minions deal 10% increased Damage");
    assert!(cluster::find_enchant("large", "zzz").is_err());
    let p = cluster::problem(
        "large",
        "minion damage",
        &names(&["HaveIncreasedEffect@1", "GrantMinionAttackAndCastSpeed@1"]),
        None,
        84,
        &[],
    )
    .unwrap();
    let effect = p
        .targets
        .iter()
        .find(|t| t.name.contains("Effect"))
        .unwrap();
    assert!(effect.name == "35% increased Effect" && effect.good.len() == 1); // T1 only
    assert_eq!(
        cluster::resolve(&p.pool, &names(&["HaveIncreasedEffect@2"])).unwrap()[0]
            .good
            .len(),
        2
    ); // T2+: 25% or 35%
    let fixed = cluster::resolve_fixed(&p.pool, &names(&["HaveIncreasedEffect@1"])).unwrap();
    assert!(fixed[0].label.ends_with("35% increased Effect"));
}

#[test]
fn bench_costs_and_essences() {
    common::setup();
    let cost = actions::bench_cost("Gloves", actions::LOCK_PREFIXES).unwrap();
    assert!(!cost.is_empty());
    assert!(actions::bench_cost("Gloves", "NoSuchMod").is_none());
    let (acts, forced) = actions::essences("Gloves", 84, 6);
    assert_eq!(acts.len(), forced.len());
    assert!(acts.iter().all(|a| a.forced.is_some()));
    assert!(acts.iter().all(|a| a.from_rarity == [NORMAL, RARE] && a.cost.len() == 1));
    let mods: BTreeMap<_, _> = forced.into_iter().collect();
    assert!(!mods.is_empty());
}

#[test]
fn high_tier_essences_reforge_rares_without_a_scour() {
    common::setup();
    let (actions, _) = actions::essences("Gloves", 84, 6);
    let action = actions.iter().find(|a| a.name == "deafening greed essence").unwrap();
    let id = action.forced.as_ref().unwrap();
    let mod_ = Mod::new(id, Side::Prefix, 0.0, &[id], false, "life", &[]);
    let target = Target {
        name: "life".into(), side: Side::Prefix, good: BTreeSet::from([id.clone()]),
        groups: BTreeSet::from([id.clone()]), notable: false,
    };
    let mut problem = ProblemSpec::new(vec![], vec![target], 1, vec![action.clone()], Kind::Gear);
    problem.extra_mods.insert(id.clone(), mod_);
    problem.begin = Some(State::new(RARE, &[MISSING], &[]));
    let problem = problem.compile();
    let prices = Prices::from([("deafening-essence-of-greed".into(), 2.0), ("scour".into(), 100.0)]);
    let plan = solve(&problem, &prices, &FREE).unwrap();
    assert!((plan.cost - 2.0).abs() < 1e-9);
    assert_eq!(plan.usage["deafening greed essence"], 1.0);
    assert!(action.from_rarity.contains(&NORMAL));
    assert!(!action.from_rarity.contains(&MAGIC));
}
#[test]
fn current_essences_support_corrupted_types_and_weapon_subclasses() {
    common::setup();
    for (subclass, parent) in [("Rune Dagger", "Dagger"), ("Warstaff", "Staff")] {
        let (sub, _) = actions::essences(subclass, 84, 6);
        let (parent, _) = actions::essences(parent, 84, 6);
        assert!(!sub.is_empty());
        assert_eq!(sub.len(), parent.len());
    }
    let (essences, _) = actions::essences("Helmet", 84, 6);
    for name in ["horror essence", "delirium essence", "hysteria essence", "insanity essence"] {
        assert!(essences.iter().any(|a| a.name == name), "{name}");
    }
    let (low, _) = actions::essences("Gloves", 84, 1);
    assert!(low.iter().any(|a| a.profile.starts_with("essence-ilvl:")));
}

#[test]
fn essence_life_must_meet_the_requested_tier() {
    common::setup();
    for (target, shrieking, deafening) in [("IncreasedLife12", false, false), ("IncreasedLife11", false, true), ("IncreasedLife10", true, true)] {
        let m = db().match_ids("Glorious Plate", &IdMods { explicit: vec![target.into()], ..Default::default() }, Some(86), &Default::default(), "RARE", "", &Default::default());
        let gp = gear::build(db(), &m, pricing(), BuildOpts::default()).unwrap();
        let good = &gp.problem.targets[0].good;
        assert_eq!(good.contains("IncreasedLife10"), shrieking, "{target}");
        assert_eq!(good.contains("IncreasedLife11"), deafening, "{target}");
    }
}

#[test]
fn harvest_prices_include_rancour_and_weights_include_generation() {
    common::setup();
    let base = db().find_base("Prophecy Wand").unwrap();
    let tags = db().item_tags(base, &Default::default());
    let pool = gear::pool(db(), base, &tags, 84);
    let physical = pool.iter().find(|m| m.id == "LocalIncreasedPhysicalDamagePercentAndAccuracyRating1").unwrap();
    assert_eq!(physical.weight, 500.0);
    let minion = Mod::new("minion", Side::Prefix, 1.0, &["minion"], false, "minion", &["minion"]);
    let (actions, _) = actions::harvest(&[minion]);
    assert!(actions[0].cost.contains(&("sandswept-lifeforce".into(), 3.0)));
    assert!(actions::basic().iter().find(|a| a.name == "annul").unwrap().from_rarity.contains(&MAGIC));
}
#[test]
fn real_bench_crafts_cannot_finish_over_conflicting_natural_mods() {
    common::setup();
    let m = db().match_ids("Bone Ring", &IdMods { explicit: vec!["IncreasedEnergyShield10".into()], crafted: vec!["EinharMasterIncreasedLife3".into()], ..Default::default() }, Some(84), &Default::default(), "RARE", "", &Default::default());
    let gp = gear::build(db(), &m, pricing(), BuildOpts { exact: true, ..Default::default() }).unwrap();
    let existing: Vec<_> = ["IncreasedEnergyShield10", "IncreasedLife1"].iter().map(|id| gp.problem.pool.iter().find(|m| m.id == *id).unwrap()).collect();
    assert!(!gp.problem.done(&gp.problem.state_of(RARE, existing)));
}
