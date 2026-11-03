use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use crate::plan::Summary;
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};

pub const PLAN_VERSION: u32 = 59;

#[derive(Clone, Debug)]
pub struct Cache {
    dir: Option<PathBuf>,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(".cache/plans")
    }
}

impl Cache {
    /// Plans under `root`, in a folder for this `PLAN_VERSION`; other versions' plans are deleted,
    /// as nothing can read them.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let dir = root.join(format!("v{PLAN_VERSION}"));
        for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let path = entry.path();
            if path == dir {
                continue;
            }
            let removed = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
            if let Err(error) = removed {
                eprintln!("could not delete the old plan {}: {error}", path.display());
            }
        }
        Self { dir: Some(dir) }
    }
    pub fn disabled() -> Self {
        Self { dir: None }
    }

    fn path(&self, key: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|dir| dir.join(format!("{key}.summary")))
    }

    /// The plan saved under `key`, if any.
    pub fn get(&self, key: &str) -> Option<Result<Arc<Summary>, String>> {
        let file = std::fs::File::open(self.path(key)?).ok()?;
        let mut raw = Vec::new();
        ZlibDecoder::new(file).take(16_000_000).read_to_end(&mut raw).ok()?;
        let (version, plan): (u32, Result<Summary, String>) = postcard::from_bytes(&raw).ok()?;
        (version == PLAN_VERSION).then(|| plan.map(Arc::new))
    }

    pub fn get_or_solve(&self, key: &str, solve: impl FnOnce() -> Result<Summary, String>) -> Result<Arc<Summary>, String> {
        if let Some(plan) = self.get(key) {
            return plan;
        }
        let plan = solve();
        self.put(key, plan.as_ref().map_err(String::as_str));
        plan.map(Arc::new)
    }

    /// Save a plan under `key`; a cache that can't be written is only slower.
    pub fn put(&self, key: &str, plan: Result<&Summary, &str>) {
        let Some(path) = self.path(key) else { return };
        let save = || -> Option<()> {
            std::fs::create_dir_all(path.parent()?).ok()?;
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
            encoder.write_all(&postcard::to_stdvec(&(PLAN_VERSION, plan)).ok()?).ok()?;
            let bytes = encoder.finish().ok()?;
            let tmp = path.with_extension(format!("{}.{:?}.tmp", std::process::id(), std::thread::current().id()));
            std::fs::write(&tmp, bytes).ok()?;
            if std::fs::rename(&tmp, &path).is_err() {
                let _ = std::fs::remove_file(tmp);
            }
            Some(())
        };
        let _ = save();
    }
}
