use super::*;

type IndexedOutcomes = Box<[(f64, usize)]>;

struct RecombinationEdges {
    heir: Heir,
    inputs: [HashMap<SideKey, usize>; 2],
    left: SelectionKernel,
    suffix: SelectionKernel,
    donors: usize,
    width: usize,
    indices: Vec<usize>,
}

struct Tables {
    donor: RecombinationEdges,
    heirs: HashMap<Header, RecombinationEdges>,
}

impl RecombinationEdges {
    fn add(&self, lens: &Lens<'_>, state: &State, out: &mut HashMap<usize, f64>) -> Option<()> {
        let keys = lens.side_keys(state, self.heir.fractured)?;
        let p = *self.inputs[0].get(&keys[0])?;
        let s = *self.inputs[1].get(&keys[1])?;
        for (prefix_mode, suffix_mode) in [(1, 0), (2, 3), (0, 1), (3, 2)] {
            for donor in 0..self.donors {
                for &(q, prefix) in &self.left[prefix_mode][p * self.donors + donor] {
                    for &(r, suffix) in &self.suffix[suffix_mode][s * self.donors + donor] {
                        let probability = 0.5 * q * r;
                        if probability <= 0.0 {
                            continue;
                        }
                        let target = self.indices[prefix * self.width + suffix];
                        if target == usize::MAX {
                            return None;
                        }
                        *out.entry(target).or_default() += probability;
                    }
                }
            }
        }
        Some(())
    }
}

impl Tables {
    fn get(&self, lens: &Lens<'_>, state: &State) -> Option<IndexedOutcomes> {
        let mut out = HashMap::default();
        self.donor.add(lens, state, &mut out)?;
        self.heirs.get(&(state.influences, state.eldritch))?.add(lens, state, &mut out)?;
        let mut out: Vec<_> = out.into_iter().map(|(s, p)| (p, s)).collect();
        out.sort_unstable_by_key(|o| o.1);
        Some(out.into_boxed_slice())
    }
}

impl<'g> Lens<'g> {
    pub fn prepare_transitions<'a, F: Fn(&State) -> Option<usize> + Sync + 'a>(
        &'a self,
        ctx: &'a Ctx<'g>,
        states: &[State],
        index: F,
    ) -> impl Fn(&State, &[Step]) -> Option<IndexedOutcomes> + Sync + 'a + use<'a, 'g, F> {
        let tables: Vec<_> = self
            .partners
            .iter()
            .map(|(_, partner)| {
                if ctx.assumptions.recombine_bonus != 0.0 {
                    return None;
                }
                let header = partner.first()?.1;
                let donor = self.transition_table(
                    ctx,
                    states,
                    partner,
                    Heir { input: None, output: (header.influences, header.eldritch), fractured: false },
                    &index,
                )?;
                let heirs = states
                    .iter()
                    .map(|s| (s.influences, s.eldritch))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .filter_map(|header| {
                        self.transition_table(ctx, states, partner, Heir { input: Some(header), output: header, fractured: true }, &index)
                            .map(|table| (header, table))
                    })
                    .collect();
                Some(Tables { donor, heirs })
            })
            .collect();
        move |state, steps| {
            let [Step::Recombine(partner), tail @ ..] = steps else { return None };
            if tail.iter().any(|s| !matches!(s, Step::RecombineBonus)) {
                return None;
            }
            let partner = self.partners.iter().position(|(p, _)| p == partner).expect("partner read by an operation");
            tables[partner].as_ref()?.get(self, state)
        }
    }

    fn transition_table(
        &self,
        ctx: &Ctx,
        states: &[State],
        partner: &[(f64, State)],
        heir: Heir,
        index: &(impl Fn(&State) -> Option<usize> + Sync),
    ) -> Option<RecombinationEdges> {
        let RecombinationKernel { inputs, donors, donor_ids, donor_keys, rows: [prefix, suffix], outputs: [prefixes, suffixes] } =
            self.recombination_kernel(ctx, states, partner, heir)?;
        let indices = prefixes
            .par_iter()
            .flat_map_iter(|p| suffixes.iter().map(move |s| index(&self.recombination_state(p, s, heir.output)).unwrap_or(usize::MAX)))
            .collect();
        let left = std::array::from_fn(|mode| {
            let rows: Vec<_> = (0..inputs[0].len())
                .into_par_iter()
                .map(|p| {
                    let mut rows: Vec<HashMap<usize, f64>> = vec![HashMap::default(); donors[1].len()];
                    for &(q, keys) in &donor_keys {
                        let d = donor_ids[0][&keys[0]];
                        let s = donor_ids[1][&keys[1]];
                        for &(r, t) in &prefix[mode][p * donors[0].len() + d] {
                            *rows[s].entry(t).or_default() += 0.5 * q * r;
                        }
                    }
                    rows.into_iter().map(|row| row.into_iter().map(|(t, p)| (p, t)).collect::<Vec<_>>()).collect::<Vec<_>>()
                })
                .collect();
            rows.into_iter().flatten().collect()
        });
        let inputs = inputs.map(|keys| keys.into_iter().enumerate().map(|(i, key)| (key, i)).collect());
        Some(RecombinationEdges { heir, inputs, left, suffix, donors: donors[1].len(), width: suffixes.len(), indices })
    }
}
