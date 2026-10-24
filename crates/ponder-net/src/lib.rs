//! poe.ninja ladder and prices, the trade site, repoe downloads, and their disk caches.
//!
//! This is the Rust port of ponder's `ladder`, `trade`, `prices` and `data.fetch`
//! (plus the share-site part of `pob`). It shares `.cache/` with the Python:
//! file names, JSON layout and cache keys are byte-compatible.
//!
//! All network access goes through `http::Transport`; time and sleeping through
//! `env::Clock`. Both are replaceable, which is how the tests run offline.

pub mod env;
pub mod http;
pub mod ladder;
pub mod prices;
pub mod pyjson;
pub mod repoe;
pub mod share;
pub mod trade;

/// Test helpers (public so other crates' tests can use them too).
pub mod testing {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A scratch directory under the system temp dir, removed on drop.
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(tag: &str) -> TempDir {
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!("ponder-net-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
