//! The runs: PoB builds to a `Report`, ladder archetypes to a `LadderReport`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use ponder_game::item::parse_item;
use ponder_game::mods::{ItemMatch, ModDB};
use ponder_game::pob;
use ponder_net::ladder::Lite;
use rayon::prelude::*;

use super::build::{ClusterOpts, GearOpts, build_gear, estimate_cluster, estimate_item};
use super::context::{Context, Progress};
use super::market::price_on_trade;
use super::solving::{in_pool, solve_estimates};
use super::types::{BuildInfo, Estimate, Key, LadderReport, Report, Shared, Slot, cmp_priority};
use crate::candidate::{self, Archetype};
use crate::pattern::{self, Pattern};

/// What `run` is asked for (Python's keyword arguments).
#[derive(Clone, Debug)]
pub struct RunOpts {
    pub settle: i64,
    pub max_targets: usize,
    pub min_builds: usize,
    pub top: usize,
    pub exact: bool,
    pub trade_lookup: bool,
    /// caps trade searches (cached answers are free)
    pub trade_budget: usize,
    /// threads for solving; `Some(1)` processes products sequentially
    pub workers: Option<usize>,
}

impl Default for RunOpts {
    fn default() -> RunOpts {
        RunOpts { settle: 0, max_targets: 6, min_builds: 2, top: 30, exact: false, trade_lookup: false, trade_budget: 40, workers: None }
    }
}

/// What `ladder` is asked for (Python's keyword arguments).
#[derive(Clone, Debug)]
pub struct LadderOpts {
    pub gear_top: usize,
    pub per_group: usize,
    pub niche: usize,
    pub cluster_top: usize,
    pub min_characters: usize,
    pub group: Option<String>,
    pub trade_lookup: bool,
    pub trade_budget: usize,
    pub workers: Option<usize>,
}

impl Default for LadderOpts {
    fn default() -> LadderOpts {
        LadderOpts {
            gear_top: 80,
            per_group: 3,
            niche: 40,
            cluster_top: 80,
            min_characters: 5,
            group: None,
            trade_lookup: false,
            trade_budget: 40,
            workers: None,
        }
    }
}

fn say(progress: Option<Progress>, msg: &str) {
    if let Some(p) = progress {
        p(msg)
    }
}

/// Load builds, match their rares to mod ids, and turn each rare into a craft: the target mods
/// plus the group of bases that roll exactly the same mods. Crafts that several builds need are
/// merged. Every craft gets a native plan when supported, and optionally trade prices.
pub fn run(cx: &Context, db: &ModDB, sources: &[String], opts: &RunOpts, progress: Option<Progress>) -> Result<Report> {
    let (table, prices_source) = cx.prices.load()?;
    let league = current_league(cx);
    let mut infos: Vec<BuildInfo> = Vec::new();
    let mut matched: Vec<(String, ItemMatch)> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut estimates: Vec<Estimate> = Vec::new();
    let mut by_key: HashMap<Key, usize> = HashMap::new();
    let gear_opts = GearOpts { settle: opts.settle, max_targets: opts.max_targets, exact: opts.exact, solve: false, ..GearOpts::default() };
    for source in expand_sources(sources) {
        say(progress, &format!("loading {}", head(&source, 70)));
        let build = match load_build(cx, &source) {
            Ok(b) => b,
            Err(e) => {
                errors.push(format!("{}: {e}", head(&source, 70))); // a dead link shouldn't sink the rest
                continue;
            }
        };
        let mut rares = 0;
        for pob_item in &build.items {
            let item = match parse_item(&pob_item.text) {
                Ok(i) => i,
                Err(e) => {
                    errors.push(format!("{}: {e}", head(&source, 70)));
                    continue;
                }
            };
            if item.rarity != "RARE" {
                continue;
            }
            rares += 1;
            let m = db.match_item(&item);
            let est = match estimate_item(cx, db, &m, &table, &league, &gear_opts) {
                Ok(est) => est,
                Err(e) => {
                    errors.push(format!("{}: {}: {e}", head(&source, 70), item.base));
                    continue;
                }
            };
            matched.push((source.clone(), m));
            let Some(est) = est.filter(|e| !e.targets.is_empty()) else {
                continue;
            };
            let i = *by_key.entry(est.key.clone()).or_insert_with(|| {
                estimates.push(est);
                estimates.len() - 1
            });
            let e = &mut estimates[i];
            if e.slot.is_none() {
                e.slot = pob_item.slot.clone();
            }
            if !e.builds.contains(&source) {
                e.builds.push(source.clone());
            }
        }
        infos.push(BuildInfo { source: source.clone(), name: build_name(&build), rares });
    }

    let mut shared: Vec<Shared> = Vec::new();
    let mut extra: Vec<Estimate> = Vec::new();
    if infos.len() >= 2 {
        say(progress, "mining shared combinations");
        for c in pattern::mine(&matched, 2, 4, opts.min_builds, true).into_iter().take(opts.top) {
            let slot = shared_estimate(cx, &c, &estimates, &mut extra, &table, &league);
            shared.push(Shared { pattern: c, estimate: slot });
        }
    }

    solve_estimates(cx, &mut estimates, &table, opts.workers, progress);
    solve_estimates(cx, &mut extra, &table, opts.workers, progress);
    if opts.trade_lookup {
        let mut order: Vec<usize> = (0..estimates.len()).collect();
        order.sort_by(|&a, &b| cmp_priority(&estimates[a], &estimates[b]));
        price_on_trade(cx, &mut estimates, &order, &table, &league, opts.trade_budget, progress);
        solve_estimates(cx, &mut estimates, &table, opts.workers, progress);
    }
    Ok(Report {
        builds: infos,
        estimates,
        extra,
        shared,
        prices_source,
        league,
        errors,
        divine: table.get("divine").copied().unwrap_or(0.0),
    })
}

/// A cluster combination's estimate: the build's own when it has one, else a new one (kept in `extra`).
fn shared_estimate(
    cx: &Context,
    c: &Pattern,
    estimates: &[Estimate],
    extra: &mut Vec<Estimate>,
    table: &crate::gear::Prices,
    league: &str,
) -> Option<Slot> {
    let (size, tag) = c.item_class.split_once(" cluster: ")?;
    let wanted: HashSet<&String> = c.families.iter().collect();
    let own = estimates.iter().position(|e| e.key.is_cluster(size, tag) && e.targets.iter().collect::<HashSet<_>>() == wanted);
    if let Some(i) = own {
        return Some(Slot::Main(i));
    }
    let opts = ClusterOpts { solve: false, ..ClusterOpts::default() };
    extra.push(estimate_cluster(cx, size, tag, &c.families, table, league, &opts));
    Some(Slot::Extra(extra.len() - 1))
}

fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Crafting candidates from ladder characters (default: the cached poe.ninja sample).
///
/// Gear archetypes are costed at the tier most of their wearers reach, with their usual number of
/// resistances as filler; clusters by enchant, usual passive count and notables. Trade lookups (in
/// demand order) give the selling price and how many are listed, which `Estimate::hole` turns
/// into a market-hole score.
pub fn ladder(
    cx: &Context,
    db: &ModDB,
    characters: Option<Vec<Lite>>,
    opts: &LadderOpts,
    progress: Option<Progress>,
) -> Result<LadderReport> {
    let (table, prices_source) = cx.prices.load()?;
    let league = current_league(cx);
    let characters = match characters {
        Some(c) => c,
        None => {
            say(progress, "reading cached ladder characters");
            cx.ladder.cached(None)?
        }
    };
    say(progress, "matching ladder items");
    let items = crate::item::normalize(db, &characters);
    let worn = items.gear;
    let clusters = items.clusters;
    let group = opts.group.as_ref().map(|g| g.to_lowercase());
    let found: Vec<Archetype> = candidate::merge(&worn, opts.min_characters, 4, 2, 0.7)
        .into_iter()
        .filter(|a| group.as_ref().is_none_or(|g| a.group.to_lowercase().contains(g.as_str())))
        .collect();
    let found_cl: Vec<candidate::Cluster> = candidate::clusters(&clusters, opts.min_characters, 0.8)
        .into_iter()
        .filter(|a| group.as_ref().is_none_or(|g| a.title().to_lowercase().contains(g.as_str())))
        .collect();
    let picked = candidate::select(&found, opts.gear_top, opts.per_group, opts.niche);
    say(progress, &format!("building {} gear crafts", picked.len()));
    let built: Vec<Option<Estimate>> =
        in_pool(opts.workers, || picked.par_iter().map(|&i| build_gear(cx, db, &found[i], &table, &league)).collect());
    let mut estimates: indexmap::IndexMap<Key, Estimate> = indexmap::IndexMap::new();
    for (&i, est) in picked.iter().zip(built) {
        let a = &found[i];
        let Some(mut est) = est.filter(|e| !estimates.contains_key(&e.key)) else {
            continue;
        };
        est.demand = a.characters;
        est.skills = a.skills.most_common().into_iter().take(3).map(|(s, _)| s.clone()).collect();
        est.used_base = String::new();
        estimates.insert(est.key.clone(), est);
    }
    let n_gear = estimates.len();
    for a in &found_cl {
        if estimates.len() - n_gear >= opts.cluster_top {
            break;
        }
        // two enchants can share a tag
        let copts = ClusterOpts { passives: Some(a.usual_passives()), filler: a.filler.clone(), solve: false, ..ClusterOpts::default() };
        let mut est = estimate_cluster(cx, &a.size, &a.enchant, &a.notables, &table, &league, &copts);
        if est.error.is_some() || estimates.contains_key(&est.key) {
            continue;
        }
        est.demand = a.characters;
        est.skills = a.skills.most_common().into_iter().take(3).map(|(s, _)| s.clone()).collect();
        estimates.insert(est.key.clone(), est);
    }
    let mut out: Vec<Estimate> = estimates.into_values().collect();
    solve_estimates(cx, &mut out, &table, opts.workers, progress);
    if opts.trade_lookup {
        let mut order: Vec<usize> = (0..out.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(out[i].demand));
        price_on_trade(cx, &mut out, &order, &table, &league, opts.trade_budget, progress);
        solve_estimates(cx, &mut out, &table, opts.workers, progress);
    }
    Ok(LadderReport {
        characters: characters.len(),
        items: worn.len(),
        clusters: clusters.len(),
        estimates: out,
        archetypes: found.len(),
        cluster_archetypes: found_cl.len(),
        prices_source,
        league,
        divine: table.get("divine").copied().unwrap_or(0.0),
    })
}

/// A text file listing links (one per line, # comments) stands for those links.
pub fn expand_sources(sources: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for source in sources {
        let path = Path::new(source);
        if source.chars().count() < 1024
            && path.is_file()
            && path.extension().is_some_and(|e| e == "txt")
            && let Ok(text) = std::fs::read_to_string(path)
        {
            let links: Vec<String> =
                text.lines().map(|l| l.split('#').next().unwrap_or("").trim().to_string()).filter(|l| !l.is_empty()).collect();
            if !links.is_empty() && links.iter().all(|l| l.starts_with("http")) {
                out.extend(links);
                continue;
            }
        }
        out.push(source.clone());
    }
    out
}

/// A build from a share link, a file or a PoB code (links are downloaded through `cx.http`).
pub fn load_build(cx: &Context, source: &str) -> Result<pob::Build, pob::PobError> {
    pob::load_with(source, |url| {
        let body = ponder_net::http::get(&*cx.http, url, "ponder", 30).map_err(|e| e.to_string())?;
        String::from_utf8(body).map_err(|e| e.to_string())
    })
}

/// E.g. "Occultist lvl 95: Winter Orb".
pub fn build_name(build: &pob::Build) -> String {
    let who = if build.ascendancy.is_empty() { &build.class_name } else { &build.ascendancy };
    format!("{who} lvl {}: {}", build.level, build.main_skill.as_deref().filter(|s| !s.is_empty()).unwrap_or("?"))
}

/// The league of the saved prices, else "Standard" (offline).
pub fn current_league(cx: &Context) -> String {
    cx.prices.cached_league().unwrap_or_else(|| "Standard".into())
}
