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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Request, Response};
    use crate::testing::TempDir;

    #[test]
    fn fetch_writes_every_file_and_the_version() {
        let dir = TempDir::new("repoe");
        let http = |req: &Request| Ok(Response::new(200, if req.url.ends_with("exported-version.txt") { " 3.99.1\n".to_string() } else { format!("{{\"from\": \"{}\"}}", req.url) }));
        assert!(!dir.path().join("mods.json").exists());
        assert_eq!(fetch(&http, dir.path(), "https://example.test", &["mods", "base_items"]).unwrap(), "3.99.1");
        assert!(dir.path().join("mods.json").is_file());
        assert_eq!(std::fs::read_to_string(dir.path().join("version.txt")).unwrap(), "3.99.1\n");
        assert!(std::fs::read_to_string(dir.path().join("base_items.json")).unwrap().contains("/data/base_items.json"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
    }
}
