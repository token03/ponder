use std::sync::Arc;
use std::time::Instant;

use ponder::{
    gear,
    pipeline::{self, Cache, Context, LadderOpts},
};
use ponder_game::mods::ModDB;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let started = Instant::now();
    let db = ModDB::load()?;
    let mut cx = Context::real();
    if args.iter().any(|arg| arg == "--offline") {
        let offline: Arc<dyn ponder_net::http::Transport> = Arc::new(|_: &ponder_net::http::Request| anyhow::bail!("network disabled"));
        cx.http = offline.clone();
        cx.prices.http = offline.clone();
        cx.trade.http = offline.clone();
        cx.ladder.http = offline;
    }
    if let Some(path) = args.windows(2).find(|args| args[0] == "--plan-cache").map(|args| &args[1]) {
        cx.plans = Cache::new(path);
    }
    if args.iter().any(|arg| arg == "--cold") {
        cx.plans = Cache::disabled();
    }
    let opts = if args.iter().any(|arg| arg == "--full") {
        LadderOpts { workers: Some(4), ..Default::default() }
    } else {
        LadderOpts { gear_top: 3, per_group: 0, niche: 0, cluster_top: 3, workers: Some(4), ..Default::default() }
    };
    let say = |message: &str| eprintln!("{:.3}s {message}", started.elapsed().as_secs_f64());
    let report = pipeline::ladder(&cx, &db, None, &opts, Some(&say))?;
    if let Some(path) = args.windows(2).find(|args| args[0] == "--snapshot").map(|args| &args[1]) {
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    println!(
        "{}",
        serde_json::json!({"estimates": report.estimates.len(), "solved": report.estimates.iter().filter(|estimate| estimate.summary.is_some()).count(), "quoted": report.estimates.iter().filter(|estimate| estimate.total().is_some()).count(), "elapsed_s": started.elapsed().as_secs_f64(), "chaos_per_hour": gear::Settings::default().chaos_per_hour})
    );
    Ok(())
}
