//! The runs: PoB builds to a `Report`, ladder archetypes to a `LadderReport`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use ponder_game::item::parse_item;
use ponder_game::mods::{ItemMatch, ModDB};
use ponder_game::pob;
use ponder_net::ladder::Lite;
use rayon::prelude::*;

use super::build::{GearOpts, build_gear, add_fractures, estimate_cluster, estimate_item, ClusterOpts};
use super::context::{Context, Progress};
use super::market::price_on_trade;
use super::solving::{slots_of, solve_slots};
use super::pivots::add_pivots;
use super::types::{BuildInfo, Estimate, Key, LadderReport, Report, Shared, Slot, cmp_priority};
use crate::demand::{self, Archetype};
use crate::mine::{self, Candidate};

/// What `run` is asked for (Python's keyword arguments).
#[derive(Clone, Debug)]
pub struct RunOpts {
    pub settle: i64,
    pub max_targets: usize,
    pub min_builds: usize,
    pub top: usize,
    pub fractures: bool,
    pub exact: bool,
    pub trade_lookup: bool,
    /// caps trade searches (cached answers are free)
    pub trade_budget: usize,
    pub recombs: bool,
    /// threads for solving; `Some(1)` solves inline
    pub workers: Option<usize>,
}

impl Default for RunOpts {
    fn default() -> RunOpts {
        RunOpts { settle: 0, max_targets: 4, min_builds: 2, top: 30, fractures: false, exact: false, trade_lookup: false, trade_budget: 40, recombs: false, workers: None }
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
    pub recombs: bool,
    pub workers: Option<usize>,
}

impl Default for LadderOpts {
    fn default() -> LadderOpts {
        LadderOpts { gear_top: 80, per_group: 3, niche: 40, cluster_top: 80, min_characters: 5, group: None, trade_lookup: false, trade_budget: 40, recombs: true, workers: None }
    }
}

fn say(progress: Option<Progress>, msg: &str) {
    if let Some(p) = progress {
        p(msg)
    }
}

/// Load builds, match their rares to mod ids, and turn each rare into a craft: the target mods
/// plus the group of bases that roll exactly the same mods. Crafts that several builds need are
/// merged. Every craft gets the cheapest plan, and optionally fracture options and trade prices.
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
            let Some(est) = est.filter(|e| !e.targets.is_empty()) else { continue };
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
        for c in mine::mine(&matched, 2, 4, opts.min_builds, true).into_iter().take(opts.top) {
            let slot = shared_estimate(cx, &c, &estimates, &mut extra, &table, &league);
            shared.push(Shared { candidate: c, estimate: slot });
        }
    }

    if opts.fractures {
        for e in estimates.iter_mut() {
            add_fractures(cx, e, &league, Some(&table), true);
        }
    }
    {
        let (n, m) = (estimates.len(), extra.len());
        let mut slots = slots_of(&mut estimates, &(0..n).collect::<Vec<_>>());
        slots.extend(slots_of(&mut extra, &(0..m).collect::<Vec<_>>()));
        solve_slots(cx, slots, &table, opts.workers, progress);
    }
    if opts.recombs {
        super::recombs::add_recombs_in_league(cx, db, &mut estimates, &table, &league, opts.workers, progress);
    }
    if opts.trade_lookup {
        let mut order: Vec<usize> = (0..estimates.len()).collect();
        order.sort_by(|&a, &b| cmp_priority(&estimates[a], &estimates[b]));
        price_on_trade(cx, &mut estimates, &order, &table, &league, opts.trade_budget, progress);
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
fn shared_estimate(cx: &Context, c: &Candidate, estimates: &[Estimate], extra: &mut Vec<Estimate>, table: &ponder_core::Prices, league: &str) -> Option<Slot> {
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
pub fn ladder(cx: &Context, db: &ModDB, characters: Option<Vec<Lite>>, opts: &LadderOpts, progress: Option<Progress>) -> Result<LadderReport> {
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
    let worn = demand::wear(db, &characters, None);
    let clusters = demand::wear_clusters(&characters, None);
    let group = opts.group.as_ref().map(|g| g.to_lowercase());
    let found: Vec<Archetype> =
        demand::archetypes(&worn, opts.min_characters, 4, 2, 0.7).into_iter().filter(|a| group.as_ref().is_none_or(|g| a.group.to_lowercase().contains(g.as_str()))).collect();
    let found_cl: Vec<demand::ClusterArchetype> = demand::cluster_archetypes(&clusters, opts.min_characters, 0.8)
        .into_iter()
        .filter(|a| group.as_ref().is_none_or(|g| a.title().to_lowercase().contains(g.as_str())))
        .collect();
    // The most worn overall, plus the top few of every base group, so niche
    // groups (quivers, claws, shields...) get costed too, not just rings and boots.
    let mut picked: Vec<usize> = Vec::new();
    let mut per: HashMap<&str, usize> = HashMap::new();
    for (i, a) in found.iter().enumerate() {
        let n = per.entry(a.group.as_str()).or_insert(0);
        if picked.len() < opts.gear_top || *n < opts.per_group {
            picked.push(i);
            *n += 1;
        }
    }
    // ...and the most niche: 3+ mods worn together far more than chance (one build's item).
    let chosen: HashSet<usize> = picked.iter().copied().collect();
    let niche_of: Vec<f64> = found.iter().map(Archetype::niche).collect();
    let mut by_niche: Vec<usize> = (0..found.len()).collect();
    by_niche.sort_by(|&a, &b| niche_of[b].partial_cmp(&niche_of[a]).unwrap_or(std::cmp::Ordering::Equal));
    let mut niches: Vec<usize> = Vec::new();
    for i in by_niche {
        let a = &found[i];
        if niches.len() >= opts.niche {
            break;
        }
        if chosen.contains(&i) || a.families.len() < 3 {
            continue;
        }
        // Variants of one niche item (the same mods give or take one) are costed once.
        let variant = niches.iter().any(|&j| {
            let o = &found[j];
            let shared = o.families.iter().filter(|f| a.families.contains(f)).count() as i64;
            o.group == a.group && shared >= o.families.len().min(a.families.len()) as i64 - 1
        });
        if !variant {
            niches.push(i);
        }
    }
    picked.extend(niches);
    say(progress, &format!("building {} gear crafts", picked.len()));
    let built: Vec<Option<Estimate>> = in_pool(opts.workers, || picked.par_iter().map(|&i| build_gear(cx, db, &found[i], &table, &league)).collect());
    let mut estimates: indexmap::IndexMap<Key, Estimate> = indexmap::IndexMap::new();
    for (&i, est) in picked.iter().zip(built) {
        let a = &found[i];
        let Some(mut est) = est.filter(|e| e.error.is_none() && !estimates.contains_key(&e.key)) else { continue };
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
        add_fractures(cx, &mut est, &league, Some(&table), true); // fractured bases are most of the cluster market's money
        estimates.insert(est.key.clone(), est);
    }
    let mut out: Vec<Estimate> = estimates.into_values().collect();
    let all: Vec<usize> = (0..out.len()).collect();
    solve_slots(cx, slots_of(&mut out, &all), &table, opts.workers, progress);
    if opts.recombs {
        super::recombs::add_recombs_in_league(cx, db, &mut out, &table, &league, opts.workers, progress);
    }
    if opts.trade_lookup {
        let mut order: Vec<usize> = (0..out.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(out[i].demand));
        price_on_trade(cx, &mut out, &order, &table, &league, opts.trade_budget, progress);
    }
    // after pricing: a priced fractured base can change the recipe the pivots follow
    say(progress, "finding pivots");
    in_pool(opts.workers, || add_pivots(&mut out));
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

/// Run `f` on a pool of `workers` threads, or on rayon's global pool.
fn in_pool<T: Send>(workers: Option<usize>, f: impl FnOnce() -> T + Send) -> T {
    match workers.filter(|w| *w > 0).and_then(|w| rayon::ThreadPoolBuilder::new().num_threads(w).build().ok()) {
        Some(pool) => pool.install(f),
        None => f(),
    }
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
            let links: Vec<String> = text.lines().map(|l| l.split('#').next().unwrap_or("").trim().to_string()).filter(|l| !l.is_empty()).collect();
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

/// The newest league poe.ninja lists, else "Standard" (offline).
pub fn current_league(cx: &Context) -> String {
    cx.prices.leagues().ok().and_then(|l| l.into_iter().next()).unwrap_or_else(|| "Standard".into())
}
