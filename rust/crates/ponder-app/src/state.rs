//! Everything the panes read and the actions that change it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use ponder::gear;
use ponder::pipeline::{self, Context, Estimate, LadderOpts};
use ponder_game::item::parse_item;
use ponder_game::mods::ModDB;
use serde::{Deserialize, Serialize};

use crate::jobs::{Finished, Jobs, Loaded, Msg};
use crate::view::{self, Filters, Sort};

/// The ladder run's knobs (`LadderOpts`), kept between sessions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunSettings {
    pub gear_top: usize,
    pub per_group: usize,
    pub niche: usize,
    pub cluster_top: usize,
    pub min_characters: usize,
    pub group: String,
    pub recombs: bool,
    pub trade: bool,
    pub trade_budget: usize,
    pub chaos_per_hour: f64,
}

impl Default for RunSettings {
    fn default() -> Self {
        let o = LadderOpts::default();
        RunSettings {
            gear_top: o.gear_top,
            per_group: o.per_group,
            niche: o.niche,
            cluster_top: o.cluster_top,
            min_characters: o.min_characters,
            group: String::new(),
            recombs: false,
            trade: false,
            trade_budget: o.trade_budget,
            chaos_per_hour: pipeline::settings_from_env().chaos_per_hour,
        }
    }
}

impl RunSettings {
    fn opts(&self) -> LadderOpts {
        LadderOpts {
            gear_top: self.gear_top,
            per_group: self.per_group,
            niche: self.niche,
            cluster_top: self.cluster_top,
            min_characters: self.min_characters,
            group: Some(self.group.trim().to_string()).filter(|g| !g.is_empty()),
            trade_lookup: self.trade,
            trade_budget: self.trade_budget,
            recombs: self.recombs,
            workers: None,
        }
    }
}

/// What survives a restart (besides the tile layout).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub filters: Filters,
    pub sort: Sort,
    pub run: RunSettings,
    pub auto_run: bool,
}

pub struct State {
    pub saved: Saved,
    pub cx: Arc<Context>,
    pub db: Option<Arc<ModDB>>,
    pub data: Option<Loaded>,
    /// bumped by every ladder run, so a late job result for an older run is dropped
    pub run: u64,
    pub ranks: Vec<usize>,
    pub groups: Vec<String>,
    /// visible estimates, in display order
    pub rows: Vec<usize>,
    pub selected: Option<usize>,
    /// scroll the crafts table to the selection on the next frame
    pub reveal: bool,
    /// recipe step to scroll to
    pub jump: Option<usize>,
    pub log: Vec<(Instant, String)>,
    pub status: String,
    pub jobs: Jobs,
    pub finish_text: String,
    pub finish: Option<Result<Box<Finished>, String>>,
    pub has_cookie: bool,
    pub has_ladder: bool,
    pub started: Instant,
}

impl State {
    pub fn new(ctx: &egui::Context, saved: Saved) -> State {
        let jobs = Jobs::new(ctx.clone());
        jobs.spawn("game data", |_| Msg::Db(ModDB::load().map(Arc::new).map_err(|e| e.to_string())));
        let has_ladder = std::fs::read_dir(ponder_net::ladder::CACHE).is_ok_and(|mut d| d.next().is_some());
        let mut s = State {
            cx: Arc::new(context(&saved.run)),
            saved,
            db: None,
            data: None,
            run: 0,
            ranks: Vec::new(),
            groups: Vec::new(),
            rows: Vec::new(),
            selected: None,
            reveal: false,
            jump: None,
            log: Vec::new(),
            status: "loading game data".into(),
            jobs,
            finish_text: String::new(),
            finish: None,
            has_cookie: ponder_net::trade::cookie().is_some(),
            has_ladder,
            started: Instant::now(),
        };
        if !s.has_ladder {
            s.note("no ladder data yet: `ponder ladder fetch` downloads it (or use Run → Fetch ladder)");
        }
        s
    }

    pub fn note(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        self.status.clone_from(&msg);
        self.log.push((Instant::now(), msg));
    }

    pub fn divine(&self) -> f64 {
        self.data.as_ref().map_or(0.0, |d| d.report.divine)
    }

    pub fn estimates(&self) -> &[Estimate] {
        self.data.as_ref().map_or(&[], |d| &d.report.estimates)
    }

    pub fn selected_estimate(&self) -> Option<&Estimate> {
        self.estimates().get(self.selected?)
    }

    pub fn refilter(&mut self) {
        self.rows = view::visible(self.estimates(), &self.saved.filters, self.saved.sort);
    }

    pub fn select(&mut self, index: usize) {
        if self.selected != Some(index) {
            self.selected = Some(index);
            self.jump = None;
        }
    }

    /// Move the selection by `delta` rows of the table.
    pub fn step_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let at = self.selected.and_then(|s| self.rows.iter().position(|&r| r == s));
        let next = match at {
            Some(p) => (p as isize + delta).clamp(0, self.rows.len() as isize - 1) as usize,
            None => 0,
        };
        self.select(self.rows[next]);
        self.reveal = true;
    }

    pub fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Status(s) => {
                self.status = s;
            }
            Msg::Db(Ok(db)) => {
                self.db = Some(db);
                self.note("game data loaded");
                if self.saved.auto_run && self.has_ladder && self.data.is_none() {
                    self.run_ladder();
                }
            }
            Msg::Db(Err(e)) => self.note(format!("game data: {e} (run `ponder data fetch`)")),
            Msg::Ladder(Ok(loaded)) => {
                let r = &loaded.report;
                let msg = format!(
                    "{} estimates from {} characters ({} archetypes) in {:.1}s",
                    r.estimates.len(),
                    r.characters,
                    r.archetypes,
                    loaded.took.as_secs_f64()
                );
                let keep = self.selected_estimate().map(|e| (e.title.clone(), e.targets.clone()));
                self.ranks = view::ranks(&loaded.report.estimates);
                self.groups = view::groups(&loaded.report.estimates);
                self.data = Some(*loaded);
                self.selected = keep.and_then(|(t, ts)| self.estimates().iter().position(|e| e.title == t && e.targets == ts));
                self.refilter();
                if self.selected.is_none() {
                    self.selected = self.rows.first().copied();
                }
                self.note(msg);
            }
            Msg::Ladder(Err(e)) => self.note(format!("ladder run failed: {e}")),
            Msg::Estimate { run, index, estimate, note } => {
                if run == self.run
                    && let Some(d) = self.data.as_mut()
                    && index < d.report.estimates.len()
                {
                    d.report.estimates[index] = *estimate;
                    self.ranks = view::ranks(&d.report.estimates);
                    self.refilter();
                }
                self.note(note);
            }
            Msg::Finished(r) => {
                match &r {
                    Ok(f) => self.note(format!("{} ways to finish the item", f.found.len())),
                    Err(e) => self.note(format!("finishing failed: {e}")),
                }
                self.finish = Some(r);
            }
        }
    }

    // -- actions -----------------------------------------------------------------------------

    pub fn run_ladder(&mut self) {
        let Some(db) = self.db.clone() else { return };
        if self.jobs.is_running("ladder") {
            return;
        }
        self.cx = Arc::new(context(&self.saved.run));
        self.run += 1;
        let cx = self.cx.clone();
        let opts = self.saved.run.opts();
        self.note("running the ladder pipeline");
        self.jobs.spawn("ladder", move |reply| {
            let t = Instant::now();
            let say = |m: &str| reply.status(m);
            let table = match cx.prices.load() {
                Ok((t, _)) => t,
                Err(e) => return Msg::Ladder(Err(format!("{e:#} (fetch prices first)"))),
            };
            let r = pipeline::ladder(&cx, &db, None, &opts, Some(&say));
            Msg::Ladder(r.map(|report| Box::new(Loaded { report, table, took: t.elapsed() })).map_err(|e| format!("{e:#}")))
        });
    }

    pub fn fetch_ladder(&mut self) {
        if self.jobs.is_running("fetch ladder") {
            return;
        }
        let cx = self.cx.clone();
        self.note("downloading ladder characters from poe.ninja");
        self.jobs.spawn("fetch ladder", move |reply| {
            let say = |m: &str| reply.status(m);
            let r = cx.ladder.snapshot(None).and_then(|snap| {
                let chars = cx.ladder.sample(&snap, &ponder_net::ladder::Sample::default(), Some(&say))?;
                Ok(format!("{} characters from {} cached", chars.len(), snap.league))
            });
            Msg::Status(r.unwrap_or_else(|e| format!("ladder fetch failed: {e:#}")))
        });
    }

    pub fn fetch_prices(&mut self) {
        let cx = self.cx.clone();
        self.jobs.spawn("fetch prices", move |_| {
            Msg::Status(match cx.prices.fetch(None) {
                Ok((league, n)) => format!("fetched {n} prices for {league}"),
                Err(e) => format!("price fetch failed: {e:#}"),
            })
        });
    }

    /// Clone the selected estimate into a job that changes it, and put the result back.
    fn job_on_selected(&mut self, name: &str, work: impl FnOnce(&Context, &ModDB, &Priced, &mut Estimate, &crate::jobs::Reply) -> String + Send + 'static) {
        let (Some(index), Some(db), Some(d)) = (self.selected, self.db.clone(), self.data.as_ref()) else { return };
        let job = format!("{name} #{index}");
        if self.jobs.is_running(&job) {
            return;
        }
        let mut est = d.report.estimates[index].clone();
        let ctx = Priced { table: d.table.clone(), league: d.report.league.clone() };
        let (cx, run) = (self.cx.clone(), self.run);
        self.note(format!("{name}: {}", est.title));
        self.jobs.spawn(&job, move |reply| {
            let note = work(&cx, &db, &ctx, &mut est, reply);
            Msg::Estimate { run, index, estimate: Box::new(est), note }
        });
    }

    pub fn price_selected(&mut self) {
        self.job_on_selected("trade prices", |cx, _, d, est, reply| {
            let say = |m: &str| reply.status(m);
            pipeline::lookup_market(cx, est, &d.table, &d.league);
            let n = pipeline::price_fractures(cx, std::slice::from_mut(est), &d.table, &d.league, 3, 6, Some(&say));
            match (&est.market, &est.market_error) {
                (Some((p, n_listed)), _) => format!("{}: {p:.0}c, {n_listed} listed; {n} fractured bases priced", est.title),
                (None, Some(e)) => format!("{}: {e}", est.title),
                _ => format!("{}: not listed", est.title),
            }
        });
    }

    pub fn starts_selected(&mut self) {
        self.job_on_selected("half-done items", |cx, db, d, est, _| match pipeline::find_starts(cx, db, est, &d.table, &d.league, 20, 5) {
            Ok(found) => {
                let n = found.len();
                est.starts = found;
                format!("{n} half-done {} on trade", est.title)
            }
            Err(e) => format!("half-done search failed: {e:#}"),
        });
    }

    pub fn recomb_selected(&mut self) {
        self.job_on_selected("recombinator", |cx, db, d, est, reply| {
            let say = |m: &str| reply.status(m);
            pipeline::add_recombs(cx, db, std::slice::from_mut(est), &d.table, None, Some(&say));
            match est.recomb_total() {
                Some(c) => format!("recombinator tree for {}: {c:.0}c", est.title),
                None => format!("no recombinator tree for {}", est.title),
            }
        });
    }

    /// Read the pasted item and find the ladder products it can be finished into.
    pub fn finish_item(&mut self) {
        let (Some(db), Some(d)) = (self.db.clone(), self.data.as_ref()) else { return };
        let item = match parse_item(&self.finish_text) {
            Ok(i) => i,
            Err(e) => {
                self.finish = Some(Err(format!("can't read the item: {e}")));
                return;
            }
        };
        let m = db.match_item(&item);
        if m.base_id.is_none() {
            self.finish = Some(Err(format!("unknown base {:?}", m.item.base)));
            return;
        }
        let group = gear::group(&db, &m).label.clone();
        let mine: Vec<Estimate> = d.report.estimates.iter().filter(|e| e.title == group).cloned().collect();
        if mine.is_empty() {
            self.finish = Some(Err(format!("the ladder run has no {group} crafts (widen the run or set its group to {group:?})")));
            return;
        }
        let (cx, table) = (self.cx.clone(), d.table.clone());
        self.jobs.spawn("finish item", move |_| {
            let found = pipeline::finishes(&cx, &db, &m, &mine, &table, 8);
            Msg::Finished(Ok(Box::new(Finished { item: m, group, found })))
        });
    }

    /// Write the report as JSON (the CLI's format) next to the working directory.
    pub fn export(&mut self) {
        let Some(d) = self.data.as_ref() else { return };
        let path = PathBuf::from(format!("ponder-{}.json", d.report.league.to_lowercase().replace(' ', "-")));
        let r = std::fs::File::create(&path).map_err(anyhow::Error::from).and_then(|f| {
            serde_json::to_writer_pretty(std::io::BufWriter::new(f), &d.report.to_dict())?;
            Ok(())
        });
        match r {
            Ok(()) => self.note(format!("saved {}", std::fs::canonicalize(&path).unwrap_or(path).display())),
            Err(e) => self.note(format!("export failed: {e:#}")),
        }
    }
}

/// The prices and league a job on one estimate needs.
pub struct Priced {
    pub table: ponder_core::Prices,
    pub league: String,
}

fn context(run: &RunSettings) -> Context {
    let mut cx = Context::real();
    if run.chaos_per_hour >= 0.0 {
        cx.settings.chaos_per_hour = run.chaos_per_hour;
    }
    cx
}
