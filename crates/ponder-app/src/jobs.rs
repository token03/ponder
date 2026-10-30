//! Work off the UI thread: the ladder pipeline and trade lookups. Each job
//! runs on its own thread and reports back through one channel; the UI drains it every frame.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ponder::gear::Prices;
use ponder::pipeline::{Estimate, LadderReport};
use ponder_game::mods::ModDB;

/// A finished ladder run, with what later jobs on it need.
pub struct Loaded {
    pub report: LadderReport,
    pub table: Prices,
    pub took: Duration,
}

pub enum Msg {
    Status(String),
    Db(Result<Arc<ModDB>, String>),
    Ladder(Result<Box<Loaded>, String>),
    LadderFetched {
        characters: usize,
        note: String,
    },
    /// a job updated one estimate of run `run`
    Estimate {
        run: u64,
        index: usize,
        estimate: Box<Estimate>,
        note: String,
    },
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
    names: Arc<Mutex<Vec<(String, Instant)>>>,
}

impl Jobs {
    pub fn new(ctx: egui::Context) -> Jobs {
        let (tx, rx) = channel();
        Jobs { reply: Reply { tx, ctx }, rx, names: Arc::default() }
    }

    /// Run `work` on a new thread; whatever it returns is sent to the UI.
    pub fn spawn(&self, name: &str, work: impl FnOnce(&Reply) -> Msg + Send + 'static) {
        let reply = self.reply.clone();
        let names = self.names.clone();
        let name = name.to_string();
        {
            let mut pending = names.lock().expect("job list");
            if pending.iter().any(|(pending, _)| pending == &name) {
                return;
            }
            pending.push((name.clone(), Instant::now()));
        }
        std::thread::Builder::new()
            .name(name.clone())
            .spawn(move || {
                let msg = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&reply))).unwrap_or_else(|error| {
                    let reason = error.downcast_ref::<String>().map(String::as_str).or_else(|| error.downcast_ref::<&str>().copied());
                    Msg::Status(format!("{name} failed: {}", reason.unwrap_or("worker panicked")))
                });
                names.lock().expect("job list").retain(|(n, _)| *n != name);
                reply.send(msg);
            })
            .expect("spawn a job thread");
    }

    pub fn drain(&self) -> Vec<Msg> {
        self.rx.try_iter().collect()
    }

    pub fn busy(&self) -> bool {
        !self.names.lock().expect("job list").is_empty()
    }

    pub fn is_running(&self, name: &str) -> bool {
        self.names.lock().expect("job list").iter().any(|(n, _)| n == name)
    }

    /// What is running, and for how long.
    pub fn running(&self) -> Vec<(String, Duration)> {
        self.names.lock().expect("job list").iter().map(|(n, t)| (n.clone(), t.elapsed())).collect()
    }
}
