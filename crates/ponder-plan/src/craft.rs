//! One craft as a solver model: lens states, priced operations, a goal. The craft begins at
//! the market, buying one of its start items, and any item can be abandoned to start over.

use std::collections::HashMap;
use std::sync::Arc;

use ponder_rules::item::Item;
use ponder_rules::lens::{Lens, State};
use ponder_rules::ops::{Ctx, Operation, Step, Supply};
use ponder_solve::Model;
use rayon::prelude::*;

use crate::market::Market;

/// A solver state of a craft.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Node {
    /// Choosing what to buy.
    Market,
    Goal,
    Item(State),
    /// Partway through an operation (shared continuation, intermediate item).
    Hub(usize, State),
    /// Back to the market: worth what the market is worth.
    Restart,
}

pub struct Craft<'a> {
    pub ctx: &'a Ctx<'a>,
    pub lens: &'a Lens<'a>,
    pub ops: Vec<(&'a dyn Operation, f64)>,
    /// Operations ruled out by a cheaper operation or a feasible finishing policy.
    pub dominated: Vec<bool>,
    /// Where an item can come from, to start or start over.
    pub starts: Vec<Arc<dyn Supply>>,
    /// Lens target index of the goal.
    pub goal: usize,
    /// Per operation, where its shared continuation begins, if it has one.
    hubs: Vec<Option<usize>>,
    continuations: Vec<usize>,
    start_states: Vec<Vec<(f64, State)>>,
    upper_bound: f64,
    finishes: Vec<usize>,
}

impl<'a> Craft<'a> {
    /// The priced operations among `ops`; unpriced ones are left out.
    pub fn new(
        ctx: &'a Ctx<'a>,
        lens: &'a Lens<'a>,
        ops: &[&'a dyn Operation],
        market: &Market,
        starts: Vec<Arc<dyn Supply>>,
        goal: usize,
    ) -> Craft<'a> {
        let ops: Vec<(&dyn Operation, f64)> = ops.iter().filter_map(|op| market.price(&op.cost()).map(|c| (*op, c))).collect();
        // An operation that does what a cheaper (or earlier, equally priced) one of the same mechanic
        // does is never best.
        let dominated = (0..ops.len())
            .map(|i| {
                (0..ops.len()).any(|j| {
                    let mechanic = |k: usize| ops[k].0.name().split(':').next().map(String::from);
                    (ops[j].1 < ops[i].1 || (ops[j].1 == ops[i].1 && j < i))
                        && mechanic(i) == mechanic(j)
                        && lens.same_effect(ops[j].0.steps(), ops[i].0.steps())
                })
            })
            .collect();
        let hubs: Vec<_> = ops.iter().map(|(op, _)| hub(op.steps())).collect();
        let continuations = (0..ops.len())
            .map(|i| {
                hubs[i]
                    .map_or(i, |k| (0..i).find(|&j| hubs[j].is_some_and(|l| ops[j].0.steps()[l..] == ops[i].0.steps()[k..])).unwrap_or(i))
            })
            .collect();
        let start_states = starts.iter().map(|s| lens.project_items(s.items())).collect();
        let finishes = ops.iter().enumerate().filter(|(_, (op, _))| op.steps().iter().all(deterministic)).map(|(i, _)| i).collect();
        let mut craft =
            Craft { ctx, lens, ops, dominated, starts, goal, hubs, continuations, start_states, upper_bound: f64::INFINITY, finishes };
        craft.upper_bound = craft.repeat_bound();
        for (i, dominated) in craft.dominated.iter_mut().enumerate() {
            *dominated |= craft.ops[i].1 > craft.upper_bound;
        }
        craft
    }

    pub fn start_states(&self, i: usize) -> &[(f64, State)] {
        &self.start_states[i]
    }

    /// Action id `i` names a purchase rather than an operation.
    pub fn start_of(&self, i: usize) -> Option<&dyn Supply> {
        i.checked_sub(self.ops.len()).and_then(|j| self.starts.get(j)).map(|s| s.as_ref())
    }

    /// The action that abandons an item.
    pub fn restart(&self) -> usize {
        self.ops.len() + self.starts.len()
    }

    pub fn action_name(&self, i: usize) -> String {
        if i == self.restart() {
            return "start over".into();
        }
        self.start_of(i).map_or_else(|| self.ops[i].0.name(), |s| format!("buy:{}", s.name()))
    }

    /// Outcomes of operation `i` on an item.
    pub fn outcomes_of(&self, s: &State, i: usize) -> Vec<(f64, State)> {
        self.lens.run(self.ctx, self.ops[i].0.steps(), s)
    }

    pub fn start_operation(&self, s: &State, i: usize) -> Option<Vec<(f64, State)>> {
        let k = self.hubs[i]?;
        Some(self.lens.run(self.ctx, &self.ops[i].0.steps()[..k], s))
    }

    /// Outcomes of operation `i`'s shared continuation.
    pub fn finish_operation(&self, s: &State, i: usize) -> Vec<(f64, State)> {
        let k = self.hubs[i].expect("an operation with a continuation");
        self.lens.run(self.ctx, &self.ops[i].0.steps()[k..], s)
    }

    fn finish_cost(&self, s: &State) -> f64 {
        if self.lens.holds(self.goal, s) {
            return 0.0;
        }
        let view = self.lens.view(s);
        self.finishes
            .iter()
            .copied()
            .filter(|&i| self.ops[i].0.applies(self.ctx, &view))
            .filter(|&i| self.outcomes_of(s, i).iter().all(|(_, t)| self.lens.holds(self.goal, t)))
            .map(|i| self.ops[i].1)
            .fold(f64::INFINITY, f64::min)
    }

    fn repeat_bound(&self) -> f64 {
        if self.ops.iter().any(|(_, price)| *price < 0.0) || self.starts.iter().any(|s| s.price() < 0.0) {
            return f64::INFINITY;
        }
        let mut groups: Vec<usize> = Vec::new();
        for (i, (op, _)) in self.ops.iter().enumerate() {
            if matches!(op.steps().first(), Some(Step::Clear { .. }))
                && self.hubs[i].is_some()
                && !groups.iter().any(|&j| self.ops[j].0.steps() == op.steps())
            {
                groups.push(i);
            }
        }
        let mut cache = HashMap::new();
        let mut bound = f64::INFINITY;
        for (source, states) in self.starts.iter().zip(&self.start_states) {
            if source.price() >= bound {
                continue;
            }
            let (mut cost, mut accepted) = (source.price(), 0.0);
            for (p, s) in states {
                let mut finish = self.finish_cost(s);
                for &i in &groups {
                    if finish == 0.0 {
                        break;
                    }
                    let group = || self.ops.iter().enumerate().filter(|(_, (op, _))| op.steps() == self.ops[i].0.steps());
                    let first = group()
                        .filter(|(_, (op, _))| op.applies(self.ctx, &self.lens.view(s)))
                        .map(|(_, (_, price))| *price)
                        .fold(f64::INFINITY, f64::min);
                    if !first.is_finite() {
                        continue;
                    }
                    let reset = self.start_operation(s, i).unwrap();
                    if reset.len() != 1 {
                        continue;
                    }
                    let reset = &reset[0].1;
                    let extra = cache.entry((i, reset.clone())).or_insert_with(|| {
                        let out = self.finish_operation(reset, i);
                        let costs: Vec<_> = out.iter().map(|(_, t)| self.finish_cost(t)).collect();
                        let chance = out.iter().zip(&costs).filter(|(_, c)| c.is_finite()).map(|((p, _), _)| p).sum::<f64>().min(1.0);
                        if chance == 0.0 {
                            return f64::INFINITY;
                        }
                        let finishing = out.iter().zip(&costs).filter(|(_, c)| c.is_finite()).map(|((p, _), c)| p * c).sum::<f64>();
                        if chance == 1.0 {
                            return finishing;
                        }
                        let failures: Vec<_> =
                            out.iter().zip(&costs).filter(|((p, _), c)| *p > 0.0 && !c.is_finite()).map(|(o, _)| o).collect();
                        let retry = group()
                            .filter(|&(j, (op, _))| {
                                failures.iter().all(|(_, t)| {
                                    op.applies(self.ctx, &self.lens.view(t))
                                        && self.start_operation(t, j).is_some_and(|out| out.len() == 1 && out[0].1 == *reset)
                                })
                            })
                            .map(|(_, (_, price))| *price)
                            .fold(f64::INFINITY, f64::min);
                        (finishing + retry * (1.0 - chance)) / chance
                    });
                    finish = finish.min(first + *extra);
                }
                if finish.is_finite() {
                    cost += p * finish;
                    accepted += p;
                }
            }
            if accepted > 0.0 {
                bound = bound.min(cost / accepted);
            }
        }
        bound
    }

    fn nodes(&self, states: impl Iterator<Item = (f64, State)>) -> Vec<(f64, Node)> {
        let mut finished = 0.0;
        let mut abandoned = 0.0;
        let mut out: Vec<_> = states
            .filter_map(|(p, s)| {
                if self.lens.holds(self.goal, &s) {
                    finished += p;
                    None
                } else if self.lens.viable(self.goal, &s) {
                    Some((p, Node::Item(s)))
                } else {
                    abandoned += p;
                    None
                }
            })
            .collect();
        if finished > 0.0 {
            out.push((finished, Node::Goal));
        }
        if abandoned > 0.0 {
            out.push((abandoned, Node::Restart));
        }
        out
    }
}

impl Model for Craft<'_> {
    type State = Node;

    fn is_goal(&self, n: &Node) -> bool {
        *n == Node::Goal || matches!(n, Node::Item(s) if self.lens.holds(self.goal, s))
    }

    fn is_restart(&self, n: &Node) -> bool {
        *n == Node::Restart
    }

    fn actions(&self, n: &Node) -> Vec<(usize, f64)> {
        match n {
            Node::Market => self
                .starts
                .iter()
                .enumerate()
                .filter(|(_, b)| b.price() <= self.upper_bound)
                .map(|(j, b)| (self.ops.len() + j, b.price()))
                .collect(),
            Node::Item(s) => {
                if !self.lens.viable(self.goal, s) {
                    return vec![(self.restart(), 0.0)];
                }
                let view = self.lens.view(s);
                let ops = (0..self.ops.len())
                    .filter(|&i| !self.dominated[i] && self.ops[i].0.applies(self.ctx, &view))
                    .map(|i| (i, self.ops[i].1));
                ops.chain([(self.restart(), 0.0)]).collect()
            }
            Node::Hub(i, _) => vec![(*i, 0.0)],
            Node::Goal | Node::Restart => Vec::new(),
        }
    }

    fn outcomes(&self, n: &Node, i: usize) -> Vec<(f64, Node)> {
        if i == self.restart() {
            return vec![(1.0, Node::Restart)];
        }
        match (n, self.start_of(i)) {
            (_, Some(_)) => self.nodes(self.start_states(i - self.ops.len()).iter().cloned()),
            (Node::Item(s), None) => match self.start_operation(s, i) {
                Some(out) => out.into_iter().map(|(p, t)| (p, Node::Hub(self.continuations[i], t))).collect(),
                None => self.nodes(self.outcomes_of(s, i).into_iter()),
            },
            (Node::Hub(_, s), None) => self.nodes(self.finish_operation(s, i).into_iter()),
            _ => Vec::new(),
        }
    }
}

/// Where an operation's shared continuation begins.
fn hub(steps: &[Step]) -> Option<usize> {
    let end = match steps.iter().position(|s| matches!(s, Step::Clear { .. })) {
        Some(clear) => clear + steps[clear..].iter().take_while(|s| deterministic(s)).count(),
        None => steps.iter().position(|s| !deterministic(s))? + 1,
    };
    (end < steps.len() && !steps[..end].iter().any(|s| matches!(s, Step::RollCount))).then_some(end)
}

fn deterministic(step: &Step) -> bool {
    matches!(
        step,
        Step::Clear { .. }
            | Step::Rarity(_)
            | Step::Settle
            | Step::Force(_)
            | Step::Craft(_)
            | Step::RemoveCrafted
            | Step::Influence(_)
            | Step::Eldritch(_)
    )
}

/// Follow a solved policy on real items: mean cost, standard error, and how often each
/// operation was used per run. Runs that reach a state the solution never saw stop early and
/// count in `lost`.
pub struct Followed {
    pub mean: f64,
    pub error: f64,
    pub uses: Vec<(String, f64)>,
    pub lost: usize,
    /// The first run's actions, repeats collapsed: (action, times in a row).
    pub example: Vec<(String, usize)>,
    /// The finished items.
    pub items: Vec<Item>,
}

impl Craft<'_> {
    /// What the solution buys first.
    pub fn first_buy(&self, solution: &ponder_solve::Solution<Node>) -> Option<&dyn Supply> {
        solution.index.get(&Node::Market).and_then(|&i| solution.policy[i]).and_then(|a| self.start_of(a))
    }

    /// Runs start at the market; run `r` is seeded with `seed + r`.
    pub fn follow(&self, solution: &ponder_solve::Solution<Node>, runs: usize, seed: u64) -> Followed {
        let policy = |n: &Node| solution.index.get(n).and_then(|&i| solution.policy[i]);
        // One run: its cost, its actions in order, and the finished item (none if lost).
        let run = |r: u64| {
            let rng = &mut ponder_rules::rng::Rng::new(seed + r);
            let mut item: Option<Item> = None;
            let (mut cost, mut actions) = (0.0, Vec::new());
            for _ in 0..200_000 {
                let state = item.as_ref().map(|i| self.lens.project(i));
                if state.as_ref().is_some_and(|s| !self.lens.viable(self.goal, s)) {
                    actions.push(self.restart());
                    item = None;
                    continue;
                }
                let node = state.map_or(Node::Market, Node::Item);
                if self.is_goal(&node) {
                    return (cost, actions, item);
                }
                let Some(op) = policy(&node) else { return (cost, actions, None) };
                actions.push(op);
                if op == self.restart() {
                    item = None;
                } else if let Some(b) = self.start_of(op) {
                    item = Some(b.item(rng));
                    cost += b.price();
                } else {
                    let (operation, price) = self.ops[op];
                    ponder_rules::ops::run(self.ctx, operation.steps(), item.as_mut().unwrap(), rng);
                    cost += price;
                }
            }
            (cost, actions, None)
        };
        let done: Vec<(f64, Vec<usize>, Option<Item>)> = (0..runs as u64).into_par_iter().map(run).collect();
        let mut uses = vec![0usize; self.restart() + 1];
        for &op in done.iter().flat_map(|d| &d.1) {
            uses[op] += 1;
        }
        let mut example: Vec<(String, usize)> = Vec::new();
        for &op in &done[0].1 {
            match example.last_mut() {
                Some((last, n)) if *last == self.action_name(op) => *n += 1,
                _ => example.push((self.action_name(op), 1)),
            }
        }
        let lost = done.iter().filter(|d| d.2.is_none()).count();
        let mean = done.iter().map(|d| d.0).sum::<f64>() / runs as f64;
        let var = done.iter().map(|d| (d.0 - mean).powi(2)).sum::<f64>() / (runs as f64 - 1.0);
        let uses = uses.iter().enumerate().filter(|(_, n)| **n > 0).map(|(i, n)| (self.action_name(i), *n as f64 / runs as f64)).collect();
        let items = done.into_iter().filter_map(|d| d.2).collect();
        Followed { mean, error: (var / runs as f64).sqrt(), uses, lost, example, items }
    }
}
