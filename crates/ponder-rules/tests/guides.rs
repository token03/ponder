//! Every guide recipe parses and names real things: bases, mod templates, tiers that can roll,
//! stages and mechanics. Prints which mechanics the recipes need (`-- --nocapture`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ponder_rules::data::{self, Repoe};
use ponder_rules::game::{Game, Influences, Kind, ModId};
use ponder_rules::goal::{Goal, Influence, ModReq, Source};
use ponder_rules::guide::Guide;
use ponder_rules::ops::parse;
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

fn raw_obtainable(repoe: &Repoe, guide: &Guide, goal: &Goal, req: &ModReq) -> bool {
    let Some(base) = repoe.base(goal.base.as_deref().unwrap_or(&guide.base)) else { return false };
    if !matches!(base.domain.as_str(), "misc" | "abyss_jewel" | "affliction_jewel") || req.source.is_some_and(|s| s != Source::Natural) {
        return false;
    }
    let tags: BTreeSet<_> = base.tags.iter().chain(&guide.base_tags).map(String::as_str).collect();
    req.text.iter().any(|text| {
        let matches: Vec<_> = repoe
            .mods
            .values()
            .filter(|m| {
                m.domain == base.domain
                    && matches!(m.generation_type.as_str(), "prefix" | "suffix")
                    && m.text.as_ref().is_some_and(|t| data::template(t).eq_ignore_ascii_case(text))
                    && m.spawn_weights.iter().find(|w| tags.contains(w.tag.as_str())).is_some_and(|w| w.weight > 0)
            })
            .collect();
        let levels: BTreeSet<_> = matches.iter().map(|m| m.required_level).collect();
        matches.iter().any(|m| {
            m.required_level <= goal.ilvl.unwrap_or(100) as u32
                && req.tier.is_none_or(|t| levels.iter().filter(|&&l| l > m.required_level).count() < t as usize)
        })
    })
}

#[test]
fn guides_name_real_things() {
    let repoe = Repoe::load(&root().join("fixtures/repoe")).unwrap();
    let game = Game::new(Repoe::load(&root().join("fixtures/repoe")).unwrap());
    let guides = guides();
    let mut problems = Vec::new();
    let mut tally: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut deferred = BTreeSet::new();
    let mut deferred_ops = BTreeSet::new();
    for (name, guide) in &guides {
        let mut local = Vec::new();
        if name.starts_with("ama-")
            && !guide.source.as_ref().is_some_and(|s| s.starts_with("https://www.reddit.com/r/PathOfExileBuilds/comments/"))
        {
            local.push("missing AMA source permalink".into());
        }
        for tag in &guide.base_tags {
            if !repoe.base(&guide.base).is_some_and(|b| b.domain == "affliction_jewel")
                || !repoe.mods.values().any(|m| m.spawn_weights.iter().any(|w| w.tag == *tag && w.weight > 0))
            {
                local.push(format!("invalid additional base tag {tag:?}"));
            }
        }
        let targets = std::iter::once("goal").chain(guide.stages.keys().map(String::as_str));
        for target in targets {
            let goal = guide.target(target).unwrap();
            match Target::compile(&game, &goal) {
                Err(_) if !goal.mods.is_empty() && goal.mods.iter().all(|req| raw_obtainable(&repoe, guide, &goal, req)) => {
                    deferred.insert(name);
                }
                Err(e) => local.push(format!("{target}: {e}")),
                Ok(t) => {
                    for (need, req) in t.needs.iter().zip(&goal.mods) {
                        let known = KNOWN.iter().any(|k| k.0 == name && (k.1 == target || k.1 == "*"));
                        if !known && !need.ok.iter().any(|m| obtainable(&game, &guide.base_name(), *m)) {
                            if raw_obtainable(&repoe, guide, &goal, req) {
                                deferred.insert(name);
                            } else {
                                local.push(format!("{target}: {:?} cannot appear on {}", req.text, guide.base));
                            }
                        }
                    }
                }
            }
        }
        let mut items = Vec::new();
        for step in &guide.steps {
            if !items.contains(&step.item) {
                items.push(step.item.clone());
            }
        }
        for (i, step) in guide.steps.iter().enumerate() {
            if step.ops.is_empty() {
                local.push(format!("step {}: empty operation list", i + 1));
            }
            if let Some(until) = &step.until
                && until != "goal"
                && !guide.stages.contains_key(until)
            {
                local.push(format!("step {}: unknown stage {until:?}", i + 1));
            }
            if step.fallback.is_some_and(|f| f == 0 || f > guide.steps.len()) {
                local.push(format!("step {}: fallback out of range", i + 1));
            }
            if name.starts_with("ama-") {
                let Some(base) = game.base(&guide.base) else { continue };
                let class = &game.bases[base.0 as usize].class;
                let avoid = guide
                    .goal
                    .mods
                    .iter()
                    .flat_map(|r| &r.text)
                    .flat_map(|t| game.with_template(t))
                    .flat_map(|m| game.def(*m).groups.iter().copied())
                    .collect::<Vec<_>>();
                for op in &step.ops {
                    if let Some(pair) = op.strip_prefix("recombinator:") {
                        let valid = pair.split_once('+').is_some_and(|(a, b)| {
                            let current = items.iter().position(|s| *s == step.item).unwrap();
                            [a, b].iter().all(|parent| items.iter().position(|s| s == parent).is_some_and(|p| p < current))
                        });
                        if !valid {
                            local.push(format!("step {}: invalid donor dependencies in {op:?}", i + 1));
                        }
                    }
                    if !op.starts_with("buy:") && !op.starts_with("recombinator:") && parse(&game, class, op, &avoid).is_none() {
                        let raw_fossils = op.strip_prefix("fossil:").is_some_and(|s| {
                            let names: Vec<_> = s.split('+').collect();
                            (1..=4).contains(&names.len())
                                && names.iter().all(|n| repoe.fossils.values().any(|f| f.name.eq_ignore_ascii_case(&format!("{n} fossil"))))
                        });
                        if raw_fossils {
                            deferred_ops.insert(format!("{name}: {op}"));
                        } else {
                            local.push(format!("step {}: unparsed operation {op:?}", i + 1));
                        }
                    }
                }
            }
        }
        for target in guide.steps.iter().flat_map(|s| &s.ops).filter_map(|op| op.strip_prefix("buy:")) {
            if target != "base" && !guide.stages.contains_key(target) {
                local.push(format!("buys unknown stage {target:?}"));
            }
            if name.starts_with("ama-")
                && target != "base"
                && !guide.stages.get(target).and_then(|s| s.price).is_some_and(|p| p.is_finite() && p >= 0.0)
            {
                local.push(format!("missing explicit starting price for {target:?}"));
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
    println!("{} reference guides require affix domains not yet simulated: {deferred:?}", deferred.len());
    println!("{} source operations are not yet simulated: {deferred_ops:?}", deferred_ops.len());
    let mut by_use: Vec<_> = tally.iter().collect();
    by_use.sort_by_key(|(m, users)| (std::cmp::Reverse(users.len()), *m));
    for (mechanic, users) in by_use {
        println!("{:3}  {mechanic}", users.len());
    }
    problems.sort();
    problems.dedup();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn curated_ama_references_have_equivalent_guides() {
    let book = std::fs::read_to_string(root().join("docs/crafting-ama-recipes.md")).unwrap();
    let guides = guides();
    for reference in book.lines().filter_map(|line| line.strip_prefix("**Reference:** `").and_then(|s| s.split('`').next())) {
        assert!(guides.iter().any(|(name, _)| name == reference), "missing guide for {reference}");
    }
    for (name, guide) in guides.iter().filter(|(name, _)| name.starts_with("ama-")) {
        let reference = name.strip_suffix("-fractured").unwrap_or(name);
        assert!(book.contains(&format!("`{reference}`")), "unlisted reference {name}");
        assert!(guide.source.as_ref().is_some_and(|s| book.contains(s)), "unlisted source for {name}");
    }
}
