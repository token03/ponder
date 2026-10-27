//! A guide's recipe as a fixed policy (SPEC §8.3): evaluated exactly through the solver, and
//! simulated on real items to check the lens.
//!
//! The recipe is a state machine over (step, operation in step, failed attempts, item). An
//! attempt applies the step's operations in order, skipping any that do not apply. After it,
//! the step's `until` stage sends the item on; otherwise it repeats, or after `max` attempts
//! returns to the `else` step, which then runs at least once. The recipe ends as soon as the item
//! meets the goal, even partway through an attempt. Steps whose stage already holds
//! are skipped on the way forward. A `buy:` replaces the item with a fresh purchase, so falling
//! back to a step that buys starts over. A lock ("Cannot Be Changed") that cannot be crafted
//! abandons the attempt instead: to the `else` step, or to the first step (buying afresh)
//! without one.
//!
//! A guide that works on several items has one recipe per item (`item` on its steps). An
//! item made from others starts from one of them, bought at what making it costs, and falling
//! back to a step of another item starts it over.

use std::collections::{HashMap, HashSet};
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
    /// The recipe for the guide's item `item`, ending at its last step's stage. `targets` maps
    /// stage names (and "goal") to lens target indices; `actions` names the craft's actions
    /// (`buy:<start>` for purchases, the first `ops` being operations). `start` is the purchase
    /// an item made from others begins with. Fails on anything not implemented yet.
    pub fn new(
        guide: &Guide,
        item: &str,
        targets: &HashMap<String, usize>,
        actions: &[String],
        ops: usize,
        start: Option<Action>,
    ) -> Result<Recipe, String> {
        let own: Vec<usize> = (0..guide.steps.len()).filter(|&g| guide.steps[g].item == item).collect();
        let first = usize::from(start.is_some());
        let local = |g: usize| own.iter().position(|&o| o == g).map_or(0, |l| l + first);
        let mut steps: Vec<Step> = start.map(|a| Step { ops: vec![a], until: None, max: None, fallback: None }).into_iter().collect();
        let mut bought: Option<(usize, (usize, usize))> = start.map(|a| (a - ops, (0, 0)));
        for &g in &own {
            let step = &guide.steps[g];
            let mut indices = Vec::new();
            for op in &step.ops {
                let i = actions.iter().position(|o| o == op).ok_or_else(|| format!("needs {op}"))?;
                if i >= ops && bought.get_or_insert((i - ops, (steps.len(), indices.len()))).0 != i - ops {
                    return Err(format!("buys more than one kind of item ({op})"));
                }
                indices.push(i);
            }
            let until = step.until.as_ref().map(|u| targets[u]);
            steps.push(Step { ops: indices, until, max: step.max, fallback: step.fallback.map(|f| local(f - 1)) });
        }
        let goal = guide.steps[*own.last().ok_or("no steps")?].until.as_ref().ok_or("the last step has no stage")?;
        let (start, begin) = bought.ok_or("buys nothing")?;
        Ok(Recipe { steps, goal: targets[goal], start, begin })
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

    fn bought(&self, holds: impl Fn(usize) -> bool) -> (usize, usize, u16) {
        let (i, j) = self.begin;
        if j + 1 < self.steps[i].ops.len() {
            (i, j + 1, 0)
        } else {
            let (i, k) = self.after(i, 0, holds);
            (i, 0, k)
        }
    }

    /// One run on a real item, as lines: each attempt's step, the operations applied or
    /// skipped, and the item afterwards. Stops at the goal, the end of the recipe or `limit`.
    pub fn trace(&self, craft: &Craft, targets: &[Target], limit: usize, rng: &mut Rng) -> Vec<String> {
        let game: &Game = craft.ctx.game;
        let holds = |t: usize, item: &Item| targets[t].holds(game, item);
        let mut item = craft.starts[self.start].item(rng);
        let mut out = Vec::new();
        let (mut i, mut j, mut k) = self.bought(|t| holds(t, &item));
        while !holds(self.goal, &item) && i < self.steps.len() && out.len() < limit {
            let mut done = Vec::new();
            let mut abandoned = false;
            for &a in self.steps[i].ops.iter().skip(j) {
                let (applied, _) = self.apply(craft, a, &mut item, rng);
                done.push(match applied {
                    Applied::Done => craft.action_name(a),
                    Applied::Skipped => format!("({} skipped)", craft.action_name(a)),
                    Applied::Abandon => format!("(cannot {}: abandoned)", craft.action_name(a)),
                });
                abandoned = applied == Applied::Abandon;
                if abandoned || holds(self.goal, &item) {
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
            j = 0;
        }
        out.push(if holds(self.goal, &item) { "goal".into() } else { format!("stuck at step {}", i + 1) });
        out
    }

    /// Apply action `a` to a real item if it can: what happened, and the cost.
    fn apply(&self, craft: &Craft, a: Action, item: &mut Item, rng: &mut Rng) -> (Applied, f64) {
        if let Some(b) = craft.start_of(a) {
            *item = b.item(rng);
            return (Applied::Done, b.price());
        }
        let (op, price) = craft.ops[a];
        if !op.applies(craft.ctx, &(craft.ctx.game, &*item)) {
            return (if is_lock(craft, a) { Applied::Abandon } else { Applied::Skipped }, 0.0);
        }
        run(craft.ctx, op.steps(), item, rng);
        (Applied::Done, price)
    }

    /// Following the recipe on real items over `runs` runs, run `r` seeded with `seed + r`:
    /// mean cost, its standard error, and the finished items. The item the recipe starts from
    /// is bought, not given. An item at a position the exact solve gave up on (`given_up`) is
    /// abandoned for a fresh one.
    pub fn simulate(&self, craft: &Craft, targets: &[Target], given_up: &HashSet<Position>, runs: usize, seed: u64) -> Simulated {
        let game: &Game = craft.ctx.game;
        let holds = |t: usize, item: &Item| targets[t].holds(game, item);
        let start = &craft.starts[self.start];
        let done: Vec<(f64, Item)> = (0..runs as u64)
            .into_par_iter()
            .map(|r| {
                let rng = &mut Rng::new(seed + r);
                let mut item = start.item(rng);
                let mut cost = start.price();
                let (mut i, mut j, mut k) = self.bought(|t| holds(t, &item));
                let mut guard = 0;
                while !holds(self.goal, &item) && i < self.steps.len() && guard < 1_000_000 {
                    guard += 1;
                    if !given_up.is_empty() && given_up.contains(&Position::At(i as u8, j as u8, k, craft.lens.project(&item))) {
                        item = start.item(rng);
                        cost += start.price();
                        (i, j, k) = self.bought(|t| holds(t, &item));
                        continue;
                    }
                    let mut abandoned = false;
                    for &a in self.steps[i].ops.iter().skip(j) {
                        let (applied, c) = self.apply(craft, a, &mut item, rng);
                        cost += c;
                        abandoned = applied == Applied::Abandon;
                        if abandoned || holds(self.goal, &item) {
                            break;
                        }
                    }
                    (i, k) = if abandoned { (self.abandon(i), 0) } else { self.after(i, k, |t| holds(t, &item)) };
                    j = 0;
                }
                (cost, item)
            })
            .collect();
        let mean = done.iter().map(|d| d.0).sum::<f64>() / runs as f64;
        let var = done.iter().map(|d| (d.0 - mean).powi(2)).sum::<f64>() / (runs as f64 - 1.0);
        let items = done.into_iter().map(|d| d.1).filter(|i| holds(self.goal, i)).collect();
        Simulated { mean, error: (var / runs as f64).sqrt(), items }
    }
}

/// A recipe followed on real items.
pub struct Simulated {
    pub mean: f64,
    pub error: f64,
    pub items: Vec<Item>,
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
    /// Item outcomes, computed once: a recipe reaches one item state at many positions. The
    /// flag marks the shared continuation of an operation.
    cache: Mutex<HashMap<(State, usize, bool), Outcomes>>,
}

/// Before the first purchase, at (step, operation within the step, failed attempts, item),
/// partway through that operation (as [`crate::craft::Node::Hub`]), or buying
/// again: worth what the fresh start is worth.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Position {
    Fresh,
    At(u8, u8, u16, State),
    Hub(u8, u8, u16, State),
    Restart,
}

impl<'a> Following<'a> {
    pub fn new(craft: &'a Craft<'a>, recipe: &'a Recipe) -> Self {
        Following { craft, recipe, cache: Mutex::default() }
    }

    fn outcomes_of(&self, s: &State, a: usize, hub: bool) -> Outcomes {
        let key = (*s, a, hub);
        if let Some(out) = self.cache.lock().unwrap().get(&key) {
            return out.clone();
        }
        let out = if hub { self.craft.finish_operation(s, a) } else { self.craft.outcomes_of(s, a) };
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

    /// A crafter abandons an item the recipe can never finish.
    fn gives_up(&self) -> bool {
        true
    }

    /// One action: 0 applies the step's next action (buying again restarts), 1 skips an
    /// operation that does not apply, 2 abandons the attempt for want of a lock.
    fn actions(&self, p: &Position) -> Vec<(usize, f64)> {
        let (i, j, s) = match p {
            Position::Fresh => return vec![(0, self.craft.starts[self.recipe.start].price())],
            Position::At(i, j, _, s) => (*i as usize, *j as usize, s),
            Position::Hub(..) => return vec![(0, 0.0)],
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
                let states = self.craft.start_states(self.recipe.start);
                return states
                    .iter()
                    .map(|(p, s)| {
                        let (i, j, k) = self.recipe.bought(|t| self.craft.lens.holds(t, s));
                        (*p, Position::At(i as u8, j as u8, k, *s))
                    })
                    .collect();
            }
            Position::At(i, j, k, s) | Position::Hub(i, j, k, s) => (*i as usize, *j as usize, *k, s),
            Position::Restart => return Vec::new(),
        };
        let a = self.recipe.steps[i].ops[j];
        let hub = matches!(p, Position::Hub(..));
        match action {
            1 => vec![(1.0, self.advance(i, j + 1, k, *s))],
            2 => vec![(1.0, Position::At(self.recipe.abandon(i) as u8, 0, 0, *s))],
            _ if self.craft.start_of(a).is_some() => vec![(1.0, Position::Restart)],
            _ if !hub && let Some(out) = self.craft.start_operation(s, a) => {
                out.into_iter().map(|(p, t)| (p, Position::Hub(i as u8, j as u8, k, t))).collect()
            }
            _ => self.outcomes_of(s, a, hub).into_iter().map(|(p, t)| (p, self.advance(i, j + 1, k, t))).collect(),
        }
    }
}
