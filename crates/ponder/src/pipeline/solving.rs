use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use anyhow::{Result, anyhow};
use ponder_plan::{
    market::Market,
    session::{Error as SessionError, Method, Request, Session, Solution},
    supply::Product,
};
use ponder_rules::game::{Game, Side};
use ponder_rules::goal::Rarity;
use ponder_rules::item::{Item, limit};
use ponder_rules::ops::{self, Ctx, Supply};
use rayon::prelude::*;

use super::{
    cache::PLAN_VERSION,
    context::{Context, Progress},
    types::Estimate,
};
use crate::candidate::Candidate;
use crate::gear::Prices;
use crate::plan::{Chain, Depth, Part, Stage, Summary};

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
    let stage = Stage { ops, uses: usage.actions.clone(), spend: usage.spend, flow: usage.flow, cost: solution.cost, ..stage };
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
        depth: Depth::default(),
    };
    Ok((summary, followed.items))
}

struct Solver<'a, 's> {
    cx: &'a Context,
    game: &'a Game,
    session: Session<'s>,
    problem: &'a Candidate,
    base_price: f64,
    unpriced: Vec<String>,
    /// Identifies the market, so parts made under other prices are not reused.
    market_key: u64,
}

impl Solver<'_, '_> {
    fn stage(&self, item: &str, request: &Request, needs: &[usize]) -> Stage {
        let game = self.game;
        let place = |need: &ponder_rules::target::Need| {
            let sides: BTreeSet<Side> = need.ok.iter().map(|&m| game.def(m).side).collect();
            let side = match sides.into_iter().collect::<Vec<_>>().as_slice() {
                [Side::Prefix] => "prefix",
                [Side::Suffix] => "suffix",
                _ => "either",
            };
            (side.to_string(), need.count)
        };
        let base = request.kind.base;
        let slots = |rarity| [limit(game, base, rarity, Side::Prefix), limit(game, base, rarity, Side::Suffix)];
        Stage {
            places: request.goal.needs.iter().map(place).collect(),
            slots: [slots(Rarity::Magic), slots(Rarity::Rare)],
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
    fn best(&mut self, game: &Game, request: &Request, market: &Market, depth: Depth, mut bound: f64) -> Result<Summary> {
        let mut routes = Vec::new();
        // A base with two influences is not for sale: such items are awakened.
        if request.kind.influences.iter().count() < 2 {
            routes.push(self.direct(request));
        }
        for chain in crate::plan::chains(game, request, market, depth) {
            if chain.mechanic == "recombinator" && depth == Depth::Ladder {
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

/// The plan for the problem at `depth`. A deep solve passes the ladder plan to `quick` first
/// and keeps it unless trying every mechanic beats it.
fn solve(cx: &Context, problem: &Candidate, prices: &Prices, base_price: f64, depth: Depth, quick: &dyn Fn(&Summary)) -> Result<Summary> {
    let (game, _) = cx.rules()?;
    let market = Market { prices: prices.clone(), chaos_per_hour: cx.settings.chaos_per_hour, base: base_price };
    let ladder = crate::plan::prepare(game, problem, &market, Depth::Ladder)?;
    let full = crate::plan::prepare(game, problem, &market, Depth::Deep)?;
    let avoid: Vec<_> = full.goal.needs.iter().flat_map(|need| &need.ok).flat_map(|&id| game.def(id).groups.iter().copied()).collect();
    let class = &game.bases[full.kind.base.0 as usize].class;
    let unpriced: Vec<String> = full
        .methods
        .iter()
        .filter_map(|method| {
            let Method::Named(name) = method else {
                return None;
            };
            market.price(&ops::parse(game, class, name, &avoid)?.cost()).is_none().then(|| name.clone())
        })
        .collect();
    let mut hasher = DefaultHasher::new();
    serde_json::to_string(&prices.iter().collect::<BTreeMap<_, _>>())?.hash(&mut hasher);
    let session = Session::new(Ctx { game, assumptions: &cx.assumptions }, &market);
    let mut solver = Solver { cx, game, session, problem, base_price, unpriced, market_key: hasher.finish() };
    let awakened = ladder.kind.influences.iter().count() >= 2;
    let first = {
        let _heavy = awakened.then(|| cx.heavy.enter());
        solver.best(game, &ladder, &market, Depth::Ladder, f64::INFINITY)
    };
    let mut summary = match (first, depth) {
        (first, Depth::Ladder) => first?,
        (Ok(first), Depth::Deep) => {
            quick(&Summary { unpriced: solver.unpriced.clone(), ..first.clone() });
            let bound = total(&first, base_price);
            let _heavy = cx.heavy.enter();
            match solver.best(game, &full, &market, Depth::Deep, bound) {
                Ok(full) if total(&full, base_price) < bound => full,
                _ => first,
            }
        }
        (Err(_), Depth::Deep) => {
            let _heavy = cx.heavy.enter();
            solver.best(game, &full, &market, Depth::Deep, f64::INFINITY)?
        }
    };
    summary.unpriced = solver.unpriced;
    summary.depth = depth;
    Ok(summary)
}

/// The cache keys of an estimate's plans.
struct Keys {
    /// solved under these prices: (ladder, deep)
    exact: [String; 2],
    /// the latest plan, under any prices
    latest: String,
}

impl Keys {
    fn new(cx: &Context, problem: &Candidate, prices: &Prices, base_price: f64) -> Result<Keys> {
        let (_, data_key) = cx.rules()?;
        let sorted: BTreeMap<_, _> = prices.iter().collect();
        let key = |priced: bool, depth: Option<Depth>| -> Result<String> {
            let bytes = serde_json::to_vec(&(
                PLAN_VERSION,
                problem.fingerprint().to_string(),
                priced.then_some(&sorted),
                priced.then_some(cx.settings.chaos_per_hour),
                depth,
                priced.then_some(base_price),
                data_key.as_bytes(),
                format!("{:?}", cx.assumptions),
            ))?;
            Ok(blake3::hash(&bytes).to_hex().to_string())
        };
        Ok(Keys { exact: [key(true, Some(Depth::Ladder))?, key(true, Some(Depth::Deep))?], latest: key(false, None)? })
    }

    fn exact(&self, depth: Depth) -> &str {
        &self.exact[depth as usize]
    }
}

/// How much of an estimate's plan the cache had.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Recalled {
    Missing,
    /// a plan solved under other prices: shown until it is solved again
    Stale,
    Fresh,
}

fn base_price(estimate: &Estimate) -> f64 {
    estimate.base_price.map_or(UNQUOTED_BASE_PRICE, |price| price.0)
}

fn stream(cx: &Context, estimate: &Estimate, refined: bool) {
    if let Some(stream) = &cx.on_solved {
        stream(estimate, refined);
    }
}

fn set(estimate: &mut Estimate, plan: Result<Arc<Summary>, String>) {
    match plan {
        Ok(plan) => {
            estimate.summary = Some(plan);
            estimate.error = None;
        }
        Err(error) => {
            estimate.summary = None;
            estimate.error = Some(error);
        }
    }
}

/// The estimate's plan from the cache, if it has one: a ladder solve takes a deep plan, and a
/// plan under other prices stands in until it is solved again.
fn recall(cx: &Context, estimate: &mut Estimate, prices: &Prices) -> Recalled {
    let Some(problem) = &estimate.candidate else {
        return Recalled::Fresh;
    };
    let keys = match Keys::new(cx, problem, prices, base_price(estimate)) {
        Ok(keys) => keys,
        Err(error) => {
            set(estimate, Err(error.to_string()));
            stream(cx, estimate, true);
            return Recalled::Fresh;
        }
    };
    let fresh = [Depth::Deep, Depth::Ladder].iter().find_map(|&depth| cx.plans.get(keys.exact(depth)));
    if let Some(plan) = fresh {
        set(estimate, plan);
        stream(cx, estimate, true);
        return Recalled::Fresh;
    }
    match cx.plans.get(&keys.latest) {
        Some(Ok(plan)) => {
            set(estimate, Ok(plan));
            stream(cx, estimate, false);
            Recalled::Stale
        }
        _ => Recalled::Missing,
    }
}

/// Solve the estimate to `depth`, unless the cache has it. A ladder solve takes a deep plan.
pub fn solve_one(cx: &Context, estimate: &mut Estimate, prices: &Prices, depth: Depth) {
    let Some(problem) = &estimate.candidate else {
        return;
    };
    let quick =
        |summary: &Summary| stream(cx, &Estimate { summary: Some(Arc::new(summary.clone())), error: None, ..estimate.clone() }, false);
    let base_price = base_price(estimate);
    let plan = Keys::new(cx, problem, prices, base_price).map_err(|error| error.to_string()).and_then(|keys| {
        if depth == Depth::Ladder
            && let Some(deep) = cx.plans.get(keys.exact(Depth::Deep))
        {
            return deep;
        }
        let plan = cx
            .plans
            .get_or_solve(keys.exact(depth), || solve(cx, problem, prices, base_price, depth, &quick).map_err(|error| error.to_string()));
        if let Ok(plan) = &plan {
            cx.plans.put(&keys.latest, Ok(plan));
        }
        plan
    });
    set(estimate, plan);
    stream(cx, estimate, true);
}

/// Peak memory a worker may need for a plain craft.
const WORKER_BYTES: u64 = 2_500_000_000;
/// Peak memory a heavy solve may need, on top of a worker's.
const HEAVY_BYTES: u64 = 3 * WORKER_BYTES;

fn available_memory() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = info.lines().find(|line| line.starts_with("MemAvailable:"))?;
    line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kib| kib * 1024)
}

/// Workers that fit in the memory available, leaving a core for everything else.
fn default_workers() -> usize {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let fit = available_memory().map_or(cores, |bytes| (bytes / WORKER_BYTES) as usize);
    fit.min(cores.saturating_sub(1)).max(1)
}

/// Heavy solves that fit in the memory available at once.
pub(super) fn heavy_slots() -> usize {
    available_memory().map_or(1, |bytes| (bytes / HEAVY_BYTES) as usize).max(1)
}

fn heavy(estimate: &Estimate) -> bool {
    estimate.candidate.as_ref().is_some_and(|problem| problem.influences.len() >= 2)
}

/// Solve the estimates on `workers` threads, by default as many as memory allows. Saved plans
/// are taken first, so they all show at once; then the rest are solved, the missing before the
/// stale and the simplest first.
pub fn solve_estimates(cx: &Context, estimates: &mut [Estimate], prices: &Prices, workers: Option<usize>, progress: Option<Progress>) {
    in_pool(Some(workers.unwrap_or_else(default_workers)), || {
        let recalled: Vec<Recalled> = estimates.par_iter_mut().map(|estimate| recall(cx, estimate, prices)).collect();
        let mut pending: Vec<(Recalled, &mut Estimate)> =
            recalled.into_iter().zip(estimates.iter_mut()).filter(|(recalled, _)| *recalled != Recalled::Fresh).collect();
        pending.sort_by_key(|(recalled, estimate)| {
            let problem = estimate.candidate.as_ref().expect("only candidates are solved");
            (*recalled, problem.influences.len(), problem.needs.len())
        });
        let (done, total) = (AtomicUsize::new(0), pending.len());
        let price = |estimate: &mut Estimate| {
            let clock = std::time::Instant::now();
            solve_one(cx, estimate, prices, Depth::Ladder);
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(say) = progress {
                say(&format!("solved {n}/{total} in {:.1}s: {}", clock.elapsed().as_secs_f64(), estimate.title));
            }
        };
        let (heavy, light): (VecDeque<_>, VecDeque<_>) = pending.into_iter().map(|(_, estimate)| estimate).partition(|e| heavy(e));
        let queues = Mutex::new((heavy, light));
        // Heavy crafts start whenever there is room for one, so they overlap the light ones
        // instead of queueing at the end.
        let next = || {
            if cx.cancelled.load(Ordering::Relaxed) {
                return None;
            }
            let mut queues = queues.lock().expect("pending estimates");
            let (heavy, light) = &mut *queues;
            if cx.heavy.has_room() {
                heavy.pop_front().or_else(|| light.pop_front())
            } else {
                light.pop_front().or_else(|| heavy.pop_front())
            }
        };
        (0..rayon::current_num_threads().min(total)).into_par_iter().for_each(|_| {
            let solver = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("solver pool");
            solver.install(|| {
                while let Some(estimate) = next() {
                    price(estimate);
                }
            });
        });
    });
}
