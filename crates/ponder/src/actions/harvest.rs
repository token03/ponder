use super::{Profiles, act};
use ponder_core::{Action, ActionKind, Mod, RARE};
use std::collections::BTreeMap;

/// Harvest "Reforge a Rare item with random modifiers, including a <tag> modifier":
/// (harvest tag, mod implicit tag, lifeforce price id, amount), from the Horticrafting
/// Station list on poedb.tw/us/Horticrafting.
pub const HARVEST: [(&str, &str, &str, f64); 15] = [
    ("fire", "fire", "wild-lifeforce", 50.0),
    ("cold", "cold", "vivid-lifeforce", 50.0),
    ("lightning", "lightning", "primal-lifeforce", 50.0),
    ("physical", "physical", "vivid-lifeforce", 50.0),
    ("life", "life", "wild-lifeforce", 75.0),
    ("defence", "defences", "primal-lifeforce", 75.0),
    ("chaos", "chaos", "vivid-lifeforce", 100.0),
    ("attack", "attack", "wild-lifeforce", 75.0),
    ("caster", "caster", "primal-lifeforce", 75.0),
    ("speed", "speed", "vivid-lifeforce", 150.0),
    ("critical", "critical", "primal-lifeforce", 150.0),
    ("minion", "minion", "primal-lifeforce", 200.0),
    ("elemental", "elemental", "wild-lifeforce", 200.0),
    ("attribute", "attribute", "vivid-lifeforce", 200.0),
    ("mana", "mana", "primal-lifeforce", 200.0),
];
/// Harvest "Add a new <tag> modifier and remove another random modifier from a non-Influenced
/// item": (harvest tag, lifeforce price id, amount), each also taking one Sacred Lifeforce.
pub const HARVEST_ADD_REMOVE: [(&str, &str, f64); 11] = [
    ("fire", "wild-lifeforce", 15000.0),
    ("cold", "vivid-lifeforce", 15000.0),
    ("lightning", "primal-lifeforce", 15000.0),
    ("physical", "vivid-lifeforce", 15000.0),
    ("life", "wild-lifeforce", 17500.0),
    ("defence", "primal-lifeforce", 17500.0),
    ("chaos", "vivid-lifeforce", 17500.0),
    ("attack", "wild-lifeforce", 17500.0),
    ("caster", "primal-lifeforce", 17500.0),
    ("speed", "vivid-lifeforce", 20000.0),
    ("critical", "primal-lifeforce", 20000.0),
];
pub const SACRED_LIFEFORCE: &str = "sacred-lifeforce";
/// Harvest reforges whose tag at least one mod in the pool carries, and their tag profiles.
pub fn harvest(pool: &[Mod]) -> (Vec<Action>, Profiles) {
    let (mut acts, mut profiles) = (Vec::new(), Profiles::new());
    for (name, tag, lifeforce, amount) in HARVEST {
        let mut tagged = BTreeMap::new();
        for m in pool.iter().filter(|m| m.tags.iter().any(|t| t == tag)) {
            tagged.insert(m.id.clone(), m.weight);
        }
        if tagged.is_empty() {
            continue;
        }
        let profile = format!("harvest:{name}");
        profiles.insert(profile.clone(), tagged.into());
        let mut action = act(&format!("harvest {name}"), ActionKind::Reforge, &[(lifeforce, amount)], &[RARE]).with_first_profile(&profile);
        let rancour = match name {
            "minion" => 3.0,
            "elemental" => 1.0,
            "attribute" | "mana" => 2.0,
            _ => 0.0,
        };
        if rancour > 0.0 {
            action.cost.push(("sandswept-lifeforce".into(), rancour));
        }
        acts.push(action);
    }
    (acts, profiles)
}

/// Harvest add/remove crafts for the tags `harvest` found mods for (`profiles`); none on an
/// influenced item.
pub fn harvest_add_remove(profiles: &Profiles, influenced: bool) -> Vec<Action> {
    if influenced {
        return Vec::new();
    }
    HARVEST_ADD_REMOVE
        .iter()
        .filter(|(name, ..)| profiles.contains_key(&format!("harvest:{name}")))
        .map(|&(name, lifeforce, amount)| {
            act(&format!("harvest add/remove {name}"), ActionKind::Augment, &[(lifeforce, amount), (SACRED_LIFEFORCE, 1.0)], &[RARE])
                .with_profile(&format!("harvest:{name}"))
        })
        .collect()
}
