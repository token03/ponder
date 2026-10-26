//! Estimates: a craft for a rare, a cluster jewel or a ladder archetype, and its fracture options.

use std::collections::HashMap;

use anyhow::Result;
use ponder_core::{self as craft, Prices, Side};
use ponder_game::mods::{ItemMatch, ModDB};
use ponder_net::trade::{self, ItemQuery, Stat};

use super::context::Context;
use super::solving::solve_one;
use super::types::{Estimate, Fracture, JEWELLERY, Key, Search};
use crate::gear::{self, BuildOpts, Pricing, Row};
use crate::mine::target_key;
use crate::{cluster, demand};

/// Options for [`estimate_item`] and [`estimate_gear`].
#[derive(Clone, Debug)]
pub struct GearOpts {
    /// accept this many tiers worse than the item's own
    pub settle: i64,
    /// craft at this level (default: the item's)
    pub item_level: Option<i64>,
    /// more target mods than this is an error
    pub max_targets: usize,
    /// resistances are targets like any other mod instead of filler
    pub exact: bool,
    /// solve now; false leaves the problem for `solve_all`
    pub solve: bool,
}

impl Default for GearOpts {
    fn default() -> GearOpts {
        GearOpts { settle: 0, item_level: None, max_targets: 6, exact: false, solve: true }
    }
}

/// Options for [`estimate_cluster`].
#[derive(Clone, Debug)]
pub struct ClusterOpts {
    pub passives: Option<i64>,
    pub item_level: i64,
    pub need: Option<usize>,
    pub fractured: Vec<String>,
    /// "Group@tier" small-passive mods that fill the open slots (`cluster::add_filler`)
    pub filler: Vec<String>,
    pub solve: bool,
}

impl Default for ClusterOpts {
    fn default() -> ClusterOpts {
        ClusterOpts { passives: None, item_level: 84, need: None, fractured: Vec::new(), filler: Vec::new(), solve: true }
    }
}

/// The craft for one rare, or None when we can't craft it (unknown base, cluster without notables).
pub fn estimate_item(cx: &Context, db: &ModDB, m: &ItemMatch, table: &Prices, league: &str, opts: &GearOpts) -> Result<Option<Estimate>> {
    let item = &m.item;
    if item.base.ends_with("Cluster Jewel") {
        let (cls, notables) = target_key(m);
        let Some((size, tag)) = cls.split_once(" cluster: ") else {
            return Ok(None);
        };
        if notables.is_empty() {
            return Ok(None);
        }
        let count = item.header.get("Cluster Jewel Node Count").and_then(|c| c.trim().parse::<i64>().ok());
        let copts = ClusterOpts { passives: count, solve: opts.solve, ..ClusterOpts::default() };
        return Ok(Some(estimate_cluster(cx, size, tag, &notables, table, league, &copts)));
    }
    if m.base_id.is_none() {
        return Ok(None);
    }
    estimate_gear(cx, db, m, table, league, opts).map(Some)
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase())
}

/// A cluster jewel craft. With `solve: false` the problem is built but left for `solve_all`.
pub fn estimate_cluster(
    cx: &Context,
    size: &str,
    enchant: &str,
    notables: &[String],
    table: &Prices,
    league: &str,
    opts: &ClusterOpts,
) -> Estimate {
    let passives = opts.passives.filter(|p| *p != 0).unwrap_or_else(|| cluster::default_passives(size));
    let ench = match cluster::find_enchant(size, enchant) {
        Ok(e) => e,
        Err(e) => {
            let mut est = Estimate::new("cluster", format!("{size} cluster, {enchant}"), notables.to_vec());
            est.error = Some(e.to_string());
            est.settings = cx.settings;
            return est;
        }
    };
    let shown = if ench.short.chars().count() <= 48 { &ench.short } else { &ench.name };
    let mut est = Estimate::new("cluster", format!("{} cluster \u{b7} {shown} \u{b7} {passives}p", title_case(size)), notables.to_vec());
    est.settings = cx.settings;
    let mut sorted = notables.to_vec();
    sorted.sort();
    let mut fractured = opts.fractured.clone();
    fractured.sort();
    est.key = Key::Cluster { size: size.into(), tag: ench.tag.clone(), notables: sorted, passives: passives.to_string(), fractured };
    est.base = format!("{} Cluster Jewel", title_case(size));
    est.item_level = opts.item_level;
    est.fixed = opts.fractured.clone();
    let problem = match cluster::problem(size, &ench.name, notables, opts.need, opts.item_level, &opts.fractured) {
        Ok(p) => cluster::add_filler(&p, &opts.filler),
        Err(e) => {
            est.error = Some(e.to_string());
            return est;
        }
    };
    if let Some(need) = opts.need.filter(|n| *n != 0 && *n < notables.len()) {
        est.title += &format!(" ({need} of {})", notables.len());
    }
    est.rows = problem.targets.iter().map(|t| Row { role: "target", side: t.side, text: t.name.clone(), tier: String::new() }).collect();
    est.rows.extend(problem.fixed.iter().map(|m| Row { role: "fractured", side: m.side, text: m.label.clone(), tier: String::new() }));
    if let Some((pre, suf)) = problem.filler_sides {
        let names: Vec<String> = opts.filler.iter().map(|f| cluster::feature_label(f, &problem.pool)).collect();
        let text = format!("any of: {}", names.join(", "));
        for side in std::iter::repeat_n(Side::Prefix, pre).chain(std::iter::repeat_n(Side::Suffix, suf)) {
            est.rows.push(Row { role: "filler", side, text: text.clone(), tier: "filler".into() });
        }
    }
    for side in [Side::Prefix, Side::Suffix] {
        let used = est.rows.iter().filter(|r| r.side == side).count();
        for _ in used..problem.min_mods / 2 {
            est.rows.push(Row { role: "any", side, text: "any mod".into(), tier: "any".into() });
        }
    }
    est.fixed_ids = problem.fixed.iter().map(|m| m.id.clone()).collect();
    let option = cx.enchant_option(&ench.short);
    est.search = Search::Cluster { size: size.into(), enchant: option.clone(), passives };
    let wanted: Vec<(String, Option<serde_json::Number>)> = problem
        .targets
        .iter()
        .map(|t| {
            let good: Vec<&str> = t.good.iter().map(String::as_str).collect();
            (t.good.iter().next().cloned().unwrap_or_default(), if t.notable { None } else { cx.stat_min(&good) })
        })
        .collect();
    let query = trade::cluster_query(cx, size, option.as_deref(), Some(passives), &wanted, Some(opts.item_level), &[]);
    est.trade_url = Some(trade::search_url(league, &query));
    est.query = Some(query);
    if !opts.fractured.is_empty() {
        let mut q = trade::cluster_query(cx, size, option.as_deref(), Some(passives), &[], Some(opts.item_level), &est.fixed_ids);
        q["query"]["filters"]["misc_filters"]["filters"]["corrupted"] = serde_json::json!({"option": "false"});
        est.base_url = Some(trade::search_url(league, &q));
        est.base_query = Some(q);
    } else if let Some(base) = cx.cluster_base(size, &ench.short, passives, opts.item_level) {
        est.base_price = Some(base);
        est.base_source = "poe.ninja".into();
    }
    est.problem = Some(problem);
    if opts.solve {
        solve_one(cx, &mut est, table);
    }
    est
}

/// A gear craft: the item's mods as targets, and the group of bases that roll the same.
/// With `opts.exact`, resistances are targets like any other mod instead of filler.
pub fn estimate_gear(cx: &Context, db: &ModDB, m: &ItemMatch, table: &Prices, league: &str, opts: &GearOpts) -> Result<Estimate> {
    let pricing = Pricing::new(table, &cx.settings);
    let gp = gear::build(db, m, pricing, BuildOpts { settle: opts.settle, item_level: opts.item_level, exact: opts.exact })?;
    let grp = gear::group(db, m);
    let p = &gp.problem;
    let mut targets: Vec<(Side, Vec<String>)> = p.targets.iter().map(|t| (t.side, t.good.iter().cloned().collect())).collect();
    targets.sort();
    let mut fixed: Vec<String> = p.fixed.iter().map(|m| m.id.clone()).collect();
    fixed.sort();
    let mut bench_ids = gp.bench_ids.clone();
    bench_ids.sort();
    let mut est = Estimate::new("gear", grp.label.clone(), p.targets.iter().map(|t| t.name.clone()).collect());
    if p.filler_need > 0 {
        est.targets.push(format!("{} elemental resistance", p.filler_need));
    }
    est.key = Key::Gear {
        item_class: grp.label.clone(),
        signature: grp.signature.clone(),
        targets,
        fixed,
        bench: bench_ids,
        item_level: gp.item_level,
        filler_need: p.filler_need,
        filler: p.filler.clone(),
        filler_sides: p.filler_sides,
        min_mods: p.min_mods,
    };
    est.base = grp.best().to_string();
    est.used_base = if m.item.base != grp.best() { m.item.base.clone() } else { String::new() };
    est.bases = grp.bases.len();
    est.item_level = gp.item_level;
    est.rows = gp.rows.clone();
    est.bench = gp.bench.iter().map(|(label, cost)| (label.clone(), gear::action_cost_of(cost, table))).collect();
    est.skipped = gp.skipped.clone();
    est.fixed = p.fixed.iter().map(|m| m.label.clone()).collect();
    est.fixed_ids = p.fixed.iter().map(|m| m.id.clone()).collect();
    est.filler_families = gp.filler_families.clone();
    est.can_fracture = m.item.influences.is_empty() && !m.item.flags.contains("synthesised");
    est.settings = cx.settings;
    est.search = if JEWELLERY.contains(&grp.item_class.as_str()) && grp.bases.len() > 1 {
        Search::Category(trade::category(&grp.item_class).map(str::to_string))
    } else {
        Search::Base(grp.best().to_string())
    };
    let mut stats: Vec<Stat> = Vec::new();
    for t in &p.targets {
        if let Some(sid) = t.good.iter().next().and_then(|g| cx.stat_id(g, "explicit")) {
            stats.push((sid, stat_min(cx, t.good.iter().map(String::as_str))));
        }
    }
    for m in &est.fixed_ids {
        if let Some(sid) = cx.stat_id(m, "explicit") {
            stats.push((sid, cx.stat_min(&[m])));
        }
    }
    let filler = filler_stats(cx, &p.filler);
    if !stats.is_empty() || !filler.is_empty() {
        let mut q = est.query_with(stats);
        q.count = Some((p.filler_need as i64, filler));
        let query = trade::item_query(&q);
        est.trade_url = Some(trade::search_url(league, &query));
        est.query = Some(query);
    }
    if !est.fixed_ids.is_empty() {
        let frac: Vec<Stat> = est.fixed_ids.iter().filter_map(|m| Some((cx.stat_id(m, "fractured")?, cx.stat_min(&[m])))).collect();
        let mut q = est.query_with(frac);
        q.item_level = Some(gp.item_level);
        q.fractured_only = true;
        q.corrupted = Some(false);
        let query = trade::item_query(&q);
        est.base_url = Some(trade::search_url(league, &query));
        est.base_query = Some(query);
    }
    if p.targets.len() > opts.max_targets {
        est.error = Some(format!("{} target mods (raise max_targets, or settle to loosen)", p.targets.len()));
    }
    est.problem = Some(gp.problem);
    if est.error.is_none() && opts.solve {
        super::solving::price_routes(cx, db, std::slice::from_mut(&mut est), table, league, Some(1), None);
    }
    Ok(est)
}

fn stat_min<'a>(cx: &Context, ids: impl Iterator<Item = &'a str>) -> Option<serde_json::Number> {
    let ids: Vec<&str> = ids.collect();
    cx.stat_min(&ids)
}

/// One (trade stat, minimum) per filler family, e.g. fire resistance at its T3 minimum.
fn filler_stats(cx: &Context, filler: &std::collections::BTreeSet<String>) -> Vec<Stat> {
    let mods = cx.mods();
    let mut by_family: indexmap::IndexMap<&str, Vec<&str>> = indexmap::IndexMap::new();
    for m in filler {
        by_family.entry(mods[m.as_str()].type_.as_str()).or_default().push(m);
    }
    by_family.values().filter_map(|ids| Some((cx.stat_id(ids[0], "explicit")?, cx.stat_min(ids)))).collect()
}

/// The same craft starting from a base with one wanted mod already fractured.
///
/// One option per target, and one per filler family the item uses (e.g. "any fractured fire
/// resistance at T3+"). Bought bases get a trade search; with `table`, gear targets are also
/// costed as self-made (reroll, then a Fracturing Orb: `craft::self_fracture`), which also
/// covers essence-only mods no bought base has. The solver's numbers come from `solve_all`.
pub fn add_fractures(cx: &Context, est: &mut Estimate, league: &str, table: Option<&Prices>, filler: bool) {
    let Some(p) = est.problem.as_ref() else {
        return;
    };
    if est.error.is_some() || !est.can_fracture || !p.fixed.is_empty() || !est.fractures.is_empty() || p.targets.len() + p.filler_need < 2 {
        return;
    }
    let pool: HashMap<&str, &craft::Mod> = p.pool.iter().map(|m| (m.id.as_str(), m)).collect();
    let mods = cx.mods();
    let bench_cost = est.bench_cost();
    let self_made = table.map(|table| cx.plans.self_fractures(p, table, &cx.settings));
    let mut made_fractures: Vec<Fracture> = Vec::new();
    let families: &[String] = if filler { &est.filler_families } else { &[] };
    for family in families {
        let ids: Vec<&str> =
            p.filler.iter().filter(|m| mods[m.as_str()].type_ == *family && pool.contains_key(m.as_str())).map(String::as_str).collect();
        if ids.is_empty() {
            continue;
        }
        let mut mod_ = pool[ids[0]];
        for id in &ids[1..] {
            if pool[id].weight > mod_.weight {
                mod_ = pool[id];
            }
        }
        // A fractured filler mod counts toward filler_need through the base state.
        let mut problem = p.to_spec();
        problem.fixed.push(craft::Mod { weight: 0.0, ..mod_.clone() });
        let mut f = Fracture::new(format!("{} (filler)", mod_.label), mod_.id.clone(), problem.compile());
        f.bench_cost = bench_cost;
        if est.kind == "gear" {
            f.donor_query =
                super::donors::donor_query(cx, &est.base, &[ids.iter().map(|id| (*id).to_string()).collect()], Some(est.item_level), 1, 6);
            f.donor_url = f.donor_query.as_ref().map(|q| trade::search_url(league, q));
        }
        let mut frac: Vec<Stat> = cx.stat_id(&mod_.id, "fractured").map(|sid| (sid, cx.stat_min(&ids))).into_iter().collect();
        frac.extend(est.fixed_ids.iter().filter_map(|m| Some((cx.stat_id(m, "fractured")?, cx.stat_min(&[m])))));
        if !frac.is_empty() {
            let mut q = est.query_with(frac);
            q.item_level = Some(est.item_level);
            q.fractured_only = true;
            q.corrupted = Some(false);
            let query = trade::item_query(&q);
            f.trade_url = Some(trade::search_url(league, &query));
            f.query = Some(query);
        }
        made_fractures.push(f);
    }
    for (i, t) in p.targets.iter().enumerate() {
        let made = self_made.as_ref().and_then(|made| made[i].clone());
        // In pool order, so ties (tiers of equal weight) go to the same mod every run; Python's set order made them random.
        let mut candidates: Vec<&craft::Mod> = p.pool.iter().filter(|m| t.good.contains(&m.id)).collect();
        if candidates.is_empty() && made.is_some() {
            // essence-only: no base drops with it, but you can make one
            let all = p.mods();
            candidates = t.good.iter().filter_map(|m| all.get(m)).collect();
        }
        let Some(&first) = candidates.first() else {
            continue;
        };
        let mod_ = candidates[1..].iter().fold(first, |best, m| if m.weight > best.weight { m } else { best }); // the commonest acceptable tier
        let mut problem = p.to_spec();
        problem.targets.remove(i);
        problem.need = p.need.saturating_sub(1);
        problem.fixed.push(craft::Mod { weight: 0.0, ..mod_.clone() });
        let mut f = Fracture::new(t.name.clone(), mod_.id.clone(), problem.compile());
        f.bench_cost = bench_cost;
        f.made = made;
        f.donor_query = (est.kind == "gear")
            .then(|| super::donors::donor_query(cx, &est.base, std::slice::from_ref(&t.good), Some(est.item_level), 1, 6))
            .flatten();
        f.donor_url = f.donor_query.as_ref().map(|q| trade::search_url(league, q));
        if est.kind == "cluster" {
            if let Search::Cluster { size, enchant, passives } = &est.search {
                let mut fractured = vec![mod_.id.clone()];
                fractured.extend(est.fixed_ids.iter().cloned());
                f.query = Some(trade::cluster_query(cx, size, enchant.as_deref(), Some(*passives), &[], Some(est.item_level), &fractured));
            }
        } else if pool.contains_key(mod_.id.as_str()) {
            let mut frac: Vec<Stat> = Vec::new();
            let mut ids = vec![mod_.id.as_str()];
            ids.extend(est.fixed_ids.iter().map(String::as_str));
            for m in ids {
                if let Some(sid) = cx.stat_id(m, "fractured") {
                    let min =
                        if m == mod_.id { cx.stat_min(&t.good.iter().map(|g| g.as_str()).collect::<Vec<_>>()) } else { cx.stat_min(&[m]) };
                    frac.push((sid, min));
                }
            }
            if !frac.is_empty() {
                let mut q: ItemQuery = est.query_with(frac);
                q.item_level = Some(est.item_level);
                q.fractured_only = true;
                f.query = Some(trade::item_query(&q));
            }
        }
        if let Some(q) = &mut f.query {
            q["query"]["filters"]["misc_filters"]["filters"]["corrupted"] = serde_json::json!({"option": "false"});
            f.trade_url = Some(trade::search_url(league, q));
        }
        made_fractures.push(f);
    }
    est.fractures.extend(made_fractures);
}

/// One ladder archetype as an unsolved Estimate with its fractures (self-made ones costed:
/// essence or reroll, then fracture). `None` when the archetype can't be built.
pub(crate) fn build_gear(cx: &Context, db: &ModDB, a: &demand::Archetype, table: &Prices, league: &str) -> Option<Estimate> {
    let m = demand::archetype_match(db, a)?;
    let opts = GearOpts { solve: false, ..GearOpts::default() };
    let mut est = estimate_gear(cx, db, &m, table, league, &opts).ok()?;
    if est.error.is_none() {
        add_fractures(cx, &mut est, league, Some(table), false);
    }
    Some(est)
}
