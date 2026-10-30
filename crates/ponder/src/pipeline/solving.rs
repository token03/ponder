use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Result, anyhow};
use ponder_plan::{
    market::Market,
    session::{Method, Session},
};
use ponder_rules::ops::{self, Ctx};
use rayon::prelude::*;

use super::{
    cache::PLAN_VERSION,
    context::{Context, Progress},
    types::Estimate,
};
use crate::candidate::Candidate;
use crate::gear::Prices;
use crate::plan::Summary;

pub(super) fn in_pool<T: Send>(workers: Option<usize>, work: impl FnOnce() -> T + Send) -> T {
    if let Some(workers) = workers {
        rayon::ThreadPoolBuilder::new().num_threads(workers.max(1)).build().expect("worker pool").install(work)
    } else {
        work()
    }
}

fn solve(cx: &Context, problem: &Candidate, prices: &Prices, base_price: f64) -> Result<Summary> {
    let (game, _) = cx.rules()?;
    let market = Market { prices: prices.clone(), chaos_per_hour: cx.settings.chaos_per_hour, base: base_price };
    let request = crate::plan::prepare(game, problem, &market)?;
    let avoid: Vec<_> = request.goal.needs.iter().flat_map(|need| &need.ok).flat_map(|&id| game.def(id).groups.iter().copied()).collect();
    let class = &game.bases[request.kind.base.0 as usize].class;
    let unpriced: Vec<String> = request
        .methods
        .iter()
        .filter_map(|method| {
            let Method::Named(name) = method else {
                return None;
            };
            let operation = ops::parse(game, class, name, &avoid)?;
            market.price(&operation.cost()).is_none().then(|| name.clone())
        })
        .collect();
    let mut session = Session::new(Ctx { game, assumptions: &cx.assumptions }, &market);
    let plan = session.solve(request, None).map_err(|error| {
        let message = match error {
            ponder_plan::session::Error::Unsupported(reason) => reason,
            ponder_plan::session::Error::Solve(ponder_solve::Error::Unreachable) => {
                "No policy reaches this goal with the available priced operations".into()
            }
            ponder_plan::session::Error::Solve(ponder_solve::Error::TooManyStates(limit)) => {
                format!("Search exceeded {limit} states")
            }
        };
        if unpriced.is_empty() { anyhow!(message) } else { anyhow!("{message}; missing prices: {}", unpriced.join(", ")) }
    })?;
    let usage = plan.usage().map_err(|error| anyhow!(error))?;
    let solution = &plan.policy;
    Ok(Summary {
        cost: solution.cost - usage.bases * base_price,
        hours: usage.seconds / 3600.0,
        bases: usage.bases,
        states: solution.explored,
        actions: solution.actions,
        outcomes: solution.outcomes,
        usage: usage.actions,
        unpriced,
    })
}

pub fn solve_one(cx: &Context, estimate: &mut Estimate, prices: &Prices) {
    let Some(problem) = &estimate.candidate else {
        return;
    };
    let result = (|| -> Result<Arc<Summary>> {
        let (_, data_key) = cx.rules()?;
        let sorted: BTreeMap<_, _> = prices.iter().collect();
        let base_price = estimate.base_price.map_or(0.0, |price| price.0);
        let key = blake3::hash(&serde_json::to_vec(&(
            PLAN_VERSION,
            problem.fingerprint().to_string(),
            sorted,
            cx.settings.chaos_per_hour,
            base_price,
            data_key.as_bytes(),
            format!("{:?}", cx.assumptions),
        ))?)
        .to_hex()
        .to_string();
        cx.plans
            .get_or_solve(&key, || solve(cx, problem, prices, base_price).map_err(|error| error.to_string()))
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
}

pub fn solve_estimates(cx: &Context, estimates: &mut [Estimate], prices: &Prices, workers: Option<usize>, progress: Option<Progress>) {
    let done = AtomicUsize::new(0);
    let total = estimates.len();
    in_pool(workers, || {
        estimates.par_iter_mut().for_each(|estimate| {
            solve_one(cx, estimate, prices);
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(say) = progress {
                say(&format!("processed {n}/{total}: {}", estimate.title));
            }
        })
    });
}
