//! Fingerprints that hash each shared modifier pool and weight-profile table once.
//!
//! Many problems (every donor of a product, every fracture variant) share one `Arc`'d pool
//! and profile table. Hashing a problem serialises its own fields and writes those shared parts
//! as their content hash, remembered per allocation, instead of re-serialising thousands of
//! weights for each problem.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

use rustc_hash::FxHashMap;
use serde::{Serialize, Serializer};

use crate::types::{Mod, WeightProfile};

thread_local! {
    static FINGERPRINTING: Cell<bool> = const { Cell::new(false) };
}

type Hashes<T> = Mutex<FxHashMap<usize, (Weak<T>, blake3::Hash)>>;

static POOLS: std::sync::LazyLock<Hashes<Vec<Mod>>> = std::sync::LazyLock::new(Default::default);
static PROFILES: std::sync::LazyLock<Hashes<BTreeMap<String, WeightProfile>>> = std::sync::LazyLock::new(Default::default);

/// The content hash of a shared value, computed once per live allocation.
fn content_hash<T: Serialize>(value: &Arc<T>, hashes: &Hashes<T>) -> blake3::Hash {
    let address = Arc::as_ptr(value) as usize;
    if let Some((weak, hash)) = hashes.lock().unwrap().get(&address)
        && weak.upgrade().is_some_and(|live| Arc::ptr_eq(&live, value))
    {
        return *hash;
    }
    let mut hasher = blake3::Hasher::new();
    serde_json::to_writer(&mut hasher, value.as_ref()).expect("shared problem data serialises");
    let hash = hasher.finalize();
    let mut hashes = hashes.lock().unwrap();
    if hashes.len() > 4096 {
        hashes.retain(|_, (weak, _)| weak.strong_count() > 0);
    }
    hashes.insert(address, (Arc::downgrade(value), hash));
    hash
}

/// A problem's identity: its own fields, with shared pools and profiles by content hash.
pub(super) fn fingerprint(spec: &impl Serialize) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    FINGERPRINTING.set(true);
    let written = serde_json::to_writer(&mut hasher, spec);
    FINGERPRINTING.set(false);
    written.expect("a problem serialises");
    hasher.finalize()
}

pub(super) fn pool<S: Serializer>(pool: &Arc<Vec<Mod>>, serializer: S) -> Result<S::Ok, S::Error> {
    if FINGERPRINTING.get() { content_hash(pool, &POOLS).as_bytes().serialize(serializer) } else { pool.serialize(serializer) }
}

pub(super) fn profiles<S: Serializer>(profiles: &Arc<BTreeMap<String, WeightProfile>>, serializer: S) -> Result<S::Ok, S::Error> {
    if FINGERPRINTING.get() {
        content_hash(profiles, &PROFILES).as_bytes().serialize(serializer)
    } else {
        profiles.serialize(serializer)
    }
}
