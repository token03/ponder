use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_notify::Toasts;
use ponder::pipeline::{self, Context, Depth, Estimate, Key, LadderOpts};
use ponder_game::mods::ModDB;
use serde::{Deserialize, Serialize};

use crate::filter::ItemCatalog;
use crate::jobs::{Jobs, Loaded, Msg};
use crate::view::{self, Filters, Sort};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanSettings {
    pub gear_top: usize,
    pub per_group: usize,
    pub niche: usize,
    pub cluster_top: usize,
    pub min_characters: usize,
    pub trade: bool,
    pub trade_budget: usize,
    pub chaos_per_hour: f64,
}

impl Default for ScanSettings {
    fn default() -> Self {
        let opts = LadderOpts::default();
        Self {
            gear_top: opts.gear_top,
            per_group: opts.per_group,
            niche: opts.niche,
            cluster_top: opts.cluster_top,
            min_characters: opts.min_characters,
            trade: false,
            trade_budget: opts.trade_budget,
            chaos_per_hour: pipeline::settings_from_env().chaos_per_hour,
        }
    }
}

impl ScanSettings {
    fn opts(&self) -> LadderOpts {
        LadderOpts {
            gear_top: self.gear_top,
            per_group: self.per_group,
            niche: self.niche,
            cluster_top: self.cluster_top,
            min_characters: self.min_characters,
            group: None,
            trade_lookup: self.trade,
            trade_budget: self.trade_budget,
            workers: None,
            solve: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub filters: Filters,
    pub sort: Sort,
    pub scan: ScanSettings,
}

pub struct State {
    pub saved: Saved,
    pub cx: Arc<Context>,
    pub db: Option<Arc<ModDB>>,
    pub data: Option<Loaded>,
    pub run: u64,
    pub ranks: Vec<usize>,
    pub catalog: ItemCatalog,
    pub rows: Vec<usize>,
    pub selected: Option<usize>,
    /// a craft to open in its own pane
    pub pin: Option<usize>,
    pub reveal: bool,
    pub log: Vec<(Instant, String)>,
    pub status: String,
    pub toasts: Toasts,
    /// Estimates showing a quick plan while their full plan is solved.
    pub quick: HashSet<Key>,
    /// Estimates of this run with their final plan, of those to solve.
    pub solved: (usize, usize),
    solve_started: Instant,
    pub jobs: Jobs,
    pub has_cookie: bool,
    pub has_ladder: bool,
    pub started: Instant,
}

impl State {
    pub fn new(ctx: &egui::Context, saved: Saved) -> Self {
        let jobs = Jobs::new(ctx.clone());
        jobs.spawn("game data", |_| Msg::Db(ModDB::load().map(Arc::new).map_err(|error| error.to_string())));
        let has_ladder = std::fs::read_dir(ponder_net::ladder::DATA_DIR).is_ok_and(|mut entries| entries.next().is_some());
        Self {
            cx: Arc::new(context(&saved.scan)),
            saved,
            db: None,
            data: None,
            run: 0,
            ranks: Vec::new(),
            catalog: ItemCatalog::default(),
            rows: Vec::new(),
            selected: None,
            pin: None,
            reveal: false,
            log: Vec::new(),
            status: "loading game data".into(),
            toasts: Toasts::default().with_anchor(egui_notify::Anchor::BottomRight).with_margin(egui::vec2(12.0, 36.0)),
            quick: HashSet::new(),
            solved: (0, 0),
            solve_started: Instant::now(),
            jobs,
            has_cookie: ponder_net::trade::cookie().is_some(),
            has_ladder,
            started: Instant::now(),
        }
    }

    pub fn note(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.status.clone_from(&message);
        self.log.push((Instant::now(), message));
    }

    /// A note worth a toast.
    pub fn announce(&mut self, message: impl Into<String>, failed: bool) {
        let message = message.into();
        let toast = if failed { self.toasts.error(message.clone()) } else { self.toasts.success(message.clone()) };
        toast.duration(Duration::from_secs(if failed { 8 } else { 4 }));
        self.note(message);
    }

    pub fn solving(&self) -> bool {
        self.solved.0 < self.solved.1
    }
    pub fn divine(&self) -> f64 {
        self.data.as_ref().map_or(0.0, |data| data.report.divine)
    }
    pub fn estimates(&self) -> &[Estimate] {
        self.data.as_ref().map_or(&[], |data| data.report.estimates.as_slice())
    }
    pub fn selected_estimate(&self) -> Option<&Estimate> {
        self.estimates().get(self.selected?)
    }
    pub fn select(&mut self, index: usize) {
        self.selected = Some(index);
    }

    /// An id for a craft that outlasts scans: its title, then its key.
    pub fn id(estimate: &Estimate) -> String {
        format!("{}\u{1f}{:?}", estimate.title, estimate.key)
    }

    pub fn find(&self, id: &str) -> Option<usize> {
        self.estimates().iter().position(|estimate| State::id(estimate) == id)
    }

    pub fn refilter(&mut self) {
        self.rows = view::visible(self.estimates(), &self.saved.filters, self.saved.sort);
        if let Some(data) = &self.data {
            self.rows.retain(|&i| self.catalog.admits(&data.report.estimates[i], &self.saved.filters.items));
        }
        if self.selected.is_none_or(|selected| !self.rows.contains(&selected)) {
            self.selected = self.rows.first().copied();
        }
    }

    pub fn step_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let current = self.selected.and_then(|index| self.rows.iter().position(|&row| row == index)).unwrap_or(0);
        let next = (current as isize + delta).clamp(0, self.rows.len() as isize - 1) as usize;
        self.select(self.rows[next]);
        self.reveal = true;
    }

    pub fn handle(&mut self, message: Msg) {
        match message {
            Msg::Status(message) => self.status = message,
            Msg::Db(Ok(db)) => {
                self.catalog = ItemCatalog::new(&db);
                self.db = Some(db);
                self.note("game data loaded");
                if self.has_ladder && self.data.is_none() {
                    self.scan();
                }
            }
            Msg::Db(Err(error)) => self.announce(format!("game data: {error}"), true),
            Msg::Ladder(Ok(loaded)) => {
                let message = format!(
                    "{} estimates from {} characters in {:.1}s",
                    loaded.report.estimates.len(),
                    loaded.report.characters,
                    loaded.took.as_secs_f64()
                );
                self.ranks = view::ranks(&loaded.report.estimates);
                self.catalog.update_counts(&loaded.report.estimates);
                self.quick.clear();
                self.solved = (0, loaded.report.estimates.iter().filter(|e| e.candidate.is_some()).count());
                self.data = Some(*loaded);
                self.refilter();
                self.announce(message, false);
                self.solve_all();
            }
            Msg::Ladder(Err(error)) => self.announce(format!("ladder run failed: {error}"), true),
            Msg::LadderFetched { characters, note } => {
                self.has_ladder |= characters > 0;
                self.note(note);
                if self.has_ladder && self.data.is_none() {
                    self.scan();
                }
            }
            Msg::Solved { run, estimate, refined } => {
                if run != self.run {
                    return;
                }
                if refined {
                    self.quick.remove(&estimate.key);
                    self.solved.0 += 1;
                } else {
                    self.quick.insert(estimate.key.clone());
                }
                if let Some(data) = self.data.as_mut()
                    && let Some(slot) = data.report.estimates.iter_mut().find(|e| e.key == estimate.key)
                {
                    let title = estimate.title.clone();
                    *slot = *estimate;
                    self.ranks = view::ranks(&data.report.estimates);
                    self.refilter();
                    self.status = format!("{} {title}", if refined { "solved" } else { "quick plan for" });
                }
                if refined && !self.solving() {
                    let took = self.solve_started.elapsed().as_secs_f64();
                    self.announce(format!("all {} crafts solved in {took:.0}s", self.solved.1), false);
                }
            }
            Msg::Estimate { run, index, estimate, note } => {
                if run != self.run {
                    return;
                }
                if let Some(data) = self.data.as_mut()
                    && index < data.report.estimates.len()
                {
                    data.report.estimates[index] = *estimate;
                    self.ranks = view::ranks(&data.report.estimates);
                    self.refilter();
                }
                self.announce(note, false);
            }
        }
    }

    pub fn scan(&mut self) {
        let Some(db) = self.db.clone() else { return };
        if self.jobs.is_running("ladder") {
            return;
        }
        self.run += 1;
        self.cx.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
        self.solved = (0, 0);
        let mut cx = context(&self.saved.scan);
        let (reply, run) = (self.jobs.reply(), self.run);
        cx.on_solved = Some(Arc::new(move |estimate: &Estimate, refined| {
            reply.send(Msg::Solved { run, estimate: Box::new(estimate.clone()), refined })
        }));
        self.cx = Arc::new(cx);
        let cx = self.cx.clone();
        let opts = self.saved.scan.opts();
        self.note("scanning the ladder");
        self.jobs.spawn("ladder", move |reply| {
            let clock = Instant::now();
            let say = |message: &str| reply.status(message);
            let table = match cx.prices.load() {
                Ok((table, _)) => table,
                Err(error) => return Msg::Ladder(Err(error.to_string())),
            };
            Msg::Ladder(
                pipeline::ladder(&cx, &db, None, &opts, Some(&say))
                    .map(|report| Box::new(Loaded { report, table, took: clock.elapsed() }))
                    .map_err(|error| error.to_string()),
            )
        });
    }

    /// Solve the loaded estimates, each shown as its plans arrive.
    fn solve_all(&mut self) {
        let Some(data) = &self.data else { return };
        let mut estimates = data.report.estimates.clone();
        let (cx, table) = (self.cx.clone(), data.table.clone());
        self.solve_started = Instant::now();
        self.jobs.spawn(&format!("solve run {}", self.run), move |_| {
            pipeline::solve_estimates(&cx, &mut estimates, &table, None, None);
            Msg::Status("solving finished".into())
        });
    }

    /// Solve the selected estimate with every mechanic, once: the plan is saved, and later
    /// ladder runs take it.
    pub fn deep_solve(&mut self) {
        let (Some(index), Some(data)) = (self.selected, self.data.as_ref()) else {
            return;
        };
        let mut estimate = data.report.estimates[index].clone();
        let (cx, run, table, divine) = (self.cx.clone(), self.run, data.table.clone(), data.report.divine);
        self.note(format!("trying every mechanic on {}", estimate.title));
        self.jobs.spawn("deep solve", move |_| {
            let before = estimate.craft_cost();
            pipeline::solve_one(&cx, &mut estimate, &table, Depth::Deep);
            let note = match (before, estimate.craft_cost()) {
                (Some(before), Some(after)) if after < before - 0.5 => format!(
                    "{}: every mechanic brings it from {} to {}",
                    estimate.title,
                    view::money(Some(before), divine),
                    view::money(Some(after), divine)
                ),
                (_, Some(_)) => format!("{}: no mechanic beats the ladder plan", estimate.title),
                (_, None) => format!("{}: {}", estimate.title, estimate.failure().unwrap_or("no plan")),
            };
            Msg::Estimate { run, index, estimate: Box::new(estimate), note }
        });
    }

    pub fn fetch_ladder(&mut self) {
        let cx = self.cx.clone();
        self.jobs.spawn("fetch ladder", move |reply| {
            let say = |message: &str| reply.status(message);
            match cx
                .ladder
                .snapshot(None)
                .and_then(|snapshot| cx.ladder.sample(&snapshot, &ponder_net::ladder::Sample::default(), Some(&say)))
            {
                Ok(characters) => {
                    Msg::LadderFetched { characters: characters.len(), note: format!("{} characters saved", characters.len()) }
                }
                Err(error) => Msg::Status(format!("ladder fetch failed: {error}")),
            }
        });
    }

    pub fn fetch_prices(&mut self) {
        let cx = self.cx.clone();
        self.jobs.spawn("fetch prices", move |_| {
            Msg::Status(match cx.prices.fetch(None) {
                Ok((league, count)) => format!("fetched {count} prices for {league}"),
                Err(error) => format!("price fetch failed: {error}"),
            })
        });
    }

    pub fn price_selected(&mut self) {
        let (Some(index), Some(data)) = (self.selected, self.data.as_ref()) else {
            return;
        };
        let mut estimate = data.report.estimates[index].clone();
        let (cx, run, table, league) = (self.cx.clone(), self.run, data.table.clone(), data.report.league.clone());
        self.jobs.spawn(&format!("trade prices #{index}"), move |_| {
            pipeline::lookup_market(&cx, &mut estimate, &table, &league);
            pipeline::solve_one(&cx, &mut estimate, &table, Depth::Ladder);
            Msg::Estimate { run, index, estimate: Box::new(estimate), note: "trade prices updated".into() }
        });
    }

    pub fn export(&mut self) {
        let Some(data) = &self.data else { return };
        let result = serde_json::to_vec_pretty(&data.report)
            .map_err(|error| error.to_string())
            .and_then(|bytes| std::fs::write("ponder-report.json", bytes).map_err(|error| error.to_string()));
        match result {
            Ok(()) => self.announce("saved ponder-report.json", false),
            Err(error) => self.announce(format!("export failed: {error}"), true),
        }
    }
}

fn context(run: &ScanSettings) -> Context {
    let mut context = Context::real();
    if run.chaos_per_hour.is_finite() && run.chaos_per_hour >= 0.0 {
        context.settings.chaos_per_hour = run.chaos_per_hour;
    }
    context
}
