//! The lens's exact distributions agree with sampling real items (SPEC immutable 5).

use std::collections::HashMap;
use std::path::PathBuf;

use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind};
use ponder_rules::guide::Guide;
use ponder_rules::item::Item;
use ponder_rules::lens::{Lens, State};
use ponder_rules::ops::{Ctx, Operation, parse, run};
use ponder_rules::rng::Rng;
use ponder_rules::target::Target;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn exact_outcomes_match_sampled_items() {
    let game = Game::new(Repoe::load(&root().join("fixtures/repoe")).unwrap());
    let assumptions = Assumptions::default();
    let ctx = Ctx { game: &game, assumptions: &assumptions };
    let text = std::fs::read_to_string(root().join("fixtures/guides/crit-mana-es-focused-amulet.toml")).unwrap();
    let guide: Guide = toml::from_str(&text).unwrap();
    let names = ["goal"].into_iter().chain(guide.stages.keys().map(String::as_str));
    let compiled: Vec<Target> = names.map(|n| Target::compile(&game, &guide.target(n).unwrap()).unwrap()).collect();
    let kind = Kind { base: game.base("Focused Amulet").unwrap(), ilvl: 86, influences: Influences::default() };
    let ops: Vec<Box<dyn Operation>> = [
        "transmutation",
        "alteration",
        "augmentation",
        "regal",
        "alchemy",
        "chaos",
        "exalted",
        "annulment",
        "scouring",
        "essence:deafening-woe",
        "bench:prefixes-cannot-be-changed",
        "bench:remove-crafted",
        "harvest:reforge-critical",
    ]
    .iter()
    .map(|n| parse(&game, "Amulet", n).unwrap())
    .collect();
    let refs: Vec<&dyn Operation> = ops.iter().map(|o| o.as_ref()).collect();
    let targets = compiled.len();
    let lens = Lens::new(&game, kind, compiled, &refs);

    let mut rng = Rng::new(3);
    let mut worst: (f64, String) = (0.0, String::new());
    let mut worst_tv: f64 = 0.0;
    for walk in 0..12 {
        let mut item = Item::new(kind);
        for _ in 0..walk {
            let usable: Vec<&dyn Operation> = refs.iter().copied().filter(|o| o.applies(&ctx, &(&game, &item))).collect();
            run(&ctx, usable[(rng.next_u64() % usable.len() as u64) as usize].steps(), &mut item, &mut rng);
        }
        let state = lens.project(&item);
        for op in &refs {
            if !op.applies(&ctx, &lens.view(&state)) {
                continue;
            }
            assert!(op.applies(&ctx, &(&game, &item)), "{} applies to the state but not the item", op.name());
            let exact: HashMap<State, f64> = lens.run(&ctx, op.steps(), &state).into_iter().map(|(p, s)| (s, p)).collect();
            let n = 4000;
            let mut seen: HashMap<State, f64> = HashMap::new();
            for _ in 0..n {
                let mut next = item.clone();
                run(&ctx, op.steps(), &mut next, &mut rng);
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
            worst_tv = worst_tv.max(distance);
            // What the solver uses: the chance each target holds afterwards.
            for t in 0..targets {
                let p: f64 = exact.iter().filter(|(s, _)| lens.holds(t, s)).map(|(_, p)| p).sum();
                let q: f64 = seen.iter().filter(|(s, _)| lens.holds(t, s)).map(|(_, p)| p).sum();
                let z = (p - q).abs() / ((p * (1.0 - p) / n as f64).sqrt() + 1e-3);
                if z > worst.0 {
                    worst = (z, format!("{} on walk {walk}, target {t}: exact {p:.4}, sampled {q:.4}", op.name()));
                }
            }
        }
    }
    println!(
        "lens: {} classes; worst z {:.2} ({}); worst total variation {worst_tv:.3} (mostly sampling noise)",
        lens.classes(),
        worst.0,
        worst.1
    );
    assert!(worst.0 < 4.0, "target chances disagree: z = {:.2}: {}", worst.0, worst.1);
}
