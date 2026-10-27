use std::collections::HashMap;
use std::sync::Arc;

use ponder_rules::game::Kind;
use ponder_rules::item::Item;
use ponder_rules::lens::Lens;
use ponder_rules::ops::recombinator::Recombinator;
use ponder_rules::ops::{Ctx, Operation, Supply, parse};
use ponder_rules::target::Target;
use ponder_solve::Solution;

use crate::craft::{Craft, Node};
use crate::market::Market;

#[derive(Clone)]
pub enum Method {
    Named(String),
    Recombine(String, Arc<dyn Supply>),
}

pub struct Request {
    pub kind: Kind,
    pub goal: Target,
    pub methods: Vec<Method>,
    pub starts: Vec<Arc<dyn Supply>>,
}

#[derive(Debug, PartialEq)]
pub enum Error {
    Unsupported(String),
    Solve(ponder_solve::Error),
}

pub struct Plan<'a> {
    pub solution: Solution<Node>,
    lens: Lens<'a>,
    ops: Vec<Box<dyn Operation>>,
    starts: Vec<Arc<dyn Supply>>,
    ctx: Ctx<'a>,
    market: &'a Market,
}

impl Plan<'_> {
    pub fn craft(&self) -> Craft<'_> {
        let ops: Vec<_> = self.ops.iter().map(|o| o.as_ref()).collect();
        Craft::new(&self.ctx, &self.lens, &ops, self.market, self.starts.clone(), 0)
    }
}

pub struct Session<'a> {
    ctx: Ctx<'a>,
    market: &'a Market,
    plans: HashMap<Key, Cached<'a>>,
    retained: usize,
    accesses: u64,
}

struct Cached<'a> {
    plan: Arc<Plan<'a>>,
    retained: usize,
    accessed: u64,
}

#[derive(PartialEq, Eq, Hash)]
struct SupplyKey {
    name: String,
    price: u64,
    items: Vec<Item>,
}

impl SupplyKey {
    fn new(supply: &dyn Supply) -> Self {
        Self { name: supply.name(), price: supply.price().to_bits(), items: supply.items().to_vec() }
    }
}

#[derive(PartialEq, Eq, Hash)]
enum MethodKey {
    Named(String),
    Recombine(String, SupplyKey),
}

#[derive(PartialEq, Eq, Hash)]
struct Key {
    kind: Kind,
    goal: Target,
    methods: Vec<MethodKey>,
    starts: Vec<SupplyKey>,
}

impl Key {
    fn new(request: &Request) -> Self {
        let methods = request
            .methods
            .iter()
            .map(|m| match m {
                Method::Named(name) => MethodKey::Named(name.clone()),
                Method::Recombine(name, supply) => MethodKey::Recombine(name.clone(), SupplyKey::new(supply.as_ref())),
            })
            .collect();
        Self {
            kind: request.kind,
            goal: request.goal.clone(),
            methods,
            starts: request.starts.iter().map(|s| SupplyKey::new(s.as_ref())).collect(),
        }
    }

    fn samples(&self) -> usize {
        self.starts.iter().map(|s| s.items.len()).sum::<usize>()
            + self
                .methods
                .iter()
                .map(|m| match m {
                    MethodKey::Named(_) => 0,
                    MethodKey::Recombine(_, s) => s.items.len(),
                })
                .sum::<usize>()
    }
}

impl<'a> Session<'a> {
    pub fn new(ctx: Ctx<'a>, market: &'a Market) -> Self {
        Self { ctx, market, plans: HashMap::new(), retained: 0, accesses: 0 }
    }

    fn trim(&mut self, limit: usize) {
        while self.retained > limit {
            let oldest = self.plans.values().map(|p| p.accessed).min().unwrap();
            self.plans.retain(|_, p| {
                if p.accessed == oldest {
                    self.retained -= p.retained;
                    false
                } else {
                    true
                }
            });
        }
    }

    pub fn solve(&mut self, request: Request, limit: usize) -> Result<Arc<Plan<'a>>, Error> {
        self.trim(limit);
        let key = Key::new(&request);
        self.accesses += 1;
        if let Some(cached) = self.plans.get_mut(&key) {
            if cached.plan.solution.explored > limit {
                return Err(Error::Solve(ponder_solve::Error::TooManyStates(limit)));
            }
            cached.accessed = self.accesses;
            let plan = cached.plan.clone();
            return Ok(plan);
        }
        let game = self.ctx.game;
        let class = &game.bases[request.kind.base.0 as usize].class;
        let avoid: Vec<_> = request.goal.needs.iter().flat_map(|n| &n.ok).flat_map(|m| game.def(*m).groups.iter().copied()).collect();
        let ops: Vec<_> = request
            .methods
            .iter()
            .map(|m| match m {
                Method::Named(name) => parse(game, class, name, &avoid).ok_or_else(|| Error::Unsupported(name.clone())),
                Method::Recombine(name, supply) => Ok(Box::new(Recombinator::new(class, name, supply.clone())) as Box<dyn Operation>),
            })
            .collect::<Result<_, _>>()?;
        let refs: Vec<_> = ops.iter().map(|o| o.as_ref()).collect();
        let inputs: Vec<_> = request.starts.iter().flat_map(|s| s.items()).collect();
        let mut lens = Lens::new(game, request.kind, vec![request.goal], &refs, &inputs);
        let mut craft = Craft::new(&self.ctx, &lens, &refs, self.market, request.starts.clone(), 0);
        let solution = craft.solve(limit).map_err(Error::Solve)?;
        lens.clear_cache();
        let plan = Arc::new(Plan {
            solution,
            lens,
            ops,
            starts: request.starts,
            ctx: Ctx { game, assumptions: self.ctx.assumptions },
            market: self.market,
        });
        let retained = plan.solution.states.len() + key.samples();
        self.trim(limit.saturating_sub(retained));
        if retained <= limit {
            self.plans.insert(key, Cached { plan: plan.clone(), retained, accessed: self.accesses });
            self.retained += retained;
        }
        Ok(plan)
    }
}
