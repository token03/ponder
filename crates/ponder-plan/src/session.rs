use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use ponder_rules::game::Game;
use ponder_rules::game::Kind;
use ponder_rules::goal::Source;
use ponder_rules::item::Item;
use ponder_rules::lens::Lens;
use ponder_rules::ops::recombinator::Recombinator;
use ponder_rules::ops::{Ctx, Operation, Step, Supply, parse};
use ponder_rules::target::Target;
use ponder_solve::{Model, Solution as Policy, policy::PolicySystem};

use crate::craft::{Craft, Node};
use crate::market::Market;

#[derive(Clone)]
pub enum Method {
    Named(String),
    Recombine(String, Arc<dyn Supply>),
}

#[derive(Clone)]
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

pub struct Solution<'a> {
    pub policy: Policy<Node>,
    lens: Lens<'a>,
    ops: Vec<Box<dyn Operation>>,
    starts: Vec<Arc<dyn Supply>>,
    ctx: Ctx<'a>,
    market: &'a Market,
}

pub struct Usage {
    pub actions: Vec<(String, f64)>,
    pub seconds: f64,
    pub bases: f64,
}

impl Solution<'_> {
    pub fn usage(&self) -> Result<Usage, String> {
        let craft = self.craft();
        let solution = &self.policy;
        let origin = *solution.index.get(&Node::Market).ok_or_else(|| "policy has no initial purchase".to_string())?;
        let action = solution.policy[origin].ok_or_else(|| "policy has no initial purchase".to_string())?;
        let mut retained = vec![(origin, action)];
        let mut indices = std::collections::HashMap::from([(Node::Market, 0)]);
        let given_up: HashSet<_> = solution.given_up.iter().collect();
        let mut rows = Vec::new();
        while rows.len() < retained.len() {
            let (state, action) = retained[rows.len()];
            let mut row = Vec::new();
            let mut missing = false;
            craft.for_each_outcome(&solution.states[state], action, &mut |probability, next| {
                if probability <= 0.0 || craft.is_goal(&next) {
                    return;
                }
                if craft.is_restart(&next) || given_up.contains(&next) {
                    row.push((probability, 0));
                    return;
                }
                let Some((&next_state, action)) = solution.index.get(&next).map(|state| (state, solution.policy[*state])) else {
                    missing = true;
                    return;
                };
                let Some(action) = action else {
                    missing = true;
                    return;
                };
                let target = *indices.entry(next).or_insert_with(|| {
                    retained.push((next_state, action));
                    retained.len() - 1
                });
                row.push((probability, target));
            });
            if missing {
                return Err("selected policy has an unexpanded continuation".into());
            }
            rows.push(row);
        }
        let system = PolicySystem::new(rows).ok_or_else(|| "invalid policy transitions".to_string())?;
        let mut start = vec![0.0; retained.len()];
        start[0] = 1.0;
        let visits = system.visits(&start).ok_or_else(|| "policy usage could not be evaluated".to_string())?;
        let mut actions = BTreeMap::<String, f64>::new();
        let mut seconds = 0.0;
        let mut bases = 0.0;
        for ((state, action), visits) in retained.iter().copied().zip(visits) {
            match &solution.states[state] {
                Node::Market => bases += visits,
                Node::Item(_) if action < craft.ops.len() => {
                    let operation = craft.ops[action].0;
                    *actions.entry(operation.name()).or_default() += visits;
                    seconds += visits * operation.cost().seconds;
                }
                _ => {}
            }
        }
        let mut actions: Vec<_> = actions.into_iter().collect();
        actions.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));

        Ok(Usage { actions, seconds, bases })
    }

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
    plan: Arc<Solution<'a>>,
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

    /// Solve the request. `None` permits any graph size; an explicit limit also bounds this
    /// session's retained plans for diagnostic callers.
    pub fn solve(&mut self, request: Request, limit: impl Into<Option<usize>>) -> Result<Arc<Solution<'a>>, Error> {
        let limit = limit.into();
        if let Some(why) = &self.ctx.game.bases[request.kind.base.0 as usize].unsupported_crafting {
            return Err(Error::Unsupported(why.clone()));
        }
        if request.goal.base.is_some_and(|base| base != request.kind.base) || request.goal.ilvl > request.kind.ilvl {
            return Err(Error::Unsupported("goal base or item level differs from the fixed crafting context".into()));
        }
        let supplies = request.starts.iter().map(|s| s.as_ref()).chain(request.methods.iter().filter_map(|method| match method {
            Method::Recombine(_, supply) => Some(supply.as_ref()),
            _ => None,
        }));
        for item in supplies.flat_map(|s| s.items()) {
            if item.kind.base != request.kind.base || item.kind.ilvl != request.kind.ilvl || item.quality < request.goal.quality {
                return Err(Error::Unsupported(
                    "input base, item level or quality is not represented by the fixed crafting context".into(),
                ));
            }
            if item.kind.influences.0 != 0 && item.eldritch.is_some() {
                return Err(Error::Unsupported("an item cannot have both conventional influence and Eldritch implicits".into()));
            }
        }
        if let Some(limit) = limit {
            self.trim(limit);
        }
        let key = Key::new(&request);
        self.accesses += 1;
        if let Some(cached) = self.plans.get_mut(&key) {
            if let Some(limit) = limit
                && cached.plan.policy.explored > limit
            {
                return Err(Error::Solve(ponder_solve::Error::TooManyStates(limit)));
            }
            cached.accessed = self.accesses;
            let plan = cached.plan.clone();
            return Ok(plan);
        }
        let game = self.ctx.game;
        let class = &game.bases[request.kind.base.0 as usize].class;
        let avoid: Vec<_> = request.goal.needs.iter().flat_map(|n| &n.ok).flat_map(|m| game.def(*m).groups.iter().copied()).collect();
        let mut ops: Vec<_> = request
            .methods
            .iter()
            .map(|m| match m {
                Method::Named(name) => parse(game, class, name, &avoid).ok_or_else(|| Error::Unsupported(name.clone())),
                Method::Recombine(name, supply) => Ok(Box::new(Recombinator::new(class, name, supply.clone())) as Box<dyn Operation>),
            })
            .collect::<Result<_, _>>()?;
        // Unpriced operations cannot be used, and must not split the lens's classes or
        // make an otherwise unavailable goal pass the producibility check.
        ops.retain(|op| self.market.price(&op.cost()).is_some());
        let refs: Vec<_> = ops.iter().map(|o| o.as_ref()).collect();
        if !producible(game, &request.goal, &refs, &request.starts) {
            return Err(Error::Solve(ponder_solve::Error::Unreachable));
        }
        let inputs: Vec<_> = request.starts.iter().flat_map(|s| s.items()).collect();
        let mut lens = Lens::try_new(game, request.kind, vec![request.goal], &refs, &inputs).map_err(Error::Unsupported)?;
        let craft = Craft::new(&self.ctx, &lens, &refs, self.market, request.starts.clone(), 0);
        let solution =
            ponder_solve::solve_with(&craft, &[(1.0, Node::Market)], limit, ponder_solve::Numerics::Hybrid).map_err(Error::Solve)?;
        lens.clear_cache();
        let plan = Arc::new(Solution {
            policy: solution,
            lens,
            ops,
            starts: request.starts,
            ctx: Ctx { game, assumptions: self.ctx.assumptions },
            market: self.market,
        });
        let retained = plan.policy.states.len() + key.samples();
        if let Some(limit) = limit {
            self.trim(limit.saturating_sub(retained));
        }
        if limit.is_none_or(|limit| retained <= limit) {
            self.plans.insert(key, Cached { plan: plan.clone(), retained, accessed: self.accesses });
            self.retained += retained;
        }
        Ok(plan)
    }
}

fn producible(game: &Game, goal: &Target, ops: &[&dyn Operation], starts: &[Arc<dyn Supply>]) -> bool {
    let mut available: HashSet<_> = starts.iter().flat_map(|s| s.items()).flat_map(|item| item.affixes.iter().map(|a| a.id)).collect();
    let (mut rolls, mut unveils) = (false, false);
    for step in ops.iter().flat_map(|op| op.steps()) {
        match step {
            Step::Force(m) => {
                available.insert(*m);
            }
            Step::Craft(mods) => available.extend(mods.iter().copied()),
            Step::Recombine(supply) => available.extend(supply.0.items().iter().flat_map(|item| item.affixes.iter().map(|a| a.id))),
            Step::Veil(_) => available.extend(game.veils),
            Step::Unveil(_) => unveils = true,
            Step::Draw(_) | Step::Fill(_) | Step::Swap { .. } | Step::RecombineBonus => rolls = true,
            Step::Clear { .. }
            | Step::Rarity(_)
            | Step::Settle
            | Step::RollCount
            | Step::Remove { .. }
            | Step::RemoveCrafted
            | Step::Influence(_)
            | Step::Eldritch(_)
            | Step::Fracture => {}
        }
    }
    goal.needs.iter().all(|need| {
        need.ok
            .iter()
            .filter(|&&m| {
                available.contains(&m)
                    || rolls && matches!(game.def(m).source, Source::Natural | Source::Influence | Source::Fossil)
                    || unveils && game.def(m).source == Source::Veiled
            })
            .count()
            >= need.count as usize
    })
}
