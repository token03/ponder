use super::*;

/// Most other mods one slot can hold (a side's affix limit), and on the whole item.
pub(super) const SLOT_MAX: usize = 3;
pub(super) const ITEM_MAX: usize = 2 * SLOT_MAX;

/// One junk slot's weight under a profile.
///
/// The state counts the slot's mods but not which groups they are, so what they keep out of
/// the next roll is unknown: `blocked` is its expectation (groups drawn in proportion to weight,
/// without replacement) and `lo`/`hi` its least and most, over every set of that many groups.
#[derive(Clone, Copy, Default)]
pub(super) struct SlotW {
    pub(super) total: f64,
    /// groups that roll at this weight, and groups that may be on the item
    pub(super) rolls: usize,
    pub(super) may: usize,
    pub(super) blocked: [f64; SLOT_MAX + 1],
    pub(super) lo: [f64; ITEM_MAX + 1],
    pub(super) hi: [f64; ITEM_MAX + 1],
}

/// A profile's weights: targets by status, junk groups by key and slot, and junk by slot.
pub(super) struct Lumped {
    pub(super) tw: Vec<[f64; 2]>,
    pub(super) kw: Vec<[f64; JUNK_SLOTS]>,
    pub(super) junk: [SlotW; JUNK_SLOTS],
    /// `cross[a][b]`: the weight in slot b a mod rolled in slot a keeps out, on average (one
    /// group can have tiers in two slots: low life is filler, high life other junk)
    pub(super) cross: [[f64; JUNK_SLOTS]; JUNK_SLOTS],
}

/// A junk group: the mods sharing one group list. Rolling one keeps them all out, in every slot.
pub(super) struct JunkKey {
    pub(super) groups: Vec<String>,
    /// the slots its mods fall in
    pub(super) slots: [bool; JUNK_SLOTS],
    /// per slot and `META_TAGS`: whether some of its mods there carry the tag, and whether some don't
    pub(super) tagged: [[(bool, bool); 2]; JUNK_SLOTS],
}

pub(super) struct Compiled {
    pub(super) mods: HashMap<String, Mod>,
    pub(super) base: State,
    pub(super) t_side: Vec<Side>,
    pub(super) t_notable: Vec<bool>,
    pub(super) profile_names: Vec<String>, // "base" first
    /// every junk group that can be on the item; fractured mods' groups left out (they can't roll)
    pub(super) keys: Vec<JunkKey>,
    /// slots some junk group has mods in both of
    pub(super) linked: [[bool; JUNK_SLOTS]; JUNK_SLOTS],
    /// keys sharing a group: the per-slot bounds don't hold then, so they are left wide open
    pub(super) overlap: bool,
    /// a bought item may carry junk from outside the pool, keeping nothing out of a roll
    pub(super) phantoms: bool,
    pub(super) lumped: Vec<Lumped>,
    /// per `META_TAGS`: each profile's lumps without the mods carrying the tag ("cannot roll" metamods)
    pub(super) lumped_no: [Vec<Lumped>; 2],
    /// per `META_TAGS`: the targets whose mods carry the tag (an annul can't take them under the metamod)
    pub(super) t_tagged: [Vec<bool>; 2],
    /// per `META_TAGS` and target: whether a GOOD (then BLOCKED) mod of it may carry the tag, and may not
    pub(super) t_tag_range: [Vec<[(bool, bool); 2]>; 2],
    /// per `META_TAGS`: the share of each junk slot's base weight carrying the tag
    pub(super) junk_tagged: [[f64; JUNK_SLOTS]; 2],
}

/// Expected weight `k` groups drawn in proportion to weight (without replacement) take from `classes` (weight, count).
fn expected_blocked(classes: &mut [(f64, usize)], k: usize) -> f64 {
    if k == 0 {
        return 0.0;
    }
    let total: f64 = classes.iter().map(|&(w, n)| w * n as f64).sum();
    if total <= 0.0 {
        return 0.0;
    }
    let mut e = 0.0;
    for i in 0..classes.len() {
        let (w, n) = classes[i];
        if n == 0 {
            continue;
        }
        classes[i].1 -= 1;
        e += w * n as f64 / total * (w + expected_blocked(classes, k - 1));
        classes[i].1 += 1;
    }
    e
}

impl SlotW {
    /// `pos`: the weights the slot's groups roll at; `all`: every group that may be on the item, at those weights.
    fn new(pos: &[f64], all: &[f64], phantoms: bool) -> SlotW {
        let mut classes: Vec<(f64, usize)> = Vec::new();
        for &w in pos.iter().filter(|w| **w > 0.0) {
            match classes.iter_mut().find(|c| c.0 == w) {
                Some(c) => c.1 += 1,
                None => classes.push((w, 1)),
            }
        }
        let mut sorted: Vec<f64> = all.to_vec();
        if phantoms {
            sorted.extend([0.0; SLOT_MAX]);
        }
        sorted.sort_by(f64::total_cmp);
        let rolls = pos.iter().filter(|w| **w > 0.0).count();
        let mut out = SlotW { total: pos.iter().filter(|w| **w > 0.0).sum(), rolls, may: sorted.len(), ..SlotW::default() };
        for k in 0..=SLOT_MAX {
            out.blocked[k] = expected_blocked(&mut classes, k);
        }
        for k in 0..=ITEM_MAX {
            out.lo[k] = sorted.iter().take(k).sum();
            out.hi[k] = sorted.iter().rev().take(k).sum();
        }
        out
    }
}

impl Problem {
    pub(super) fn compile_pool(&self) -> Compiled {
        assert!(self.targets.len() + self.bench.len() <= MAX_TARGETS, "at most {MAX_TARGETS} targets and bench groups");
        let mut mods = HashMap::new();
        for (k, m) in &self.extra_mods {
            mods.insert(k.clone(), m.clone());
        }
        for m in self.pool.iter().chain(&self.fixed) {
            mods.insert(m.id.clone(), m.clone());
        }
        let mut base = State::new(NORMAL, &[], &[]);
        for m in &self.fixed {
            let (i, v) = self.classify_mod(m);
            if i >= 0 {
                base.status[i as usize] = v;
            } else {
                base.junk[v as usize] += 1;
            }
        }
        let mut profile_names = vec!["base".to_string()];
        profile_names.extend(self.profiles.keys().filter(|k| *k != "base").cloned());
        // The junk groups, in pool order then the forced mods outside it.
        let taken: BTreeSet<&str> = self.fixed.iter().flat_map(|m| m.groups.iter()).map(String::as_str).collect();
        let mut keys: Vec<JunkKey> = Vec::new();
        let mut extra: Vec<&Mod> = self.extra_mods.values().collect();
        extra.sort_by(|a, b| a.id.cmp(&b.id));
        for m in self.pool.iter().chain(extra) {
            let (i, slot) = self.classify_mod(m);
            if i >= 0 || m.groups.iter().any(|g| taken.contains(g.as_str())) {
                continue;
            }
            let k = match keys.iter().position(|k| k.groups == m.groups) {
                Some(k) => k,
                None => {
                    keys.push(JunkKey { groups: m.groups.clone(), slots: [false; JUNK_SLOTS], tagged: [[(false, false); 2]; JUNK_SLOTS] });
                    keys.len() - 1
                }
            };
            let slot = slot as usize;
            keys[k].slots[slot] = true;
            for (t, tag) in META_TAGS.iter().enumerate() {
                let has = m.tags.iter().any(|x| x == tag);
                keys[k].tagged[slot][t].0 |= has;
                keys[k].tagged[slot][t].1 |= !has;
            }
        }
        let mut seen: HashMap<&str, usize> = HashMap::new();
        let mut overlap = false;
        let mut linked = [[false; JUNK_SLOTS]; JUNK_SLOTS];
        for (k, key) in keys.iter().enumerate() {
            for g in &key.groups {
                overlap |= seen.insert(g.as_str(), k).is_some_and(|other| other != k);
            }
            for (a, row) in linked.iter_mut().enumerate() {
                for (b, cell) in row.iter_mut().enumerate() {
                    *cell |= a != b && key.slots[a] && key.slots[b];
                }
            }
        }
        let mut c = Compiled {
            mods,
            base,
            t_side: self.targets.iter().map(|t| t.side).chain(self.bench.iter().map(|m| m.side)).collect(),
            t_notable: self.targets.iter().map(|t| t.notable).chain(self.bench.iter().map(|_| false)).collect(),
            profile_names,
            keys,
            linked,
            overlap,
            phantoms: self.begin.is_some(),
            lumped: Vec::new(),
            lumped_no: [Vec::new(), Vec::new()],
            t_tagged: [Vec::new(), Vec::new()],
            t_tag_range: [Vec::new(), Vec::new()],
            junk_tagged: [[0.0; JUNK_SLOTS]; 2],
        };
        c.lumped = c.profile_names.iter().map(|name| self.lump(&c, name, &[], None)).collect();
        for (k, tag) in META_TAGS.iter().enumerate() {
            c.lumped_no[k] = c.profile_names.iter().map(|name| self.lump(&c, name, &[], Some(tag))).collect();
            let has = |id: &String| c.mods.get(id).is_some_and(|m| m.tags.iter().any(|x| x == tag));
            c.t_tagged[k] = self
                .targets
                .iter()
                .map(|t| 2 * t.good.iter().filter(|g| has(g)).count() > t.good.len())
                .chain(self.bench.iter().map(|m| m.tags.iter().any(|x| x == tag)))
                .collect();
            // What an annul may find on a target: one of its good mods, or one sharing its groups.
            c.t_tag_range[k] = self
                .targets
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let mut range = [(false, false); 2];
                    for m in c.mods.values() {
                        let v = if t.good.contains(&m.id) {
                            0
                        } else if self.classify_mod(m) == (i as i32, BLOCKED) {
                            1
                        } else {
                            continue;
                        };
                        let tagged = m.tags.iter().any(|x| x == tag);
                        range[v].0 |= tagged;
                        range[v].1 |= !tagged;
                    }
                    range
                })
                .chain(self.bench.iter().map(|m| {
                    let tagged = m.tags.iter().any(|x| x == tag);
                    [(tagged, !tagged); 2]
                }))
                .collect();
            let (all, no) = (&c.lumped[0].junk, &c.lumped_no[k][0].junk);
            for slot in 0..JUNK_SLOTS {
                c.junk_tagged[k][slot] = if all[slot].total > 0.0 { 1.0 - no[slot].total / all[slot].total } else { 0.0 };
            }
        }
        c
    }

    /// A profile's lumps as the item's metamod leaves them ("cannot roll" drops its tag).
    pub(super) fn lump_for<'c>(&self, c: &'c Compiled, profile: usize, meta: u8) -> &'c Lumped {
        match meta_tag(meta) {
            Some(t) => &c.lumped_no[t][profile],
            None => &c.lumped[profile],
        }
    }

    /// profile -> its weights: targets by status, junk by key and slot.
    ///
    /// Nothing sharing a group with a fractured mod (they never leave) or with `also` can roll,
    /// nor anything tagged `skip_tag`.
    pub(super) fn lump(&self, c: &Compiled, name: &str, also: &[String], skip_tag: Option<&str>) -> Lumped {
        let mut tw = vec![[0.0; 2]; c.t_side.len()];
        let mut kw = vec![[0.0; JUNK_SLOTS]; c.keys.len()];
        let taken: BTreeSet<&str> = self.fixed.iter().flat_map(|m| m.groups.iter()).chain(also).map(String::as_str).collect();
        for (mod_id, w) in self.weights_ordered(name) {
            if w <= 0.0 {
                continue;
            }
            let m = c.mods.get(mod_id).unwrap_or_else(|| panic!("weight profile {name:?} names unknown mod {mod_id:?}"));
            if m.groups.iter().any(|g| taken.contains(g.as_str())) || skip_tag.is_some_and(|t| m.tags.iter().any(|x| x == t)) {
                continue;
            }
            let (i, v) = self.classify_mod(m);
            if i >= 0 {
                tw[i as usize][v as usize - 1] += w;
            } else if let Some(k) = c.keys.iter().position(|k| k.groups == m.groups) {
                kw[k][v as usize] += w;
            }
        }
        let mut junk = [SlotW::default(); JUNK_SLOTS];
        let mut cross = [[0.0; JUNK_SLOTS]; JUNK_SLOTS];
        for (slot, j) in junk.iter_mut().enumerate() {
            // Groups `also` keeps out can't be rolled, but one of them may be on the item (the forced mod).
            let all: Vec<f64> = c.keys.iter().zip(&kw).filter(|(k, _)| k.slots[slot]).map(|(_, w)| w[slot]).collect();
            *j = SlotW::new(&all, &all, c.phantoms);
            for other in (0..JUNK_SLOTS).filter(|&o| c.linked[slot][o]) {
                cross[slot][other] = kw.iter().map(|w| w[slot] * w[other]).sum::<f64>() / j.total.max(f64::MIN_POSITIVE);
            }
        }
        Lumped { tw, kw, junk, cross }
    }

    pub(super) fn weights_ordered(&self, profile: &str) -> Vec<(&str, f64)> {
        if profile == "base" {
            let mut out: Vec<(&str, f64)> = Vec::new();
            let mut at: HashMap<&str, usize> = HashMap::new();
            for m in &self.pool {
                match at.get(m.id.as_str()) {
                    Some(&i) => out[i].1 = m.weight,
                    None => {
                        at.insert(&m.id, out.len());
                        out.push((&m.id, m.weight));
                    }
                }
            }
            out
        } else {
            let p = self.profiles.get(profile).unwrap_or_else(|| panic!("unknown weight profile {profile:?}"));
            p.iter().map(|(k, v)| (k.as_str(), *v)).collect()
        }
    }

    pub(super) fn profile_index(&self, name: &str) -> usize {
        self.c().profile_names.iter().position(|n| n == name).unwrap_or_else(|| panic!("unknown weight profile {name:?}"))
    }

    pub fn mods(&self) -> &HashMap<String, Mod> {
        &self.c().mods
    }
}
