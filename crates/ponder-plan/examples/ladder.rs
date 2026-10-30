use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use ponder_plan::market::Market;
use ponder_plan::session::{Error as SessionError, Method, Request, Session};
use ponder_plan::supply::Stock;
use ponder_rules::assumptions::Assumptions;
use ponder_rules::data::Repoe;
use ponder_rules::game::{Game, Influences, Kind, ModId};
use ponder_rules::goal::{Goal, Influence, ModReq, Rarity, Source};
use ponder_rules::item::{Affix, Item};
use ponder_rules::ops::currency::Currency;
use ponder_rules::ops::{Ctx, Operation, Step, Supply, catalogue, parse};
use ponder_rules::target::Target;
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn mods(game: &Game, value: &Value) -> Result<Vec<ModId>> {
    value
        .as_array()
        .ok_or("modifier list must be an array")?
        .iter()
        .map(|value| {
            let name = value.as_str().ok_or("modifier id must be a string")?;
            game.mod_id(name).ok_or_else(|| format!("unknown modifier {name}").into())
        })
        .collect()
}

fn item(game: &Game, kind: Kind, mods: &[ModId], fractured: &[ModId]) -> Item {
    let mut item = Item::new(kind);
    item.rarity = Rarity::Rare;
    item.affixes =
        mods.iter().map(|&id| Affix { id, crafted: game.def(id).source == Source::Crafted, fractured: fractured.contains(&id) }).collect();
    item
}

fn solve(case: &Value) -> Result<Value> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let game = Game::new(Repoe::load(&root.join("fixtures/repoe"))?);
    let market = Market::load(&root.join("fixtures/prices.json.gz"))?;
    let assumptions = Assumptions::default();
    let name = case["base"].as_str().ok_or("case has no base")?;
    let base = game.base(name).ok_or_else(|| format!("unknown base {name}"))?;
    let ilvl = u8::try_from(case["ilvl"].as_u64().ok_or("case has no item level")?)?;
    let influences: Vec<Influence> = serde_json::from_value(case["influences"].clone())?;
    let kind = Kind { base, ilvl, influences: Influences::of(&influences) };
    let required = mods(&game, &case["mods"])?;
    let fractured = mods(&game, &case["fractured"])?;
    let requirements = required
        .iter()
        .map(|&m| {
            let def = game.def(m);
            let tier = match def.source {
                Source::Natural | Source::Influence | Source::Essence => {
                    Some(game.tier(kind, m).ok_or_else(|| format!("{} cannot roll on {name} at level {ilvl}", def.key))?)
                }
                _ => None,
            };
            let source = match def.source {
                Source::Natural | Source::Influence | Source::Essence => None,
                source => Some(source),
            };
            Ok(ModReq {
                text: vec![def.template.clone()],
                tier,
                count: 1,
                fractured: case["require_fractures"].as_bool().unwrap_or(false) && fractured.contains(&m),
                source,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let goal = Target::compile(
        &game,
        &Goal { base: Some(name.into()), ilvl: Some(ilvl), influences, rarity: Some(Rarity::Rare), mods: requirements, ..Goal::default() },
    )?;
    if !goal.holds(&game, &item(&game, kind, &required, &fractured)) {
        return Err("derived goal rejects the observed affixes".into());
    }
    let wanted = goal.needs.iter().flat_map(|n| n.ok.iter().copied()).collect();
    let class = &game.bases[base.0 as usize].class;
    let mut methods: Vec<_> = Currency::ALL
        .iter()
        .map(|op| Method::Named(op.name()))
        .chain(std::iter::once(Method::Named("bench:remove-crafted".into())))
        .chain(catalogue(&game, class, &wanted).iter().map(|op| Method::Named(op.name())))
        .collect();
    if let Some(candidates) = case["candidate_methods"].as_array() {
        let avoid: Vec<_> = wanted.iter().flat_map(|&m| game.def(m).groups.iter().copied()).collect();
        for name in candidates.iter().filter_map(Value::as_str) {
            if let Some(operation) = parse(&game, class, name, &avoid) {
                let target = operation.steps().iter().any(|step| match step {
                    Step::Craft(mods) => mods.iter().any(|m| wanted.contains(m)),
                    Step::Unveil(preferences) => preferences.iter().flatten().any(|m| wanted.contains(m)),
                    _ => false,
                });
                let general = matches!(
                    name,
                    "bench:prefixes-cannot-be-changed"
                        | "bench:suffixes-cannot-be-changed"
                        | "bench:cannot-roll-attack"
                        | "bench:cannot-roll-caster"
                        | "bench:multimod"
                        | "bench:any-prefix"
                        | "bench:any-suffix"
                        | "veiled-chaos"
                        | "veiled-exalted"
                );
                if target || general {
                    methods.push(Method::Named(name.into()));
                }
            }
        }
    }
    if let Some(extra) = case["methods"].as_array() {
        for method in extra {
            methods.push(Method::Named(method.as_str().ok_or("method must be a string")?.into()));
        }
    }
    if case["donor"].is_array() {
        let donor = mods(&game, &case["donor"])?;
        let price = case["donor_price"].as_f64().ok_or("donor has no price")?;
        let supply: Arc<dyn Supply> = Arc::new(Stock::new("observed fragment", price, item(&game, kind, &donor, &fractured)));
        methods.push(Method::Recombine("observed fragment".into(), supply));
    }
    let start = if case["start"].is_array() { item(&game, kind, &mods(&game, &case["start"])?, &fractured) } else { Item::new(kind) };
    let starts: Vec<Arc<dyn Supply>> = vec![Arc::new(Stock::new("input", market.base, start))];
    let mut session = Session::new(Ctx { game: &game, assumptions: &assumptions }, &market);
    let clock = Instant::now();
    let result = session.solve(Request { kind, goal: goal.clone(), methods, starts }, 200_000);
    let solve_seconds = clock.elapsed().as_secs_f64();
    let plan = match result {
        Ok(plan) => plan,
        Err(error) => {
            let status = match &error {
                SessionError::Unsupported(_) => "unsupported",
                SessionError::Solve(ponder_solve::Error::TooManyStates(_)) => "state-limit",
                SessionError::Solve(ponder_solve::Error::Unreachable) => "unreachable",
            };
            return Ok(json!({"status": status, "case": case, "solve_s": solve_seconds, "error": format!("{error:?}")}));
        }
    };
    eprintln!(
        "{}",
        json!({"phase": "solved", "name": case["name"], "solve_s": solve_seconds, "cost_c": plan.policy.cost, "states": plan.policy.explored})
    );
    let stats = &plan.policy.stats;
    let mut report = json!({
        "status": "solved",
        "case": case,
        "solve_s": solve_seconds,
        "cost_c": plan.policy.cost,
        "explored": plan.policy.explored,
        "policy_states": plan.policy.states.len(),
        "actions": plan.policy.actions,
        "outcomes": plan.policy.outcomes,
        "explore_s": stats.explore,
        "prune_s": stats.prune,
        "values_s": stats.values,
        "separation_s": stats.separation,
        "solves": stats.solves,
        "newton": stats.newton,
    });
    if case["timing_only"].as_bool().unwrap_or(false) {
        return Ok(report);
    }
    let steps = case["simulation_steps"].as_u64().map(usize::try_from).transpose()?.unwrap_or(200_000);
    let clock = Instant::now();
    let followed = plan.craft().follow_with_limit(&plan.policy, 500, 1_000_000, steps);
    let simulation_seconds = clock.elapsed().as_secs_f64();
    let bad_items = followed.items.iter().filter(|item| !goal.holds(&game, item)).count();
    let valid =
        followed.lost == 0 && bad_items == 0 && (followed.mean - plan.policy.cost).abs() <= 4.0 * followed.error + 0.05 * plan.policy.cost;
    let status = if bad_items > 0 || followed.lost > followed.truncated {
        "validation-failed"
    } else if followed.truncated > 0 {
        "simulation-limit"
    } else if valid {
        "passed"
    } else {
        "validation-failed"
    };
    let simulation = json!({
        "status": status,
        "simulation_s": simulation_seconds,
        "simulation_mean": (followed.lost == 0).then_some(followed.mean),
        "simulation_error": (followed.lost == 0).then_some(followed.error),
        "lost": followed.lost,
        "truncated": followed.truncated,
        "simulation_steps_per_run": steps,
        "simulation_runs": 500,
        "invalid_items": bad_items
    });
    report.as_object_mut().unwrap().extend(simulation.as_object().unwrap().clone());
    Ok(report)
}

fn run() -> Result<Value> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: ladder CASES.json INDEX")?;
    let index: usize = args.next().ok_or("usage: ladder CASES.json INDEX")?.parse()?;
    if args.next().is_some() {
        return Err("usage: ladder CASES.json INDEX".into());
    }
    let cases: Vec<Value> = serde_json::from_slice(&std::fs::read(path)?)?;
    solve(cases.get(index).ok_or("case index is out of range")?)
}

fn main() -> std::process::ExitCode {
    let result = run().unwrap_or_else(|error| json!({"status": "input-error", "error": error.to_string()}));
    println!("{result}");
    if result["status"] == "passed" || result["status"] == "solved" {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}
