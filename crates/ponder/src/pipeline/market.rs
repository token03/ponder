//! Trade prices: what finished items sell for and what fractured bases cost.

use ponder_core::Prices;

use super::context::{Context, Progress};
use super::types::{Estimate, Fracture};

/// What a trade lookup prices.
pub enum Lookup<'a> {
    /// an Estimate's selling price
    Market(&'a mut Estimate),
    /// an Estimate's fractured base
    Base(&'a mut Estimate),
    /// a Fracture's base
    Fracture(&'a mut Fracture),
}

/// The words for a failed search: the site's status, or what went wrong.
fn describe_error(e: &anyhow::Error) -> String {
    let text = e.to_string();
    let code = text.strip_prefix("HTTP ").and_then(|r| r.split_whitespace().next()).and_then(|c| c.parse::<u16>().ok());
    match code {
        Some(c) => format!("trade {c}: {}", if c == 401 || c == 403 { "check POESESSID in .env" } else { "try again later" }),
        None => format!("trade failed: {text}"),
    }
}

/// One trade search for an Estimate's market or base, or a Fracture's base.
///
/// Returns 1 if it hit the site, 0 if the answer was cached (or there was no query).
pub fn lookup(cx: &Context, obj: Lookup, table: &Prices, league: &str) -> usize {
    let query = match &obj {
        Lookup::Market(e) => e.query.clone(),
        Lookup::Base(e) => e.base_query.clone(),
        Lookup::Fracture(f) => f.query.clone(),
    };
    let Some(query) = query else { return 0 };
    let before = cx.trade.searches();
    let (found, err) = match cx.trade.price(league, &query, table, 10, None) {
        Ok(Some(found)) => (Some(found), None),
        Ok(None) => (None, Some("no listings".to_string())),
        Err(e) => (None, Some(describe_error(&e))),
    };
    match obj {
        Lookup::Market(e) => (e.market, e.market_error) = (found, err),
        Lookup::Base(e) => {
            (e.base_price, e.base_error) = (found, err);
            if found.is_some() {
                e.base_source = "trade".into();
            }
        }
        Lookup::Fracture(f) => (f.base_price, f.base_error) = (found, err),
    }
    cx.trade.searches() - before
}

/// Price one estimate: its selling price and, if it needs one, its fractured base.
pub fn lookup_market(cx: &Context, est: &mut Estimate, table: &Prices, league: &str) {
    lookup(cx, Lookup::Market(est), table, league);
    if est.base_query.is_some() && est.base_price.is_none() {
        lookup(cx, Lookup::Base(est), table, league);
    }
}

#[derive(Clone, Copy)]
enum Job {
    Market(usize),
    Base(usize),
    Fracture(usize, usize),
}

/// The indices of `e`'s fractures that have a query and a plan, cheapest to finish first.
fn ranked_fractures(e: &Estimate) -> Vec<usize> {
    let mut v: Vec<usize> = (0..e.fractures.len())
        .filter(|&j| e.fractures[j].query.is_some() && (e.fractures[j].plan.is_some() || e.fractures[j].craft_lower_bound.is_some()))
        .collect();
    let cost = |i: usize| e.fractures[i].craft_cost().or(e.fractures[i].craft_lower_bound);
    v.sort_by(|&a, &b| cost(a).partial_cmp(&cost(b)).unwrap_or(std::cmp::Ordering::Equal));
    v
}

/// Price `estimates[i]` for each `i` in `order`: each one's selling price, then its fractured
/// base, then fracture bases. Stops after `budget` searches that weren't cached.
/// Returns searches used.
pub fn price_on_trade(
    cx: &Context,
    estimates: &mut [Estimate],
    order: &[usize],
    table: &Prices,
    league: &str,
    budget: usize,
    progress: Option<Progress>,
) -> usize {
    let say = |m: &str| {
        if let Some(p) = progress {
            p(m)
        }
    };
    let mut jobs: Vec<Job> = Vec::new();
    for &i in order {
        let e = &estimates[i];
        if e.query.is_some() && e.market.is_none() {
            jobs.push(Job::Market(i));
        }
        if e.base_query.is_some() && e.base_price.is_none() {
            jobs.push(Job::Base(i));
        }
    }
    // Fractured bases: every item's two most promising first (cheapest to finish), then the rest.
    let ranked: Vec<(usize, Vec<usize>)> = order.iter().map(|&i| (i, ranked_fractures(&estimates[i]))).collect();
    for first in [true, false] {
        for (i, r) in &ranked {
            let part = if first { &r[..r.len().min(2)] } else { &r[r.len().min(2)..] };
            jobs.extend(part.iter().filter(|&&j| estimates[*i].fractures[j].base_price.is_none()).map(|&j| Job::Fracture(*i, j)));
        }
    }
    let mut used = 0;
    for (n, job) in jobs.iter().enumerate() {
        if used >= budget {
            say(&format!("trade budget of {budget} searches used; {} left unpriced", jobs.len() - n));
            break;
        }
        let (what, name) = match *job {
            Job::Market(i) => ("market", estimates[i].title.clone()),
            Job::Base(i) => ("base", estimates[i].title.clone()),
            Job::Fracture(i, j) => ("fracture", estimates[i].fractures[j].target.clone()),
        };
        say(&format!("trade {}/{}: {what} \u{b7} {name}", n + 1, jobs.len()));
        used += match *job {
            Job::Market(i) => lookup(cx, Lookup::Market(&mut estimates[i]), table, league),
            Job::Base(i) => lookup(cx, Lookup::Base(&mut estimates[i]), table, league),
            Job::Fracture(i, j) => lookup(cx, Lookup::Fracture(&mut estimates[i].fractures[j]), table, league),
        };
    }
    super::solving::solve_estimates(cx, estimates, table, None, progress);
    used
}

/// Batch-price fractured bases: each item's `per_item` most promising fractures (cheapest to finish).
pub fn price_fractures(
    cx: &Context,
    estimates: &mut [Estimate],
    table: &Prices,
    league: &str,
    per_item: usize,
    budget: usize,
    progress: Option<Progress>,
) -> usize {
    let say = |m: &str| {
        if let Some(p) = progress {
            p(m)
        }
    };
    let mut jobs: Vec<(usize, usize)> = Vec::new();
    for (i, e) in estimates.iter().enumerate() {
        let best: Vec<usize> = ranked_fractures(e).into_iter().filter(|&j| e.fractures[j].base_price.is_none()).collect();
        jobs.extend(best.into_iter().take(per_item).map(|j| (i, j)));
    }
    let mut used = 0;
    for (n, &(i, j)) in jobs.iter().enumerate() {
        if used >= budget {
            say(&format!("trade budget of {budget} searches used; {} fractured bases left unpriced", jobs.len() - n));
            break;
        }
        say(&format!("fractured base {}/{}: {}", n + 1, jobs.len(), estimates[i].fractures[j].target));
        used += lookup(cx, Lookup::Fracture(&mut estimates[i].fractures[j]), table, league);
    }
    super::solving::solve_estimates(cx, estimates, table, None, progress);
    used
}
