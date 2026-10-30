use std::sync::Arc;
use std::time::Instant;

use ponder::pipeline::{self, Context, Estimate, LadderOpts};
use ponder_game::mods::ModDB;
use serde::{Deserialize, Serialize};

use crate::filter::ItemCatalog;
use crate::jobs::{Jobs, Loaded, Msg};
use crate::view::{self, Filters, Sort};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunSettings {
    pub gear_top: usize,
    pub per_group: usize,
    pub niche: usize,
    pub cluster_top: usize,
    pub min_characters: usize,
    pub group: String,
    pub trade: bool,
    pub trade_budget: usize,
    pub chaos_per_hour: f64,
}

impl Default for RunSettings {
    fn default() -> Self {
        let opts = LadderOpts::default();
        Self {
            gear_top: opts.gear_top,
            per_group: opts.per_group,
            niche: opts.niche,
            cluster_top: opts.cluster_top,
            min_characters: opts.min_characters,
            group: String::new(),
            trade: false,
            trade_budget: opts.trade_budget,
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
            group: Some(self.group.trim().into()).filter(|group: &String| !group.is_empty()),
            trade_lookup: self.trade,
            trade_budget: self.trade_budget,
            workers: None,
        }
    }
}

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
    pub run: u64,
    pub ranks: Vec<usize>,
    pub catalog: ItemCatalog,
    pub rows: Vec<usize>,
    pub selected: Option<usize>,
    pub reveal: bool,
    pub log: Vec<(Instant, String)>,
    pub status: String,
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
            cx: Arc::new(context(&saved.run)),
            saved,
            db: None,
            data: None,
            run: 0,
            ranks: Vec::new(),
            catalog: ItemCatalog::default(),
            rows: Vec::new(),
            selected: None,
            reveal: false,
            log: Vec::new(),
            status: "loading game data".into(),
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
                if self.saved.auto_run && self.has_ladder && self.data.is_none() {
                    self.run_ladder();
                }
            }
            Msg::Db(Err(error)) => self.note(format!("game data: {error}")),
            Msg::Ladder(Ok(loaded)) => {
                let message = format!(
                    "{} estimates from {} characters in {:.1}s",
                    loaded.report.estimates.len(),
                    loaded.report.characters,
                    loaded.took.as_secs_f64()
                );
                self.ranks = view::ranks(&loaded.report.estimates);
                self.catalog.update_counts(&loaded.report.estimates);
                self.data = Some(*loaded);
                self.refilter();
                self.note(message);
            }
            Msg::Ladder(Err(error)) => self.note(format!("ladder run failed: {error}")),
            Msg::LadderFetched { characters, note } => {
                self.has_ladder |= characters > 0;
                self.note(note);
                if self.saved.auto_run && self.has_ladder && self.data.is_none() {
                    self.run_ladder();
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
                self.note(note);
            }
        }
    }

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
            pipeline::solve_one(&cx, &mut estimate, &table);
            Msg::Estimate { run, index, estimate: Box::new(estimate), note: "trade prices updated".into() }
        });
    }

    pub fn export(&mut self) {
        let Some(data) = &self.data else { return };
        let result = serde_json::to_vec_pretty(&data.report)
            .map_err(|error| error.to_string())
            .and_then(|bytes| std::fs::write("ponder-report.json", bytes).map_err(|error| error.to_string()));
        self.note(match result {
            Ok(()) => "saved ponder-report.json".into(),
            Err(error) => format!("export failed: {error}"),
        });
    }
}

fn context(run: &RunSettings) -> Context {
    let mut context = Context::real();
    if run.chaos_per_hour.is_finite() && run.chaos_per_hour >= 0.0 {
        context.settings.chaos_per_hour = run.chaos_per_hour;
    }
    context
}
