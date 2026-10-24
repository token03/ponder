//! Pivots: while crafting one product, a click that lands a good start on another.
//!
//! Each product's plan is played out on real mods (`craft::replay`). After every click the item
//! is read as each other product of the same base group would read it; when finishing that one
//! from here costs well under its cheapest way from scratch, the click was a pivot. They are
//! counted at the recipe step the click was made from. Products wanting the same mods (twins
//! that differ only in filler) aren't pivots of each other.

use std::collections::{BTreeSet, HashMap};

use ponder_core::{self as craft, GOOD, MISSING, Mod, Problem, State};
use rustc_hash::FxHashMap;
use rayon::prelude::*;
use regex::Regex;
use serde_json::{Value, json};

use super::types::{Estimate, ProductRef};

/// Clicks replayed per product (split over as many runs as that takes).
pub const PIVOT_CLICKS: f64 = 50_000.0;
/// A start must save this share of the product's fresh cost, and at least `PIVOT_MIN_CHAOS`.
pub const PIVOT_SHARE: f64 = 0.3;
pub const PIVOT_MIN_CHAOS: f64 = 20.0;
/// Pivots kept per recipe step, and the least often one may come up per finished item.
pub const PIVOTS_PER_STEP: usize = 3;
pub const PIVOT_MIN_PER_CRAFT: f64 = 0.02;

/// At recipe step `step`, a click sometimes lands a start on another product.
#[derive(Clone, Debug, PartialEq)]
pub struct Pivot {
    /// the recipe step (`Estimate::steps`) the click is made from
    pub step: usize,
    /// index of the other product in the list it was found in
    pub to: usize,
    pub product: ProductRef,
    /// chance one click at that step lands such a start
    pub per_click: f64,
    /// expected such starts per finished item of this product
    pub per_craft: f64,
    /// average chaos to finish the other product from those starts, its bench crafts included
    pub to_finish: f64,
    /// the other product's cheapest way from scratch (`Estimate::cheapest`), less the plain base you already hold
    pub fresh: f64,
    /// the commonest such start, in the other product's terms
    pub have: String,
}

impl Pivot {
    pub fn to_dict(&self) -> Value {
        json!({
            "step": self.step, "product": self.product.title, "targets": self.product.targets,
            "per_click": self.per_click, "per_craft": self.per_craft, "to_finish": self.to_finish,
            "fresh": self.fresh, "have": self.have,
        })
    }
}

/// Fill in every estimate's `pivots` from the other products of its base group.
pub fn add_pivots(estimates: &mut [Estimate]) {
    let found: Vec<Vec<Pivot>> = (0..estimates.len()).into_par_iter().map(|i| pivots_of(estimates, i)).collect();
    for (e, p) in estimates.iter_mut().zip(found) {
        e.pivots = p;
    }
}

/// What making `e` costs someone already holding a plain base of it.
fn fresh(e: &Estimate) -> Option<f64> {
    let plain = if e.fixed.is_empty() { e.base_price.map_or(0.0, |b| b.0) } else { 0.0 };
    Some(e.cheapest()? - plain)
}

/// (rarity, which targets are on) as `craft::steps` groups states.
fn step_key(problem: &Problem, s: &State) -> (u8, u8) {
    (s.rarity, (0..problem.targets.len()).filter(|&i| s.status[i] == GOOD).fold(0u8, |m, i| m | 1 << i))
}

/// A rolled item read as another product's start: (which of `others`, chaos left, its state there).
type Start = (usize, f64, State);

struct Seen {
    starts: usize,
    to_finish: f64,
    have: HashMap<State, usize>,
}

/// The pivots crafting `estimates[i]` (along its cheapest route) runs into.
pub fn pivots_of(estimates: &[Estimate], i: usize) -> Vec<Pivot> {
    let e = &estimates[i];
    let Some((problem, plan)) = e.cheapest_plan() else { return Vec::new() };
    if problem.begin.is_some() {
        return Vec::new(); // the replay starts from a fresh base
    }
    let wants: BTreeSet<&String> = e.targets.iter().collect();
    // (index, problem, plan, bench crafts, cheapest from scratch)
    let others: Vec<(usize, &Problem, &craft::Plan, f64, f64)> = estimates
        .iter()
        .enumerate()
        .filter(|&(j, o)| j != i && o.title == e.title && o.error.is_none() && o.targets.iter().collect::<BTreeSet<_>>() != wants)
        .filter_map(|(j, o)| Some((j, o.problem.as_ref()?, o.plan.as_ref()?, o.bench_cost(), fresh(o)?)))
        .filter(|(_, p, _, _, fresh)| p.begin.is_none() && *fresh > 0.0)
        .collect();
    if others.is_empty() {
        return Vec::new();
    }
    let steps = craft::steps(problem, plan, 0.02);
    let numbers: HashMap<(u8, u8), usize> = steps
        .iter()
        .map(|st| {
            let mask = problem.targets.iter().enumerate().filter(|(_, t)| st.have.contains(&t.name)).fold(0u8, |m, (k, _)| m | 1 << k);
            ((st.rarity, mask), st.number)
        })
        .collect();
    // A long spam gets fewer runs: the clicks, not the runs, are what's budgeted.
    let clicks_per_item: f64 = plan.usage.values().sum::<f64>().max(1.0);
    let runs = (PIVOT_CLICKS / clicks_per_item).clamp(1.0, 400.0) as usize;
    let max_steps = (PIVOT_CLICKS as usize / runs).max(1);
    // Every mod the replay can put on the item, numbered; how each product reads each of them.
    let by_index: Vec<&Mod> = problem.pool.iter().chain(&problem.fixed).chain(problem.extra_mods.values()).collect();
    let index: FxHashMap<*const Mod, u16> = by_index.iter().enumerate().map(|(i, m)| (*m as *const Mod, i as u16)).collect();
    let reads: Vec<Vec<(i32, u8)>> = others.iter().map(|(_, p, ..)| by_index.iter().map(|m| p.classify_mod(m)).collect()).collect();
    // A spam rolls the same few thousand items again and again: what they start is worked out once.
    let mut memo: FxHashMap<(u8, u8, [u16; 8]), Vec<Start>> = FxHashMap::default();
    let mut clicks: HashMap<usize, usize> = HashMap::new();
    let mut seen: HashMap<(usize, usize), Seen> = HashMap::new();
    let _ = craft::replay_with_meta(problem, plan, runs, 7, max_steps, |before, _, rarity, mods, meta| {
        let Some(&step) = numbers.get(&step_key(problem, before)) else { return };
        *clicks.entry(step).or_default() += 1;
        let mut key = [u16::MAX; 8];
        for (k, m) in key.iter_mut().zip(mods) {
            *k = index[&(*m as *const Mod)];
        }
        key.sort_unstable();
        let starts = memo.entry((rarity, meta, key)).or_insert_with(|| {
            let mut out = Vec::new();
            for (o, &(_, p, other, bench, fresh)) in others.iter().enumerate() {
                let mut s = State::new(rarity, &[], &[]).with_meta(meta);
                for &k in key.iter().filter(|&&k| k != u16::MAX) {
                    match reads[o][k as usize] {
                        (i, v) if i >= 0 => {
                            let st = &mut s.status[i as usize];
                            if *st == MISSING {
                                *st = v;
                            }
                        }
                        (_, slot) => s.junk[slot as usize] += 1,
                    }
                }
                let left = bench + if p.done(&s) { 0.0 } else if let Some(&v) = other.values.get(&s) { v } else { continue };
                if fresh - left >= PIVOT_MIN_CHAOS.max(PIVOT_SHARE * fresh) {
                    out.push((o, left, s));
                }
            }
            out
        });
        for &(o, left, s) in starts.iter() {
            let x = seen.entry((step, others[o].0)).or_insert_with(|| Seen { starts: 0, to_finish: 0.0, have: HashMap::new() });
            x.starts += 1;
            x.to_finish += left;
            *x.have.entry(s).or_default() += 1;
        }
    });
    let mut out: Vec<Pivot> = seen
        .into_iter()
        .map(|((step, j), x)| {
            let p = estimates[j].problem.as_ref().expect("pivots go to solved products");
            let have = x.have.iter().max_by_key(|(_, n)| **n).map(|(s, _)| craft::describe(p, s)).unwrap_or_default();
            Pivot {
                step,
                to: j,
                product: estimates[j].product(),
                per_click: x.starts as f64 / clicks[&step] as f64,
                per_craft: x.starts as f64 / runs as f64,
                to_finish: x.to_finish / x.starts as f64,
                fresh: fresh(&estimates[j]).unwrap_or(0.0),
                have,
            }
        })
        .filter(|p| p.per_craft >= PIVOT_MIN_PER_CRAFT)
        .collect();
    // Per step, the ones worth most per item made: how often, times what they save.
    let worth = |p: &Pivot| p.per_craft * (p.fresh - p.to_finish);
    out.sort_by(|a, b| a.step.cmp(&b.step).then(worth(b).total_cmp(&worth(a))).then(a.to.cmp(&b.to)));
    let mut kept: Vec<Pivot> = Vec::new();
    for p in out {
        if kept.iter().filter(|k| k.step == p.step).count() < PIVOTS_PER_STEP {
            kept.push(p);
        }
    }
    kept
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
