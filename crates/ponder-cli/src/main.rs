//! `ponder`: the command line. Every command prints JSON on stdout (for scripts and LLMs);
//! progress goes to stderr. The desktop app is for browsing.

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use clap::{Args, Parser, Subcommand};
use ponder::pipeline::{self, Context, LadderOpts, RunOpts};
use ponder::{demand, gear};
use ponder_game::item::parse_item;
use ponder_game::mods::ModDB;
use ponder_net::ladder::Sample;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "ponder", version, about = "PoE 3.30 market holes: what the ladder wears, what it costs to craft, what trade has.")]
struct Cli {
    /// pretty-print the JSON
    #[arg(long, global = true)]
    pretty: bool,
    /// no progress lines on stderr
    #[arg(long, short, global = true)]
    quiet: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Cost every rare in some builds, mine combos they share, optionally price on trade
    Run(RunArgs),
    /// What ladder characters wear (poe.ninja), and what it costs to craft
    #[command(subcommand)]
    Ladder(LadderCmd),
    /// Downloaded game data (repoe)
    Data {
        #[arg(value_parser = ["fetch", "version"])]
        action: String,
    },
    /// poe.ninja prices used for costs
    Prices {
        #[arg(value_parser = ["fetch", "show"])]
        action: String,
        /// poe.ninja league (default: the current challenge league)
        #[arg(long)]
        league: Option<String>,
    },
    /// Read an item (in-game Ctrl+C / Ctrl+Alt+C or PoB text) and match its mods
    Item {
        /// file with the item text; - reads stdin
        item: String,
    },
}

#[derive(Args)]
struct RunArgs {
    /// pobb.in / pastebin URLs, PoB codes, files or directories
    #[arg(required = true)]
    sources: Vec<String>,
    /// price finished items and fractured bases on the trade site (POESESSID in .env)
    #[arg(long)]
    trade: bool,
    /// most trade searches to send (cached answers are free)
    #[arg(long, default_value_t = 40)]
    trade_budget: usize,
    /// also cost starting from a base with each target already fractured
    #[arg(long)]
    fractures: bool,
    /// every mod as on the PoB item (resistances aren't interchangeable filler)
    #[arg(long)]
    exact: bool,
    /// accept tiers this many steps worse
    #[arg(long, default_value_t = 0)]
    settle: i64,
    /// skip items with more target mods than this
    #[arg(long, default_value_t = 6)]
    max_targets: usize,
    #[arg(long, default_value_t = 2)]
    min_builds: usize,
    /// threads solving in parallel (default: all cores)
    #[arg(long)]
    workers: Option<usize>,
}

#[derive(Subcommand)]
enum LadderCmd {
    /// Download the top characters of every popular skill and class
    Fetch {
        #[arg(long)]
        league: Option<String>,
        #[arg(long, default_value_t = 10)]
        per_skill: usize,
        #[arg(long, default_value_t = 20)]
        per_class: usize,
        /// skills with fewer players are skipped
        #[arg(long, default_value_t = 40)]
        min_players: u64,
    },
    /// Mod combinations many characters share, per base group (crafting candidates)
    Archetypes {
        /// only base groups containing this text
        #[arg(long)]
        group: Option<String>,
        #[arg(long, default_value_t = 10)]
        min_characters: usize,
        #[arg(long, default_value_t = 40)]
        top: usize,
    },
    /// Per base group: how often each mod is worn, and how much its tier matters
    Tiers {
        /// only base groups containing this text, e.g. "Gloves (ES)"
        #[arg(long)]
        group: Option<String>,
        #[arg(long, default_value_t = 15)]
        min_characters: usize,
    },
    /// Fit what each core mod's tier is worth from trade listings (POESESSID in .env)
    Prices {
        /// only base groups containing this text
        #[arg(long)]
        group: Option<String>,
        #[arg(long, default_value_t = 10)]
        min_characters: usize,
        /// archetypes to price (one search and a few fetches each)
        #[arg(long, default_value_t = 5)]
        top: usize,
        #[arg(long, default_value_t = 50)]
        listings: usize,
    },
    /// Cost what the ladder wears (gear archetypes + cluster jewels); rank market holes
    Crafts(CraftsArgs),
    /// What a real item (dropped, bought, half-crafted) can be finished into, and how
    Finish {
        /// file with the item text; - reads stdin
        item: String,
        #[arg(long, default_value_t = 5)]
        top: usize,
    },
}

#[derive(Args)]
struct CraftsArgs {
    /// only base groups / cluster enchants containing this text
    #[arg(long)]
    group: Option<String>,
    /// rarer combinations are skipped
    #[arg(long, default_value_t = 5)]
    min_characters: usize,
    /// gear archetypes to cost, most worn first
    #[arg(long, default_value_t = 80)]
    gear: usize,
    /// also cost the top few of every base group
    #[arg(long, default_value_t = 3)]
    per_group: usize,
    /// also cost the most niche 3+ mod combos
    #[arg(long, default_value_t = 40)]
    niche: usize,
    /// cluster notable combos to cost
    #[arg(long, default_value_t = 80)]
    clusters: usize,
    /// price finished items on trade (POESESSID in .env)
    #[arg(long)]
    trade: bool,
    #[arg(long, default_value_t = 40)]
    trade_budget: usize,
    /// only the best N estimates (all by default)
    #[arg(long)]
    top: Option<usize>,
    /// search trade for half-done items for this many of the best gear crafts (POESESSID in .env)
    #[arg(long, default_value_t = 0)]
    starts: usize,
    #[arg(long)]
    workers: Option<usize>,
}

fn main() {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(v) => {
            let mut out = std::io::stdout().lock();
            let text = if cli.pretty { serde_json::to_string_pretty(&v) } else { serde_json::to_string(&v) };
            let _ = writeln!(out, "{}", text.expect("JSON values serialise"));
        }
        Err(e) => {
            println!("{}", json!({"error": format!("{e:#}")}));
            std::process::exit(1);
        }
    }
}

fn progress_printer(quiet: bool) -> Box<dyn Fn(&str) + Sync> {
    if quiet {
        return Box::new(|_| {});
    }
    let tty = std::io::stderr().is_terminal();
    Box::new(move |msg: &str| {
        let mut err = std::io::stderr().lock();
        let _ = if tty { write!(err, "\r\x1b[2K{}", msg.chars().take(100).collect::<String>()) } else { writeln!(err, "{msg}") };
        let _ = err.flush();
    })
}

fn context(quiet: bool) -> Context {
    let mut cx = Context::real();
    if !quiet {
        cx.trade.on_wait = Some(Arc::new(|secs| eprintln!("waiting {secs:.0}s for the trade site's rate limit")));
    }
    cx
}

/// The cached ladder's items, matched to mod ids.
fn ladder_worn(cx: &Context, progress: &(dyn Fn(&str) + Sync)) -> Result<(ModDB, Vec<demand::Worn>)> {
    let db = ModDB::load()?;
    progress("reading cached ladder characters");
    let characters = cx.ladder.cached(None)?;
    if characters.is_empty() {
        bail!("no ladder data yet: run `ponder ladder fetch`");
    }
    progress("matching ladder items");
    let worn = demand::wear(&db, &characters, None);
    Ok((db, worn))
}

/// Gear archetypes (the ladder run's settings), only base groups containing `group`.
fn archetypes(worn: &[demand::Worn], min_characters: usize, group: Option<&str>) -> Vec<demand::Archetype> {
    let group = group.map(str::to_lowercase);
    demand::archetypes(worn, min_characters, 4, 2, 0.7)
        .into_iter()
        .filter(|a| group.as_ref().is_none_or(|g| a.group.to_lowercase().contains(g.as_str())))
        .collect()
}

fn trade_error(e: &anyhow::Error) -> String {
    let e = format!("{e:#}");
    if e.contains("401") || e.contains("403") { format!("{e}: check POESESSID in .env") } else { e }
}

fn top_counts(c: &demand::Counter<String>, n: usize) -> Value {
    c.most_common().into_iter().take(n).map(|(k, v)| (k.clone(), json!(v))).collect::<serde_json::Map<_, _>>().into()
}

fn read_text(path: &str) -> Result<String> {
    if path == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        return Ok(s);
    }
    std::fs::read_to_string(path).with_context(|| format!("reading {path}"))
}

fn run(cli: &Cli) -> Result<Value> {
    let say = progress_printer(cli.quiet);
    let progress: &(dyn Fn(&str) + Sync) = &*say;
    let done = || {
        if !cli.quiet && std::io::stderr().is_terminal() {
            eprint!("\r\x1b[2K");
        }
    };
    let v = match &cli.command {
        Command::Data { action } => {
            let dir = ponder_game::data::data_dir();
            if action == "fetch" {
                let version =
                    ponder_net::repoe::fetch(&*ponder_net::http::real(), &dir, ponder_game::data::SOURCE, ponder_game::data::FILES)?;
                json!({"version": version, "dir": dir})
            } else {
                json!({"version": ponder_game::data::version(Some(&dir)), "dir": dir})
            }
        }
        Command::Prices { action, league } => {
            let cx = context(cli.quiet);
            let fetched = if action == "fetch" { Some(cx.prices.fetch(league.as_deref())?) } else { None };
            let (table, source) = cx.prices.load()?;
            let mut ids: Vec<_> = table.into_iter().collect();
            ids.sort_by(|a, b| a.0.cmp(&b.0));
            json!({"source": source, "fetched": fetched.map(|(l, n)| json!({"league": l, "count": n})), "chaos": ids.into_iter().map(|(k, v)| (k, json!(v))).collect::<serde_json::Map<_, _>>()})
        }
        Command::Item { item } => {
            let db = ModDB::load()?;
            let parsed = parse_item(&read_text(item)?).map_err(|e| anyhow::anyhow!("can't read the item: {e}"))?;
            let m = db.match_item(&parsed);
            let group = m.base_id.as_ref().map(|_| gear::group(&db, &m).label.clone());
            json!({
                "name": m.item.name, "base": m.item.base, "base_id": m.base_id, "group": group, "item_level": m.item.item_level,
                "influences": m.item.influences, "flags": m.item.flags,
                "mods": m.mods.iter().map(|x| json!({"mod_id": x.mod_id, "kind": x.kind, "family": x.family, "tier": x.tier, "source": x.source, "fractured": x.fractured()})).collect::<Vec<_>>(),
                "unmatched": m.unmatched.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            })
        }
        Command::Run(a) => {
            let cx = context(cli.quiet);
            let db = ModDB::load()?;
            let opts = RunOpts {
                settle: a.settle,
                max_targets: a.max_targets,
                min_builds: a.min_builds,
                fractures: a.fractures,
                exact: a.exact,
                trade_lookup: a.trade,
                trade_budget: a.trade_budget,
                workers: a.workers,
                ..RunOpts::default()
            };
            let r = pipeline::run(&cx, &db, &a.sources, &opts, Some(progress))?;
            done();
            r.to_dict()
        }
        Command::Ladder(LadderCmd::Fetch { league, per_skill, per_class, min_players }) => {
            let cx = context(cli.quiet);
            let snap = cx.ladder.snapshot(league.as_deref())?;
            let opts = Sample { per_skill: *per_skill, per_class: *per_class, min_players: *min_players, ..Sample::default() };
            let chars = cx.ladder.sample(&snap, &opts, Some(progress))?;
            done();
            json!({"characters": chars.len(), "league": snap.league, "version": snap.version, "cache": PathBuf::from(ponder_net::ladder::CACHE)})
        }
        Command::Ladder(LadderCmd::Crafts(a)) => {
            let cx = context(cli.quiet);
            let db = ModDB::load()?;
            let opts = LadderOpts {
                gear_top: a.gear,
                per_group: a.per_group,
                niche: a.niche,
                cluster_top: a.clusters,
                min_characters: a.min_characters,
                group: a.group.clone(),
                trade_lookup: a.trade,
                trade_budget: a.trade_budget,
                workers: a.workers,
            };
            let mut r = pipeline::ladder(&cx, &db, None, &opts, Some(progress))?;
            if r.characters == 0 {
                bail!("no ladder data yet: run `ponder ladder fetch`");
            }
            if a.starts > 0 {
                let (table, _) = cx.prices.load()?;
                let order: Vec<usize> = {
                    let ranked = r.ranked();
                    ranked
                        .iter()
                        .filter(|e| e.plan.is_some() && e.kind == "gear")
                        .take(a.starts)
                        .map(|e| r.estimates.iter().position(|x| std::ptr::eq(x, *e)).expect("ranked from estimates"))
                        .collect()
                };
                for (n, i) in order.iter().enumerate() {
                    progress(&format!("half-done items {}/{}: {}", n + 1, order.len(), r.estimates[*i].title));
                    let league = r.league.clone();
                    if let Err(e) = pipeline::find_starts(&cx, &db, &mut r.estimates[*i], &table, &league, 20, 5) {
                        r.estimates[*i].market_error = Some(format!("{e:#}"));
                    }
                }
            }
            done();
            // starts aren't in Estimate::to_dict (its keys are fixed): added where there are any
            let dict = |e: &pipeline::Estimate| {
                let mut v = e.to_dict();
                if !e.starts.is_empty() {
                    v["starts"] = e.starts.iter().map(|f| f.to_dict()).collect();
                    v["starts_url"] = json!(e.starts_url);
                }
                v
            };
            let mut v = r.to_dict();
            v["estimates"] = r.ranked().into_iter().take(a.top.unwrap_or(usize::MAX)).map(dict).collect();
            v
        }
        Command::Ladder(LadderCmd::Archetypes { group, min_characters, top }) => {
            let (db, worn) = ladder_worn(&context(cli.quiet), progress)?;
            let found = archetypes(&worn, *min_characters, group.as_deref());
            done();
            json!({"archetypes": found.len(), "shown": found.iter().take(*top).map(|a| json!({
                "group": a.group, "mods": demand::archetype_names(&db, a), "families": a.families, "characters": a.characters, "items": a.items,
                "lift": a.lift, "niche": a.niche(), "skills": top_counts(&a.skills, 5), "bases": top_counts(&a.bases, 5),
                "resistances": a.resistances.most_common().first().map(|(n, _)| **n),
            })).collect::<Vec<_>>()})
        }
        Command::Ladder(LadderCmd::Tiers { group, min_characters }) => {
            let (db, worn) = ladder_worn(&context(cli.quiet), progress)?;
            let want = group.as_ref().map(|g| g.to_lowercase());
            let worn: Vec<_> =
                worn.into_iter().filter(|w| want.as_ref().is_none_or(|g| w.group.to_lowercase().contains(g.as_str()))).collect();
            let stats = demand::family_stats(&db, &worn, *min_characters);
            done();
            json!({"families": stats.iter().map(|f| json!({
                "group": f.group, "family": f.family, "side": f.side, "text": f.text, "characters": f.characters,
                "t1": f.share_at_or_above(1), "t2_or_better": f.share_at_or_above(2),
                "t1_natural": f.natural_at_or_above(1), "t1_lift": f.lift(1), "t2_lift": f.lift(2),
            })).collect::<Vec<_>>()})
        }
        Command::Ladder(LadderCmd::Prices { group, min_characters, top, listings }) => {
            let cx = context(cli.quiet);
            let (db, worn) = ladder_worn(&cx, progress)?;
            let (table, _) = cx.prices.load()?;
            let league = pipeline::current_league(&cx);
            let td = ponder::pricing::TradeData::load();
            let mut out = Vec::new();
            for a in archetypes(&worn, *min_characters, group.as_deref()).iter().filter(|a| a.families.len() >= 2).take(*top) {
                let keys: Vec<&str> = a.families.iter().map(String::as_str).collect();
                let Some(query) = ponder::pricing::archetype_query(&db, &a.examples[0], &keys, ponder::pricing::FLOOR_TIER, td) else {
                    continue;
                };
                progress(&format!("trade: {}", a.group));
                let url = ponder_net::trade::search_url(&league, &query);
                let row = match cx.trade.listings(&league, &query, *listings, None) {
                    // one failure (a 403, a rate limit) doesn't stop the rest
                    Err(e) => {
                        json!({"group": a.group, "mods": demand::archetype_names(&db, a), "search": url, "error": trade_error(&e)})
                    }
                    Ok(found) => {
                        let model = ponder::pricing::fit(&db, &a.group, &keys, &found, &table);
                        json!({"group": a.group, "mods": demand::archetype_names(&db, a), "search": url, "model": model.map(|m| json!({
                            "listings": m.listings, "base_price": m.base_price, "per_res": m.per_res, "per_open_affix": m.per_open_affix, "r2": m.r2,
                            "tiers": m.tiers.iter().map(|t| json!({"key": t.key, "text": t.text, "t1": t.t1, "t2": t.t2, "n_t1": t.n_t1, "n_t2": t.n_t2})).collect::<Vec<_>>(),
                        }))})
                    }
                };
                out.push(row);
            }
            done();
            json!({"league": league, "archetypes": out})
        }
        Command::Ladder(LadderCmd::Finish { item, top }) => {
            let cx = context(cli.quiet);
            let db = ModDB::load()?;
            let parsed = parse_item(&read_text(item)?).map_err(|e| anyhow::anyhow!("can't read the item: {e}"))?;
            let m = db.match_item(&parsed);
            if m.base_id.is_none() {
                bail!("unknown base {:?}", m.item.base);
            }
            let label = gear::group(&db, &m).label.clone();
            let opts = LadderOpts { group: Some(label.clone()), ..LadderOpts::default() };
            let r = pipeline::ladder(&cx, &db, None, &opts, Some(progress))?;
            if r.characters == 0 {
                bail!("no ladder data yet: run `ponder ladder fetch`");
            }
            let (table, _) = cx.prices.load()?;
            let mine: Vec<_> = r.estimates.into_iter().filter(|e| e.title == label).collect();
            let found = pipeline::finishes(&cx, &db, &m, &mine, &table, *top);
            done();
            json!({"item": if m.item.name.is_empty() { &m.item.base } else { &m.item.name }, "base": m.item.base, "group": label, "finishes": found.iter().map(|f| f.to_dict()).collect::<Vec<_>>()})
        }
    };
    Ok(v)
}
