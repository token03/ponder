//! One craft as a solver model: lens states, priced operations, a goal.

use std::cell::RefCell;
use std::collections::HashMap;

use ponder_rules::lens::{Lens, State};
use ponder_rules::ops::{Ctx, Operation};
use ponder_solve::Model;

use crate::market::Market;

type Outcomes = Vec<(f64, State)>;

pub struct Craft<'a> {
    pub ctx: &'a Ctx<'a>,
    pub lens: &'a Lens<'a>,
    pub ops: Vec<(&'a dyn Operation, f64)>,
    /// Lens target index of the goal.
    pub goal: usize,
    cache: RefCell<HashMap<(State, usize), Outcomes>>,
}

impl<'a> Craft<'a> {
    /// The priced operations among `ops`; unpriced ones are left out.
    pub fn new(ctx: &'a Ctx<'a>, lens: &'a Lens<'a>, ops: &[&'a dyn Operation], market: &Market, goal: usize) -> Craft<'a> {
        let ops = ops.iter().filter_map(|op| market.price(&op.cost()).map(|c| (*op, c))).collect();
        Craft { ctx, lens, ops, goal, cache: RefCell::default() }
    }

    /// Outcomes of operation `i`, computed once per state.
    pub fn outcomes_of(&self, s: &State, i: usize) -> Vec<(f64, State)> {
        let key = (s.clone(), i);
        if let Some(out) = self.cache.borrow().get(&key) {
            return out.clone();
        }
        let out = self.lens.run(self.ctx, self.ops[i].0.steps(), s);
        self.cache.borrow_mut().insert(key, out.clone());
        out
    }
}

impl Model for Craft<'_> {
    type State = State;

    fn is_goal(&self, s: &State) -> bool {
        self.lens.holds(self.goal, s)
    }

    fn actions(&self, s: &State) -> Vec<(usize, f64)> {
        let view = self.lens.view(s);
        (0..self.ops.len()).filter(|&i| self.ops[i].0.applies(self.ctx, &view)).map(|i| (i, self.ops[i].1)).collect()
    }

    fn outcomes(&self, s: &State, i: usize) -> Vec<(f64, State)> {
        self.outcomes_of(s, i)
    }
}

/// Follow a solved policy on real items: mean cost, standard error, and how often each
/// operation was used per run. Runs that reach a state the solution never saw stop early and
/// count in `lost`.
pub struct Followed {
    pub mean: f64,
    pub error: f64,
    pub uses: Vec<(String, f64)>,
    pub lost: usize,
}

impl Craft<'_> {
    pub fn follow(
        &self,
        solution: &ponder_solve::Solution<State>,
        start: &ponder_rules::item::Item,
        runs: usize,
        rng: &mut ponder_rules::rng::Rng,
    ) -> Followed {
        let mut costs = Vec::with_capacity(runs);
        let mut uses = vec![0usize; self.ops.len()];
        let mut lost = 0;
        for _ in 0..runs {
            let mut item = start.clone();
            let mut cost = 0.0;
            for _ in 0..200_000 {
                let s = self.lens.project(&item);
                if self.lens.holds(self.goal, &s) {
                    break;
                }
                let Some(op) = solution.index.get(&s).and_then(|&i| solution.policy[i]) else {
                    lost += 1;
                    break;
                };
                let (operation, price) = self.ops[op];
                ponder_rules::ops::run(self.ctx, operation.steps(), &mut item, rng);
                cost += price;
                uses[op] += 1;
            }
            costs.push(cost);
        }
        let mean = costs.iter().sum::<f64>() / runs as f64;
        let var = costs.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / (runs as f64 - 1.0);
        let uses = uses.iter().enumerate().filter(|(_, n)| **n > 0).map(|(i, n)| (self.ops[i].0.name(), *n as f64 / runs as f64)).collect();
        Followed { mean, error: (var / runs as f64).sqrt(), uses, lost }
    }
}
