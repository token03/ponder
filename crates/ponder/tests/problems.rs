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

// -- helpers ----------------------------------------------------------------------------

fn table() -> &'static Prices {
    static T: OnceLock<Prices> = OnceLock::new();
    T.get_or_init(|| serde_json::from_value(fixture("prices.json.gz")).unwrap())
}

const SETTINGS: Settings = Settings { chaos_per_hour: 1500.0 };

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

const FREE: Settings = Settings { chaos_per_hour: 0.0 };

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn cluster_two_prefix_notables_need_a_rare() {
    common::setup();
    let prices = craft_prices();
    let (problem, plan) =
        cluster::cost("medium", "area damage", &names(&["Vast Power", "Assert Dominance"]), Pricing::new(&prices, &FREE), None, 84, &[])
            .unwrap();
    assert!(problem.targets.iter().all(|t| t.side == Side::Prefix));
    // a magic jewel holds one prefix, so every plan must make the item rare
    assert!(["regal", "alch", "chaos", "scour+alch"].iter().any(|a| plan.usage.get(*a).is_some_and(|u| *u > 0.0)));
    let simulated = simulate(&problem, &plan, &prices, &FREE, 4000, 2, 100_000);
    assert!((simulated - plan.cost).abs() <= 0.15 * plan.cost, "{simulated} vs {}", plan.cost);
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
    let gp = gear::build(db(), &m, Pricing::new(&craft_prices(), &FREE), BuildOpts::default()).unwrap();
    let life = gp.problem.targets.iter().find(|t| t.name.contains("Life")).unwrap();
    assert!(life.good.iter().all(|id| db().mods[id.as_str()].required_level >= 54)); // T1 only on boots
    assert!(gp.problem.actions.iter().any(|a| a.name.contains("essence")));
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
    let gp = gear::build(db(), &m, Pricing::new(&craft_prices(), &FREE), BuildOpts::default()).unwrap();
    assert!(!gp.problem.targets.is_empty());
}

#[test]
fn armour_goals_require_suppression_and_chaos_with_flexible_elemental_resistance() {
    common::setup();
    let bid = db().find_base("Slink Boots").unwrap();
    let tags = db().item_tags(bid, &BTreeSet::new());
    let pool = gear::pool(db(), bid, &tags, 86);
    let best = |family: &str| {
        pool.iter().filter(|m| db().mods[&m.id].type_ == family).min_by_key(|m| db().tier(&m.id, &tags).unwrap_or(99)).unwrap().id.clone()
    };
    let ids: Vec<String> = ["IncreasedLife", "MovementVelocity", "ChanceToSuppressSpells", "ChaosResistance", "FireResistance"]
        .iter()
        .map(|family| best(family))
        .collect();
    let matched = db().match_ids(
        "Slink Boots",
        &IdMods { explicit: ids.clone(), ..Default::default() },
        Some(86),
        &BTreeSet::new(),
        "RARE",
        "",
        &Default::default(),
    );
    let gp = gear::build(db(), &matched, pricing(), BuildOpts::default()).unwrap();
    let p = &gp.problem;
    assert_eq!(p.filler_need, 1);
    assert!(p.filler.iter().all(|id| gear::is_filler_family(&db().mods[id].type_)));
    assert!(p.targets.iter().any(|t| t.good.contains(&ids[2])));
    assert!(p.targets.iter().any(|t| t.good.contains(&ids[3])));
    assert!(gp.rows.iter().all(|row| !matches!(row.role, "any" | "filler")));
    let read = |mods: &[String]| p.state_of(RARE, mods.iter().map(|id| &p.mods()[id]));
    assert!(p.done(&read(&ids)));
    let missing: Vec<_> = ids.iter().filter(|id| **id != ids[2]).cloned().collect();
    assert!(!p.done(&read(&missing)));
    for family in gear::FILLER_FAMILIES {
        let mut variant = ids.clone();
        variant[4] = best(family);
        assert!(p.done(&read(&variant)));
    }
    for m in pool.iter().filter(|m| gear::is_filler_family(&db().mods[&m.id].type_)) {
        assert_eq!(p.filler.contains(&m.id), db().tier(&m.id, &tags).unwrap_or(99) <= gear::FILLER_TIER);
    }
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
            .filter(|m| db().mods[m.id.as_str()].type_ == family && db().tier(&m.id, &tags).unwrap_or(99) <= 2)
            .map(|m| m.id.clone())
            .collect();
        let side = pool.iter().find(|m| ids.contains(&m.id)).unwrap().side;
        let groups = pool.iter().filter(|m| ids.contains(&m.id)).flat_map(|m| m.groups.iter().cloned()).collect();
        Target { name: name.into(), side, good: ids, groups, notable: false }
    };
    let targets = vec![target("IncreasedLife", "life"), target("Intelligence", "int")];
    let p = gear::problem_spec_on(db(), &base, targets, 84, &BTreeSet::new(), Some((0, 0)), pricing());
    assert_eq!(p.junk_cap, Some((0, 0)));
    assert_eq!((p.need, p.kind, p.pool.len()), (2, Kind::Gear, pool.len()));
    assert!(p.actions.iter().any(|a| a.name.contains("fossil")));
    assert!(p.actions.iter().any(|a| matches!(a.kind, ActionKind::ProtectedScour(_))));
    assert!(!p.actions.iter().any(|a| matches!(a.kind.effect(), ponder_core::Effect::Meta(_))));
    assert!(p.actions.iter().any(|a| a.kind == ActionKind::Augment));
    assert!(p.targets.iter().flat_map(|t| &t.good).all(|id| p.pool.iter().any(|m| &m.id == id)));
    assert!(!p.actions.iter().any(|a| matches!(a.kind, ActionKind::VeiledChaos | ActionKind::VeiledExalt)));
    assert!(p.actions.iter().any(|a| a.name.starts_with("eldritch")));
    // Influenced bases lose the eldritch currencies.
    let shaped = gear::problem_spec_on(db(), &base, Vec::new(), 84, &BTreeSet::from(["shaper".to_string()]), None, pricing());
    assert!(!shaped.actions.iter().any(|a| a.name.starts_with("eldritch")));
}

// -- smaller facts -----------------------------------------------------------------------

#[test]
fn spawn_tags_and_enchants() {
    common::setup();
    assert!(gear::spawn_tags().contains("ring") && gear::spawn_tags().contains("default"));
    let e = cluster::find_enchant("large", "minion damage").unwrap();
    assert_eq!(e.short, "Minions deal 10% increased Damage");
    assert!(cluster::find_enchant("large", "zzz").is_err());
    let p =
        cluster::problem("large", "minion damage", &names(&["HaveIncreasedEffect@1", "GrantMinionAttackAndCastSpeed@1"]), None, 84, &[])
            .unwrap();
    let effect = p.targets.iter().find(|t| t.name.contains("Effect")).unwrap();
    assert!(effect.name == "35% increased Effect" && effect.good.len() == 1); // T1 only
    assert_eq!(cluster::resolve(&p.pool, &names(&["HaveIncreasedEffect@2"])).unwrap()[0].good.len(), 2); // T2+: 25% or 35%
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
        name: "life".into(),
        side: Side::Prefix,
        good: BTreeSet::from([id.clone()]),
        groups: BTreeSet::from([id.clone()]),
        notable: false,
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
    for (target, shrieking, deafening) in
        [("IncreasedLife12", false, false), ("IncreasedLife11", false, true), ("IncreasedLife10", true, true)]
    {
        let m = db().match_ids(
            "Glorious Plate",
            &IdMods { explicit: vec![target.into()], ..Default::default() },
            Some(86),
            &Default::default(),
            "RARE",
            "",
            &Default::default(),
        );
        let gp = gear::build(db(), &m, pricing(), BuildOpts::default()).unwrap();
        let good = &gp.problem.targets[0].good;
        assert_eq!(good.contains("IncreasedLife10"), shrieking, "{target}");
        assert_eq!(good.contains("IncreasedLife11"), deafening, "{target}");
    }
}

#[test]
fn an_essence_observation_accepts_equivalent_or_stronger_natural_rolls() {
    common::setup();
    let m = db().match_ids(
        "Bone Ring",
        &IdMods { explicit: vec!["StrengthEssence7_".into()], ..Default::default() },
        Some(86),
        &Default::default(),
        "RARE",
        "",
        &Default::default(),
    );
    assert_eq!(m.mods[0].source, "essence");
    let gp = gear::build(db(), &m, pricing(), BuildOpts::default()).unwrap();
    let good = &gp.problem.targets[0].good;
    assert!(good.contains("StrengthEssence7_"));
    assert!(good.iter().any(|id| db().mods[id].spawn_weights.iter().any(|w| w.weight > 0)
        && db().mods[id].stats.iter().any(|s| s.id == "additional_strength" && s.min >= 51.0)));
    assert!(good.iter().all(|id| db().mods[id].stats.iter().any(|s| s.id == "additional_strength" && s.min >= 51.0)));
}

#[test]
fn a_hybrid_alternative_cannot_make_two_valid_prefixes_conflict() {
    common::setup();
    let ids = vec!["IncreasedEnergyShield4".into(), "IncreasedPhysicalDamageReductionRating5".into()];
    let m = db().match_ids(
        "Leather Belt",
        &IdMods { explicit: ids.clone(), ..Default::default() },
        Some(86),
        &Default::default(),
        "RARE",
        "",
        &Default::default(),
    );
    assert_eq!(m.mods.len(), 2);
    let gp = gear::build(db(), &m, pricing(), BuildOpts::default()).unwrap();
    let p = &gp.problem;
    let mods = p.mods();
    let state = p.state_of(RARE, ids.iter().map(|id| &mods[id]));
    assert!(p.done(&state));
    assert!(p.targets.iter().all(|t| !t.good.contains("JunMasterVeiledArmourAndEnergyShield")));
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
fn fossil_weight_profiles_are_shared_across_prices_and_keep_pool_identities_separate() {
    use actions::{FossilOpts, FossilRow, RESONATOR, fossil_combos_with};

    let pool = vec![
        Mod::new("wanted", Side::Prefix, 1.0, &["wanted"], false, "wanted", &[]),
        Mod::new("junk", Side::Prefix, 1.0, &["junk"], false, "junk", &[]),
    ];
    let rows = vec![
        FossilRow { name: "a".into(), pid: "a".into(), effects: vec![4.0.into(), 1.0.into()] },
        FossilRow { name: "b".into(), pid: "b".into(), effects: vec![1.0.into(), 4.0.into()] },
    ];
    let wanted = BTreeSet::from(["wanted".into()]);
    let opts = FossilOpts { mods_per_side: 1.0, max_fossils: 1, keep: 1 };
    let prices = Prices::from([("a".into(), 1.0), ("b".into(), 100.0), (RESONATOR.into(), 1.0)]);
    let (first, profiles) = fossil_combos_with(&rows, &pool, &[&wanted], &prices, &FREE, opts);
    assert_eq!(first[0].profile, "fossil:a");
    let original = profiles["fossil:a"].clone();
    let mut repriced = prices.clone();
    repriced.insert("a".into(), 100.0);
    repriced.insert("b".into(), 1.0);
    let (second, _) = fossil_combos_with(&rows, &pool, &[&wanted], &repriced, &FREE, opts);
    assert_eq!(second[0].profile, "fossil:b");
    let (_, profiles) = fossil_combos_with(&rows, &pool, &[&wanted], &prices, &FREE, opts);
    assert!(std::sync::Arc::ptr_eq(&original, &profiles["fossil:a"]));
    let mut renamed = pool.clone();
    renamed[0].id = "other wanted".into();
    renamed[1].id = "other junk".into();
    let wanted = BTreeSet::from(["other wanted".into()]);
    let (_, profiles) = fossil_combos_with(&rows, &renamed, &[&wanted], &prices, &FREE, opts);
    assert_eq!(profiles["fossil:a"]["other wanted"], 4.0);
    assert!(!profiles["fossil:a"].contains_key("wanted"));
    repriced.remove(RESONATOR);
    let wanted = BTreeSet::from(["wanted".into()]);
    let (actions, _) = fossil_combos_with(&rows, &pool, &[&wanted], &repriced, &FREE, opts);
    assert!(actions.is_empty());
}

#[test]
fn fossil_effects_follow_craft_of_exile_additive_boosts_and_multiplicative_reductions() {
    setup();
    let pool = vec![
        Mod::new("physical attack", Side::Prefix, 100.0, &["p"], false, "", &["physical", "attack"]),
        Mod::new("attack caster", Side::Prefix, 100.0, &["c"], false, "", &["attack", "caster"]),
        Mod::new("all three", Side::Prefix, 100.0, &["a"], false, "", &["physical", "attack", "caster"]),
    ];
    let rows = actions::fossil_rows(&pool);
    let effect = |ids: &[&str], modifier| {
        actions::fossil_multiplier(ids.iter().map(|id| rows.iter().find(|row| row.pid == *id).unwrap().effects[modifier]))
    };
    assert_eq!(effect(&["jagged-fossil", "serrated-fossil"], 0), 20.0);
    assert!((effect(&["aetheric-fossil"], 1) - 1.5).abs() < 1e-12);
    assert!((effect(&["jagged-fossil", "serrated-fossil", "aetheric-fossil"], 2) - 0.675).abs() < 1e-12);
    assert_eq!(effect(&["jagged-fossil", "metallic-fossil"], 0), 0.0);
    assert_eq!(effect(&["jagged-fossil"], 1), 1.0);
}

#[test]
fn component_goals_share_natural_pools_and_profiles_across_currency_prices() {
    common::setup();
    let base = db().find_base("Two-Toned Boots").unwrap();
    let first = gear::problem_spec_on(db(), base, Vec::new(), 84, &BTreeSet::new(), None, pricing());
    let mut changed = table().clone();
    changed.insert("chaos".into(), 7.0);
    let second = gear::problem_spec_on(db(), base, Vec::new(), 84, &BTreeSet::new(), None, Pricing::new(&changed, &SETTINGS));
    assert!(std::sync::Arc::ptr_eq(&first.pool, &second.pool));
    assert!(!first.profiles.is_empty());
    for (name, profile) in first.profiles.iter() {
        assert!(std::sync::Arc::ptr_eq(profile, &second.profiles[name]));
    }
}
#[test]
fn real_bench_crafts_cannot_finish_over_conflicting_natural_mods() {
    common::setup();
    let m = db().match_ids(
        "Bone Ring",
        &IdMods {
            explicit: vec!["IncreasedEnergyShield10".into()],
            crafted: vec!["EinharMasterIncreasedLife3".into()],
            ..Default::default()
        },
        Some(84),
        &Default::default(),
        "RARE",
        "",
        &Default::default(),
    );
    let gp = gear::build(db(), &m, pricing(), BuildOpts { exact: true, ..Default::default() }).unwrap();
    assert!(gp.bench.is_empty() && gp.problem.reserve.is_empty());
    assert!(gp.problem.actions.iter().any(|a| a.kind == ActionKind::FinishBench));
    let chaos = gp.problem.actions.iter().find(|a| a.name == "chaos").unwrap();
    assert!(gp.problem.outcomes(&State::new(RARE, &[], &[]), chaos).iter().any(|(_, s)| gp.problem.total(s) == 6 && !gp.problem.done(s)));
    let existing: Vec<_> =
        ["IncreasedEnergyShield10", "IncreasedLife1"].iter().map(|id| gp.problem.pool.iter().find(|m| m.id == *id).unwrap()).collect();
    assert!(!gp.problem.done(&gp.problem.state_of(RARE, existing)));
}

#[test]
fn experimented_jewellery_has_its_own_groups_and_data_driven_limits() {
    common::setup();
    for (base, limits) in [
        ("Simplex Amulet", [1, 2]),
        ("Focused Amulet", [2, 1]),
        ("Helical Ring", [1, 4]),
        ("Cogwork Ring", [2, 4]),
        ("Geodesic Ring", [4, 2]),
        ("Manifold Ring", [4, 1]),
        ("Composite Ring", [6, 0]),
        ("Ratcheting Ring", [0, 6]),
    ] {
        let id = db().find_base(base).unwrap();
        assert_eq!(gear::base_limits(db(), id), limits);
        let m = db().match_ids(base, &IdMods::default(), Some(84), &BTreeSet::new(), "RARE", "", &Default::default());
        let group = gear::group(db(), &m);
        assert_eq!(group.label, base);
        assert_eq!(group.bases, [base]);
        let spec = gear::problem_spec_on(db(), id, Vec::new(), 84, &BTreeSet::new(), None, pricing());
        assert_eq!(spec.affix_limits, Some(limits));
    }
    let m = db().match_ids("Gold Ring", &IdMods::default(), Some(84), &BTreeSet::new(), "RARE", "", &Default::default());
    assert!(!gear::group(db(), &m).bases.iter().any(|b| b == "Helical Ring" || b == "Composite Ring"));
}

#[test]
fn recombination_accepts_focused_and_simplex_goals_with_legal_donors() {
    common::setup();
    for (base, ids, limits) in [
        ("Focused Amulet", vec!["IncreasedLife7".into(), "IncreasedEnergyShield10".into(), "Strength9".into()], [2, 1]),
        ("Simplex Amulet", vec!["IncreasedLife7".into(), "Strength9".into(), "Dexterity9".into()], [1, 2]),
    ] {
        let m = db().match_ids(
            base,
            &IdMods { explicit: ids, ..Default::default() },
            Some(86),
            &Default::default(),
            "RARE",
            "",
            &Default::default(),
        );
        let gp = gear::build(db(), &m, pricing(), BuildOpts { exact: true, ..Default::default() }).unwrap();
        let planner = ponder::recomb::Planner::new(db(), &gp.problem, m.base_id.as_deref().unwrap(), 86, pricing());
        assert!(planner.ok, "{base}");
        let all = planner.want.iter().map(|w| w.name.clone()).collect();
        let state = planner.state_after(&gp.problem, &all);
        assert!(gp.problem.done(&state));
        assert_eq!([gp.problem.count(&state, Side::Prefix), gp.problem.count(&state, Side::Suffix)], limits);
        for (_, donors) in planner.leaf_sets() {
            for donor in donors {
                let p = donor.problem.as_ref().unwrap();
                assert!(
                    [Side::Prefix, Side::Suffix]
                        .iter()
                        .all(|&side| p.targets.iter().filter(|t| t.side == side).count() <= p.side_limit(RARE, side))
                );
            }
        }
    }
}

#[test]
fn veiled_targets_use_only_generic_unveils_on_the_actual_base() {
    common::setup();
    let bid = db().find_base("Sorcerer Boots").unwrap();
    let tags = db().item_tags(bid, &BTreeSet::new());
    let (id, _) = db()
        .mods
        .iter()
        .find(|(_, m)| {
            m.domain == "unveiled"
                && m.name == "Chosen"
                && m.required_level <= 84
                && ponder_game::mods::ModDB::generation_weight(m, &tags) > 0.0
        })
        .unwrap();
    let item = db().match_ids(
        "Sorcerer Boots",
        &IdMods { explicit: vec![id.clone()], ..Default::default() },
        Some(84),
        &BTreeSet::new(),
        "RARE",
        "",
        &Default::default(),
    );
    let built = gear::build(db(), &item, pricing(), BuildOpts::default()).unwrap();
    assert!(built.problem.targets.iter().any(|t| t.good.contains(id)));
    assert!(!built.problem.unveils.is_empty());
    assert!(built.problem.unveils.iter().all(|m| matches!(db().mods[&m.id].name.as_str(), "Chosen" | "of the Order")));
    assert!(built.problem.actions.iter().any(|a| a.kind == ActionKind::VeiledChaos));
    assert!(built.problem.actions.iter().any(|a| a.kind == ActionKind::VeiledExalt));
}

#[test]
fn beast_recipes_use_the_correct_red_beast() {
    common::setup();
    let actions = actions::beasts("Ring");
    assert_eq!(actions.iter().find(|a| a.kind == ActionKind::BeastPrefixToSuffix).unwrap().cost[0].0, "farric-lynx-alpha");
    assert_eq!(actions.iter().find(|a| a.kind == ActionKind::BeastSuffixToPrefix).unwrap().cost[0].0, "farric-wolf-alpha");
}

#[test]
fn awakener_donors_transfer_one_modifier_each_then_reforge() {
    common::setup();
    let base = db().find_base("Gold Amulet").unwrap();
    let natural = db().item_tags(base, &BTreeSet::new());
    let influences: BTreeSet<String> = BTreeSet::from(["shaper".into(), "hunter".into()]);
    let mut targets: Vec<Target> = Vec::new();
    for influence in &influences {
        let tags = db().item_tags(base, &BTreeSet::from([influence.clone()]));
        let (id, m) = db()
            .mods
            .iter()
            .find(|(_, m)| {
                m.domain == "item"
                    && matches!(m.generation_type.as_str(), "prefix" | "suffix")
                    && m.required_level <= 84
                    && ponder_game::mods::ModDB::generation_weight(m, &tags) > 0.0
                    && ponder_game::mods::ModDB::generation_weight(m, &natural) == 0.0
                    && targets.iter().all(|t| m.groups.iter().all(|g| !t.groups.contains(g)))
            })
            .unwrap();
        targets.push(Target {
            name: id.clone(),
            side: if m.generation_type == "prefix" { Side::Prefix } else { Side::Suffix },
            good: BTreeSet::from([id.clone()]),
            groups: m.groups.iter().cloned().collect(),
            notable: false,
        });
    }
    let p = gear::problem_spec_on(db(), base, targets, 84, &influences, None, pricing()).compile();
    let planner = ponder::awakener::Planner::new(db(), &p, base, 84, pricing());
    assert!(!planner.pairs.is_empty());
    for pair in &planner.pairs {
        assert_eq!(pair.donors[0].problem.as_ref().unwrap().influences.len(), 1);
        assert_eq!(pair.donors[1].problem.as_ref().unwrap().influences.len(), 1);
        assert!(pair.donors.iter().all(|d| d.problem.as_ref().unwrap().junk_cap == Some((0, 0))));
        assert!((pair.outcomes.iter().map(|o| o.0).sum::<f64>() - 1.0).abs() < 1e-8);
        assert!(pair.outcomes.iter().all(|(_, s)| pair.targets.iter().all(|&i| s.status[i] == GOOD) && p.total(s) >= 4));
    }
}
