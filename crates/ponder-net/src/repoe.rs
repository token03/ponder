//! Download game data exported by repoe-fork.
//!
//! repoe-fork publishes its JSON exports to the repoe-fork.github.io repository;
//! we read them through raw.githubusercontent.com and cache them locally.
//! (Loading them is ponder-game's job; `FILES` and `SOURCE` are repeated here so
//! this crate stands alone.)

use crate::http::{self, Transport};
use anyhow::Result;
use std::path::{Path, PathBuf};

pub const SOURCE: &str = "https://raw.githubusercontent.com/repoe-fork/repoe-fork.github.io/master";
pub const FILES: &[&str] = &[
    "mods",
    "base_items",
    "item_classes",
    "tags",
    "essences",
    "fossils",
    "crafting_bench_options",
    "cluster_jewels",
    "cluster_jewel_notables",
    "stat_translations",
];

/// `$PONDER_DATA`, else `.cache/repoe` under the working directory.
pub fn data_dir() -> PathBuf {
    match std::env::var_os("PONDER_DATA") {
        Some(d) if !d.is_empty() => d.into(),
        _ => std::env::current_dir().unwrap_or_default().join(".cache").join("repoe"),
    }
}

/// Download the data files into `dest` and return the game version they were exported from.
pub fn fetch(http: &dyn Transport, dest: &Path) -> Result<String> {
    std::fs::create_dir_all(dest)?;
    let get = |url: String| http::get(http, &url, "ponder", 120);
    let version = String::from_utf8(get(format!("{SOURCE}/exported-version.txt"))?)?.trim().to_string();
    for name in FILES {
        std::fs::write(dest.join(format!("{name}.json")), get(format!("{SOURCE}/data/{name}.json"))?)?;
    }
    std::fs::write(dest.join("version.txt"), format!("{version}\n"))?;
    Ok(version)
}

/// Whether the files the pipeline can't do without are in `dest`.
pub fn available(dest: &Path) -> bool {
    ["mods", "base_items"].iter().all(|n| dest.join(format!("{n}.json")).is_file())
}

/// The game version `fetch` saved, if any.
pub fn version(dest: &Path) -> Option<String> {
    Some(std::fs::read_to_string(dest.join("version.txt")).ok()?.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Request, Response};
    use crate::testing::TempDir;

    #[test]
    fn fetch_writes_every_file_and_the_version() {
        let dir = TempDir::new("repoe");
        let http = |req: &Request| Ok(Response::new(200, if req.url.ends_with("exported-version.txt") { " 3.99.1\n".to_string() } else { format!("{{\"from\": \"{}\"}}", req.url) }));
        assert!(!available(dir.path()));
        assert_eq!(fetch(&http, dir.path()).unwrap(), "3.99.1");
        assert!(available(dir.path()));
        assert_eq!(version(dir.path()).as_deref(), Some("3.99.1"));
        assert!(std::fs::read_to_string(dir.path().join("stat_translations.json")).unwrap().contains("/data/stat_translations.json"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), FILES.len() + 1);
    }
}
