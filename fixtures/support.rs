#![allow(dead_code)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::de::DeserializeOwned;

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

pub fn fixtures() -> PathBuf {
    repo_root().join("fixtures")
}

pub fn fixture<T: DeserializeOwned>(name: &str) -> T {
    let path = fixtures().join(name);
    let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let reader: Box<dyn Read> = if name.ends_with(".gz") { Box::new(flate2::read::GzDecoder::new(file)) } else { Box::new(file) };
    serde_json::from_reader(reader).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

pub fn temp_dir(tag: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("ponder-{tag}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir(&dir).unwrap();
    dir
}

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        Self(temp_dir(tag))
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
