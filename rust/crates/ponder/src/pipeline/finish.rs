//! Finishing real items: what an item you have (dropped, bought or half-crafted) costs to finish.

use std::borrow::Cow;
use std::collections::BTreeSet;

use anyhow::Result;
use ponder_core::{self as craft, GOOD, MAGIC, NORMAL, Plan, Prices, Problem, RARE, Side, State};
use ponder_game::data;
use ponder_game::item::{Item, ModLine};
use ponder_game::mods::{ItemMatch, ModDB};
use ponder_net::trade::{self, ItemQuery, ListingItem, Stat};
use serde_json::{Value, json};

use super::context::Context;
use super::solving::solve_one;
use super::types::Estimate;
use crate::gear;

/// Finishing one real item (dropped, bought or half-crafted) into an Estimate's product.
///
/// The solver doesn't care where an item comes from: the item's mods are a state, and the plan
/// already knows the cost to finish from every state it reaches (`Plan::values`). An item the plan
/// never meets is solved from its own state.
#[derive(Clone, Debug, Default)]
pub struct Finish {
    /// the product's title, targets, cheapest cost and market price when this was made
    pub product: String,
    pub targets: Vec<String>,
    pub cheapest: Option<f64>,
    pub market: Option<f64>,
    /// its targets already on, at a good tier
    pub have: Vec<String>,
    /// the estimate's problem (or a fracture's), starting at the item
    pub problem: Option<Problem>,
    pub state: Option<State>,
    pub plan: Option<Plan>,
    /// expected chaos to finish, bench crafts included
    pub craft_cost: Option<f64>,
    /// what the item costs, when bought
    pub price: Option<f64>,
    /// the trade listing, when bought
    pub listing: Option<Value>,
    pub error: Option<String>,
}

impl Finish {
    fn of(est: &Estimate) -> Finish {
        Finish {
            product: est.title.clone(),
            targets: est.targets.clone(),
            cheapest: est.cheapest(),
            market: est.market.map(|m| m.0),
            ..Finish::default()
        }
    }

    pub fn total(&self) -> Option<f64> {
        Some(self.craft_cost? + self.price.unwrap_or(0.0))
    }

    /// What the finished item is worth: its market price, else the cheapest way to make it.
    pub fn worth(&self) -> Option<f64> {
        self.market.or(self.cheapest)
    }

    /// Chaos saved against making the product the cheapest other way.
    pub fn saves(&self) -> Option<f64> {
        Some(self.cheapest? - self.total()?)
    }

    pub fn steps(&self) -> Vec<craft::Step> {
        match (&self.problem, &self.plan) {
            (Some(p), Some(plan)) => craft::steps(p, plan, 0.02),
            _ => Vec::new(),
        }
    }

    pub fn to_dict(&self) -> Value {
        let lst = self.listing.as_ref().and_then(|l| l.get("listing")).filter(|l| !l.is_null()).cloned().unwrap_or(json!({}));
        json!({
            "product": self.product,
            "targets": self.targets,
            "have": self.have,
            "state": match (&self.problem, &self.state) { (Some(p), Some(s)) => Some(craft::describe(p, s)), _ => None },
            "craft_cost": self.craft_cost,
            "price": self.price,
            "total": self.total(),
            "saves": self.saves(),
            "seller": lst.get("account").and_then(|a| a.get("name")),
            "whisper": lst.get("whisper"),
            "steps": self.steps(),
            "error": self.error,
        })
    }
}

impl serde::Serialize for Finish {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_dict().serialize(s)
    }
}

/// `gear._mod(mod_id, mod, 0)`: a mod of the item as a solver mod (weight 0: it is already on).
fn solver_mod(mod_id: &str, m: &data::Mod) -> Option<craft::Mod> {
    let side = match m.generation_type.as_str() {
        "prefix" => Side::Prefix,
        "suffix" => Side::Suffix,
        _ => return None,
    };
    let text = m.text_or_empty();
    Some(craft::Mod {
        id: mod_id.to_string(),
        side,
        weight: 0.0,
        groups: m.groups.clone(),
        notable: false,
        label: if text.is_empty() { mod_id.to_string() } else { text.lines().next().unwrap_or("").to_string() },
        tags: m.implicit_tags.clone(),
    })
}

/// The cheapest way to finish `m` (a real item) into `est`'s product.
///
/// Its crafted mods are left out (the bench removes them), fractured ones fix the problem (a
/// fracture of `est` when one matches). With `solve`, an item the plan never meets is solved from
/// its own state, and so is every item whose recipe will be shown (`plan` is then the plan from
/// the item).
pub fn finish(cx: &Context, db: &ModDB, est: &Estimate, m: &ItemMatch, table: &Prices, solve: bool) -> Finish {
    let item = &m.item;
    let mut f = Finish::of(est);
    let (Some(est_problem), true) = (&est.problem, est.plan.is_some()) else {
        f.error = Some(est.error.clone().unwrap_or_else(|| "not solved".into()));
        return f;
    };
    if ["corrupted", "mirrored", "split"].iter().any(|k| item.flags.contains(*k)) {
        f.error = Some("corrupted or mirrored: can't be crafted".into());
        return f;
    }
    if m.base_id.is_none() || gear::group(db, m).label != est.title {
        f.error = Some(format!("{} isn't a {}", item.base, est.title));
        return f;
    }
    if let Some(line) = m.unmatched.iter().find(|l| !l.crafted()) {
        f.error = Some(format!("can't read {}: the item's state would be wrong", py_repr(&line.text)));
        return f;
    }
    let mods_db = data::mods().unwrap_or_else(|e| panic!("{e}"));
    let mut on: Vec<craft::Mod> = Vec::new();
    let mut fractured: BTreeSet<String> = BTreeSet::new();
    for x in &m.mods {
        if x.source == "crafted" || x.source == "legacy-crafted" {
            continue;
        }
        let Some(data_mod) = mods_db.get(&x.mod_id) else { continue };
        let known = est_problem.mods().get(&x.mod_id).cloned().or_else(|| solver_mod(&x.mod_id, data_mod));
        let Some(known) = known else { continue };
        on.push(known);
        if x.fractured() {
            fractured.insert(x.mod_id.clone());
        }
    }
    let ilvl = item.item_level.filter(|l| *l != 0).unwrap_or(est.item_level);
    for t in &est_problem.targets {
        let can_roll = t.good.iter().any(|g| mods_db.get(g).is_some_and(|d| d.required_level <= ilvl));
        if !can_roll && !on.iter().any(|x| t.good.contains(&x.id)) {
            f.error = Some(format!("item level {ilvl} can't roll {}", t.name));
            return f;
        }
    }
    let mut fixed = fractured.clone();
    fixed.extend(est.fixed_ids.iter().cloned());
    let Some((problem, plan)) = fixed_problem(est, &fixed, &on) else {
        f.error = Some("its fractured mods aren't ones this product can keep".into());
        return f;
    };
    let mut rarity = match item.rarity.as_str() {
        "NORMAL" => NORMAL,
        "MAGIC" => MAGIC,
        _ => RARE,
    };
    if rarity == RARE && on.is_empty() {
        rarity = NORMAL; // every mod would be scoured anyway
    }
    let state = problem.state_of(rarity, &on);
    f.have = problem.targets.iter().zip(&state.status).filter(|(_, v)| **v == GOOD).map(|(t, _)| t.name.clone()).collect();
    f.have.extend(problem.fixed.iter().filter(|m| fractured.contains(&m.id)).map(|m| m.label.clone()));
    let mut cost = plan.and_then(|p| p.to_finish(&problem, &state));
    f.state = Some(state);
    if solve || cost.is_none() {
        let mut begun = problem.clone().into_owned();
        begun.begin = Some(state);
        if plan.is_none() || fractured.iter().any(|x| !est.fixed_ids.contains(x)) {
            begun.actions.retain(|a| a.kind != "sell"); // sold, it can't be bought again
        }
        let mut holder = Finish { problem: Some(begun), ..Finish::default() };
        solve_one(cx, &mut holder, table);
        let Some(solved) = holder.plan.take() else {
            f.problem = Some(problem.into_owned());
            f.error = Some(holder.error.unwrap_or_else(|| "can't finish it".into()));
            return f;
        };
        cost = Some(solved.cost);
        f.problem = holder.problem.take();
        f.plan = Some(solved);
    } else {
        f.problem = Some(problem.into_owned());
    }
    f.craft_cost = cost.map(|c| c + est.bench_cost());
    f
}

/// Python's `repr` of a string, close enough for an error message.
fn py_repr(s: &str) -> String {
    if s.contains('\'') && !s.contains('"') { format!("\"{s}\"") } else { format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")) }
}

/// `est`'s problem with these mods fractured (and its plan, when `est` or a fracture of it has one).
fn fixed_problem<'a>(est: &'a Estimate, fixed: &BTreeSet<String>, on: &[craft::Mod]) -> Option<(Cow<'a, Problem>, Option<&'a Plan>)> {
    let same = |p: &Problem| p.fixed.iter().map(|m| &m.id).collect::<BTreeSet<_>>() == fixed.iter().collect();
    if let Some(p) = est.problem.as_ref().filter(|p| same(p)) {
        return Some((Cow::Borrowed(p), est.plan.as_ref()));
    }
    for f in &est.fractures {
        if let Some(p) = f.problem.as_ref().filter(|p| same(p)) {
            return Some((Cow::Borrowed(p), f.plan.as_ref()));
        }
    }
    let p = est.problem.as_ref()?;
    let already: BTreeSet<&String> = p.fixed.iter().map(|m| &m.id).collect();
    let extra: Vec<craft::Mod> = on.iter().filter(|m| fixed.contains(&m.id) && !already.contains(&m.id)).map(|m| craft::Mod { weight: 0.0, ..m.clone() }).collect();
    let targets: Vec<craft::Target> = p.targets.iter().filter(|t| !extra.iter().any(|m| t.good.contains(&m.id))).cloned().collect();
    let blocks = extra.iter().any(|m| targets.iter().any(|t| m.groups.iter().any(|g| t.groups.contains(g)) && !t.good.contains(&m.id)));
    if blocks {
        return None; // a fractured mod blocks a target for good
    }
    let mut q = p.clone();
    q.need = p.need.saturating_sub(p.targets.len() - targets.len());
    q.targets = targets;
    q.fixed.extend(extra);
    Some((Cow::Owned(q), None))
}

/// What `m` can become: every product of its base group it can be finished into, the most chaos
/// saved (against making that product another way) first; the `top` few get a plan.
pub fn finishes(cx: &Context, db: &ModDB, m: &ItemMatch, estimates: &[Estimate], table: &Prices, top: usize) -> Vec<Finish> {
    let mut found: Vec<(&Estimate, Finish)> = estimates
        .iter()
        .filter(|e| e.kind == "gear")
        .map(|e| (e, finish(cx, db, e, m, table, false)))
        .filter(|(_, f)| f.craft_cost.is_some())
        .collect();
    // most saved first; items with no known saving by their (negated) total
    let key = |f: &Finish| -f.saves().unwrap_or_else(|| -f.total().unwrap_or(0.0));
    found.sort_by(|a, b| key(&a.1).partial_cmp(&key(&b.1)).unwrap_or(std::cmp::Ordering::Equal));
    found.into_iter().take(top).map(|(e, _)| finish(cx, db, e, m, table, true)).collect()
}

/// A trade listing's item as an `Item`, its mod lines marked fractured or crafted, for `ModDB::match_item`.
pub fn listing_to_item(l: &ListingItem) -> Item {
    let mut item = Item::new(l.rarity.as_str(), l.name.as_str(), l.base.as_str());
    item.item_level = l.ilvl;
    item.influences = l.influences.clone();
    item.flags = l.flags.iter().cloned().collect();
    item.implicits = l.implicits.iter().map(ModLine::new).collect();
    item.explicits = l
        .explicits
        .iter()
        .map(|e| {
            if e.fractured {
                ModLine::with_marker(&e.text, "fractured")
            } else if e.crafted {
                ModLine::with_marker(&e.text, "crafted")
            } else {
                ModLine::new(&e.text)
            }
        })
        .collect();
    item
}

/// Half-done items for sale that save grinding: rares of `est`'s bases with any of its targets.
///
/// One search (plus a fetch per 10 listings); each listing is finished from its own mods, and the
/// cheapest totals (price + finishing) are kept, best first, in `est.starts`.
pub fn find_starts(cx: &Context, db: &ModDB, est: &mut Estimate, table: &Prices, league: &str, listings: usize, keep: usize) -> Result<Vec<Finish>> {
    let (Some(problem), true) = (&est.problem, est.kind == "gear" && est.plan.is_some()) else { return Ok(Vec::new()) };
    let mut stats: Vec<Stat> = Vec::new();
    for t in &problem.targets {
        if let Some(sid) = t.good.iter().next().and_then(|g| cx.stat_id(g, "explicit")) {
            let ids: Vec<&str> = t.good.iter().map(String::as_str).collect();
            stats.push((sid, cx.stat_min(&ids)));
        }
    }
    if stats.is_empty() {
        return Ok(Vec::new());
    }
    let fixed: Vec<Stat> = est.fixed_ids.iter().filter_map(|m| Some((cx.stat_id(m, "fractured")?, cx.stat_min(&[m.as_str()])))).collect();
    let mut q: ItemQuery = est.query_with(fixed);
    q.count = Some((1, stats));
    q.item_level = None;
    q.rarity = "rare".into();
    q.corrupted = Some(false);
    let query = trade::item_query(&q);
    est.starts_url = Some(trade::search_url(league, &query));
    let mut out: Vec<Finish> = Vec::new();
    for listing in cx.trade.listings(league, &query, listings, None)? {
        let Some(price) = trade::chaos_price(&listing, table) else { continue };
        let item = listing_to_item(&trade::listing_item(&listing));
        let mut f = finish(cx, db, est, &db.match_item(&item), table, false);
        if f.craft_cost.is_none() {
            continue;
        }
        f.price = Some(price);
        f.listing = Some(listing);
        out.push(f);
    }
    out.sort_by(|a, b| a.total().partial_cmp(&b.total()).unwrap_or(std::cmp::Ordering::Equal));
    let mut best: Vec<Finish> = Vec::new();
    for f in out.into_iter().take(keep) {
        let item = listing_to_item(&trade::listing_item(f.listing.as_ref().expect("listed")));
        let mut g = finish(cx, db, est, &db.match_item(&item), table, true);
        g.price = f.price;
        g.listing = f.listing.clone();
        best.push(if g.craft_cost.is_some() { g } else { f });
    }
    best.sort_by(|a, b| a.total().partial_cmp(&b.total()).unwrap_or(std::cmp::Ordering::Equal));
    est.starts = best.clone();
    Ok(best)
}
