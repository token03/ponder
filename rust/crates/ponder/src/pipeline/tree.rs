//! The product tree: which crafts contain which, and what a bricked item sells for.

use std::collections::BTreeSet;

use indexmap::IndexMap;
use ponder_core::{self as craft, Action, MAGIC, Problem, RARE, Sale};
use regex::Regex;

use super::cache::round_sig;
use super::types::Estimate;

/// What a bricked item sells for, as a share of the cheapest way to make what it passes as.
pub const SALE_SHARE: f64 = 0.7;
/// Chaos: below this a sale isn't worth listing.
pub const MIN_SALE: f64 = 20.0;
/// Sale actions per craft (the best few).
pub const MAX_SALES: usize = 6;

/// What an item crafted toward `big` needs to pass as `small`'s product (a sale action's `Sale`), or None.
///
/// Each of `small`'s targets must be one of `big`'s at the same tier or better (or fractured on
/// the base). `small`'s filler can be `big`'s filler slots or `big`'s other targets that are
/// `small`'s filler (strength on a ring that only asks for "any attribute or res"), side by side.
pub fn covers(big: &Problem, small: &Problem) -> Option<Sale> {
    let fixed: BTreeSet<&String> = big.fixed.iter().map(|m| &m.id).collect();
    let mut need: BTreeSet<usize> = BTreeSet::new();
    for t2 in &small.targets {
        if t2.good.iter().any(|g| fixed.contains(g)) {
            continue;
        }
        let i = big.targets.iter().enumerate().position(|(i, t)| t.good.is_subset(&t2.good) && !need.contains(&i))?;
        need.insert(i);
    }
    if small.targets.is_empty() {
        return None;
    }
    let spare: BTreeSet<usize> = big.targets.iter().enumerate().filter(|(i, t)| !need.contains(i) && t.good.is_subset(&small.filler)).map(|(i, _)| i).collect();
    let (p, q) = small.filler_sides.unwrap_or((0, 0));
    let big_sides = big.filler_sides.unwrap_or((0, 0));
    let room = |side: craft::Side, k: usize| spare.iter().filter(|&&i| big.targets[i].side == side).count() + [big_sides.0, big_sides.1][k];
    let (prefix_room, suffix_room) = (room(craft::Side::Prefix, 0), room(craft::Side::Suffix, 1));
    let extra = if big.filler_sides.is_some() { 0 } else { big.filler_need };
    if prefix_room < p || suffix_room < q || prefix_room + suffix_room + extra < small.filler_need {
        return None;
    }
    Some(Sale { need, prefix: p, suffix: q, any: small.filler_need, spare })
}

/// The product tree: `smaller` and `bigger` between products of one base group (or cluster kind).
///
/// Product A is under B when every mod A wants is one B makes at the same tier or better, A has
/// fewer wanted mods, and B has room for A's filler (`covers`). The links are indices into `estimates`.
pub fn link_products(estimates: &mut [Estimate]) {
    let mut by_title: IndexMap<&str, Vec<usize>> = IndexMap::new();
    for (i, e) in estimates.iter().enumerate() {
        if e.problem.is_some() && e.error.is_none() {
            by_title.entry(e.title.as_str()).or_default().push(i);
        }
    }
    let mut links: Vec<(usize, usize)> = Vec::new(); // (big, small)
    for group in by_title.values() {
        for &big in group {
            for &small in group {
                let (b, s) = (estimates[big].problem.as_ref().expect("grouped"), estimates[small].problem.as_ref().expect("grouped"));
                if s.targets.len() < b.targets.len() && covers(b, s).is_some() {
                    links.push((big, small));
                }
            }
        }
    }
    for (big, small) in links {
        estimates[big].smaller.push(small);
        estimates[small].bigger.push(big);
    }
}

/// Sell actions for `estimates[i]` and its fractures: a bricked item that passes as a smaller,
/// already solved product sells for `SALE_SHARE` of that product's cheapest cost, then you start
/// over (buying the base again: a cluster base, or the fractured one).
pub fn add_sales(estimates: &mut [Estimate], i: usize) {
    let est = &estimates[i];
    // (None: the estimate itself, Some(j): its j-th fracture) with what starting over costs
    let mut targets: Vec<(Option<usize>, Option<f64>)> = vec![(None, Some(est.base_price.map_or(0.0, |b| b.0)))];
    targets.extend(est.fractures.iter().enumerate().map(|(j, f)| (Some(j), f.made.as_ref().map(|m| m.cost))));
    let mut updates: Vec<(Option<usize>, Vec<Action>)> = Vec::new();
    for (which, restart) in targets {
        let (problem, plan) = match which {
            None => (est.problem.as_ref(), est.plan.as_ref()),
            Some(j) => (est.fractures[j].problem.as_ref(), est.fractures[j].plan.as_ref()),
        };
        let (Some(problem), Some(restart), None) = (problem, restart, plan) else { continue };
        let mut best: Vec<(f64, usize, Sale)> = Vec::new();
        for &s in &est.smaller {
            let small = &estimates[s];
            let (Some(cheapest), Some(small_problem)) = (small.cheapest(), small.problem.as_ref()) else { continue };
            let Some(sale) = covers(problem, small_problem) else { continue };
            let credit = SALE_SHARE * cheapest - restart;
            if credit >= MIN_SALE {
                match best.iter_mut().find(|b| b.2 == sale) {
                    Some(b) if credit > b.0 => *b = (credit, s, sale),
                    Some(_) => {}
                    None => best.push((credit, s, sale)),
                }
            }
        }
        best.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut acts: Vec<Action> = Vec::new();
        for (credit, s, sale) in best.into_iter().take(MAX_SALES) {
            let mut name = format!("sell as {}", estimates[s].targets.iter().map(|t| short_name(t, 26)).collect::<Vec<_>>().join(" + "));
            while acts.iter().any(|a| a.name == name) {
                name.push('\'');
            }
            // 3 figures: the plan cache key holds it
            acts.push(Action::new(&name, "sell", &[], &[MAGIC, RARE]).with_sale(round_sig(credit, 3), sale));
        }
        updates.push((which, acts));
    }
    for (which, acts) in updates {
        let problem = match which {
            None => estimates[i].problem.as_mut(),
            Some(j) => estimates[i].fractures[j].problem.as_mut(),
        };
        if let Some(p) = problem {
            p.actions.retain(|a| a.kind != "sell");
            p.actions.extend(acts);
        }
    }
}

/// A target's name without its numbers: "+(100-114) to maximum Life (T1)" -> "Life T1".
pub fn short_name(name: &str, width: usize) -> String {
    use std::sync::LazyLock;
    static TIER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\((T\d+\+?)\)\s*$").unwrap());
    static TIER_TAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\(T\d+\+?\)\s*$").unwrap());
    static LEAD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[+-]?\(?[\d.]+(?:-[\d.]+)?\)?%?\s+(?:to\s+|increased\s+|additional\s+)?(?:maximum\s+)?").unwrap());
    static RANGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\([\d.]+-[\d.]+\)").unwrap());
    let tier = TIER.captures(name).map(|c| c[1].to_string());
    let text = TIER_TAIL.replace(name, "");
    let text = LEAD.replace(&text, "");
    let text = RANGE.replace_all(&text, "#");
    let text = if text.chars().count() <= width { text.to_string() } else { text.chars().take(width - 1).collect::<String>() + "\u{2026}" };
    match tier {
        Some(t) => format!("{text} {t}"),
        None => text,
    }
}
