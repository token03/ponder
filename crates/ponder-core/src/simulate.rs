use std::collections::HashMap;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::action::{Action, step_cost};
use crate::mechanics::{Mechanics, PreparedEffect};
use crate::problem::Problem;
use crate::solve::Plan;
use crate::types::*;

/// Average chaos spent replaying `plan` with real mods and exact group exclusion.
///
/// Panics if the policy has no action for a state the replay reaches.
pub fn simulate(problem: &Problem, plan: &Plan, prices: &Prices, settings: &Settings, runs: usize, seed: u64, max_steps: usize) -> f64 {
    let costs: HashMap<&str, f64> = problem.actions.iter().map(|a| (a.name.as_str(), step_cost(a, prices, settings))).collect();
    let mut total = 0.0;
    let lost = replay(problem, plan, runs, seed, max_steps, |_, a, _, _| total += costs[a.name.as_str()]);
    if let Some(s) = lost {
        panic!("the policy has no action for {s:?}");
    }
    total / runs as f64
}

/// Play `plan` out `runs` times on real mods, from a fresh base (with its fractured mods).
///
/// `on_click(state before, action, rarity after, mods after)` sees every action, so a caller can
/// watch what each click really rolls. A run stops when the item is done, after `max_steps`, or
/// at a state the policy has no action for: real mods can reach states the lumped weights rule
/// out (a slot's groups running dry). Returns the last such state, if any run met one.
pub fn replay<'p>(
    problem: &'p Problem,
    plan: &Plan,
    runs: usize,
    seed: u64,
    max_steps: usize,
    mut on_click: impl FnMut(&State, &'p Action, u8, &[&'p Mod]),
) -> Option<State> {
    replay_with_meta(problem, plan, runs, seed, max_steps, |s, a, r, mods, _| on_click(s, a, r, mods))
}

pub fn replay_with_meta<'p>(
    problem: &'p Problem,
    plan: &Plan,
    runs: usize,
    seed: u64,
    max_steps: usize,
    mut on_click: impl FnMut(&State, &'p Action, u8, &[&'p Mod], u8),
) -> Option<State> {
    let mut sim = Mechanics::new(problem, seed);
    let used: FxHashSet<&str> = plan.policy.values().map(String::as_str).collect();
    let acts: Vec<PreparedEffect> = problem.actions.iter().filter(|a| used.contains(a.name.as_str())).map(|a| sim.act(a)).collect();
    let by_name: FxHashMap<&str, usize> = acts.iter().enumerate().map(|(i, a)| (a.action.name.as_str(), i)).collect();
    let class: Vec<_> = sim.mods.iter().map(|m| problem.classify_mod(m)).collect();
    let start: Vec<u16> = (0..sim.mods.len() as u16).filter(|&k| sim.fixed[k as usize]).collect();
    let mut lost = None;
    let mut view: Vec<&Mod> = Vec::with_capacity(8);
    for _ in 0..runs {
        let (mut rarity, mut item, mut meta) = (NORMAL, start.clone(), 0u8);
        for _ in 0..max_steps {
            let mut state = State::new(rarity, &[], &[]).with_meta(meta);
            for &k in &item {
                match class[k as usize] {
                    (i, v) if i >= 0 => {
                        let slot = &mut state.status[i as usize];
                        if *slot == MISSING {
                            *slot = v;
                        }
                    }
                    (_, slot) => state.junk[slot as usize] += 1,
                }
            }
            if problem.done(&state) {
                break;
            }
            let Some(name) = plan.policy.get(&state) else {
                lost = Some(state);
                break;
            };
            let act = &acts[by_name[name.as_str()]];
            rarity = sim.apply(act, rarity, &mut item, &mut meta);
            view.clear();
            view.extend(item.iter().map(|&k| sim.mods[k as usize]));
            on_click(&state, act.action, rarity, &view, meta);
        }
    }
    lost
}
