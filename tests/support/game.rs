use std::sync::OnceLock;

use ponder_game::{data, mods::ModDB};

pub fn setup() {
    static SETUP: OnceLock<()> = OnceLock::new();
    SETUP.get_or_init(|| data::set_data_dir(super::fixtures().join("repoe")).expect("test data configured before loading"));
}

pub fn db() -> &'static ModDB {
    static DB: OnceLock<ModDB> = OnceLock::new();
    setup();
    DB.get_or_init(|| ModDB::load().expect("fixed game data"))
}
