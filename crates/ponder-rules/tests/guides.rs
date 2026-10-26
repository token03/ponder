//! Every guide recipe parses and names real things: bases, mod templates, tiers that can roll,
//! stages and mechanics. Prints which mechanics the recipes need (`-- --nocapture`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ponder_rules::data::Repoe;
use ponder_rules::game::Game;
use ponder_rules::guide::Guide;
use ponder_rules::target::Target;

/// Operation vocabulary of the guides. A mechanic here is not necessarily implemented.
const MECHANICS: &[&str] = &[
    "buy",
    "transmutation",
    "alteration",
    "augmentation",
    "regal",
    "alchemy",
    "chaos",
    "exalted",
    "annulment",
    "scouring",
    "essence",
    "fossil",
    "bench",
    "harvest",
    "fracturing",
    "eldritch-implicit",
    "eldritch-chaos",
    "eldritch-exalted",
    "eldritch-annulment",
    "influence-orb",
    "influence-exalted",
    "veiled-chaos",
    "veiled-exalted",
    "unveil",
    "recombinator",
    "awakener",
    "dominance",
    "imprint",
    "beast",
    "remembrance",
    "chromatic",
    "catalyst",
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn guides() -> Vec<(String, Guide)> {
    let dir = root().join("fixtures/guides");
    let mut out: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            let guide = toml::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
            (name, guide)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn guides_name_real_things() {
    let game = Game::new(Repoe::load(&root().join("fixtures/repoe")).unwrap());
    let guides = guides();
    let mut problems = Vec::new();
    let mut tally: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (name, guide) in &guides {
        let mut local = Vec::new();
        let targets = std::iter::once("goal").chain(guide.stages.keys().map(String::as_str));
        for target in targets {
            if let Err(e) = Target::compile(&game, &guide.target(target).unwrap()) {
                local.push(format!("{target}: {e}"));
            }
        }
        for (i, step) in guide.steps.iter().enumerate() {
            if let Some(until) = &step.until
                && until != "goal"
                && !guide.stages.contains_key(until)
            {
                local.push(format!("step {}: unknown stage {until:?}", i + 1));
            }
            if step.fallback.is_some_and(|f| f == 0 || f > guide.steps.len()) {
                local.push(format!("step {}: fallback out of range", i + 1));
            }
        }
        for target in guide.steps.iter().flat_map(|s| &s.ops).filter_map(|op| op.strip_prefix("buy:")) {
            if target != "base" && !guide.stages.contains_key(target) {
                local.push(format!("buys unknown stage {target:?}"));
            }
        }
        for mechanic in guide.mechanics() {
            if !MECHANICS.contains(&mechanic) {
                local.push(format!("unknown mechanic {mechanic:?}"));
            }
            tally.entry(mechanic).or_default().insert(name);
        }
        problems.extend(local.into_iter().map(|p| format!("{name}: {p}")));
    }
    println!("{} recipes from {} sheets", guides.len(), guides.iter().map(|g| &g.1.sheet).collect::<BTreeSet<_>>().len());
    let mut by_use: Vec<_> = tally.iter().collect();
    by_use.sort_by_key(|(m, users)| (std::cmp::Reverse(users.len()), *m));
    for (mechanic, users) in by_use {
        println!("{:3}  {mechanic}", users.len());
    }
    problems.sort();
    problems.dedup();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
