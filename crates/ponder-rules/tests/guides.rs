//! Every guide recipe parses and names real things: bases, mod templates, tiers that can roll,
//! stages and mechanics. Prints which mechanics the recipes need (`-- --nocapture`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind, ModId};
use ponder_rules::goal::{Influence, Source};
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

/// Requirements that name mods their base cannot get, until the mechanic that explains them
/// exists: (guide, stage, why).
const KNOWN: &[(&str, &str, &str)] = &[
    ("chill-convert-gloves", "quality_ready", "M4: the donor is another glove base"),
    ("necrotic-dmg-shocked", "armour_only", "M4: the donor is another body armour base"),
    ("plus-two-minion-gloves", "mana_base", "M4: the donor is another glove base"),
    ("plus-two-minion-fire-chaos-armour-gloves", "mana_base", "M4: the donor is another glove base"),
    ("int-mana-stacking-simplex", "*", "M5: Simplex influence pools"),
    ("dagger-plus-two-explode-synth", "*", "M5: synthesis implicits"),
    ("plus-one-phys-spell-crit-shield", "*", "M5: corrupted +1 socketed gems on the imprint"),
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

/// Can some mechanic put `m` on this base: rolling (natural, influence, fossil, veiled), an
/// essence, the bench, or a legacy or elevated mod bought or made elsewhere?
fn obtainable(game: &Game, base: &str, m: ModId) -> bool {
    let base = game.base(base).unwrap();
    let class = &game.bases[base.0 as usize].class;
    let def = game.def(m);
    match def.source {
        Source::Essence => game.essences.iter().any(|e| e.mods.get(class).is_some_and(|k| k.as_deref() == Some(def.key.as_str()))),
        Source::Crafted => {
            game.bench.iter().any(|b| b.item_classes.contains(class) && b.actions.add_explicit_mod.as_deref() == Some(def.key.as_str()))
        }
        Source::Legacy | Source::Elevated => true,
        _ => Influence::ALL.iter().any(|i| {
            let kind = Kind { base, ilvl: 100, influences: Influences::of(&[*i]) };
            game.pool(kind).mods.iter().any(|(id, _)| *id == m)
        }),
    }
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
            let goal = guide.target(target).unwrap();
            match Target::compile(&game, &goal) {
                Err(e) => local.push(format!("{target}: {e}")),
                Ok(t) => {
                    for (need, req) in t.needs.iter().zip(&goal.mods) {
                        let known = KNOWN.iter().any(|k| k.0 == name && (k.1 == target || k.1 == "*"));
                        if !known && !need.ok.iter().any(|m| obtainable(&game, &guide.base, *m)) {
                            local.push(format!("{target}: {:?} cannot appear on {}", req.text, guide.base));
                        }
                    }
                }
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
