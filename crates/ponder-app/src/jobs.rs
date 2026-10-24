//! Work off the UI thread: the ladder pipeline, trade lookups, finishing an item. Each job
//! runs on its own thread and reports back through one channel; the UI drains it every frame.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ponder::pipeline::{Estimate, Finish, LadderReport};
use ponder_core::Prices;
use ponder_game::mods::{ItemMatch, ModDB};

/// A finished ladder run, with what later jobs on it need.
pub struct Loaded {
    pub report: LadderReport,
    pub table: Prices,
    pub took: Duration,
}

pub struct Finished {
    pub item: ItemMatch,
    pub group: String,
    pub found: Vec<Finish>,
}

pub enum Msg {
    Status(String),
    Db(Result<Arc<ModDB>, String>),
    Ladder(Result<Box<Loaded>, String>),
    /// a job updated one estimate of run `run`
    Estimate { run: u64, index: usize, estimate: Box<Estimate>, note: String },
    Finished(Result<Box<Finished>, String>),
}

/// Sends messages and wakes the UI so it draws them.
#[derive(Clone)]
pub struct Reply {
    tx: Sender<Msg>,
    ctx: egui::Context,
}

impl Reply {
    pub fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }

    pub fn status(&self, s: impl Into<String>) {
        self.send(Msg::Status(s.into()));
    }
}

pub struct Jobs {
    reply: Reply,
    rx: Receiver<Msg>,
    running: Arc<AtomicUsize>,
    names: Arc<Mutex<Vec<(String, Instant)>>>,
}

impl Jobs {
    pub fn new(ctx: egui::Context) -> Jobs {
        let (tx, rx) = channel();
        Jobs { reply: Reply { tx, ctx }, rx, running: Arc::default(), names: Arc::default() }
    }

    /// Run `work` on a new thread; whatever it returns is sent to the UI.
    pub fn spawn(&self, name: &str, work: impl FnOnce(&Reply) -> Msg + Send + 'static) {
        let reply = self.reply.clone();
        let running = self.running.clone();
        let names = self.names.clone();
        let name = name.to_string();
        running.fetch_add(1, Ordering::SeqCst);
        names.lock().expect("job list").push((name.clone(), Instant::now()));
        std::thread::Builder::new()
            .name(name.clone())
            .spawn(move || {
                let msg = work(&reply);
                names.lock().expect("job list").retain(|(n, _)| *n != name);
                running.fetch_sub(1, Ordering::SeqCst);
                reply.send(msg);
            })
            .expect("spawn a job thread");
    }

    pub fn drain(&self) -> Vec<Msg> {
        self.rx.try_iter().collect()
    }

    pub fn busy(&self) -> bool {
        self.running.load(Ordering::SeqCst) > 0
    }

    pub fn is_running(&self, name: &str) -> bool {
        self.names.lock().expect("job list").iter().any(|(n, _)| n == name)
    }

    /// What is running, and for how long.
    pub fn running(&self) -> Vec<(String, Duration)> {
        self.names.lock().expect("job list").iter().map(|(n, t)| (n.clone(), t.elapsed())).collect()
    }
}
