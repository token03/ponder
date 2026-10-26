mod common;

use common::*;
use ponder::{
    actions,
    gear::{self, BuildOpts, Pricing},
};
use ponder_core::{Action, ActionKind, FOSSIL_SECONDS, NORMAL, Prices, RARE, Settings, Side, solve};
use ponder_game::item::parse_item;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Deserialize)]
struct GuideSuite {
    source: String,
    checked_on: String,
    game_version: String,
    max_seconds: f64,
    cases: Vec<GuideCase>,
}

#[derive(Deserialize)]
struct GuideCase {
    name: String,
    sheet: String,
    scope: String,
    item: String,
    targets: usize,
    fixed: usize,
    reserve_suffix: usize,
    guide_actions: Vec<String>,
    #[serde(default)]
    guide_fossils: Vec<String>,
}

#[test]
fn sourced_crafting_methods_remain_feasible_and_competitive_within_ten_seconds() {
    setup();
    let suite: GuideSuite = fixture("crafting-guides.json");
    assert_eq!(ponder_game::data::version(None).as_deref(), Some(suite.game_version.as_str()));
    assert!(suite.source.starts_with("https://docs.google.com/spreadsheets/"));
    assert!(!suite.checked_on.is_empty());
    let prices: Prices = fixture("prices.json.gz");
    let settings = Settings::default();
    let started = Instant::now();
    for case in suite.cases {
        let case_started = Instant::now();
        let item = parse_item(&case.item).unwrap();
        let matched = db().match_item(&item);
        let built = gear::build(db(), &matched, Pricing::new(&prices, &settings), BuildOpts { exact: true, ..Default::default() }).unwrap();
        assert!(built.skipped.is_empty(), "{}: {:?}", case.name, built.skipped);
        assert_eq!(built.problem.targets.len(), case.targets, "{}", case.name);
        assert_eq!(built.problem.fixed.len(), case.fixed, "{}", case.name);
        let mut spec = built.problem.to_spec();
        if case.reserve_suffix > 0 {
            spec.reserve.insert(Side::Suffix, case.reserve_suffix);
        }
        let problem = spec.compile();
        let mut reference = problem.to_spec();
        reference.actions.retain(|action| case.guide_actions.contains(&action.name));
        assert_eq!(reference.actions.len(), case.guide_actions.len(), "{}: guide actions unavailable", case.name);
        if !case.guide_fossils.is_empty() {
            let rows = actions::fossil_rows(&reference.pool);
            let fossils: Vec<_> = case.guide_fossils.iter().map(|id| rows.iter().find(|row| &row.pid == id).unwrap()).collect();
            let weights: BTreeMap<_, _> = reference
                .pool
                .iter()
                .enumerate()
                .map(|(i, modifier)| {
                    (modifier.id.clone(), modifier.weight * actions::fossil_multiplier(fossils.iter().map(|fossil| fossil.effects[i])))
                })
                .collect();
            std::sync::Arc::make_mut(&mut reference.profiles).insert("guide:fossils".into(), weights.into());
            let mut action = Action::new("guide fossil resonator", ActionKind::RerollRare, &[], &[NORMAL, RARE])
                .with_profile("guide:fossils")
                .with_seconds(2.0 + FOSSIL_SECONDS * fossils.len() as f64);
            action.cost = case.guide_fossils.iter().map(|id| (id.clone(), 1.0)).collect();
            action.cost.push((actions::RESONATORS[fossils.len()].into(), 1.0));
            reference.actions.push(action);
        }
        let reference = reference.compile();
        let guide = solve(&reference, &prices, &settings).unwrap_or_else(|error| panic!("{}: {error}", case.name));
        let plan = solve(&problem, &prices, &settings).unwrap_or_else(|error| panic!("{}: {error}", case.name));
        assert!(guide.cost.is_finite() && guide.cost > 0.0, "{}", case.name);
        assert!(plan.cost <= guide.cost * 1.05, "{}: {} versus guide method {}", case.name, plan.cost, guide.cost);
        assert!(plan.currency(&settings) >= 0.0, "{}", case.name);
        eprintln!(
            "{}: {:.3}s, {} states, {:.1}c versus {:.1}c reference method; {} / {}",
            case.name,
            case_started.elapsed().as_secs_f64(),
            plan.states,
            plan.cost,
            guide.cost,
            case.sheet,
            case.scope
        );
    }
    assert!(started.elapsed().as_secs_f64() < suite.max_seconds, "guide cases took {:?}", started.elapsed());
}
