use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use crate::plan::Summary;
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};

pub const PLAN_VERSION: u32 = 28;

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
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { dir: Some(path.into()) }
    }
    pub fn disabled() -> Self {
        Self { dir: None }
    }

    pub fn get_or_solve(&self, key: &str, solve: impl FnOnce() -> Result<Summary, String>) -> Result<Arc<Summary>, String> {
        let path = self.dir.as_ref().map(|dir| dir.join(format!("{key}.summary")));
        if let Some(path) = &path {
            let cached = || -> Option<Result<Summary, String>> {
                let file = std::fs::File::open(path).ok()?;
                let mut raw = Vec::new();
                ZlibDecoder::new(file).take(16_000_000).read_to_end(&mut raw).ok()?;
                let (version, plan): (u32, Result<Summary, String>) = postcard::from_bytes(&raw).ok()?;
                (version == PLAN_VERSION).then_some(plan)
            };
            if let Some(plan) = cached() {
                return plan.map(Arc::new);
            }
        }
        let plan = solve();
        if let Some(path) = &path {
            let save = || -> Option<()> {
                std::fs::create_dir_all(path.parent()?).ok()?;
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
                encoder.write_all(&postcard::to_stdvec(&(PLAN_VERSION, &plan)).ok()?).ok()?;
                let bytes = encoder.finish().ok()?;
                let tmp = path.with_extension(format!("{}.{:?}.tmp", std::process::id(), std::thread::current().id()));
                std::fs::write(&tmp, bytes).ok()?;
                if std::fs::rename(&tmp, path).is_err() {
                    let _ = std::fs::remove_file(tmp);
                }
                Some(())
            };
            let _ = save();
        }
        plan.map(Arc::new)
    }
}
