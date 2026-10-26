//! A guide's recipe as a fixed policy (SPEC §8.3): evaluated exactly through the solver, and
//! simulated on real items to check the lens.
//!
//! The recipe is a state machine over (step, operation in step, failed attempts, item). An
//! attempt applies the step's operations in order, skipping any that do not apply. After it,
//! the step's `until` stage sends the item on; otherwise it repeats, or after `max` attempts
//! returns to the `else` step, which then runs at least once. Steps whose stage already holds
//! are skipped on the way forward. The recipe ends when the item meets the goal.

use std::collections::HashMap;

use ponder_rules::game::Game;
use ponder_rules::guide::Guide;
use ponder_rules::item::Item;
use ponder_rules::lens::State;
use ponder_rules::ops::run;
use ponder_rules::rng::Rng;
use ponder_rules::target::Target;
use ponder_solve::Model;

use crate::craft::Craft;

struct Step {
    /// Index into the craft's operations; `None` for purchases (the start item).
    ops: Vec<Option<usize>>,
    until: Option<usize>,
    max: Option<u32>,
    fallback: Option<usize>,
}

pub struct Recipe {
    steps: Vec<Step>,
    goal: usize,
}

impl Recipe {
    /// `targets` maps stage names (and "goal") to lens target indices; `ops` names the craft's
    /// operations. Fails on anything not implemented yet or on more than one item.
    pub fn new(guide: &Guide, targets: &HashMap<String, usize>, ops: &[String]) -> Result<Recipe, String> {
        let mut steps = Vec::new();
        for step in &guide.steps {
            if step.item != "main" {
                return Err(format!("works on several items ({})", step.item));
            }
            let mut indices = Vec::new();
            for op in &step.ops {
                if op.starts_with("buy:") {
                    if op != "buy:base" {
                        return Err(format!("buys an intermediate ({op})"));
                    }
                    indices.push(None);
                } else {
                    let i = ops.iter().position(|o| o == op).ok_or_else(|| format!("needs {op}"))?;
                    indices.push(Some(i));
                }
            }
            let until = step.until.as_ref().map(|u| targets[u]);
            steps.push(Step { ops: indices, until, max: step.max, fallback: step.fallback.map(|f| f - 1) });
        }
        Ok(Recipe { steps, goal: targets["goal"] })
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

    /// The first step from `i` whose stage does not hold yet.
    fn forward(&self, mut i: usize, holds: impl Fn(usize) -> bool) -> usize {
        while i < self.steps.len() && self.steps[i].until.is_some_and(&holds) {
            i += 1;
        }
        i
    }

    /// Mean cost of following the recipe on real items, and its standard error.
    pub fn simulate(&self, craft: &Craft, targets: &[Target], start: &Item, runs: usize, rng: &mut Rng) -> (f64, f64) {
        let game: &Game = craft.ctx.game;
        let holds = |t: usize, item: &Item| targets[t].holds(game, item);
        let mut costs = Vec::with_capacity(runs);
        for _ in 0..runs {
            let mut item = start.clone();
            let mut cost = 0.0;
            let mut i = self.forward(0, |t| holds(t, &item));
            let mut k = 0;
            let mut guard = 0;
            while !holds(self.goal, &item) && i < self.steps.len() && guard < 1_000_000 {
                for &op in &self.steps[i].ops {
                    if let Some(op) = op {
                        let (op, price) = craft.ops[op];
                        if op.applies(craft.ctx, &(game, &item)) {
                            run(craft.ctx, op.steps(), &mut item, rng);
                            cost += price;
                        }
                    }
                }
                (i, k) = self.after(i, k, |t| holds(t, &item));
                guard += 1;
            }
            costs.push(cost);
        }
        let mean = costs.iter().sum::<f64>() / runs as f64;
        let var = costs.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / (runs as f64 - 1.0);
        (mean, (var / runs as f64).sqrt())
    }
}

/// The recipe over lens states, for exact evaluation by the solver.
pub struct Following<'a> {
    pub craft: &'a Craft<'a>,
    pub recipe: &'a Recipe,
}

/// (step, operation within the step, failed attempts, item).
pub type Position = (u8, u8, u16, State);

impl Following<'_> {
    pub fn start(&self, s: State) -> Position {
        let i = self.recipe.forward(0, |t| self.craft.lens.holds(t, &s));
        (i as u8, 0, 0, s)
    }

    fn advance(&self, i: usize, j: usize, k: u16, s: State) -> Position {
        if j < self.recipe.steps[i].ops.len() {
            return (i as u8, j as u8, k, s);
        }
        let (i, k) = self.recipe.after(i, k, |t| self.craft.lens.holds(t, &s));
        (i as u8, 0, k, s)
    }
}

impl Model for Following<'_> {
    type State = Position;

    fn is_goal(&self, p: &Position) -> bool {
        self.craft.lens.holds(self.recipe.goal, &p.3)
    }

    /// One action: 0 applies the operation, 1 skips one that does not apply.
    fn actions(&self, (i, j, _, s): &Position) -> Vec<(usize, f64)> {
        let Some(step) = self.recipe.steps.get(*i as usize) else { return Vec::new() };
        match step.ops[*j as usize] {
            Some(op) if self.craft.ops[op].0.applies(self.craft.ctx, &self.craft.lens.view(s)) => vec![(0, self.craft.ops[op].1)],
            _ => vec![(1, 0.0)],
        }
    }

    fn outcomes(&self, (i, j, k, s): &Position, action: usize) -> Vec<(f64, Position)> {
        let (i, j) = (*i as usize, *j as usize);
        match (action, self.recipe.steps[i].ops[j]) {
            (0, Some(op)) => self.craft.outcomes_of(s, op).into_iter().map(|(p, t)| (p, self.advance(i, j + 1, *k, t))).collect(),
            _ => vec![(1.0, self.advance(i, j + 1, *k, s.clone()))],
        }
    }
}
