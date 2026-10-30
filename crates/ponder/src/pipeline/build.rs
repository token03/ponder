use anyhow::Result;
use ponder_game::mods::{ItemMatch, ModDB};
use ponder_net::trade::{self, Stat};
use serde_json::{Value, json};

use super::{
    context::Context,
    solving::solve_one,
    types::{Estimate, JEWELLERY, Key, Search},
};
use crate::{
    candidate, cluster,
    gear::{self, BuildOpts, Prices, Row, Side},
    pattern::target_key,
};

#[derive(Clone, Debug)]
pub struct GearOpts {
    pub settle: i64,
    pub item_level: Option<i64>,
    pub max_targets: usize,
    pub exact: bool,
    pub solve: bool,
}

impl Default for GearOpts {
    fn default() -> Self {
        Self { settle: 0, item_level: None, max_targets: 6, exact: false, solve: true }
    }
}

#[derive(Clone, Debug)]
pub struct ClusterOpts {
    pub passives: Option<i64>,
    pub item_level: i64,
    pub fractured: Vec<String>,
    pub filler: Vec<String>,
    pub solve: bool,
}

impl Default for ClusterOpts {
    fn default() -> Self {
        Self { passives: None, item_level: 84, fractured: Vec::new(), filler: Vec::new(), solve: true }
    }
}

pub fn estimate_item(
    cx: &Context,
    db: &ModDB,
    matched: &ItemMatch,
    table: &Prices,
    league: &str,
    opts: &GearOpts,
) -> Result<Option<Estimate>> {
    if matched.item.base.ends_with("Cluster Jewel") {
        let (class, notables) = target_key(matched);
        let Some((size, tag)) = class.split_once(" cluster: ") else {
            return Ok(None);
        };
        if notables.is_empty() {
            return Ok(None);
        }
        let passives = matched.item.header.get("Cluster Jewel Node Count").and_then(|count| count.parse().ok());
        return Ok(Some(estimate_cluster(
            cx,
            size,
            tag,
            &notables,
            table,
            league,
            &ClusterOpts { passives, solve: opts.solve, ..Default::default() },
        )));
    }
    if matched.base_id.is_none() {
        return Ok(None);
    }
    estimate_gear(cx, db, matched, table, league, opts).map(Some)
}

pub fn estimate_cluster(
    cx: &Context,
    size: &str,
    enchant: &str,
    notables: &[String],
    table: &Prices,
    league: &str,
    opts: &ClusterOpts,
) -> Estimate {
    let passives = opts.passives.unwrap_or_else(|| cluster::default_passives(size));
    let mut estimate = Estimate::new("cluster", format!("{size} cluster · {enchant} · {passives}p"), notables.to_vec());
    estimate.settings = cx.settings;
    let build = || -> Result<_> {
        let enchant = cluster::find_enchant(size, enchant)?;
        let problem = cluster::problem(size, &enchant.tag, notables, opts.item_level, &opts.fractured, &opts.filler)?;
        Ok((enchant, problem))
    };
    let (enchant, problem) = match build() {
        Ok(value) => value,
        Err(error) => {
            estimate.error = Some(error.to_string());
            return estimate;
        }
    };
    estimate.base = problem.base.split(" [").next().unwrap_or(&problem.base).into();
    estimate.item_level = opts.item_level;
    estimate.key = Key::Cluster { size: size.into(), tag: enchant.tag.clone(), fingerprint: problem.fingerprint().to_string() };
    estimate.fixed = opts.fractured.clone();
    estimate.fixed_ids = problem.fixed.clone();
    estimate.rows = problem
        .needs
        .iter()
        .map(|need| Row {
            role: if need.fractured { "fractured" } else { "target" },
            side: need
                .mods
                .iter()
                .next()
                .and_then(|id| cx.mods().get(id))
                .map_or(Side::Prefix, |m| if m.generation_type == "suffix" { Side::Suffix } else { Side::Prefix }),
            text: need.name.clone(),
            tier: String::new(),
        })
        .collect();
    let option = cx.enchant_option(&enchant.short);
    estimate.search = Search::Cluster { size: size.into(), enchant: option.clone(), passives };
    if let Some(stats) = goal_stats(cx, &problem) {
        let mut query = trade::cluster_query(cx, size, option.as_deref(), Some(passives), &[], Some(opts.item_level), &estimate.fixed_ids);
        query["query"]["stats"].as_array_mut().expect("trade stat groups").extend(stats);
        estimate.trade_url = Some(trade::search_url(league, &query));
        estimate.query = Some(query);
    } else {
        estimate.market_error = Some("Goal cannot be expressed with the available trade stats".into());
    }
    let query = trade::cluster_query(cx, size, option.as_deref(), Some(passives), &[], Some(opts.item_level), &estimate.fixed_ids);
    estimate.base_url = Some(trade::search_url(league, &query));
    estimate.base_query = Some(query);
    if estimate.fixed_ids.is_empty() {
        estimate.base_price = cx.cluster_base(size, &enchant.short, passives, opts.item_level);
        if estimate.base_price.is_some() {
            estimate.base_source = "poe.ninja".into();
        }
    }
    estimate.candidate = Some(problem);
    if opts.solve {
        solve_one(cx, &mut estimate, table);
    }
    estimate
}

pub fn estimate_gear(cx: &Context, db: &ModDB, matched: &ItemMatch, table: &Prices, league: &str, opts: &GearOpts) -> Result<Estimate> {
    let built = gear::build(cx.rules()?.0, db, matched, BuildOpts { settle: opts.settle, item_level: opts.item_level, exact: opts.exact })?;
    let group = gear::group(db, matched);
    let mut estimate = Estimate::new("gear", &group.label, built.candidate.needs.iter().map(|need| need.name.clone()).collect());
    estimate.settings = cx.settings;
    estimate.base = group.best().into();
    estimate.used_base = if matched.item.base != estimate.base { matched.item.base.clone() } else { String::new() };
    estimate.bases = group.bases.len();
    estimate.item_level = built.candidate.item_level as i64;
    estimate.key = Key::Gear(built.candidate.fingerprint().to_string());
    estimate.rows = built.rows;
    estimate.fixed_ids = built.candidate.fixed.clone();
    estimate.fixed = estimate.rows.iter().filter(|row| row.role == "fractured").map(|row| row.text.clone()).collect();
    estimate.search = if JEWELLERY.contains(&group.item_class.as_str()) && group.bases.len() > 1 {
        Search::Category { category: trade::category(&group.item_class).map(str::to_string) }
    } else {
        Search::Base { base: estimate.base.clone() }
    };
    if let Some(stats) = goal_stats(cx, &built.candidate) {
        let mut query = trade::item_query(&estimate.query_with(Vec::new()));
        query["query"]["stats"] = json!(stats);
        estimate.trade_url = Some(trade::search_url(league, &query));
        estimate.query = Some(query);
    } else {
        estimate.market_error = Some("Goal cannot be expressed with the available trade stats".into());
    }
    let fixed: Vec<Stat> = estimate.fixed_ids.iter().filter_map(|id| Some((cx.stat_id(id, "fractured")?, cx.stat_min(&[id])))).collect();
    let mut base_query = estimate.query_with(fixed);
    base_query.item_level = Some(estimate.item_level);
    base_query.fractured_only = !estimate.fixed_ids.is_empty();
    base_query.corrupted = Some(false);
    let query = trade::item_query(&base_query);
    estimate.base_url = Some(trade::search_url(league, &query));
    estimate.base_query = Some(query);
    if built.candidate.needs.len() > opts.max_targets {
        estimate.error = Some(format!("{} target groups exceeds max_targets", built.candidate.needs.len()));
        return Ok(estimate);
    }
    estimate.candidate = Some(built.candidate);
    if opts.solve {
        solve_one(cx, &mut estimate, table);
    }
    Ok(estimate)
}

pub(crate) fn build_gear(cx: &Context, db: &ModDB, archetype: &candidate::Archetype, table: &Prices, league: &str) -> Option<Estimate> {
    let matched = candidate::representative(db, archetype)?;
    Some(estimate_gear(cx, db, &matched, table, league, &GearOpts { solve: false, ..Default::default() }).unwrap_or_else(|error| {
        let mut estimate = Estimate::new("gear", &archetype.group, archetype.families.clone());
        estimate.key = Key::Gear(format!("unsupported/{}/{:?}", archetype.group, archetype.families));
        estimate.base = matched.item.base.clone();
        estimate.settings = cx.settings;
        estimate.error = Some(error.to_string());
        estimate
    }))
}

fn goal_stats(cx: &Context, problem: &candidate::Candidate) -> Option<Vec<Value>> {
    problem
        .needs
        .iter()
        .map(|need| {
            let mut groups = std::collections::BTreeMap::<String, Vec<&str>>::new();
            for id in &need.mods {
                if let Some(stat) = cx.stat_id(id, if need.fractured { "fractured" } else { "explicit" }) {
                    groups.entry(stat).or_default().push(id);
                }
            }
            if groups.len() < need.count as usize {
                return None;
            }
            let filters: Vec<_> = groups
                .into_iter()
                .map(|(id, mods)| {
                    let value = cx.stat_min(&mods).map_or_else(|| json!({}), |min| json!({"min": min}));
                    json!({"id": id, "value": value, "disabled": false})
                })
                .collect();
            Some(json!({"type": "count", "value": {"min": need.count}, "filters": filters}))
        })
        .collect()
}
