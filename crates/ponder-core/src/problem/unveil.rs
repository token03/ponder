use super::*;

type OfferWeights = Vec<(u8, f64)>;
type OfferGroup = (Vec<String>, OfferWeights);
type OfferClass = (usize, OfferWeights);

impl Problem {
    pub(super) fn pending_side(s: &State) -> Option<Side> {
        match s.veiled {
            VEILED_PREFIX => Some(Side::Prefix),
            VEILED_SUFFIX => Some(Side::Suffix),
            _ => None,
        }
    }

    pub(super) fn veil_class(&self, m: &Mod) -> u8 {
        match self.classify_mod(m) {
            (i, v) if i >= 0 => i as u8 * 2 + v - 1,
            (_, slot) => UNVEIL_TARGET_CLASSES + slot,
        }
    }

    pub(super) fn veiled_side(&self, s: &State) -> Option<Side> {
        Self::pending_side(s).or_else(|| {
            if s.veiled == 0 {
                None
            } else if s.veiled <= UNVEIL_TARGET_CLASSES {
                self.c().t_side.get((s.veiled as usize - 1) / 2).copied()
            } else {
                Some(if Side::Prefix.slots().contains(&(s.veiled as usize - 17)) { Side::Prefix } else { Side::Suffix })
            }
        })
    }

    pub(super) fn veil_action(&self, s: &State, chaos: bool) -> Option<Arc<Distribution>> {
        if self.unveils.is_empty() || self.fixed.iter().any(|m| self.unveils.iter().any(|u| u.id == m.id)) || (!chaos && s.veiled != 0) {
            return None;
        }
        let c = self.c();
        let seed = if chaos {
            let mut state = meta_locks(s.meta).map_or(State { rarity: RARE, ..c.base }, |side| self.keep(s, side)[0].1);
            state.rarity = RARE;
            if state.veiled != 0 {
                return None;
            }
            state
        } else {
            *s
        };
        let key = (chaos, seed);
        if let Some(hit) = self.caches.unveils.lock().unwrap().get(&key) {
            return Some(hit.clone());
        }
        let (starts, start_bounds) = if chaos {
            (vec![(1.0, seed)], vec![(1.0, 1.0)])
        } else {
            let (mut states, mut ranges) = (Vec::new(), Vec::new());
            self.remove_into(c, s, None, None, &mut states, &mut ranges);
            if states.is_empty() {
                return None;
            }
            (states, ranges)
        };
        let mut result = Dist::new();
        for (&(p, state), &(lo, hi)) in starts.iter().zip(&start_bounds) {
            let sides: Vec<_> = [Side::Prefix, Side::Suffix]
                .into_iter()
                .filter(|&side| {
                    self.count(&state, side) < self.side_limit(RARE, side) && self.unveils.iter().any(|m| m.side == side && m.weight > 0.0)
                })
                .collect();
            if sides.is_empty() {
                return None;
            }
            for side in &sides {
                let pending = State { veiled: VEILED_PREFIX + side.index() as u8, ..state };
                let (rolled, ranges) = if chaos {
                    self.fill_roll(pending, 0, None, &[Side::Prefix, Side::Suffix], &self.rare_counts())
                } else {
                    (vec![(1.0, pending)], None)
                };
                for (i, &(q, item)) in rolled.iter().enumerate() {
                    let (ql, qh) = ranges.as_ref().map_or((q, q), |b| b[i]);
                    if !self.blockers.is_empty() {
                        let share = 1.0 / sides.len() as f64;
                        result.add(item, p * q * share, lo * ql * share, hi * qh * share);
                        continue;
                    }
                    let offers = self.unveil_offers(&item, *side, 0);
                    if offers.is_empty() {
                        return None;
                    }
                    for (j, &(r, next)) in offers.iter().enumerate() {
                        let (rl, rh) = offers.bounds().map_or((r, r), |b| b[j]);
                        let share = 1.0 / sides.len() as f64;
                        result.add(next, p * q * r * share, lo * ql * rl * share, hi * qh * rh * share);
                    }
                }
            }
        }
        let (out, bounds) = result.finish();
        if out.is_empty() {
            return None;
        }
        let made = Arc::new(Distribution::new(out, bounds));
        Some(self.caches.unveils.lock().unwrap().entry(key).or_insert(made).clone())
    }

    pub(super) fn reveal_unveil(&self, s: &State, blocker: u8) -> Option<Distribution> {
        let side = Self::pending_side(s)?;
        if s.unveil != 0 {
            return None;
        }
        if blocker != 0 {
            let m = self.blockers.get(blocker as usize - 1)?;
            let status = self.targets.len() + self.bench.len() + blocker as usize - 1;
            if s.meta != 0 || s.status[status] != MISSING || self.count(s, m.side) >= self.side_limit(s.rarity, m.side) {
                return None;
            }
        }
        Some(self.unveil_offers(s, side, blocker))
    }

    fn unveil_offers(&self, s: &State, side: Side, blocker: u8) -> Distribution {
        let uncertain = s.junk.iter().zip(self.c().base.junk).any(|(&n, fixed)| n > fixed);
        let relevant = self.caches.offer_relevance.get_or_init(|| {
            [Side::Prefix, Side::Suffix].map(|side| {
                let conflicts = |group: &String| self.unveils.iter().any(|m| m.side == side && m.groups.contains(group));
                self.targets
                    .iter()
                    .map(|t| t.groups.iter().any(conflicts))
                    .chain(self.bench.iter().chain(&self.blockers).map(|m| m.groups.iter().any(conflicts)))
                    .enumerate()
                    .fold(0, |mask, (i, blocks)| mask | u8::from(blocks) << i)
            })
        })[side.index()];
        let occupied = s.status.iter().enumerate().fold(0, |mask, (i, &status)| mask | u8::from(status != MISSING) << i);
        let key = (side, occupied & relevant, uncertain, blocker);
        if let Some(masks) = self.caches.offer_masks.lock().unwrap().get(&key) {
            return Distribution::new(
                masks.iter().map(|&(mask, p, _, _)| (p, State { unveil: mask, ..*s })).collect(),
                Some(masks.iter().map(|&(_, _, lo, hi)| (lo, hi)).collect()),
            );
        }
        let taken: BTreeSet<&str> = self
            .fixed
            .iter()
            .flat_map(|m| m.groups.iter().map(String::as_str))
            .chain(
                self.targets
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| s.status[*i] != MISSING)
                    .flat_map(|(_, t)| t.groups.iter().map(String::as_str)),
            )
            .chain(
                self.bench
                    .iter()
                    .chain(&self.blockers)
                    .enumerate()
                    .filter(|(i, _)| s.status[self.targets.len() + i] != MISSING)
                    .flat_map(|(_, m)| m.groups.iter().map(String::as_str)),
            )
            .chain(
                blocker
                    .checked_sub(1)
                    .and_then(|i| self.blockers.get(i as usize))
                    .into_iter()
                    .flat_map(|m| m.groups.iter().map(String::as_str)),
            )
            .collect();
        let candidates: Vec<&Mod> = self
            .unveils
            .iter()
            .filter(|m| m.side == side && m.weight > 0.0 && m.groups.iter().all(|g| !taken.contains(g.as_str())))
            .collect();
        let mut result = Dist::new();
        let mut groups: Vec<OfferGroup> = Vec::new();
        for m in &candidates {
            let class = self.veil_class(m);
            if let Some((_, options)) = groups.iter_mut().find(|g| g.0 == m.groups) {
                if let Some(option) = options.iter_mut().find(|o| o.0 == class) {
                    option.1 += m.weight;
                } else {
                    options.push((class, m.weight));
                }
            } else {
                groups.push((m.groups.clone(), vec![(class, m.weight)]));
            }
        }
        let overlap = groups.iter().enumerate().any(|(i, g)| groups[..i].iter().any(|h| crate::mechanics::groups_overlap(&g.0, &h.0)));
        if overlap {
            self.offer_draw(s, &candidates, &[], 0, 1.0, uncertain, &mut result);
        } else {
            let mut classes: Vec<OfferClass> = Vec::new();
            for (_, mut options) in groups {
                options.sort_by_key(|o| o.0);
                if let Some(class) = classes.iter_mut().find(|c| c.1 == options) {
                    class.0 += 1;
                } else {
                    classes.push((1, options));
                }
            }
            Self::offer_classes(s, &mut classes, 0, 0, 1.0, uncertain, &mut result);
        }
        let (out, bounds) = result.finish();
        let masks = out
            .iter()
            .enumerate()
            .map(|(i, &(p, state))| {
                let (lo, hi) = bounds.as_ref().map_or((p, p), |b| b[i]);
                (state.unveil, p, lo, hi)
            })
            .collect();
        self.caches.offer_masks.lock().unwrap().insert(key, masks);
        Distribution::new(out, bounds)
    }

    fn offer_classes(s: &State, classes: &mut [OfferClass], drawn: usize, mask: u32, p: f64, uncertain: bool, result: &mut Dist) {
        let total: f64 = classes.iter().map(|(n, options)| *n as f64 * options.iter().map(|o| o.1).sum::<f64>()).sum();
        if uncertain && mask != 0 && drawn < 3 && total > 0.0 {
            result.add(State { unveil: mask, ..*s }, 0.0, 0.0, 1.0);
        }
        if drawn == 3 || total <= 0.0 {
            if mask != 0 {
                result.add(State { unveil: mask, ..*s }, p, if uncertain { 0.0 } else { p }, if uncertain { 1.0 } else { p });
            }
            return;
        }
        for i in 0..classes.len() {
            let count = classes[i].0;
            if count == 0 {
                continue;
            }
            classes[i].0 -= 1;
            for j in 0..classes[i].1.len() {
                let (class, weight) = classes[i].1[j];
                Self::offer_classes(s, classes, drawn + 1, mask | 1 << class, p * count as f64 * weight / total, uncertain, result);
            }
            classes[i].0 += 1;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn offer_draw(&self, s: &State, pool: &[&Mod], chosen: &[usize], mask: u32, probability: f64, uncertain: bool, result: &mut Dist) {
        if chosen.len() == 3 {
            if mask != 0 {
                result.add(
                    State { unveil: mask, ..*s },
                    probability,
                    if uncertain { 0.0 } else { probability },
                    if uncertain { 1.0 } else { probability },
                );
            }
            return;
        }
        let available: Vec<usize> = (0..pool.len())
            .filter(|&i| chosen.iter().all(|&j| !crate::mechanics::groups_overlap(&pool[i].groups, &pool[j].groups)))
            .collect();
        if uncertain && mask != 0 && !available.is_empty() {
            result.add(State { unveil: mask, ..*s }, 0.0, 0.0, 1.0);
        }
        if available.is_empty() {
            if mask != 0 {
                result.add(
                    State { unveil: mask, ..*s },
                    probability,
                    if uncertain { 0.0 } else { probability },
                    if uncertain { 1.0 } else { probability },
                );
            }
            return;
        }
        let total: f64 = available.iter().map(|&i| pool[i].weight).sum();
        for i in available {
            let mut next = chosen.to_vec();
            next.push(i);
            self.offer_draw(s, pool, &next, mask | 1 << self.veil_class(pool[i]), probability * pool[i].weight / total, uncertain, result);
        }
    }
}

impl ProblemSpec {
    pub(super) fn refresh_unveil_actions(&mut self) {
        if self.unveils.is_empty() || !self.actions.iter().any(|a| matches!(a.kind.effect(), Effect::Veil(_))) {
            return;
        }
        self.actions.retain(|a| !matches!(a.kind.effect(), Effect::Unveil(_)));
        if !self.blockers.is_empty() && !self.actions.iter().any(|a| matches!(a.kind.effect(), Effect::RevealUnveil(0))) {
            self.actions.push(Action::new("reveal unveil", crate::action::ActionKind::RevealUnveil(0), &[], &[RARE]).with_seconds(0.0));
        }
        let classes: BTreeSet<u8> = self
            .unveils
            .iter()
            .map(|m| {
                let (i, v) = self.classify_mod(m);
                if i >= 0 { i as u8 * 2 + v - 1 } else { UNVEIL_TARGET_CLASSES + v }
            })
            .collect();
        for class in classes {
            let label = if class < UNVEIL_TARGET_CLASSES {
                self.targets.get(class as usize / 2).map_or_else(|| "bench conflict".into(), |t| t.name.clone())
            } else {
                JUNK_LABELS[class as usize - 16].into()
            };
            self.actions.push(
                Action::new(&format!("unveil {label} ({class})"), crate::action::ActionKind::Unveil(class), &[], &[RARE]).with_seconds(0.0),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offer_cache_reuses_irrelevant_tiers_but_preserves_cross_side_group_exclusion_and_junk_bounds() {
        let target = |id: &str, group: &str, side| Target {
            name: id.into(),
            side,
            good: BTreeSet::from([id.into()]),
            groups: BTreeSet::from([group.into()]),
            notable: false,
        };
        let targets = vec![
            target("wanted", "wanted", Side::Prefix),
            target("unrelated", "unrelated", Side::Suffix),
            target("cross", "a", Side::Suffix),
        ];
        let mut spec = ProblemSpec::new(Vec::new(), targets, 3, Vec::new(), Kind::Gear);
        spec.unveils = ["wanted", "a", "b", "c"].map(|id| Mod::new(id, Side::Prefix, 1.0, &[id], false, id, &[])).to_vec();
        let problem = spec.compile();
        let chance = |out: &Distribution| out.iter().filter(|(_, state)| state.unveil & 1 != 0).map(|outcome| outcome.0).sum::<f64>();
        for status in [MISSING, GOOD, BLOCKED] {
            let out = problem.unveil_offers(&State::new(RARE, &[MISSING, status], &[]), Side::Prefix, 0);
            assert!(out.is_exact());
            assert!((chance(&out) - 0.75).abs() < 1e-12);
            assert!(out.iter().all(|(_, state)| state.status[1] == status));
        }
        assert_eq!(problem.caches.offer_masks.lock().unwrap().len(), 1);
        for status in [GOOD, BLOCKED] {
            let out = problem.unveil_offers(&State::new(RARE, &[MISSING, GOOD, status], &[]), Side::Prefix, 0);
            assert!(out.is_exact());
            assert!((chance(&out) - 1.0).abs() < 1e-12);
        }
        assert_eq!(problem.caches.offer_masks.lock().unwrap().len(), 2);
        let out = problem.unveil_offers(&State::new(RARE, &[MISSING, GOOD, GOOD], &[0, 0, 0, 1]), Side::Prefix, 0);
        assert!(!out.is_exact());
        assert_eq!(problem.caches.offer_masks.lock().unwrap().len(), 3);
    }

    #[test]
    fn overlapping_unveil_groups_are_excluded_before_each_of_three_draws() {
        let target = Target {
            name: "wanted".into(),
            side: Side::Prefix,
            good: BTreeSet::from(["wanted".into()]),
            groups: BTreeSet::from(["wanted".into()]),
            notable: false,
        };
        let mut spec = ProblemSpec::new(Vec::new(), vec![target], 1, Vec::new(), Kind::Gear);
        spec.unveils = vec![
            Mod::new("wanted", Side::Prefix, 1.0, &["wanted"], false, "wanted", &[]),
            Mod::new("a", Side::Prefix, 1.0, &["shared"], false, "a", &[]),
            Mod::new("b", Side::Prefix, 1.0, &["shared", "other"], false, "b", &[]),
            Mod::new("c", Side::Prefix, 1.0, &["c"], false, "c", &[]),
        ];
        let problem = spec.compile();
        let out = problem.unveil_offers(&State::new(RARE, &[], &[]), Side::Prefix, 0);
        assert!(out.is_exact());
        assert!(out.iter().all(|(_, state)| state.unveil & 1 != 0));
        assert!((out.iter().map(|outcome| outcome.0).sum::<f64>() - 1.0).abs() < 1e-12);
    }
}
