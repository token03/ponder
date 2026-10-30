//! Download game data exported by repoe-fork.
//!
//! repoe-fork publishes its JSON exports to the repoe-fork.github.io repository;
//! we read them through raw.githubusercontent.com and cache them locally.
use crate::http::{self, Transport};
use anyhow::Result;
use std::path::Path;

/// Download the data files into `dest` and return the game version they were exported from.
pub fn fetch(http: &dyn Transport, dest: &Path, source: &str, files: &[&str]) -> Result<String> {
    std::fs::create_dir_all(dest)?;
    let get = |url: String| http::get(http, &url, "ponder", 120);
    let version = String::from_utf8(get(format!("{source}/exported-version.txt"))?)?.trim().to_string();
    for name in files {
        std::fs::write(dest.join(format!("{name}.json")), get(format!("{source}/data/{name}.json"))?)?;
    }
    std::fs::write(dest.join("version.txt"), format!("{version}\n"))?;
    Ok(version)
}
