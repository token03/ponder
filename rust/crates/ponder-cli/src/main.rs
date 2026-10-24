//! `ponder`: the command line. Every command prints JSON on stdout (for scripts and LLMs);
//! progress goes to stderr. The desktop app is for browsing.

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use clap::{Args, Parser, Subcommand};
use ponder::gear;
use ponder::pipeline::{self, Context, LadderOpts, RunOpts};
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
    /// also cost recombinator trees
    #[arg(long)]
    recomb: bool,
    /// every mod as on the PoB item (resistances aren't interchangeable filler)
    #[arg(long)]
    exact: bool,
    /// accept tiers this many steps worse
    #[arg(long, default_value_t = 0)]
    settle: i64,
    /// skip items with more target mods than this
    #[arg(long, default_value_t = 4)]
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
    /// also cost recombinator trees
    #[arg(long)]
    recomb: bool,
    /// only the best N estimates (all by default)
    #[arg(long)]
    top: Option<usize>,
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
            let dir = ponder_net::repoe::data_dir();
            if action == "fetch" {
                let version = ponder_net::repoe::fetch(&*ponder_net::http::real(), &dir)?;
                json!({"version": version, "dir": dir})
            } else {
                json!({"version": ponder_net::repoe::version(&dir), "dir": dir})
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
                recombs: a.recomb,
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
                recombs: a.recomb,
                workers: a.workers,
            };
            let r = pipeline::ladder(&cx, &db, None, &opts, Some(progress))?;
            done();
            if r.characters == 0 {
                bail!("no ladder data yet: run `ponder ladder fetch`");
            }
            let mut v = r.to_dict();
            if let Some(top) = a.top {
                v["estimates"] = r.ranked().into_iter().take(top).map(|e| e.to_dict()).collect();
            }
            v
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
