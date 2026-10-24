//! Settings (`.env`) and the clock: the two bits of ambient state the Python reads freely.

use std::path::Path;
use std::sync::Arc;

/// `data.env(name)`: a setting from the environment, else from `.env` in the working directory.
pub fn env(name: &str) -> Option<String> {
    env_in(name, &|n| std::env::var(n).ok(), Path::new(".env"))
}

/// `env` with the environment lookup and the `.env` path given (so tests need neither).
pub fn env_in(name: &str, lookup: &dyn Fn(&str) -> Option<String>, dotenv: &Path) -> Option<String> {
    if let Some(v) = lookup(name).filter(|v| !v.is_empty()) {
        return Some(v);
    }
    let text = std::fs::read_to_string(dotenv).ok()?;
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (key, value) = line.split_once('=').unwrap_or((line, ""));
        let value = value.trim();
        if key.trim() == name && !value.is_empty() {
            return Some(value.trim_matches(|c| c == '\'' || c == '"').to_string());
        }
    }
    None
}

/// Time and sleeping, replaceable so tests run instantly and see fixed times.
#[derive(Clone)]
pub struct Clock {
    now: Arc<dyn Fn() -> f64 + Send + Sync>,
    sleep: Arc<dyn Fn(f64) + Send + Sync>,
}

impl Clock {
    pub fn real() -> Clock {
        Clock {
            now: Arc::new(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)),
            sleep: Arc::new(|s| {
                if s > 0.0 {
                    std::thread::sleep(std::time::Duration::from_secs_f64(s))
                }
            }),
        }
    }

    pub fn new(now: impl Fn() -> f64 + Send + Sync + 'static, sleep: impl Fn(f64) + Send + Sync + 'static) -> Clock {
        Clock { now: Arc::new(now), sleep: Arc::new(sleep) }
    }

    /// `time.time()`
    pub fn now(&self) -> f64 {
        (self.now)()
    }

    /// `time.sleep(seconds)`
    pub fn sleep(&self, seconds: f64) {
        (self.sleep)(seconds)
    }
}

impl Default for Clock {
    fn default() -> Self {
        Clock::real()
    }
}
