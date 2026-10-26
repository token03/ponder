//! One craft as a solver model: lens states, priced operations, a goal. The craft begins at
//! the market, buying one of its start items, and any item can be abandoned to start over.

use ponder_rules::item::Item;
use ponder_rules::lens::{Lens, State};
use ponder_rules::ops::{Ctx, Operation};
use ponder_solve::Model;

use crate::market::Market;

/// A solver state of a craft.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Node {
    /// Choosing what to buy.
    Market,
    Item(State),
    /// Back to the market: worth what the market is worth.
    Restart,
}

pub struct Craft<'a> {
    pub ctx: &'a Ctx<'a>,
    pub lens: &'a Lens<'a>,
    pub ops: Vec<(&'a dyn Operation, f64)>,
    /// Items that can be bought to start over: name, price, item.
    pub starts: Vec<Start>,
    /// Lens target index of the goal.
    pub goal: usize,
}

#[derive(Clone)]
pub struct Start {
    pub name: String,
    pub price: f64,
    pub item: Item,
    pub state: State,
}

impl<'a> Craft<'a> {
    /// The priced operations among `ops`; unpriced ones are left out.
    pub fn new(
        ctx: &'a Ctx<'a>,
        lens: &'a Lens<'a>,
        ops: &[&'a dyn Operation],
        market: &Market,
        starts: Vec<(String, f64, Item)>,
        goal: usize,
    ) -> Craft<'a> {
        let ops = ops.iter().filter_map(|op| market.price(&op.cost()).map(|c| (*op, c))).collect();
        let starts = starts.into_iter().map(|(name, price, item)| Start { name, price, state: lens.project(&item), item }).collect();
        Craft { ctx, lens, ops, starts, goal }
    }

    /// Action id `i` names a purchase rather than an operation.
    pub fn start_of(&self, i: usize) -> Option<&Start> {
        i.checked_sub(self.ops.len()).and_then(|j| self.starts.get(j))
    }

    /// The action that abandons an item.
    pub fn restart(&self) -> usize {
        self.ops.len() + self.starts.len()
    }

    pub fn action_name(&self, i: usize) -> String {
        if i == self.restart() {
            return "start over".into();
        }
        self.start_of(i).map_or_else(|| self.ops[i].0.name(), |s| format!("buy:{}", s.name))
    }

    /// Outcomes of operation `i` on an item.
    pub fn outcomes_of(&self, s: &State, i: usize) -> Vec<(f64, State)> {
        self.lens.run(self.ctx, self.ops[i].0.steps(), s)
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
            Node::Market => self.starts.iter().enumerate().map(|(j, b)| (self.ops.len() + j, b.price)).collect(),
            Node::Item(s) => {
                let view = self.lens.view(s);
                let ops = (0..self.ops.len()).filter(|&i| self.ops[i].0.applies(self.ctx, &view)).map(|i| (i, self.ops[i].1));
                ops.chain([(self.restart(), 0.0)]).collect()
            }
            Node::Restart => Vec::new(),
        }
    }

    fn outcomes(&self, n: &Node, i: usize) -> Vec<(f64, Node)> {
        if i == self.restart() {
            return vec![(1.0, Node::Restart)];
        }
        match (n, self.start_of(i)) {
            (_, Some(b)) => vec![(1.0, Node::Item(b.state.clone()))],
            (Node::Item(s), None) => self.outcomes_of(s, i).into_iter().map(|(p, t)| (p, Node::Item(t))).collect(),
            _ => Vec::new(),
        }
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
    /// The first run's actions, repeats collapsed: (action, times in a row).
    pub example: Vec<(String, usize)>,
}

impl Craft<'_> {
    /// What the solution buys first.
    pub fn first_buy(&self, solution: &ponder_solve::Solution<Node>) -> Option<&Start> {
        solution.index.get(&Node::Market).and_then(|&i| solution.policy[i]).and_then(|a| self.start_of(a))
    }

    /// Runs start at the market.
    pub fn follow(&self, solution: &ponder_solve::Solution<Node>, runs: usize, rng: &mut ponder_rules::rng::Rng) -> Followed {
        let mut costs = Vec::with_capacity(runs);
        let mut uses = vec![0usize; self.restart() + 1];
        let mut lost = 0;
        let mut example: Vec<(String, usize)> = Vec::new();
        let policy = |n: &Node| solution.index.get(n).and_then(|&i| solution.policy[i]);
        for _ in 0..runs {
            let mut item: Option<Item> = None;
            let mut cost = 0.0;
            for _ in 0..200_000 {
                let node = item.as_ref().map_or(Node::Market, |i| Node::Item(self.lens.project(i)));
                if self.is_goal(&node) {
                    break;
                }
                let Some(op) = policy(&node) else {
                    lost += 1;
                    break;
                };
                uses[op] += 1;
                if costs.is_empty() {
                    match example.last_mut() {
                        Some((last, n)) if *last == self.action_name(op) => *n += 1,
                        _ => example.push((self.action_name(op), 1)),
                    }
                }
                if op == self.restart() {
                    item = None;
                    continue;
                }
                if let Some(b) = self.start_of(op) {
                    item = Some(b.item.clone());
                    cost += b.price;
                    continue;
                }
                let (operation, price) = self.ops[op];
                ponder_rules::ops::run(self.ctx, operation.steps(), item.as_mut().unwrap(), rng);
                cost += price;
            }
            costs.push(cost);
        }
        let mean = costs.iter().sum::<f64>() / runs as f64;
        let var = costs.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / (runs as f64 - 1.0);
        let uses = uses.iter().enumerate().filter(|(_, n)| **n > 0).map(|(i, n)| (self.action_name(i), *n as f64 / runs as f64)).collect();
        Followed { mean, error: (var / runs as f64).sqrt(), uses, lost, example }
    }
}
