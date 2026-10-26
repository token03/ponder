//! One craft as a solver model: lens states, priced operations, a goal. The craft begins at
//! the market, buying one of its start items, and any item can be abandoned to start over.

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
    Item(State),
    /// Partway through a reroll (operation, item after its reset): every item that resets to
    /// the same remains shares these outcomes, so they are stored once.
    Hub(usize, State),
    /// Back to the market: worth what the market is worth.
    Restart,
}

pub struct Craft<'a> {
    pub ctx: &'a Ctx<'a>,
    pub lens: &'a Lens<'a>,
    pub ops: Vec<(&'a dyn Operation, f64)>,
    /// Per operation, whether a cheaper one does the same; the best plan leaves these out.
    pub dominated: Vec<bool>,
    /// Where an item can come from, to start or start over.
    pub starts: Vec<Arc<dyn Supply>>,
    /// Lens target index of the goal.
    pub goal: usize,
    /// Per operation, where its reset ends, if it has one.
    hubs: Vec<Option<usize>>,
    start_states: Vec<Vec<(f64, State)>>,
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
        let hubs = ops.iter().map(|(op, _)| hub(op.steps())).collect();
        let start_states = starts.iter().map(|s| lens.project_items(s.items())).collect();
        Craft { ctx, lens, ops, dominated, starts, goal, hubs, start_states }
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

    /// The item after operation `i`'s reset, when it has one and it is certain.
    pub fn reset(&self, s: &State, i: usize) -> Option<State> {
        let k = self.hubs[i]?;
        let mut out = self.lens.run(self.ctx, &self.ops[i].0.steps()[..k], s);
        (out.len() == 1).then(|| out.remove(0).1)
    }

    /// Outcomes of the rest of operation `i` after its reset.
    pub fn after_reset(&self, s: &State, i: usize) -> Vec<(f64, State)> {
        let k = self.hubs[i].expect("an operation with a reset");
        self.lens.run(self.ctx, &self.ops[i].0.steps()[k..], s)
    }
}

impl Model for Craft<'_> {
    type State = Node;

    fn is_goal(&self, n: &Node) -> bool {
        matches!(n, Node::Item(s) if self.lens.holds(self.goal, s))
    }

    fn is_restart(&self, n: &Node) -> bool {
        *n == Node::Restart
    }

    fn actions(&self, n: &Node) -> Vec<(usize, f64)> {
        match n {
            Node::Market => self.starts.iter().enumerate().map(|(j, b)| (self.ops.len() + j, b.price())).collect(),
            Node::Item(s) => {
                let view = self.lens.view(s);
                let ops = (0..self.ops.len())
                    .filter(|&i| !self.dominated[i] && self.ops[i].0.applies(self.ctx, &view))
                    .map(|i| (i, self.ops[i].1));
                ops.chain([(self.restart(), 0.0)]).collect()
            }
            Node::Hub(i, _) => vec![(*i, 0.0)],
            Node::Restart => Vec::new(),
        }
    }

    fn outcomes(&self, n: &Node, i: usize) -> Vec<(f64, Node)> {
        if i == self.restart() {
            return vec![(1.0, Node::Restart)];
        }
        match (n, self.start_of(i)) {
            (_, Some(_)) => self.start_states(i - self.ops.len()).iter().map(|(p, s)| (*p, Node::Item(s.clone()))).collect(),
            (Node::Item(s), None) => match self.reset(s, i) {
                Some(t) => vec![(1.0, Node::Hub(i, t))],
                None => self.outcomes_of(s, i).into_iter().map(|(p, t)| (p, Node::Item(t))).collect(),
            },
            (Node::Hub(_, s), None) => self.after_reset(s, i).into_iter().map(|(p, t)| (p, Node::Item(t))).collect(),
            _ => Vec::new(),
        }
    }
}

/// Where an operation's reset ends: after its first `Clear` and the certain steps that follow.
fn hub(steps: &[Step]) -> Option<usize> {
    let clear = steps.iter().position(|s| matches!(s, Step::Clear { .. }))?;
    let certain = |s: &Step| {
        matches!(
            s,
            Step::Clear { .. }
                | Step::Rarity(_)
                | Step::Settle
                | Step::Force(_)
                | Step::Craft(_)
                | Step::RemoveCrafted
                | Step::Influence(_)
                | Step::Eldritch(_)
        )
    };
    let end = clear + steps[clear..].iter().take_while(|s| certain(s)).count();
    (end < steps.len()).then_some(end)
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
                let node = item.as_ref().map_or(Node::Market, |i| Node::Item(self.lens.project(i)));
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
