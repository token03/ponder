use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::plan::Summary;
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use ponder_plan::session::{Policies, Saved};

/// Bump when summaries change shape: they are summarized again from the saved policies.
pub const PLAN_VERSION: u32 = 59;
/// Bump when solving changes (rules, models, the solver): every craft is solved again.
pub const POLICY_VERSION: u32 = 1;

/// Saved work: the policies solving found, which are dear to make again, and the summaries
/// shown from them, which are quick to make again from the policies.
#[derive(Clone, Debug)]
pub struct Cache {
    summaries: Folder,
    policies: Folder,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(".cache/plans")
    }
}

impl Cache {
    /// Summaries and policies under `root`, each in a folder for its version; other versions'
    /// folders are deleted, as nothing can read them.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let summaries = root.join(format!("v{PLAN_VERSION}"));
        let policies = root.join(format!("policies-v{POLICY_VERSION}"));
        for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let path = entry.path();
            if path == summaries || path == policies {
                continue;
            }
            let removed = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
            if let Err(error) = removed {
                eprintln!("could not delete the old plan {}: {error}", path.display());
            }
        }
        Self { summaries: Folder(Some(summaries)), policies: Folder(Some(policies)) }
    }

    pub fn disabled() -> Self {
        Self { summaries: Folder(None), policies: Folder(None) }
    }

    /// The plan saved under `key`, if any.
    pub fn get(&self, key: &str) -> Option<Result<Arc<Summary>, String>> {
        let (version, plan): (u32, Result<Summary, String>) =
            postcard::from_bytes(&self.summaries.read(&format!("{key}.summary"))?).ok()?;
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
        if let Ok(bytes) = postcard::to_stdvec(&(PLAN_VERSION, plan)) {
            self.summaries.write(&format!("{key}.summary"), &bytes);
        }
    }

    /// The policies solved in `context` (game data, prices, assumptions): one store per market.
    pub fn policies(&self, context: &[u8]) -> PolicyFiles<'_> {
        PolicyFiles { folder: &self.policies, context: blake3::hash(context) }
    }
}

/// Saved policies of one solving context, by request.
pub struct PolicyFiles<'a> {
    folder: &'a Folder,
    context: blake3::Hash,
}

impl PolicyFiles<'_> {
    fn name(&self, key: &[u8]) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.context.as_bytes());
        hasher.update(key);
        format!("{}.policy", hasher.finalize().to_hex())
    }
}

impl Policies for PolicyFiles<'_> {
    fn get(&self, key: &[u8]) -> Option<Saved> {
        postcard::from_bytes(&self.folder.read(&self.name(key))?).ok()
    }

    fn put(&self, key: &[u8], saved: &Saved) {
        if let Ok(bytes) = postcard::to_stdvec(saved) {
            self.folder.write(&self.name(key), &bytes);
        }
    }
}

/// A folder of compressed files, or none when caching is off.
#[derive(Clone, Debug)]
struct Folder(Option<PathBuf>);

/// Largest file read back: a policy of a few hundred thousand states.
const MAX_BYTES: u64 = 1 << 30;

impl Folder {
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        let file = std::fs::File::open(self.0.as_ref()?.join(name)).ok()?;
        let mut raw = Vec::new();
        ZlibDecoder::new(file).take(MAX_BYTES).read_to_end(&mut raw).ok()?;
        Some(raw)
    }

    /// Write `bytes` as `name`, whole or not at all; a folder that can't be written is only slower.
    fn write(&self, name: &str, bytes: &[u8]) {
        let Some(dir) = &self.0 else { return };
        let save = |path: &Path| -> Option<()> {
            std::fs::create_dir_all(dir).ok()?;
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
            encoder.write_all(bytes).ok()?;
            let compressed = encoder.finish().ok()?;
            let tmp = path.with_extension(format!("{}.{:?}.tmp", std::process::id(), std::thread::current().id()));
            std::fs::write(&tmp, compressed).ok()?;
            if std::fs::rename(&tmp, path).is_err() {
                let _ = std::fs::remove_file(tmp);
            }
            Some(())
        };
        let _ = save(&dir.join(name));
    }
}
