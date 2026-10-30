use rustc_hash::FxHashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use ponder_game::{data, mods::ModDB};

#[derive(Clone)]
pub(crate) struct DatabaseKey {
    mods: Arc<data::Mods>,
    bases: Arc<data::BaseItems>,
}

impl DatabaseKey {
    pub fn new(db: &ModDB) -> Self {
        Self { mods: db.mods.clone(), bases: db.bases.clone() }
    }
}

impl PartialEq for DatabaseKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.mods, &other.mods) && Arc::ptr_eq(&self.bases, &other.bases)
    }
}

impl Eq for DatabaseKey {}

impl Hash for DatabaseKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::ptr::hash(Arc::as_ptr(&self.mods), state);
        std::ptr::hash(Arc::as_ptr(&self.bases), state);
    }
}

pub(crate) struct Memo<K, V> {
    entries: OnceLock<Mutex<FxHashMap<K, V>>>,
}

impl<K: Eq + Hash, V: Clone> Memo<K, V> {
    pub const fn new() -> Self {
        Self { entries: OnceLock::new() }
    }

    pub fn get_or_init(&self, key: K, make: impl FnOnce() -> V) -> V {
        let entries = self.entries.get_or_init(Default::default);
        if let Some(value) = entries.lock().unwrap().get(&key) {
            return value.clone();
        }
        let value = make();
        entries.lock().unwrap().entry(key).or_insert(value).clone()
    }
}
