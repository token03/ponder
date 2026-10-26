//! The lens's exact distributions agree with sampling real items (SPEC immutable 5).

use std::collections::HashMap;
use std::path::PathBuf;

use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind};
use ponder_rules::guide::Guide;
use ponder_rules::item::Item;
use ponder_rules::lens::{Lens, State};
use std::sync::Arc;

use ponder_rules::ops::recombinator::Recombinator;
use ponder_rules::ops::{Ctx, Operation, Supply, parse, run};
use ponder_rules::rng::Rng;
use ponder_rules::target::Target;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// (guide, base, item class, operations besides the basic currency, stage whose plainest item
/// starts the walks instead of a plain base).
type Case = (&'static str, &'static str, &'static str, &'static [&'static str], Option<&'static str>);
const CASES: &[Case] = &[
    (
        "crit-mana-es-focused-amulet",
        "Focused Amulet",
        "Amulet",
        &["essence:deafening-woe", "bench:prefixes-cannot-be-changed", "bench:remove-crafted", "harvest:reforge-critical"],
        None,
    ),
    (
        "bv-ele-gloves",
        "Phantom Mitts",
        "Gloves",
        &[
            "harvest:reforge-life",
            "fracturing",
            "fossil:aetheric+frigid+hollow+sanctified",
            "bench:suffixes-cannot-be-changed",
            "veiled-chaos",
            "unveil:area-of-effect",
            "bench:convert-cold",
        ],
        None,
    ),
    (
        "dex-armour-chaos-gloves",
        "Titan Gauntlets",
        "Gloves",
        &[
            "essence:deafening-envy",
            "eldritch-implicit:lesser-ember",
            "eldritch-chaos",
            "eldritch-exalted",
            "eldritch-annulment",
            "bench:armour",
        ],
        None,
    ),
    (
        "life-plus-two-phys-focused-amulet",
        "Focused Amulet",
        "Amulet",
        &["harvest:reforge-life", "bench:cannot-roll-attack", "harvest:add-remove-physical", "influence-exalted:crusader"],
        None,
    ),
    (
        "dex-armour-chaos-gloves",
        "Titan Gauntlets",
        "Gloves",
        &["essence:deafening-sorrow", "eldritch-implicit:lesser-ember", "bench:armour", "eldritch-annulment"],
        Some("resist"),
    ),
    (
        "minion-plus-two-wand",
        "Convoking Wand",
        "Wand",
        &[
            "fossil:jagged+metallic+shuddering+corroded",
            "bench:prefixes-cannot-be-changed",
            "bench:remove-crafted",
            "veiled-chaos",
            "bench:chaos-damage-over-time-multiplier",
            "unveil:minion-cast-speed",
            "bench:trigger-on-skill-use",
        ],
        None,
    ),
];

const CURRENCY: [&str; 10] =
    ["transmutation", "alteration", "augmentation", "regal", "alchemy", "chaos", "exalted", "annulment", "scouring", "fracturing"];

#[test]
fn exact_outcomes_match_sampled_items() {
    let game = Game::new(Repoe::load(&root().join("fixtures/repoe")).unwrap());
    for (case, &(file, base, class, extra, start)) in CASES.iter().enumerate() {
        agree(&game, file, base, class, extra, start, case as u64);
    }
}

fn agree(game: &Game, file: &str, base: &str, class: &str, extra: &[&str], start: Option<&str>, seed: u64) {
    let assumptions = Assumptions::default();
    let ctx = Ctx { game, assumptions: &assumptions };
    let text = std::fs::read_to_string(root().join(format!("fixtures/guides/{file}.toml"))).unwrap();
    let guide: Guide = toml::from_str(&text).unwrap();
    let names: Vec<&str> = ["goal"].into_iter().chain(guide.stages.keys().map(String::as_str)).collect();
    let compiled: Vec<Target> = names.iter().map(|n| Target::compile(game, &guide.target(n).unwrap()).unwrap()).collect();
    let kind = Kind { base: game.base(base).unwrap(), ilvl: 86, influences: Influences::default() };
    let ops: Vec<Box<dyn Operation>> =
        CURRENCY.iter().chain(extra).map(|n| parse(game, class, n, &[]).unwrap_or_else(|| panic!("{n}"))).collect();
    let refs: Vec<&dyn Operation> = ops.iter().map(|o| o.as_ref()).collect();
    let targets = compiled.len();
    let lens = Lens::new(game, kind, compiled, &refs, &[]);

    let mut rng = Rng::new(3 + seed);
    let mut worst: (f64, String) = (0.0, String::new());
    let mut worst_tv: (f64, String) = (0.0, String::new());
    for walk in 0..12 {
        let mut item = match start {
            Some(stage) => lens_start(game, kind, &guide, &names, stage),
            None => Item::new(kind),
        };
        for _ in 0..walk {
            // From a stage, walk the guide's own loop: its extra ops and exalts.
            let looped = |o: &&dyn Operation| start.is_none() || o.name() == "exalted" || extra[1..].contains(&o.name().as_str());
            let usable: Vec<&dyn Operation> = refs.iter().copied().filter(looped).filter(|o| o.applies(&ctx, &(game, &item))).collect();
            run(&ctx, usable[(rng.next_u64() % usable.len() as u64) as usize].steps(), &mut item, &mut rng);
        }
        for op in &refs {
            compare(&ctx, &lens, *op, &item, targets, &format!("walk {walk}"), &mut rng, &mut worst, &mut worst_tv);
        }
    }
    println!(
        "{file}: {} classes; worst z {:.2} ({}); worst total variation {:.3} ({})",
        lens.classes(),
        worst.0,
        worst.1,
        worst_tv.0,
        worst_tv.1
    );
    assert!(worst.0 < 4.0, "{file}: target chances disagree: z = {:.2}: {}", worst.0, worst.1);
}

/// Compare one operation on one item: the lens's exact outcomes against 4000 real samples,
/// keeping the worst target z-score and total variation seen.
#[allow(clippy::too_many_arguments)]
fn compare(
    ctx: &Ctx,
    lens: &Lens,
    op: &dyn Operation,
    item: &Item,
    targets: usize,
    label: &str,
    rng: &mut Rng,
    worst: &mut (f64, String),
    worst_tv: &mut (f64, String),
) {
    let game = ctx.game;
    let state = lens.project(item);
    if !op.applies(ctx, &lens.view(&state)) {
        assert!(!op.applies(ctx, &(game, item)), "{} applies to the item but not the state", op.name());
        return;
    }
    assert!(op.applies(ctx, &(game, item)), "{} applies to the state but not the item", op.name());
    let exact: HashMap<State, f64> = lens.run(ctx, op.steps(), &state).into_iter().map(|(p, s)| (s, p)).collect();
    let n = 4000;
    let mut seen: HashMap<State, f64> = HashMap::new();
    for _ in 0..n {
        let mut next = item.clone();
        run(ctx, op.steps(), &mut next, rng);
        *seen.entry(lens.project(&next)).or_default() += 1.0 / n as f64;
    }
    let distance: f64 = exact
        .keys()
        .chain(seen.keys())
        .collect::<std::collections::HashSet<_>>()
        .iter()
        .map(|s| (exact.get(*s).unwrap_or(&0.0) - seen.get(*s).unwrap_or(&0.0)).abs())
        .sum::<f64>()
        / 2.0;
    if std::env::var("DIFF").is_ok() && distance > 0.05 {
        let mut d: Vec<(f64, &State)> =
            exact.keys().chain(seen.keys()).map(|s| (exact.get(s).unwrap_or(&0.0) - seen.get(s).unwrap_or(&0.0), s)).collect();
        d.sort_by(|a, b| b.0.abs().total_cmp(&a.0.abs()));
        d.dedup_by(|a, b| a.1 == b.1);
        println!("{label}: item {:?}", item.affixes.iter().map(|a| game.def(a.id).key.clone()).collect::<Vec<_>>());
        for (x, s) in d.iter().take(6) {
            let classes: Vec<String> = (0..s.counts.len())
                .filter(|&c| s.counts[c] > 0)
                .map(|c| format!("{}×{}", s.counts[c], lens.describe(c % lens.classes()).chars().take(90).collect::<String>()))
                .collect();
            println!("  {x:+.3} {:?} {classes:?}", s.rarity);
        }
    }
    if distance > worst_tv.0 {
        *worst_tv = (distance, format!("{} on {label}", op.name()));
    }
    // What the solver uses: the chance each target holds afterwards.
    for t in 0..targets {
        let p: f64 = exact.iter().filter(|(s, _)| lens.holds(t, s)).map(|(_, p)| p).sum();
        let q: f64 = seen.iter().filter(|(s, _)| lens.holds(t, s)).map(|(_, p)| p).sum();
        let z = (p - q).abs() / ((p * (1.0 - p) / n as f64).sqrt() + 1e-3);
        if z > worst.0 {
            *worst = (z, format!("{} on {label}, target {t}: exact {p:.4}, sampled {q:.4}", op.name()));
        }
    }
}

/// Supply items for recombination tests: a fixed handful, equally likely.
struct Handful {
    items: Vec<Item>,
}

impl Supply for Handful {
    fn name(&self) -> String {
        "handful".into()
    }
    fn price(&self) -> f64 {
        1.0
    }
    fn items(&self) -> &[Item] {
        &self.items
    }
    fn item(&self, rng: &mut Rng) -> Item {
        self.items[rng.below(self.items.len())].clone()
    }
}

#[test]
fn recombination_matches_sampled_items() {
    let game = Game::new(Repoe::load(&root().join("fixtures/repoe")).unwrap());
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: &game, assumptions: &assumptions };
    let text = std::fs::read_to_string(root().join("fixtures/guides/cheap-high-es-regalia.toml")).unwrap();
    let guide: Guide = toml::from_str(&text).unwrap();
    let names: Vec<&str> = ["goal"].into_iter().chain(guide.stages.keys().map(String::as_str)).collect();
    let compiled: Vec<Target> = names.iter().map(|n| Target::compile(&game, &guide.target(n).unwrap()).unwrap()).collect();
    let kind = Kind { base: game.base("Twilight Regalia").unwrap(), ilvl: 86, influences: Influences::default() };
    let mut rng = Rng::new(11);
    let walk = |rng: &mut Rng, ops: &[&str]| {
        let mut item = Item::new(kind);
        for name in ops {
            let op = parse(&game, "Body Armour", name, &[]).unwrap();
            if op.applies(&ctx, &(&game, &item)) {
                run(&ctx, op.steps(), &mut item, rng);
            }
        }
        item
    };
    // Magic and rare partners, some with a crafted suffix.
    let plans: [&[&str]; 4] =
        [&["transmutation", "augmentation"], &["transmutation", "bench:any-suffix"], &["alchemy"], &["transmutation", "regal"]];
    let partner = Arc::new(Handful { items: plans.iter().map(|p| walk(&mut rng, p)).collect() });
    let mut ops: Vec<Box<dyn Operation>> =
        CURRENCY.iter().chain(&["bench:any-suffix"]).map(|n| parse(&game, "Body Armour", n, &[]).unwrap()).collect();
    ops.push(Box::new(Recombinator::new("Body Armour", "a+b", partner.clone())));
    let refs: Vec<&dyn Operation> = ops.iter().map(|o| o.as_ref()).collect();
    let lens = Lens::new(&game, kind, compiled, &refs, &[]);
    let recombine = *refs.last().unwrap();
    let (mut worst, mut worst_tv) = ((0.0, String::new()), (0.0, String::new()));
    for (i, plan) in plans.iter().chain(&[&["alchemy", "exalted"][..], &[][..]]).enumerate() {
        let item = walk(&mut rng, plan);
        compare(&ctx, &lens, recombine, &item, names.len(), &format!("item {i}"), &mut rng, &mut worst, &mut worst_tv);
    }
    println!("recombination: worst z {:.2} ({}); worst total variation {:.3} ({})", worst.0, worst.1, worst_tv.0, worst_tv.1);
    // Untracked mods conflict by chance in the lens, so whole outcomes need not match one
    // partner set; what the solver reads must.
    assert!(worst.0 < 4.0, "target chances disagree: z = {:.2}: {}", worst.0, worst.1);
}

/// A witness item of a stage: where a walk starts when sampling items the empty base rarely reaches.
fn lens_start(game: &Game, kind: Kind, guide: &Guide, names: &[&str], stage: &str) -> Item {
    let i = names.iter().position(|n| *n == stage).unwrap();
    Target::compile(game, &guide.target(names[i]).unwrap()).unwrap().witness(game, kind).unwrap()
}
