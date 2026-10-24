//! Match rates on real PoB exports.

mod common;

use common::{db, skip_without_data};
use ponder_game::item::parse_item;
use ponder_game::pob;
use std::collections::HashSet;

fn real(name: &str) -> pob::Build {
    let path = common::fixtures().join("real").join(name);
    pob::load_with(path.to_str().unwrap(), |_| Err("no network".into())).unwrap()
}

#[test]
fn rare_mod_lines_match() {
    if skip_without_data() {
        return;
    }
    for name in ["occultist_winter_orb.txt", "marauder_rf_leveling.txt"] {
        let build = real(name);
        let (mut matched, mut missed) = (0usize, 0usize);
        for pob_item in &build.items {
            let item = parse_item(&pob_item.text).unwrap();
            if item.rarity != "RARE" {
                continue;
            }
            let m = db().match_item(&item);
            assert!(m.base_id.is_some(), "{}", item.base);
            matched += m
                .mods
                .iter()
                .flat_map(|x| x.line_ids.iter())
                .collect::<HashSet<_>>()
                .len();
            missed += m.unmatched.len(); // author notes don't count against us
        }
        let rate = matched as f64 / (matched + missed) as f64;
        eprintln!("{name}: {matched} matched, {missed} missed ({rate:.3})");
        assert!(rate >= 0.95, "{name}: {rate}");
    }
}

#[test]
fn item_sets_are_recorded() {
    let build = real("occultist_winter_orb.txt");
    assert_eq!(
        (build.ascendancy.as_str(), build.main_skill.as_deref()),
        ("Occultist", Some("Winter Orb"))
    );
    let sets: HashSet<&str> = build
        .items
        .iter()
        .flat_map(|i| i.sets.iter().map(String::as_str))
        .collect();
    for want in ["Early game", "Mid game", "Endgame Optional"] {
        assert!(sets.contains(want), "{want} in {sets:?}");
    }
}
