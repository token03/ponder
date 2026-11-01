use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use anyhow::{Result, anyhow};
use ponder_plan::{
    market::Market,
    session::{Error as SessionError, Method, Request, Session, Solution, remembers},
    supply::Product,
};
use ponder_rules::game::Game;
use ponder_rules::item::Item;
use ponder_rules::ops::{self, Ctx, Supply};
use rayon::prelude::*;

use super::{
    cache::PLAN_VERSION,
    context::{Context, Progress},
    types::Estimate,
};
use crate::candidate::Candidate;
use crate::gear::Prices;
use crate::plan::{Chain, Part, Stage, Summary};

pub(super) fn in_pool<T: Send>(workers: Option<usize>, work: impl FnOnce() -> T + Send) -> T {
    if let Some(workers) = workers {
        rayon::ThreadPoolBuilder::new().num_threads(workers.max(1)).build().expect("worker pool").install(work)
    } else {
        work()
    }
}

/// Chaos for a base with no trade quote: a fresh base is never free.
const UNQUOTED_BASE_PRICE: f64 = 1.0;

/// A part of a staged craft: what it costs and the items it yields.
pub struct Made {
    product: Arc<Product>,
    summary: Summary,
}

/// Runs followed to sample the items a part yields.
const PART_RUNS: usize = 100;

fn explain(error: SessionError, unpriced: &[String]) -> anyhow::Error {
    let message = match error {
        SessionError::Unsupported(reason) => reason,
        SessionError::Solve(ponder_solve::Error::Unreachable) => "No policy reaches this goal with the available priced operations".into(),
        SessionError::Solve(ponder_solve::Error::TooManyStates(limit)) => {
            format!("Search exceeded {limit} states")
        }
    };
    if unpriced.is_empty() { anyhow!(message) } else { anyhow!("{message}; missing prices: {}", unpriced.join(", ")) }
}

/// Everything included, bases too.
fn total(summary: &Summary, base_price: f64) -> f64 {
    summary.cost + summary.bases * base_price
}

/// The summary of one solved item, followed `runs` times for its recipe's order and its items.
fn summarize(plan: &Solution, stage: Stage, base_price: f64, runs: usize) -> Result<(Summary, Vec<Item>)> {
    let usage = plan.usage().map_err(|error| anyhow!(error))?;
    let followed = plan.craft().follow(&plan.policy, runs, 1);
    let mut ops: Vec<String> = Vec::new();
    for (action, _) in &followed.example {
        if action != "start over" && !ops.contains(action) {
            ops.push(action.clone());
        }
    }
    let solution = &plan.policy;
    let stage = Stage { ops, uses: usage.actions.clone(), cost: solution.cost, ..stage };
    let summary = Summary {
        cost: solution.cost - usage.bases * base_price,
        hours: usage.seconds / 3600.0,
        bases: usage.bases,
        states: solution.explored,
        actions: solution.actions,
        outcomes: solution.outcomes,
        usage: usage.actions,
        unpriced: Vec::new(),
        stages: vec![stage],
    };
    Ok((summary, followed.items))
}

struct Solver<'a, 's> {
    cx: &'a Context,
    session: Session<'s>,
    problem: &'a Candidate,
    base_price: f64,
    unpriced: Vec<String>,
    /// Identifies the market, so parts made under other prices are not reused.
    market_key: u64,
}

impl Solver<'_, '_> {
    fn stage(&self, item: &str, request: &Request, needs: &[usize]) -> Stage {
        Stage {
            item: item.into(),
            influences: request.kind.influences.iter().map(|i| format!("{i:?}").to_lowercase()).collect(),
            rarity: request.goal.rarity.map(|r| format!("{r:?}").to_lowercase()),
            needs: needs.iter().map(|&i| self.problem.needs[i].name.clone()).collect(),
            ..Stage::default()
        }
    }

    fn made(&mut self, part: &Part) -> Result<Arc<Made>> {
        let names: Vec<String> =
            part.request.methods.iter().filter_map(|m| if let Method::Named(n) = m { Some(n.clone()) } else { None }).collect();
        let mut hasher = DefaultHasher::new();
        (self.market_key, part.request.kind, &part.request.goal, names, self.base_price.to_bits()).hash(&mut hasher);
        let key = hasher.finish();
        if let Some(made) = self.cx.parts.lock().expect("parts").get(&key) {
            return made.clone().map_err(|error| anyhow!(error));
        }
        let made = self
            .session
            .solve(part.request.clone(), None)
            .map_err(|error| explain(error, &[]))
            .and_then(|plan| summarize(&plan, self.stage(&part.name, &part.request, &part.needs), self.base_price, PART_RUNS))
            .map(|(summary, items)| {
                let product = Arc::new(Product::new(&part.name));
                product.fill(total(&summary, self.base_price), items);
                Arc::new(Made { product, summary })
            })
            .map_err(|error| format!("{}: {error}", part.name));
        self.cx.parts.lock().expect("parts").entry(key).or_insert(made).clone().map_err(|error| anyhow!(error))
    }

    fn direct(&mut self, request: &Request) -> Result<Summary> {
        let plan = self.session.solve(request.clone(), None).map_err(|error| explain(error, &self.unpriced))?;
        let all: Vec<usize> = (0..request.goal.needs.len()).collect();
        Ok(summarize(&plan, self.stage("main", request, &all), self.base_price, 1)?.0)
    }

    /// The chain's craft, cheaper than `bound` if it can be.
    fn staged(&mut self, chain: &Chain, request: &Request, bound: f64) -> Result<Summary> {
        let heirs = chain.heirs.iter().map(|part| self.made(part)).collect::<Result<Vec<_>>>()?;
        let partners = chain.partners.iter().map(|part| self.made(part)).collect::<Result<Vec<_>>>()?;
        let supplies = |made: &[Arc<Made>]| made.iter().map(|m| m.product.clone() as Arc<dyn Supply>).collect();
        let finish = chain.finish(supplies(&heirs), supplies(&partners));
        let plan = if bound.is_finite() { self.session.solve_under(finish, bound) } else { self.session.solve(finish, None) };
        let plan = plan.map_err(|error| explain(error, &self.unpriced))?;
        let all: Vec<usize> = (0..request.goal.needs.len()).collect();
        let (mut summary, _) = summarize(&plan, self.stage("main", request, &all), self.base_price, 1)?;
        let usage = plan.usage().map_err(|error| anyhow!(error))?;
        let times = |name: &str, list: &[(String, f64)]| list.iter().find(|(n, _)| n == name).map_or(0.0, |(_, uses)| *uses);
        let used: Vec<(&Made, f64)> = chain
            .heirs
            .iter()
            .zip(&heirs)
            .map(|(part, made)| (made.as_ref(), times(&part.name, &usage.buys)))
            .chain(
                chain
                    .partners
                    .iter()
                    .zip(&partners)
                    .map(|(part, made)| (made.as_ref(), times(&format!("{}:{}", chain.mechanic, part.name), &usage.actions))),
            )
            .filter(|(_, uses)| *uses > 0.0)
            .collect();
        summary.bases = used.iter().map(|(made, uses)| uses * made.summary.bases).sum();
        summary.hours += used.iter().map(|(made, uses)| uses * made.summary.hours).sum::<f64>();
        summary.cost = plan.policy.cost - summary.bases * self.base_price;
        summary.states += used.iter().map(|(made, _)| made.summary.states).sum::<usize>();
        let mut stages: Vec<Stage> = used.iter().flat_map(|(made, _)| made.summary.stages.clone()).collect();
        stages.append(&mut summary.stages);
        summary.stages = stages;
        Ok(summary)
    }
}

impl Solver<'_, '_> {
    /// The cheapest route: bought and worked directly, unless the item has two influences, or
    /// made in stages, each capped by the cheapest found before it.
    fn best(&mut self, game: &Game, request: &Request, market: &Market, quick: bool, mut bound: f64) -> Result<Summary> {
        let mut routes = Vec::new();
        // A base with two influences is not for sale: such items are awakened.
        if request.kind.influences.iter().count() < 2 {
            routes.push(self.direct(request));
        }
        for chain in crate::plan::chains(game, request, market) {
            if chain.mechanic == "recombinator" && (quick || !self.cx.settings.recombine) {
                continue;
            }
            bound = routes.iter().flatten().map(|s| total(s, self.base_price)).fold(bound, f64::min);
            routes.push(self.staged(&chain, request, bound).map_err(|error| anyhow!("{}: {error}", chain.mechanic)));
        }
        routes
            .iter()
            .flatten()
            .min_by(|a, b| total(a, self.base_price).total_cmp(&total(b, self.base_price)))
            .cloned()
            .ok_or_else(|| anyhow!(routes.iter().filter_map(|r| r.as_ref().err()).map(|e| e.to_string()).collect::<Vec<_>>().join("; ")))
    }
}

/// The plan for the problem. The quick plan, without the mechanics that make items remember
/// or recombining, is solved first and passed to `quick`; the full solve reuses its work.
fn solve(cx: &Context, problem: &Candidate, prices: &Prices, base_price: f64, quick: &dyn Fn(&Summary)) -> Result<Summary> {
    let (game, _) = cx.rules()?;
    let market = Market { prices: prices.clone(), chaos_per_hour: cx.settings.chaos_per_hour, base: base_price };
    let request = crate::plan::prepare(game, problem, &market)?;
    let avoid: Vec<_> = request.goal.needs.iter().flat_map(|need| &need.ok).flat_map(|&id| game.def(id).groups.iter().copied()).collect();
    let class = &game.bases[request.kind.base.0 as usize].class;
    let parsed = |name: &str| ops::parse(game, class, name, &avoid);
    let unpriced: Vec<String> = request
        .methods
        .iter()
        .filter_map(|method| {
            let Method::Named(name) = method else {
                return None;
            };
            market.price(&parsed(name)?.cost()).is_none().then(|| name.clone())
        })
        .collect();
    let mut plain = request.clone();
    plain.methods.retain(|m| !matches!(m, Method::Named(name) if parsed(name).is_some_and(|op| remembers(op.as_ref()))));
    let mut hasher = DefaultHasher::new();
    serde_json::to_string(&prices.iter().collect::<BTreeMap<_, _>>())?.hash(&mut hasher);
    let session = Session::new(Ctx { game, assumptions: &cx.assumptions }, &market);
    let mut solver = Solver { cx, session, problem, base_price, unpriced, market_key: hasher.finish() };
    let first = solver.best(game, &plain, &market, true, f64::INFINITY);
    let slow = plain.methods.len() < request.methods.len()
        || cx.settings.recombine && crate::plan::chains(game, &request, &market).iter().any(|c| c.mechanic == "recombinator");
    let mut summary = match (first, slow) {
        (Ok(first), true) => {
            quick(&Summary { unpriced: solver.unpriced.clone(), ..first.clone() });
            let bound = total(&first, base_price);
            match solver.best(game, &request, &market, false, bound) {
                Ok(full) if total(&full, base_price) < bound => full,
                _ => first,
            }
        }
        (Ok(first), false) => first,
        (Err(_), true) => solver.best(game, &request, &market, false, f64::INFINITY)?,
        (Err(error), false) => return Err(error),
    };
    summary.unpriced = solver.unpriced;
    Ok(summary)
}

pub fn solve_one(cx: &Context, estimate: &mut Estimate, prices: &Prices) {
    let Some(problem) = &estimate.candidate else {
        return;
    };
    let quick = |summary: &Summary| {
        if let Some(stream) = &cx.on_solved {
            stream(&Estimate { summary: Some(Arc::new(summary.clone())), error: None, ..estimate.clone() }, false);
        }
    };
    let result = (|| -> Result<Arc<Summary>> {
        let (_, data_key) = cx.rules()?;
        let sorted: BTreeMap<_, _> = prices.iter().collect();
        let base_price = estimate.base_price.map_or(UNQUOTED_BASE_PRICE, |price| price.0);
        let key = blake3::hash(&serde_json::to_vec(&(
            PLAN_VERSION,
            problem.fingerprint().to_string(),
            sorted,
            cx.settings.chaos_per_hour,
            cx.settings.recombine,
            base_price,
            data_key.as_bytes(),
            format!("{:?}", cx.assumptions),
        ))?)
        .to_hex()
        .to_string();
        cx.plans
            .get_or_solve(&key, || solve(cx, problem, prices, base_price, &quick).map_err(|error| error.to_string()))
            .map_err(|error| anyhow!(error))
    })();
    match result {
        Ok(plan) => {
            estimate.summary = Some(plan);
            estimate.error = None;
        }
        Err(error) => {
            estimate.summary = None;
            estimate.error = Some(error.to_string());
        }
    }
    if let Some(stream) = &cx.on_solved {
        stream(estimate, true);
    }
}

pub fn solve_estimates(cx: &Context, estimates: &mut [Estimate], prices: &Prices, workers: Option<usize>, progress: Option<Progress>) {
    let done = AtomicUsize::new(0);
    let total = estimates.len();
    let price = |estimate: &mut Estimate| {
        solve_one(cx, estimate, prices);
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(say) = progress {
            say(&format!("processed {n}/{total}: {}", estimate.title));
        }
    };
    in_pool(workers, || {
        if total <= 1 || rayon::current_num_threads() == 1 {
            estimates.iter_mut().for_each(price);
            return;
        }
        let pending = Mutex::new(estimates.iter_mut());
        (0..rayon::current_num_threads().min(total)).into_par_iter().for_each(|_| {
            let solver = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("solver pool");
            solver.install(|| {
                loop {
                    let next = pending.lock().expect("pending estimates").next();
                    let Some(estimate) = next else { break };
                    price(estimate);
                }
            });
        })
    });
}
