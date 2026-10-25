//! Recombinator plans.

mod common;
use common::*;

use std::collections::{BTreeSet, HashSet};
use std::sync::{Arc, OnceLock};

use ponder::gear::{self, Pricing};
use ponder::recomb::{self, Counts, Donor, Node, Planner, Step, Want};
use ponder_core::{Plan, Prices, Rng, Settings, Side, Target};
use ponder_game::mods::ModDB;
use serde_json::{Value, json};

// -- helpers ----------------------------------------------------------------------------

fn table() -> &'static Prices {
    static T: OnceLock<Prices> = OnceLock::new();
    T.get_or_init(|| fixture("prices.json.gz"))
}

const SETTINGS: Settings = Settings { chaos_per_hour: 1500.0 };
const FREE_CLICKS: Settings = Settings { chaos_per_hour: 0.0 };

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-12 + 1e-9 * a.abs().max(b.abs()), "{a} != {b}");
}

fn plan_costing(cost: f64) -> Option<Arc<Plan>> {
    Some(Arc::new(Plan { cost, ..Plan::default() }))
}

fn donor(base: &str, base_id: &str, carries: &[&str], cost: f64) -> Donor {
    let mut d = Donor::new(base, base_id, carries);
    d.plan = plan_costing(cost);
    d
}

const NONE: Counts = [(0, 0); 2];

fn counts(prefix: (i64, i64), suffix: (i64, i64)) -> Counts {
    [prefix, suffix]
}

fn leaf_node(d: &Donor) -> Node {
    Node::Donor(Arc::new(d.leaf()))
}

// -- recombinators ----------------------------------------------------------------------

#[test]
fn side_counts_are_distributions() {
    for (n, row) in recomb::SIDE_COUNTS.iter().enumerate() {
        approx(row.iter().map(|(_, p)| p).sum::<f64>(), 1.0);
        assert!(row.iter().map(|(k, _)| *k).max().unwrap() <= n.min(3), "{n}");
    }
}

#[test]
fn nnn_padding_raises_the_chance_to_keep_everything() {
    // Two wanted prefixes, one per donor: a pool of 2 keeps both a third of the time.
    let plain = recomb::chance(&counts((1, 0), (0, 0)), &counts((1, 0), (0, 0)), true);
    approx(plain, 0.33);
    // Two NNN prefixes on a partner base make a pool of 4: kept 2+ 90% of the time, but the base is a coin flip.
    let padded = recomb::chance(&counts((1, 0), (0, 0)), &counts((1, 2), (0, 0)), false);
    approx(padded, 0.5 * 0.9);
    // Sides multiply; a side with nothing wanted doesn't matter.
    let both = recomb::chance(&counts((1, 0), (1, 0)), &counts((1, 0), (0, 0)), true);
    approx(both, 0.33 * 0.59);
    assert_eq!(NONE, counts((0, 0), (0, 0)));
}

#[test]
fn step_cost_divides_by_the_chance() {
    let a = donor("A", "a", &["x"], 10.0);
    let b = donor("B", "b", &["y"], 20.0);
    let step = Step::new(leaf_node(&a), leaf_node(&b), 0.25, 0.0, 0.0, 0.0);
    approx(step.cost.unwrap(), 120.0);
    let kinds: Vec<String> = step.steps().into_iter().map(|s| s.0).collect();
    assert_eq!(kinds, ["craft", "craft", "recombine"]);
    // With clicking charged, the recombination's 15 seconds are added before dividing.
    let paid = Step::new(leaf_node(&a), leaf_node(&b), 0.25, 0.0, 0.0, 1500.0);
    approx(paid.cost.unwrap(), (30.0 + 15.0 * 1500.0 / 3600.0) / 0.25);
    // A donor without a plan, or no chance, has no cost.
    let unsolved = Donor::new("C", "c", &["z"]);
    assert!(Step::new(leaf_node(&a), leaf_node(&unsolved), 0.25, 0.0, 0.0, 0.0).cost.is_none());
    assert!(Step::new(leaf_node(&a), leaf_node(&b), 0.0, 0.0, 0.0, 0.0).cost.is_none());
}

#[test]
fn step_text_names_the_odds_salvage_and_doubling() {
    let a = donor("A", "a", &["life", "es"], 10.0);
    let b = donor("B", "b", &["es"], 20.0);
    let first = Step::new(leaf_node(&a), leaf_node(&b), 0.6, 0.0, 0.0, 0.0);
    assert_eq!(first.carries, ["life", "es"]);
    assert_eq!(first.doubled, ["es"]);
    let c = donor("C", "c", &["int"], 5.0);
    let top = Step::new(Node::Step(Arc::new(first)), leaf_node(&c), 0.59, 0.34, 0.001, 0.0);
    let steps = top.steps();
    assert_eq!(steps.len(), 5);
    assert_eq!(steps[0].1, "A: life + es");
    assert_eq!(steps[2].1, "#1 + #2 \u{2192} life + es  (60%; es doubled)");
    assert_eq!(steps[4].1, "#3 + #4 \u{2192} life + es + int  (59%; #3 back 34%)");
    assert_eq!(top.donors().len(), 3);
    let dict = top.to_dict();
    assert_eq!(dict["table"], "poewiki / Butsicles (Aug 2026)");
    assert_eq!(dict["salvage"], json!([0.34, 0.001]));
    assert_eq!(dict["steps"][0], json!({"do": "craft", "what": "A: life + es", "chaos": 10.0}));
}

fn target(db: &ModDB, pool: &[ponder_core::Mod], tags: &BTreeSet<String>, family: &str, name: &str) -> Target {
    let ids: BTreeSet<String> = pool
        .iter()
        .filter(|m| db.mods[m.id.as_str()].type_ == family && db.tier(&m.id, tags).unwrap() <= 2)
        .map(|m| m.id.clone())
        .collect();
    let side = pool.iter().find(|m| ids.contains(&m.id)).unwrap().side;
    let groups = pool.iter().filter(|m| ids.contains(&m.id)).flat_map(|m| m.groups.iter().cloned()).collect();
    Target { name: name.into(), side, good: ids, groups, notable: false }
}

#[test]
fn planner_builds_padded_leaves_and_a_tree() {
    common::setup();
    let table = table();
    let pricing = Pricing::new(table, &FREE_CLICKS);
    let db = db();
    let base = db.find_base("Warlock Gloves").unwrap().to_string();
    let tags = db.item_tags(&base, &BTreeSet::new());
    let pool = gear::pool(db, &base, &tags, 84);
    let targets = vec![target(db, &pool, &tags, "IncreasedLife", "life"), target(db, &pool, &tags, "Intelligence", "int")];
    let problem = gear::problem_spec_on(db, &base, targets, 84, &BTreeSet::new(), None, pricing).compile();
    let mut planner = Planner::new(db, &problem, &base, 84, pricing);
    assert!(planner.leaves().iter().any(|d| !d.on_target && !d.padding.is_empty())); // partner bases with NNN pads
    // padded donors wait for their problems until a padded tree could pay
    assert!(planner.leaves().iter().all(|d| d.padded == d.problem.is_none()));
    assert!(planner.leaves().iter().filter(|d| d.padded).all(|d| d.singles.len() == d.carries.len()));
    planner.prepare_padded(db, pricing);
    assert!(planner.leaves().iter().all(|d| d.problem.as_ref().unwrap().junk_cap == Some((0, 0)))); // donors must be clean
    for d in planner.leaves_mut() {
        // stand-in costs: padding costs more
        d.plan = plan_costing(5.0 + 3.0 * d.padding.values().sum::<usize>() as f64);
    }
    let best = planner.best().unwrap();
    let mut carries = best.carries.clone();
    carries.sort();
    assert_eq!(carries, ["int", "life"]);
    let plain = 10.0 / (0.59 * 0.59); // life on one donor, int on the other, both on the target base
    assert!(best.cost.unwrap() <= plain + 1e-9);
}

#[test]
fn filler_slots_get_distinct_resistance_families() {
    common::setup();
    let table = table();
    let pricing = Pricing::new(table, &FREE_CLICKS);
    let db = db();
    let base = db.find_base("Warlock Gloves").unwrap().to_string();
    let mut spec = gear::problem_spec_on(db, &base, vec![], 84, &BTreeSet::new(), None, pricing);
    let res: BTreeSet<String> = spec.pool.iter().filter(|m| gear::is_filler_family(&db.mods[&m.id].type_)).map(|m| m.id.clone()).collect();
    spec.filler = res;
    spec.filler_need = 3;
    let problem = spec.compile();
    let slots: Vec<Target> = recomb::wants(&problem).into_iter().map(|w| w.target.unwrap()).collect();
    assert!(slots.len() == 3 && slots.iter().all(|t| !t.good.is_empty()));
    let groups: Vec<BTreeSet<String>> =
        slots.iter().map(|t| t.good.iter().flat_map(|m| problem.mods()[m].groups.iter().cloned()).collect()).collect();
    // two donors can't both bring fire
    assert!(groups[0].is_disjoint(&groups[1]) && groups[0].is_disjoint(&groups[2]) && groups[1].is_disjoint(&groups[2]));
}

fn lists<'a>(prefix: &[&'a str], suffix: &[&'a str]) -> [Vec<&'a str>; 2] {
    [prefix.to_vec(), suffix.to_vec()]
}

#[test]
fn chance_matches_a_simulation_with_nnn_and_junk_free_donors() {
    let a = lists(&["life"], &["res"]);
    let b = lists(&["es", "nnn1", "nnn2"], &["nnn3"]);
    let legal: HashSet<&str> = ["life", "res", "es"].into();
    let sim = recomb::simulate(&a, &b, &legal, false, &legal, 40000, 0);
    let exact = recomb::chance(&counts((1, 0), (1, 0)), &counts((1, 2), (0, 1)), false);
    assert!((sim - exact).abs() < 0.01, "{sim} vs {exact}");
}

#[test]
fn failures_that_hand_back_an_input_cut_the_cost() {
    // life (prefix) + int (suffix) on two target-base donors: each side keeps its mod 59% of the time.
    let (a, b) = (counts((1, 0), (0, 0)), counts((0, 0), (1, 0)));
    approx(recomb::salvage(&a, &b, true), 0.59 * 0.41); // life kept, int lost
    // Exact salvage chances plus success and the rest add up over the four outcomes.
    let total = recomb::chance(&a, &b, true) + recomb::salvage(&a, &b, true) + recomb::salvage(&b, &a, true) + 0.41 * 0.41;
    approx(total, 1.0);
    let x = donor("A", "a", &["life"], 10.0);
    let y = donor("A", "a", &["int"], 10.0);
    let step = Step::new(leaf_node(&x), leaf_node(&y), 0.59 * 0.59, 0.59 * 0.41, 0.59 * 0.41, 0.0);
    approx(step.cost.unwrap(), 20.0 * (1.0 - 0.59 * 0.41) / (0.59 * 0.59));
}

#[test]
fn salvage_matches_a_simulation() {
    let a = lists(&["life", "es"], &["res"]);
    let b = lists(&["nnn1"], &["int", "nnn2"]);
    let legal: HashSet<&str> = ["life", "es", "res", "int"].into();
    let exact_set: HashSet<&str> = ["life", "es", "res"].into();
    let mut rng = Rng::new(1);
    let trials = 40000;
    let hits = (0..trials).filter(|_| recomb::draw(&mut rng, &a, &b, &legal, false).is_some_and(|kept| kept == exact_set)).count();
    let exact = recomb::salvage(&counts((2, 0), (1, 0)), &counts((0, 1), (1, 1)), false);
    assert!((hits as f64 / trials as f64 - exact).abs() < 0.01, "{} vs {exact}", hits as f64 / trials as f64);
}

#[test]
fn doubling_a_wanted_mod_pads_the_pool_on_the_target_base() {
    // Two prefixes wanted (life, es). Plain: pool 2 keeps both 33%. With life on both
    // donors the pool is 3: two picks always get both (life's group blocks its copy).
    let want = |name: &str, id: &str, group: &str| Want {
        name: name.into(),
        side: Side::Prefix,
        target: Some(Target {
            name: name.into(),
            side: Side::Prefix,
            good: [id.to_string()].into(),
            groups: [group.to_string()].into(),
            notable: false,
        }),
    };
    let one = |names: &[&str], cost: f64| donor("T", "t", names, cost);
    let leaves = vec![
        (vec!["life"], vec![one(&["life"], 10.0)]),
        (vec!["es"], vec![one(&["es"], 10.0)]),
        (vec!["es", "life"], vec![one(&["es", "life"], 12.0)]),
    ];
    let planner = Planner::from_leaves(vec![want("life", "l", "L"), want("es", "e", "E")], leaves, 0.0);
    let best = planner.best().unwrap();
    assert_eq!(best.doubled.len(), 1); // either mod, doubled, does it
    approx(best.chance, 0.60); // pool 3 keeps 2+ 60%
    approx(best.cost.unwrap(), (10.0 + 12.0) / 0.60); // beats (10 + 10) / 0.33 without doubling
}

#[test]
fn a_planner_without_a_plan_or_with_one_want_has_no_tree() {
    let w = |name: &str| Want { name: name.into(), side: Side::Prefix, target: None };
    let leaves = vec![(vec!["a"], vec![Donor::new("T", "t", &["a"])]), (vec!["b"], vec![Donor::new("T", "t", &["b"])])];
    // unsolved donors are dropped from the options
    assert!(Planner::from_leaves(vec![w("a"), w("b")], leaves, 0.0).best().is_none());
}

#[test]
fn combination_counts_follow_the_pool_table() {
    // Pools over 6 use row 6; an empty side keeps nothing; wanting more than is kept never succeeds.
    approx(recomb::keep_at_least(9, 2), 1.0);
    approx(recomb::keep_at_least(0, 0), 1.0);
    approx(recomb::keep_at_least(2, 3), 0.0);
    approx(recomb::keep_exactly(3, 3, 3), 0.10);
    approx(recomb::keep_exactly(4, 0, 0), 1.0);
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

#[test]
fn plan_solves_and_merges_in_one_call() {
    common::setup();
    let table = table();
    let pricing = Pricing::new(table, &SETTINGS);
    let db = db();
    let base = db.find_base("Warlock Gloves").unwrap().to_string();
    let tags = db.item_tags(&base, &BTreeSet::new());
    let pool = gear::pool(db, &base, &tags, 84);
    let targets = vec![target(db, &pool, &tags, "IncreasedLife", "life"), target(db, &pool, &tags, "Intelligence", "int")];
    let problem = gear::problem_spec_on(db, &base, targets, 84, &BTreeSet::new(), None, pricing).compile();
    let step = recomb::plan(db, &problem, &base, 84, pricing).expect("a tree for a small item");
    assert!(step.cost.unwrap() > 0.0 && step.donors().iter().all(|d| d.cost.is_some()));
    assert_eq!(strings(&json!(step.carries)).len(), step.carries.len());
}
#[test]
fn doubled_mod_simulation_excludes_the_other_copy() {
    let a = [vec!["life"], vec![]];
    let b = [vec!["life", "es"], vec![]];
    let legal: HashSet<_> = ["life", "es"].into_iter().collect();
    let simulated = recomb::simulate(&a, &b, &legal, true, &legal, 100_000, 7);
    assert!((simulated - 0.60).abs() < 0.01, "{simulated}");
}
