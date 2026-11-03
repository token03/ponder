use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use ponder_rules::game::Game;
use ponder_rules::game::{Kind, Side};
use ponder_rules::goal::Source;
use ponder_rules::item::Item;
use ponder_rules::lens::Lens;
use ponder_rules::ops::awakener::Awakener;
use ponder_rules::ops::recombinator::Recombinator;
use ponder_rules::ops::{Ctx, Operation, Step, Supply, parse};
use ponder_rules::target::Target;
use ponder_solve::{Model, Solution as Policy, policy::PolicySystem};
use serde::{Deserialize, Serialize};

use crate::craft::{Craft, Node};
use crate::market::Market;

#[derive(Clone)]
pub enum Method {
    Named(String),
    Recombine(String, Arc<dyn Supply>),
    Awaken(String, Arc<dyn Supply>),
}

impl Method {
    fn partner(&self) -> Option<&dyn Supply> {
        match self {
            Method::Named(_) => None,
            Method::Recombine(_, supply) | Method::Awaken(_, supply) => Some(supply.as_ref()),
        }
    }
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
    /// Expected chaos per finished item by operation, clicking time included.
    pub spend: Vec<(String, f64)>,
    pub seconds: f64,
    pub bases: f64,
    /// Expected purchases per supply.
    pub buys: Vec<(String, f64)>,
    pub flow: Flow,
}

/// The policy as a flowchart: one node per kind of item people can tell apart (which of the
/// goal's needs it holds, its rarity), what is done to it there, and where that leads, in
/// expected visits per finished item.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Flow {
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FlowKind {
    Buy,
    Work,
    Finished,
    StartOver,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlowNode {
    pub kind: FlowKind,
    /// indices of the goal's needs the item holds
    pub held: Vec<usize>,
    /// those of `held` its fractured mods hold
    pub fractured: Vec<usize>,
    /// how many mods each side holds (prefixes, suffixes), needed or not
    pub filled: [u8; 2],
    /// crafted mods by key, with their sides ("prefix" or "suffix")
    pub crafted: Vec<(String, String)>,
    pub rarity: Option<String>,
    /// the side Eldritch currency works on ("prefix" or "suffix"), set by the dominant implicit
    pub eldritch: Option<String>,
    /// whether it holds a veiled mod, to unveil
    pub veiled: bool,
    /// operations done here, with expected uses, most used first
    pub actions: Vec<(String, f64)>,
    pub visits: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlowEdge {
    pub from: usize,
    /// the operation done at `from`
    pub action: String,
    pub to: usize,
    /// expected times taken per finished item
    pub flow: f64,
}

/// Where a state leads: the boxes next reached, with their chances, and the chance of
/// finishing on the way.
#[derive(Clone, Default)]
struct Leads {
    next: BTreeMap<usize, f64>,
    finish: f64,
}

/// Where a policy state belongs in the flowchart.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct FlowKey {
    kind: FlowKind,
    held: u128,
    fractured: u128,
    filled: [u8; 2],
    crafted: Vec<(String, Side)>,
    rarity: Option<String>,
    eldritch: Option<Side>,
    veiled: bool,
}

impl Flow {
    /// The likeliest way from buying to finishing, as the nodes passed through: from each node,
    /// on to the one with the best odds of finishing, counting only results that change it.
    pub fn route(&self) -> Vec<usize> {
        self.nodes.iter().position(|node| node.kind == FlowKind::Buy).map_or_else(Vec::new, |buy| self.route_from(buy, &self.likeliest()))
    }

    /// The likeliest way on from box `at` to a finished item: `at` and the boxes after it, from
    /// `likeliest`.
    fn route_from(&self, at: usize, likeliest: &[Option<usize>]) -> Vec<usize> {
        let mut at = Some(at);
        let mut route = Vec::new();
        while let Some(i) = at.filter(|i| !route.contains(i) && self.nodes[*i].kind != FlowKind::Finished) {
            route.push(i);
            at = likeliest[i];
        }
        route
    }

    /// For each box, the box next on its likeliest way to a finished item (the most likely
    /// product of chances).
    pub fn likeliest(&self) -> Vec<Option<usize>> {
        let nodes = &self.nodes;
        let mut leaving = BTreeMap::<usize, BTreeMap<usize, f64>>::new();
        for edge in self.edges.iter().filter(|edge| edge.from != edge.to) {
            *leaving.entry(edge.from).or_default().entry(edge.to).or_default() += edge.flow;
        }
        let mut odds: Vec<f64> = nodes.iter().map(|node| if node.kind == FlowKind::Finished { 1.0 } else { 0.0 }).collect();
        let mut next = vec![None; nodes.len()];
        let mut changed = true;
        while changed {
            changed = false;
            for (&from, exits) in &leaving {
                let total: f64 = exits.values().sum();
                for (&to, &flow) in exits {
                    if flow / total * odds[to] > odds[from] * (1.0 + 1e-12) {
                        odds[from] = flow / total * odds[to];
                        next[from] = Some(to);
                        changed = true;
                    }
                }
            }
        }
        next
    }

    /// Where acting at `s` leads: the boxes of the acting states next reached (`boxes` has one
    /// for each acting state), through the steps inside the operation, and the chance of
    /// finishing on the way. `inside` remembers this for the steps already followed, since
    /// operations share them.
    fn through(s: usize, boxes: &[Option<usize>], system: &PolicySystem, finishes: &[f64], inside: &mut HashMap<usize, Leads>) -> Leads {
        let mut leads = Leads { next: BTreeMap::new(), finish: finishes[s] };
        for &(p, t) in system.outcomes(s) {
            if let Some(to) = boxes[t] {
                *leads.next.entry(to).or_default() += p;
                continue;
            }
            let after = match inside.get(&t) {
                Some(after) => after.clone(),
                None => {
                    inside.insert(t, Leads::default());
                    let after = Flow::through(t, boxes, system, finishes, inside);
                    inside.insert(t, after.clone());
                    after
                }
            };
            leads.finish += p * after.finish;
            for (to, q) in after.next {
                *leads.next.entry(to).or_default() += p * q;
            }
        }
        leads
    }

    /// The flowchart of the policy states, keyed by `keys` and doing `actions`, visited
    /// `visits` times, moving as `system` does and finishing with `finishes`; what neither moves
    /// on nor finishes starts over. Only states `acting` (buying or starting an operation) are
    /// drawn: the steps inside an operation are followed through to the next.
    fn new(keys: &[FlowKey], actions: &[String], acting: &[bool], visits: &[f64], system: &PolicySystem, finishes: &[f64]) -> Flow {
        let end = |kind| FlowKey {
            kind,
            held: 0,
            fractured: 0,
            filled: [0; 2],
            crafted: Vec::new(),
            rarity: None,
            eldritch: None,
            veiled: false,
        };
        let (finished, start_over) = (end(FlowKind::Finished), end(FlowKind::StartOver));
        let mut index = BTreeMap::<FlowKey, usize>::new();
        let mut flow = Flow::default();
        let mut node = |key: &FlowKey, flow: &mut Flow| -> usize {
            *index.entry(key.clone()).or_insert_with(|| {
                let bits = |mask: u128| (0..128).filter(|i| mask >> i & 1 == 1).collect();
                flow.nodes.push(FlowNode {
                    kind: key.kind,
                    held: bits(key.held),
                    fractured: bits(key.fractured),
                    filled: key.filled,
                    crafted: key.crafted.iter().map(|(m, side)| (m.clone(), format!("{side:?}").to_lowercase())).collect(),
                    rarity: key.rarity.clone(),
                    eldritch: key.eldritch.map(|side| format!("{side:?}").to_lowercase()),
                    veiled: key.veiled,
                    actions: Vec::new(),
                    visits: 0.0,
                });
                flow.nodes.len() - 1
            })
        };
        let boxes: Vec<Option<usize>> = keys.iter().zip(acting).map(|(key, &acting)| acting.then(|| node(key, &mut flow))).collect();
        let mut edges = BTreeMap::<(usize, &str, usize), f64>::new();
        let mut inside = HashMap::new();
        let mut done = BTreeMap::<(usize, &str), f64>::new();
        for (s, from) in boxes.iter().enumerate().filter_map(|(s, from)| Some((s, (*from)?))) {
            flow.nodes[from].visits += visits[s];
            let action = actions[s].as_str();
            *done.entry((from, action)).or_default() += visits[s];
            let Leads { next, finish } = Flow::through(s, &boxes, system, finishes, &mut inside);
            let mut moved = 0.0;
            for (to, p) in next {
                *edges.entry((from, action, to)).or_default() += visits[s] * p;
                moved += p;
            }
            if finish > 0.0 {
                *edges.entry((from, action, node(&finished, &mut flow))).or_default() += visits[s] * finish;
            }
            let lost = 1.0 - moved - finish;
            if lost > 1e-9 {
                *edges.entry((from, action, node(&start_over, &mut flow))).or_default() += visits[s] * lost;
            }
        }
        for end in [&finished, &start_over] {
            if let Some(&i) = index.get(end) {
                flow.nodes[i].visits = edges.iter().filter(|((_, _, to), _)| *to == i).map(|(_, f)| f).sum();
            }
        }
        for ((node, action), uses) in done {
            flow.nodes[node].actions.push((action.to_string(), uses));
        }
        for node in &mut flow.nodes {
            node.actions.sort_by(|a, b| b.1.total_cmp(&a.1));
        }
        flow.edges = edges
            .into_iter()
            .filter(|(_, f)| *f > 1e-9)
            .map(|((from, action, to), flow)| FlowEdge { from, action: action.to_string(), to, flow })
            .collect();
        flow
    }
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
        let mut finishes = Vec::new();
        while rows.len() < retained.len() {
            let (state, action) = retained[rows.len()];
            let mut row = Vec::new();
            let mut missing = false;
            let mut finished = 0.0;
            craft.for_each_outcome(&solution.states[state], action, &mut |probability, next| {
                if probability <= 0.0 {
                    return;
                }
                if craft.is_goal(&next) {
                    finished += probability;
                    return;
                }
                if craft.is_restart(&next) || given_up.contains(&next) {
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
            finishes.push(finished);
        }
        let system = PolicySystem::new(rows).ok_or_else(|| "invalid policy transitions".to_string())?;
        let keys: Vec<FlowKey> = retained
            .iter()
            .map(|&(state, _)| match solution.states[state].state() {
                None => FlowKey {
                    kind: FlowKind::Buy,
                    held: 0,
                    fractured: 0,
                    filled: [0; 2],
                    crafted: Vec::new(),
                    rarity: None,
                    eldritch: None,
                    veiled: false,
                },
                Some(s) => {
                    let (filled, crafted) = craft.lens.layout(s);
                    FlowKey {
                        kind: FlowKind::Work,
                        held: craft.lens.held(craft.goal, s),
                        fractured: craft.lens.held_fractured(craft.goal, s),
                        filled,
                        crafted,
                        rarity: Some(format!("{:?}", s.rarity).to_lowercase()),
                        eldritch: s.eldritch,
                        veiled: craft.lens.veiled(s),
                    }
                }
            })
            .collect();
        let mut labels = std::collections::HashMap::new();
        let labels: Vec<_> = retained
            .iter()
            .zip(&finishes)
            .zip(&keys)
            .map(|((&(state, action), &finish), key)| {
                let label = match &solution.states[state] {
                    Node::Market => craft.ops.len() + 1,
                    node if craft.applies_operation(node, action) => action + 1,
                    _ => 0,
                };
                let next = labels.len();
                *labels.entry((label, finish.to_bits(), key)).or_insert(next)
            })
            .collect();
        let (system, retained, finishes, keys, origin) = match system.quotient(&labels) {
            Some(quotient) => {
                let pick = |s: &usize| retained[*s];
                let retained = quotient.representatives.iter().map(pick).collect::<Vec<_>>();
                let finishes = quotient.representatives.iter().map(|&s| finishes[s]).collect::<Vec<_>>();
                let keys = quotient.representatives.iter().map(|&s| keys[s].clone()).collect::<Vec<_>>();
                (quotient.system, retained, finishes, keys, quotient.classes[0])
            }
            None => (system, retained, finishes, keys, 0),
        };
        let mut start = vec![0.0; retained.len()];
        start[origin] = 1.0;
        let mut visits = system.visits(&start).ok_or_else(|| "policy usage could not be evaluated".to_string())?;
        let chance: f64 = visits.iter().zip(&finishes).map(|(visits, probability)| visits * probability).sum();
        if !chance.is_finite() || chance <= 0.0 {
            return Err("policy has no finite finishing probability".into());
        }
        for visits in &mut visits {
            *visits /= chance;
        }
        let mut actions = BTreeMap::<String, f64>::new();
        let mut spend = BTreeMap::<String, f64>::new();
        let mut seconds = 0.0;
        let mut bases = 0.0;
        let mut buys = BTreeMap::<String, f64>::new();
        for ((state, action), visits) in retained.iter().copied().zip(visits.iter().copied()) {
            match &solution.states[state] {
                Node::Market => {
                    bases += visits;
                    let supply = craft.start_of(action).ok_or_else(|| "policy buys from no supply".to_string())?;
                    *buys.entry(supply.name()).or_default() += visits;
                }
                node if craft.applies_operation(node, action) => {
                    let operation = craft.ops[action].0;
                    *actions.entry(operation.name()).or_default() += visits;
                    *spend.entry(operation.name()).or_default() +=
                        visits * self.market.price(&operation.cost()).expect("solved operations are priced");
                    seconds += visits * operation.cost().seconds;
                }
                _ => {}
            }
        }
        let names: Vec<String> = retained.iter().map(|&(_, action)| craft.action_name(action)).collect();
        let acting: Vec<bool> = retained
            .iter()
            .map(|&(state, action)| match &solution.states[state] {
                Node::Market => true,
                node => craft.applies_operation(node, action),
            })
            .collect();
        let flow = Flow::new(&keys, &names, &acting, &visits, &system, &finishes);
        let mut actions: Vec<_> = actions.into_iter().collect();
        actions.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut spend: Vec<_> = spend.into_iter().collect();
        spend.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));

        Ok(Usage { actions, spend, seconds, bases, buys: buys.into_iter().collect(), flow })
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
    Awaken(String, SupplyKey),
}

#[derive(PartialEq, Eq, Hash)]
struct Key {
    kind: Kind,
    goal: Target,
    methods: Vec<MethodKey>,
    starts: Vec<SupplyKey>,
    cap: Option<u64>,
}

impl Key {
    fn new(request: &Request, cap: Option<f64>) -> Self {
        let methods = request
            .methods
            .iter()
            .map(|m| match m {
                Method::Named(name) => MethodKey::Named(name.clone()),
                Method::Recombine(name, supply) => MethodKey::Recombine(name.clone(), SupplyKey::new(supply.as_ref())),
                Method::Awaken(name, supply) => MethodKey::Awaken(name.clone(), SupplyKey::new(supply.as_ref())),
            })
            .collect();
        Self {
            kind: request.kind,
            goal: request.goal.clone(),
            methods,
            starts: request.starts.iter().map(|s| SupplyKey::new(s.as_ref())).collect(),
            cap: cap.map(f64::to_bits),
        }
    }

    fn samples(&self) -> usize {
        self.starts.iter().map(|s| s.items.len()).sum::<usize>()
            + self
                .methods
                .iter()
                .map(|m| match m {
                    MethodKey::Named(_) => 0,
                    MethodKey::Recombine(_, s) | MethodKey::Awaken(_, s) => s.items.len(),
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
        self.solve_capped(request, limit.into(), None)
    }

    /// Solve the request only for plans cheaper than `cap`, within the extended state limit: the
    /// plan returned may cost more when none is cheaper, and the search may give up.
    pub fn solve_under(&mut self, request: Request, cap: f64) -> Result<Arc<Solution<'a>>, Error> {
        self.solve_capped(request, Some(EXTENDED_LIMIT), Some(cap))
    }

    fn solve_capped(&mut self, request: Request, limit: Option<usize>, cap: Option<f64>) -> Result<Arc<Solution<'a>>, Error> {
        if request.goal.base.is_some_and(|base| base != request.kind.base) || request.goal.ilvl > request.kind.ilvl {
            return Err(Error::Unsupported("goal base or item level differs from the fixed crafting context".into()));
        }
        let supplies = request.starts.iter().map(|s| s.as_ref()).chain(request.methods.iter().filter_map(Method::partner));
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
        let key = Key::new(&request, cap);
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
                Method::Awaken(name, supply) => Ok(Box::new(Awakener::new(name, supply.clone())) as Box<dyn Operation>),
            })
            .collect::<Result<_, _>>()?;
        // Unpriced operations cannot be used, and must not split the lens's classes or
        // make an otherwise unavailable goal pass the producibility check.
        ops.retain(|op| self.market.price(&op.cost()).is_some());
        let refs: Vec<_> = ops.iter().map(|o| o.as_ref()).collect();
        if !producible(game, &request.goal, &refs, &request.starts) {
            return Err(Error::Solve(ponder_solve::Error::Unreachable));
        }
        // Mechanics that make items remember more (strands, imprints, locks, elevation) are
        // solved after the plain craft, capped by its cost and falling back on it.
        let tiered = (limit.is_none() || cap.is_some()) && ops.iter().any(|op| remembers(op.as_ref()));
        let plain = if tiered {
            let methods = request
                .methods
                .iter()
                .filter(|m| !matches!(m, Method::Named(name) if parse(game, class, name, &avoid).is_some_and(|op| remembers(op.as_ref()))))
                .cloned()
                .collect();
            self.solve_capped(Request { methods, ..request.clone() }, limit, cap).ok()
        } else {
            None
        };
        let inputs: Vec<_> = request.starts.iter().flat_map(|s| s.items()).collect();
        let mut lens = Lens::try_new(game, request.kind, vec![request.goal], &refs, &inputs).map_err(Error::Unsupported)?;
        let mut craft = Craft::new(&self.ctx, &lens, &refs, self.market, request.starts.clone(), 0);
        if let Some(bound) = plain.as_ref().map(|plain| plain.policy.cost).into_iter().chain(cap).reduce(f64::min) {
            craft.cap(bound);
        }
        let solution = match ponder_solve::solve_with(
            &craft,
            &[(1.0, Node::Market)],
            limit.or(plain.as_ref().map(|_| EXTENDED_LIMIT)),
            ponder_solve::Numerics::Hybrid,
        ) {
            Err(ponder_solve::Error::TooManyStates(_)) if let Some(plain) = plain => return Ok(plain),
            solved => solved.map_err(Error::Solve)?,
        };
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

/// States a capped craft with remembering mechanics may explore before its plain plan stands.
const EXTENDED_LIMIT: usize = 2_000_000;

/// Does the operation make items carry more than their mods (strands, an imprint, a lock) or
/// elevate them?
fn remembers(op: &dyn Operation) -> bool {
    op.steps().iter().any(|s| matches!(s, Step::Remember | Step::Imprint | Step::Restore | Step::Foresee | Step::Dominate))
}

fn producible(game: &Game, goal: &Target, ops: &[&dyn Operation], starts: &[Arc<dyn Supply>]) -> bool {
    let mut available: HashSet<_> = starts.iter().flat_map(|s| s.items()).flat_map(|item| item.affixes.iter().map(|a| a.id)).collect();
    let (mut rolls, mut unveils, mut elevates) = (false, false, false);
    for step in ops.iter().flat_map(|op| op.steps()) {
        match step {
            Step::Force(m) => {
                available.insert(*m);
            }
            Step::Craft(mods) | Step::CraftAny(mods) => available.extend(mods.iter().copied()),
            Step::Recombine(supply) | Step::Awaken(supply) => {
                available.extend(supply.0.items().iter().flat_map(|item| item.affixes.iter().map(|a| a.id)))
            }
            Step::Veil(_) => available.extend(game.veils),
            Step::Unveil(_) => unveils = true,
            Step::Dominate => elevates = true,
            Step::Draw(_) | Step::Fill(_) | Step::Swap { .. } | Step::RecombineBonus => rolls = true,
            Step::Clear { .. }
            | Step::Rarity(_)
            | Step::Settle
            | Step::RollCount
            | Step::Remove { .. }
            | Step::RemoveCrafted
            | Step::Influence(_)
            | Step::Eldritch(_)
            | Step::Fracture
            | Step::Forget(_)
            | Step::Remember
            | Step::Imprint
            | Step::Restore
            | Step::Foresee => {}
        }
    }
    goal.needs.iter().all(|need| {
        need.ok
            .iter()
            .filter(|&&m| {
                available.contains(&m)
                    || rolls && matches!(game.def(m).source, Source::Natural | Source::Influence | Source::Fossil)
                    || unveils && game.def(m).source == Source::Veiled
                    || elevates && game.def(m).source == Source::Elevated
            })
            .count()
            >= need.count as usize
    })
}
