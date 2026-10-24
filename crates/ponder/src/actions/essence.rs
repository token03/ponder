use super::{Profiles, act, loaded, slug};
use ponder_core::{Action, ActionKind, Mod, NORMAL, RARE};
use ponder_game::data;

/// Shrieking and Deafening essences that work on this item class.
///
/// Returns the actions and the forced mod id of each (by "<name> essence", in order).
pub fn essences(item_class: &str, item_level: i64, min_level: i64) -> (Vec<Action>, Vec<(String, String)>) {
    let (mut acts, mut forced): (Vec<Action>, Vec<(String, String)>) = (Vec::new(), Vec::new());
    let classes = loaded(data::item_classes());
    let essence_class = classes.get(item_class).and_then(|c| c.category_id.as_deref()).unwrap_or(item_class);
    // The raw file: the typed loader rejects the null mods of corruption-only essences.
    let all = loaded(data::load("essences"));
    for essence in all.as_object().into_iter().flat_map(|o| o.values()) {
        let Some(mod_id) = essence["mods"][essence_class].as_str().filter(|m| !m.is_empty()) else {
            continue;
        };
        let name = essence["name"].as_str().unwrap_or("");
        if essence["level"].as_i64().unwrap_or(0) < min_level {
            continue;
        }
        let pid = slug(name);
        let name = name.replace("Essence of ", "").to_lowercase();
        let label = format!("{name} essence");
        let start = acts.len();
        if essence["level"].as_i64().unwrap_or(0) >= 5 {
            acts.push(act(&label, ActionKind::RerollRare, &[(&pid, 1.0)], &[NORMAL, RARE]).with_forced(mod_id));
        } else {
            acts.push(act(&label, ActionKind::RerollRare, &[(&pid, 1.0)], &[NORMAL]).with_forced(mod_id));
            acts.push(
                act(&format!("scour+{name} essence"), ActionKind::RerollRare, &[("scour", 1.0), (&pid, 1.0)], &[RARE]).with_forced(mod_id),
            );
        }
        if let Some(cap) = essence["item_level_restriction"].as_i64().filter(|&cap| cap > 0 && cap < item_level) {
            for a in &mut acts[start..] {
                a.profile = format!("essence-ilvl:{cap}");
            }
        }
        match forced.iter_mut().find(|(k, _)| *k == label) {
            Some(e) => e.1 = mod_id.to_string(), // a dict: the later essence of that name wins its place
            None => forced.push((label, mod_id.to_string())),
        }
    }
    (acts, forced)
}

pub fn essence_profiles(pool: &[Mod], actions: &[Action]) -> Profiles {
    let mods = loaded(data::mods());
    actions
        .iter()
        .filter_map(|a| {
            let cap = a.profile.strip_prefix("essence-ilvl:")?.parse::<i64>().ok()?;
            let weights = pool.iter().filter(|m| mods[&m.id].required_level <= cap).map(|m| (m.id.clone(), m.weight)).collect();
            Some((a.profile.clone(), weights))
        })
        .collect()
}
