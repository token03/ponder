//! A guide's recipe as a fixed policy (SPEC §8.3): evaluated exactly through the solver, and
//! simulated on real items to check the lens.
//!
//! The recipe is a state machine over (step, operation in step, failed attempts, item). An
//! attempt applies the step's operations in order, skipping any that do not apply. After it,
//! the step's `until` stage sends the item on; otherwise it repeats, or after `max` attempts
//! returns to the `else` step, which then runs at least once. Steps whose stage already holds
//! are skipped on the way forward. A `buy:` replaces the item with a fresh purchase, so falling
//! back to a step that buys starts over. A lock ("Cannot Be Changed") that cannot be crafted
//! abandons the attempt instead: to the `else` step, or to the first step (buying afresh)
//! without one. The recipe ends when the item meets the goal.

use std::collections::HashMap;
use std::sync::Mutex;

use rayon::prelude::*;

use ponder_rules::game::Game;
use ponder_rules::guide::Guide;
use ponder_rules::item::Item;
use ponder_rules::lens::State;
use ponder_rules::ops::run;
use ponder_rules::rng::Rng;
use ponder_rules::target::Target;
use ponder_solve::Model;

use crate::craft::Craft;

/// An action id of the craft: an operation or a purchase.
type Action = usize;

struct Step {
    ops: Vec<Action>,
    until: Option<usize>,
    max: Option<u32>,
    fallback: Option<usize>,
}

pub struct Recipe {
    steps: Vec<Step>,
    goal: usize,
    /// The purchase the recipe starts from (index into the craft's starts), and where.
    pub start: usize,
    begin: (usize, usize),
}

impl Recipe {
    /// `targets` maps stage names (and "goal") to lens target indices; `actions` names the
    /// craft's actions (`buy:<start>` for purchases). Fails on anything not implemented yet or on
    /// more than one item.
    pub fn new(guide: &Guide, targets: &HashMap<String, usize>, actions: &[String], ops: usize) -> Result<Recipe, String> {
        let mut steps = Vec::new();
        let mut start: Option<(usize, (usize, usize))> = None;
        for (n, step) in guide.steps.iter().enumerate() {
            if step.item != "main" {
                return Err(format!("works on several items ({})", step.item));
            }
            let mut indices = Vec::new();
            for op in &step.ops {
                let i = actions.iter().position(|o| o == op).ok_or_else(|| format!("needs {op}"))?;
                if i >= ops && start.get_or_insert((i - ops, (n, indices.len()))).0 != i - ops {
                    return Err(format!("buys more than one kind of item ({op})"));
                }
                indices.push(i);
            }
            let until = step.until.as_ref().map(|u| targets[u]);
            steps.push(Step { ops: indices, until, max: step.max, fallback: step.fallback.map(|f| f - 1) });
        }
        let (start, begin) = start.ok_or("buys nothing")?;
        Ok(Recipe { steps, goal: targets["goal"], start, begin })
    }

    /// Where an item that just finished an attempt at step `i` goes next.
    fn after(&self, i: usize, k: u16, holds: impl Fn(usize) -> bool) -> (usize, u16) {
        let step = &self.steps[i];
        if step.until.is_none_or(&holds) {
            return (self.forward(i + 1, &holds), 0);
        }
        match step.max {
            Some(m) if k as u32 + 1 >= m => (step.fallback.unwrap_or(i), 0),
            Some(_) => (i, k + 1),
            None => (i, 0),
        }
    }

    /// Where an attempt at step `i` goes when it cannot lock.
    fn abandon(&self, i: usize) -> usize {
        self.steps[i].fallback.unwrap_or(0)
    }

    /// The first step from `i` whose stage does not hold yet.
    fn forward(&self, mut i: usize, holds: impl Fn(usize) -> bool) -> usize {
        while i < self.steps.len() && self.steps[i].until.is_some_and(&holds) {
            i += 1;
        }
        i
    }

    /// One run on a real item, as lines: each attempt's step, the operations applied or
    /// skipped, and the item afterwards. Stops at the goal, the end of the recipe or `limit`.
    pub fn trace(&self, craft: &Craft, targets: &[Target], limit: usize, rng: &mut Rng) -> Vec<String> {
        let game: &Game = craft.ctx.game;
        let holds = |t: usize, item: &Item| targets[t].holds(game, item);
        let mut item = craft.starts[self.start].item.clone();
        let mut out = Vec::new();
        let (mut i, mut k) = (self.forward(0, |t| holds(t, &item)), 0);
        while !holds(self.goal, &item) && i < self.steps.len() && out.len() < limit {
            let mut done = Vec::new();
            let mut abandoned = false;
            for &a in &self.steps[i].ops {
                let (applied, _) = self.apply(craft, a, &mut item, rng);
                done.push(match applied {
                    Applied::Done => craft.action_name(a),
                    Applied::Skipped => format!("({} skipped)", craft.action_name(a)),
                    Applied::Abandon => format!("(cannot {}: abandoned)", craft.action_name(a)),
                });
                abandoned = applied == Applied::Abandon;
                if abandoned {
                    break;
                }
            }
            let mods: Vec<String> = item
                .affixes
                .iter()
                .map(|a| format!("{}{}{}", game.def(a.id).key, if a.fractured { "*" } else { "" }, if a.crafted { "^" } else { "" }))
                .collect();
            out.push(format!("step {} [{}] -> {}", i + 1, done.join(", "), mods.join(" ")));
            (i, k) = if abandoned { (self.abandon(i), 0) } else { self.after(i, k, |t| holds(t, &item)) };
        }
        out.push(if holds(self.goal, &item) { "goal".into() } else { format!("stuck at step {}", i + 1) });
        out
    }

    /// Apply action `a` to a real item if it can: what happened, and the cost.
    fn apply(&self, craft: &Craft, a: Action, item: &mut Item, rng: &mut Rng) -> (Applied, f64) {
        if let Some(b) = craft.start_of(a) {
            *item = b.item.clone();
            return (Applied::Done, b.price);
        }
        let (op, price) = craft.ops[a];
        if !op.applies(craft.ctx, &(craft.ctx.game, &*item)) {
            return (if is_lock(craft, a) { Applied::Abandon } else { Applied::Skipped }, 0.0);
        }
        run(craft.ctx, op.steps(), item, rng);
        (Applied::Done, price)
    }

    /// Mean cost of following the recipe on real items, and its standard error. The item the
    /// recipe starts from is bought, not given.
    /// Mean cost and standard error over `runs` runs, run `r` seeded with `seed + r`.
    pub fn simulate(&self, craft: &Craft, targets: &[Target], runs: usize, seed: u64) -> (f64, f64) {
        let game: &Game = craft.ctx.game;
        let holds = |t: usize, item: &Item| targets[t].holds(game, item);
        let costs: Vec<f64> = (0..runs as u64)
            .into_par_iter()
            .map(|r| {
                let rng = &mut Rng::new(seed + r);
                let mut item = craft.starts[self.start].item.clone();
                let mut cost = craft.starts[self.start].price;
                let mut i = self.forward(0, |t| holds(t, &item));
                let mut k = 0;
                let mut guard = 0;
                while !holds(self.goal, &item) && i < self.steps.len() && guard < 1_000_000 {
                    let mut abandoned = false;
                    for &a in &self.steps[i].ops {
                        let (applied, c) = self.apply(craft, a, &mut item, rng);
                        cost += c;
                        abandoned = applied == Applied::Abandon;
                        if abandoned {
                            break;
                        }
                    }
                    (i, k) = if abandoned { (self.abandon(i), 0) } else { self.after(i, k, |t| holds(t, &item)) };
                    guard += 1;
                }
                cost
            })
            .collect();
        let mean = costs.iter().sum::<f64>() / runs as f64;
        let var = costs.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / (runs as f64 - 1.0);
        (mean, (var / runs as f64).sqrt())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Applied {
    Done,
    Skipped,
    Abandon,
}

/// Is action `a` a bench lock ("Prefixes/Suffixes Cannot Be Changed")?
fn is_lock(craft: &Craft, a: Action) -> bool {
    craft.start_of(a).is_none() && craft.ops[a].0.name().ends_with("cannot-be-changed")
}

/// The recipe over lens states, for exact evaluation by the solver.
type Outcomes = Vec<(f64, State)>;

pub struct Following<'a> {
    pub craft: &'a Craft<'a>,
    pub recipe: &'a Recipe,
    /// Item outcomes, computed once: a recipe reaches one item state at many positions.
    cache: Mutex<HashMap<(State, usize), Outcomes>>,
}

/// Before the first purchase, at (step, operation within the step, failed attempts, item), or
/// buying again: worth what the fresh start is worth.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Position {
    Fresh,
    At(u8, u8, u16, State),
    Restart,
}

impl<'a> Following<'a> {
    pub fn new(craft: &'a Craft<'a>, recipe: &'a Recipe) -> Self {
        Following { craft, recipe, cache: Mutex::default() }
    }

    fn outcomes_of(&self, s: &State, a: usize) -> Outcomes {
        let key = (s.clone(), a);
        if let Some(out) = self.cache.lock().unwrap().get(&key) {
            return out.clone();
        }
        let out = self.craft.outcomes_of(s, a);
        self.cache.lock().unwrap().insert(key, out.clone());
        out
    }

    fn advance(&self, i: usize, j: usize, k: u16, s: State) -> Position {
        if j < self.recipe.steps[i].ops.len() {
            return Position::At(i as u8, j as u8, k, s);
        }
        let (i, k) = self.recipe.after(i, k, |t| self.craft.lens.holds(t, &s));
        Position::At(i as u8, 0, k, s)
    }
}

impl Model for Following<'_> {
    type State = Position;

    fn is_goal(&self, p: &Position) -> bool {
        matches!(p, Position::At(_, _, _, s) if self.craft.lens.holds(self.recipe.goal, s))
    }

    fn is_restart(&self, p: &Position) -> bool {
        *p == Position::Restart
    }

    /// One action: 0 applies the step's next action (buying again restarts), 1 skips an
    /// operation that does not apply, 2 abandons the attempt for want of a lock.
    fn actions(&self, p: &Position) -> Vec<(usize, f64)> {
        let (i, j, s) = match p {
            Position::Fresh => return vec![(0, self.craft.starts[self.recipe.start].price)],
            Position::At(i, j, _, s) => (*i as usize, *j as usize, s),
            Position::Restart => return Vec::new(),
        };
        let Some(step) = self.recipe.steps.get(i) else { return Vec::new() };
        let a = step.ops[j];
        match self.craft.start_of(a) {
            Some(_) => vec![(0, 0.0)],
            None if self.craft.ops[a].0.applies(self.craft.ctx, &self.craft.lens.view(s)) => vec![(0, self.craft.ops[a].1)],
            None if is_lock(self.craft, a) => vec![(2, 0.0)],
            None => vec![(1, 0.0)],
        }
    }

    fn outcomes(&self, p: &Position, action: usize) -> Vec<(f64, Position)> {
        let (i, j, k, s) = match p {
            Position::Fresh => {
                let (i, j) = self.recipe.begin;
                return vec![(1.0, self.advance(i, j + 1, 0, self.craft.starts[self.recipe.start].state.clone()))];
            }
            Position::At(i, j, k, s) => (*i as usize, *j as usize, *k, s),
            Position::Restart => return Vec::new(),
        };
        let a = self.recipe.steps[i].ops[j];
        match action {
            1 => vec![(1.0, self.advance(i, j + 1, k, s.clone()))],
            2 => vec![(1.0, Position::At(self.recipe.abandon(i) as u8, 0, 0, s.clone()))],
            _ if self.craft.start_of(a).is_some() => vec![(1.0, Position::Restart)],
            _ => self.outcomes_of(s, a).into_iter().map(|(p, t)| (p, self.advance(i, j + 1, k, t))).collect(),
        }
    }
}
