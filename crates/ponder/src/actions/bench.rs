use super::{Cost, act, loaded};
use ponder_core::{Action, ActionKind, META_TAGS, Mod, RARE};
use ponder_game::data;
use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

/// Crafting bench costs are game metadata ids; these are their poe.ninja price ids.
pub const BENCH_CURRENCY: [(&str, &str); 16] = [
    ("CurrencyUpgradeToMagic", "transmute"),
    ("CurrencyRerollMagic", "alt"),
    ("CurrencyAddModToMagic", "aug"),
    ("CurrencyUpgradeMagicToRare", "regal"),
    ("CurrencyUpgradeToRare", "alch"),
    ("CurrencyRerollRare", "chaos"),
    ("CurrencyAddModToRare", "exalted"),
    ("CurrencyUpgradeRandomly", "chance"),
    ("CurrencyModValues", "divine"),
    ("CurrencyCorrupt", "vaal"),
    ("CurrencyConvertToNormal", "scour"),
    ("CurrencyInstillingOrb", "instilling-orb"),
    ("CurrencyRerollSocketNumbers", "jewellers"),
    ("CurrencyRerollSocketLinks", "fusing"),
    ("CurrencyRerollSocketColours", "chrome"),
    ("CurrencyArmourQuality", "scrap"),
];
pub const LOCK_PREFIXES: &str = "StrMasterItemGenerationCannotChangePrefixes"; // a suffix
pub const LOCK_SUFFIXES: &str = "DexMasterItemGenerationCannotChangeSuffixes"; // a prefix
pub const NO_ATTACK: &str = "IntMasterItemGenerationCannotRollAttackAffixes"; // a suffix
pub const NO_CASTER: &str = "StrDexMasterItemGenerationCannotRollCasterAffixes"; // a suffix
pub const MULTIMOD: &str = "StrIntMasterItemGenerationCanHaveMultipleCraftedMods"; // a suffix

type BenchIndex = HashMap<String, HashMap<String, Cost>>;

/// mod id -> item class -> cost, from the first bench option (in file order) that adds the mod to the class.
fn bench_index() -> &'static BenchIndex {
    static INDEX: OnceLock<BenchIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index: BenchIndex = HashMap::new();
        for option in loaded(data::crafting_bench_options()).iter() {
            let Some(mod_id) = option.add_explicit_mod() else {
                continue;
            };
            let cost: Cost = option
                .cost
                .iter()
                .map(|(k, n)| {
                    let id = k.rsplit('/').next().unwrap_or(k);
                    (BENCH_CURRENCY.iter().find(|(c, _)| *c == id).map_or(id, |(_, p)| p).to_string(), *n)
                })
                .collect();
            let by_class = index.entry(mod_id.to_string()).or_default();
            for class in &option.item_classes {
                by_class.entry(class.clone()).or_insert_with(|| cost.clone());
            }
        }
        index
    })
}

/// What the crafting bench charges to add `mod_id`, or None if it can't.
pub fn bench_cost(item_class: &str, mod_id: &str) -> Option<Cost> {
    bench_index().get(mod_id)?.get(item_class).cloned()
}

/// Bench metamods (`ponder_core::META_NAMES`), taking them off ("remove crafted mods": a
/// scour), and scouring a locked item. "Cannot roll Attack/Caster" only where the pool has such
/// mods. None where the bench can't craft them (jewels).
pub fn metamods(item_class: &str, pool: &[Mod]) -> Vec<Action> {
    let mut acts = Vec::new();
    let has = |tag: &str| pool.iter().any(|m| m.tags.iter().any(|t| t == tag));
    for (mod_id, kind, name, wanted) in [
        (LOCK_PREFIXES, ActionKind::MetaLockPrefixes, "lock prefixes", true),
        (LOCK_SUFFIXES, ActionKind::MetaLockSuffixes, "lock suffixes", true),
        (NO_ATTACK, ActionKind::MetaNoAttack, "cannot roll attack", has(META_TAGS[0])),
        (NO_CASTER, ActionKind::MetaNoCaster, "cannot roll caster", has(META_TAGS[1])),
    ] {
        if let (true, Some(cost)) = (wanted, bench_cost(item_class, mod_id)) {
            let mut a = act(name, kind, &[], &[RARE]);
            a.cost = cost;
            acts.push(a);
        }
    }
    if !acts.is_empty() {
        acts.push(act("remove crafted mods", ActionKind::RemoveCrafted, &[("scour", 1.0)], &[RARE]));
        acts.push(act("scour (locked)", ActionKind::Scour, &[("scour", 1.0)], &[RARE]));
    }
    acts
}

pub const ELDRITCH_CLASSES: [&str; 4] = ["Helmet", "Body Armour", "Gloves", "Boots"];

/// Eldritch Exalted Orb / Orb of Annulment: add or remove a mod on one side only.
///
/// They need eldritch implicits (Exarch/Eater, from embers and ichors) whose
/// dominance picks the side, and don't work on influenced items. Setting up
/// the implicits is cheap next to the orbs and isn't counted.
pub fn eldritch(item_class: &str, influences: &BTreeSet<String>) -> Vec<Action> {
    if !ELDRITCH_CLASSES.contains(&item_class) || !influences.is_empty() {
        return Vec::new();
    }
    vec![
        act("eldritch exalt (prefix)", ActionKind::ExaltPrefix, &[("eldritch-exalted-orb", 1.0)], &[RARE]),
        act("eldritch exalt (suffix)", ActionKind::ExaltSuffix, &[("eldritch-exalted-orb", 1.0)], &[RARE]),
        act("eldritch annul (prefix)", ActionKind::AnnulPrefix, &[("eldritch-orb-of-annulment", 1.0)], &[RARE]),
        act("eldritch annul (suffix)", ActionKind::AnnulSuffix, &[("eldritch-orb-of-annulment", 1.0)], &[RARE]),
    ]
}
