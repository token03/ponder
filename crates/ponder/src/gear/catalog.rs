use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CatalogOptions {
    Product,
    Component,
}

#[derive(Clone)]
struct NaturalCatalog {
    harvest: Vec<Action>,
    essence: Vec<Action>,
    forced: Vec<(String, String)>,
    profiles: Profiles,
}

fn natural_catalog(db: &ModDB, base_id: &str, tags: &BTreeSet<String>, item_level: i64, pool: &[Mod]) -> NaturalCatalog {
    type Key = (DatabaseKey, String, String, BTreeSet<String>, i64);
    static CACHE: Memo<Key, NaturalCatalog> = Memo::new();
    let base = &db.bases[base_id];
    let key = (DatabaseKey::new(db), base.domain.clone(), base.item_class.clone(), tags.clone(), item_level);
    CACHE.get_or_init(key, || {
        let (harvest, mut profiles) = actions::harvest(pool);
        let (essence, forced) = actions::essences(&base.item_class, item_level, 6);
        profiles.extend(actions::essence_profiles(pool, &essence));
        NaturalCatalog { harvest, essence, forced, profiles }
    })
}

pub(super) fn configure_actions(
    db: &ModDB,
    p: &mut ponder_core::ProblemSpec,
    base_id: &str,
    item_level: i64,
    pricing: Pricing,
    opts: CatalogOptions,
) {
    let item_class = db.bases[base_id].item_class.as_str();
    let tags = db.item_tags(base_id, &p.influences);
    let NaturalCatalog { harvest, essence, forced, mut profiles } = natural_catalog(db, base_id, &tags, item_level, &p.pool);
    let native_ids: BTreeSet<&str> = p.pool.iter().map(|m| m.id.as_str()).collect();
    let candidates = p.pool.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
    for target in &mut p.targets {
        if target.good.iter().all(|id| !native_ids.contains(id.as_str())) {
            specials_count(db, &p.pool, &candidates, std::slice::from_mut(target), &mut BTreeSet::new());
        }
    }
    specials_count(db, &p.pool, &forced.iter().map(|(_, id)| id.clone()).collect::<Vec<_>>(), &mut p.targets, &mut p.filler);
    if opts == CatalogOptions::Component {
        for target in &mut p.targets {
            target.good.retain(|id| native_ids.contains(id.as_str()));
        }
        p.filler.retain(|id| native_ids.contains(id.as_str()));
    }
    for target in &mut p.targets {
        target.good.retain(|id| p.bench.iter().all(|b| db.mods[id].groups.iter().all(|g| !b.groups.contains(g))));
        target.groups = target.good.iter().flat_map(|id| db.mods[id].groups.iter().cloned()).collect();
    }
    let (fossil, fossil_profiles) = fossils(&p.pool, &p.targets, &p.filler, p.filler_need, item_class, pricing, 4);
    let mut all = actions::basic();
    all.extend(harvest);
    all.extend(actions::harvest_add_remove(&profiles, !p.influences.is_empty()));
    all.extend(fossil);
    all.extend(essence);
    let metamods = actions::metamods(item_class, &p.pool);
    if opts == CatalogOptions::Product {
        all.extend(metamods);
    } else {
        for mut action in metamods {
            if action.kind == ponder_core::ActionKind::BristleMatron {
                action.kind = ponder_core::ActionKind::MatronScour;
                action.name = "wild bristle matron; scour prefixes".into();
                action.cost.push(("scour".into(), 1.0));
                action.seconds += 1.0;
                all.push(action);
                continue;
            }
            let side = match action.kind {
                ponder_core::ActionKind::MetaLockPrefixes => Side::Prefix,
                ponder_core::ActionKind::MetaLockSuffixes => Side::Suffix,
                _ => continue,
            };
            action.kind = ponder_core::ActionKind::ProtectedScour(side);
            action.name = format!("lock {}es; scour {}es", side.name(), side.other().name());
            action.cost.push(("scour".into(), 1.0));
            action.seconds += 1.0;
            all.push(action);
        }
    }
    if opts == CatalogOptions::Product && !JEWEL_CLASSES.contains(&item_class) {
        configure_unveils(db, p, item_class, &tags, item_level, pricing, &mut all);
    }
    all.extend(actions::beasts(item_class));
    all.extend(actions::eldritch(item_class, &p.influences));
    if opts == CatalogOptions::Product {
        configure_terminal_bench(db, p, item_class, pricing, &mut all);
    }
    profiles.extend(fossil_profiles);
    p.actions = all;
    p.profiles = profiles.into();
    for (_, id) in forced {
        p.extra_mods.entry(id.clone()).or_insert_with(|| to_mod(&id, &db.mods[&id], 0.0));
    }
}

fn configure_terminal_bench(db: &ModDB, p: &mut ponder_core::ProblemSpec, item_class: &str, pricing: Pricing, actions: &mut Vec<Action>) {
    if !p.bench.is_empty() || !p.reserve.is_empty() {
        return;
    }
    let crafts: BTreeMap<String, Cost> = actions::bench_crafts(item_class)
        .into_iter()
        .filter(|(id, cost)| db.mods.contains_key(id) && cost.iter().all(|(pid, _)| pricing.prices.contains_key(pid)))
        .collect();
    let ids = crafts.keys().cloned().collect::<Vec<_>>();
    let original: Vec<_> = p.targets.iter().map(|t| t.good.clone()).collect();
    specials_count(db, &p.pool, &ids, &mut p.targets, &mut BTreeSet::new());
    for (target, original) in p.targets.iter_mut().zip(original) {
        let selected = target
            .good
            .iter()
            .filter_map(|id| {
                let cost = crafts.get(id)?;
                let m = &db.mods[id];
                let mut action =
                    Action::new(&format!("bench {}", m.text_or_empty()), ponder_core::ActionKind::FinishBench, &[], &[ponder_core::RARE])
                        .with_seconds(5.0);
                action.forced = Some(id.clone());
                action.cost = cost.clone();
                Some((id.clone(), action))
            })
            .min_by(|(ia, a), (ib, b)| {
                ponder_core::step_cost(a, pricing.prices, pricing.settings)
                    .total_cmp(&ponder_core::step_cost(b, pricing.prices, pricing.settings))
                    .then_with(|| ia.cmp(ib))
            });
        target.good = original;
        if let Some((id, action)) = selected {
            target.good.insert(id.clone());
            p.extra_mods.insert(id.clone(), to_mod(&id, &db.mods[&id], 0.0));
            actions.push(action);
            p.allow_terminal_bench = true;
        }
    }
}

fn configure_unveils(
    db: &ModDB,
    p: &mut ponder_core::ProblemSpec,
    item_class: &str,
    tags: &BTreeSet<String>,
    item_level: i64,
    pricing: Pricing,
    actions: &mut Vec<Action>,
) {
    p.unveils = db
        .mods
        .iter()
        .filter(|(_, m)| {
            m.domain == "unveiled"
                && matches!(m.name.as_str(), "Chosen" | "of the Order")
                && m.required_level <= item_level
                && matches!(m.generation_type.as_str(), "prefix" | "suffix")
        })
        .filter_map(|(id, m)| {
            let weight = ModDB::generation_weight(m, tags);
            (weight > 0.0).then(|| to_mod(id, m, weight))
        })
        .collect();
    specials_count(db, &p.pool, &p.unveils.iter().map(|m| m.id.clone()).collect::<Vec<_>>(), &mut p.targets, &mut p.filler);
    for target in &mut p.targets {
        target.good.retain(|id| p.bench.iter().all(|b| db.mods[id].groups.iter().all(|g| !b.groups.contains(g))));
        target.groups = target.good.iter().flat_map(|id| db.mods[id].groups.iter().cloned()).collect();
    }
    if !p.targets.iter().any(|t| p.unveils.iter().any(|m| t.good.contains(&m.id))) {
        p.unveils.clear();
        return;
    }
    for m in &p.unveils {
        p.extra_mods.insert(m.id.clone(), m.clone());
    }
    for id in ["VeiledPrefix", "VeiledSuffix"] {
        if let Some(m) = db.mods.get(id) {
            p.extra_mods.insert(id.into(), to_mod(id, m, 0.0));
        }
    }
    actions.push(Action::new("veiled chaos", ponder_core::ActionKind::VeiledChaos, &[("veiled-chaos-orb", 1.0)], &[ponder_core::RARE]));
    actions.push(Action::new("veiled exalt", ponder_core::ActionKind::VeiledExalt, &[("veiled-exalted-orb", 1.0)], &[ponder_core::RARE]));
    configure_unveil_blockers(db, p, item_class, pricing, actions);
}

fn configure_unveil_blockers(db: &ModDB, p: &mut ponder_core::ProblemSpec, item_class: &str, pricing: Pricing, actions: &mut Vec<Action>) {
    let wanted: BTreeSet<&str> = p
        .targets
        .iter()
        .flat_map(|t| t.groups.iter().map(String::as_str))
        .chain(p.fixed.iter().chain(&p.bench).flat_map(|m| m.groups.iter().map(String::as_str)))
        .collect();
    let capacity = ponder_core::MAX_TARGETS.saturating_sub(p.targets.len() + p.bench.len());
    let mut candidates = Vec::new();
    for (id, m) in db.mods.iter() {
        if m.domain != "crafted"
            || !matches!(m.generation_type.as_str(), "prefix" | "suffix")
            || m.groups.iter().any(|g| wanted.contains(g.as_str()))
        {
            continue;
        }
        let blocked: f64 = p.unveils.iter().filter(|u| u.groups.iter().any(|g| m.groups.contains(g))).map(|u| u.weight).sum();
        if blocked <= 0.0 {
            continue;
        }
        let Some(mut cost) = actions::bench_cost(item_class, id) else {
            continue;
        };
        cost.push(("scour".into(), 1.0));
        let Some(price) = cost.iter().map(|(id, count)| pricing.prices.get(id).map(|price| price * count)).sum::<Option<f64>>() else {
            continue;
        };
        candidates.push((blocked, price, id.clone(), cost, to_mod(id, m, 0.0)));
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.total_cmp(&b.1)).then_with(|| a.2.cmp(&b.2)));
    for (_, _, _, cost, m) in candidates {
        if p.blockers.len() >= capacity {
            break;
        }
        if p.blockers.iter().any(|b| b.groups.iter().any(|g| m.groups.contains(g))) {
            continue;
        }
        p.blockers.push(m.clone());
        let mut action = Action::new(
            &format!("bench {}; reveal unveil; remove craft after choosing", m.label),
            ponder_core::ActionKind::RevealUnveil(p.blockers.len() as u8),
            &[],
            &[ponder_core::RARE],
        )
        .with_seconds(10.0);
        action.cost = cost;
        actions.push(action);
    }
}

fn specials_count(db: &ModDB, pool: &[Mod], special: &[String], targets: &mut [Target], filler: &mut BTreeSet<String>) {
    let compatible = |a: &str, b: &str| {
        let (a, b) = (&db.mods[a], &db.mods[b]);
        a.generation_type == b.generation_type && a.groups.len() == b.groups.len() && a.groups.iter().all(|g| b.groups.contains(g))
    };
    let satisfies = |id: &str, good: &BTreeSet<String>| {
        let candidate = &db.mods[id];
        good.iter().filter(|g| compatible(g, id)).any(|g| {
            let wanted = &db.mods[g];
            let mut tiers: Vec<_> = pool.iter().map(|m| &db.mods[&m.id]).filter(|m| m.type_ == wanted.type_).collect();
            tiers.sort_by_key(|m| m.required_level);
            !wanted.stats.is_empty()
                && wanted.stats.iter().all(|stat| {
                    let Some(value) = candidate.stats.iter().find(|s| s.id == stat.id) else {
                        return false;
                    };
                    let first = tiers.iter().find_map(|m| m.stats.iter().find(|s| s.id == stat.id));
                    let last = tiers.iter().rev().find_map(|m| m.stats.iter().find(|s| s.id == stat.id));
                    let decreasing = first.zip(last).map_or(value.max < 0.0 && stat.max < 0.0, |(a, b)| b.min + b.max < a.min + a.max);
                    if decreasing { value.max <= stat.max } else { value.min >= stat.min }
                })
        })
    };
    for id in special {
        for t in targets.iter_mut().filter(|t| satisfies(id, &t.good)) {
            t.good.insert(id.clone());
            t.groups.extend(db.mods[id.as_str()].groups.iter().cloned());
        }
        if satisfies(id, filler) && filler.iter().any(|existing| db.mods[existing].type_ == db.mods[id].type_) {
            filler.insert(id.clone());
        }
    }
}

pub(super) fn mods_conflict(pool: &[Mod], bench: &Mod) -> bool {
    pool.iter().any(|m| m.groups.iter().any(|g| bench.groups.contains(g)))
}

/// Fossil rerolls for these wants: combinations of up to `max_fossils` in a resonator.
fn fossils(
    mods: &[Mod],
    targets: &[Target],
    filler: &BTreeSet<String>,
    filler_need: usize,
    item_class: &str,
    pricing: Pricing,
    max_fossils: usize,
) -> (Vec<Action>, Profiles) {
    if JEWEL_CLASSES.contains(&item_class) {
        return (Vec::new(), Profiles::new()); // resonators don't go on jewels
    }
    if !FOSSIL_COMBOS {
        return actions::fossils(mods);
    }
    let mut wants: Vec<&BTreeSet<String>> = targets.iter().map(|t| &t.good).collect();
    wants.extend(std::iter::repeat_n(filler, filler_need));
    actions::fossil_combos(mods, &wants, pricing.prices, pricing.settings, actions::FossilOpts { max_fossils, ..Default::default() })
}
