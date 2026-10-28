# Sceptre and regalia: why they are slow, and a phased-craft plan

Handoff written 2026-10-28 against `overhaul` at 699399b. It covers the two
remaining slow finals, `ama-327-fire-dot-spell-crit-sceptre` and
`cheap-high-es-regalia`, and proposes the change most likely to make every
recipe solve in under a second cold. It complements [the research report](research.md);
read its "Current bottleneck diagnosis" and "Experiments not to repeat" first.

All numbers below come from release builds on a 4-core machine with
`RAYON_NUM_THREADS=4`, one guide at a time, using the guide harness's final-item
solver time (`PONDER_BENCH main`). This machine is about 1.7× slower than the
one used for research.md (sceptre 18.4 s here against 10.3 s there), so compare
ratios, not absolute seconds. The experiment switches were temporary patches and
are not on the branch.

## Implemented (2026-10-28)

Three changes are now on the branch:

- **Fracture rule** (`Phase::Fracture` in `crates/ponder-plan/src/craft.rs`): a
  fracture that locks a mod serving no need ends the attempt. The model maps
  that outcome to `Restart`, and `Craft::follow` does the same on real items.
- **Eldritch rule** (`Phase::Eldritch`, `Phase::Restarts`): an Eldritch
  implicit is applied only with at most one need missing. An item that carries
  one takes no rerolls, rarity changes, recombination, new influences or
  fractures, so the craft is no longer mirrored with the implicit.
- **Settled recombination checks** (`ponder-solve`, exact): between admission
  rounds, a state whose deferred actions stayed further above its value than
  any value has since fallen is not checked again. Recombination tables are
  prepared only for the states still checked, and the first pass's expectations
  are reused. Costs are unchanged to the last digit.

The bench-craft gate measured below was **not** shipped. With metamods free it
cost +11.5% on the recombined bleed axe and +7% on the chaos claw, saved
nothing on the regalia, and saved only about 10% on the sceptre. Deferring
bench crafts instead of banning them restores the exact costs but also all of
the time.

Full cohort, final-item solver time on this 4-vCPU Xeon at 2.1 GHz. All 44
active guides pass, and the 22 inactive guides are unchanged.

| Guide | Before (s) | After (s) | Cost change |
|---|---:|---:|---:|
| cheap-high-es-regalia | 15.39 | 2.44 | +1.8% |
| ama-327-fire-dot-spell-crit-sceptre | 16.99 | 6.29 | +2.8% |
| cheap-ele-bow | 24.45 | 11.28 | 0 |
| chaos-dmg-claw | 2.25 | 0.85 | 0 |
| int-chaos-armour-boots | 2.46 | 1.69 | +1.3% |
| plus-two-minion-reservation-helmet | 2.34 | 1.68 | 0 |
| one-hand-bleed-axe | 1.76 | 1.39 | 0 |
| minion-plus-two-wand | 1.27 | 0.48 | +0.0% |
| ama-326-overlord-staff-cluster | 0.006 | 0.006 | **+4.6%** |

Other cost changes are under 3%: physical weapon +2.8%, regen helmet +1.4%,
pact weapon +0.7%, dex/resist ring +0.2%. The overlord cluster is the one
excess: after fracturing an unneeded notable, carrying on is sometimes cheaper
than starting over.

Still over one second here: bow 11.3 s, sceptre 6.3 s, regalia 2.4 s, then
five guides at 1.1–1.7 s. In all three slow crafts, what is left is
recombination. It is the bow's main strategy (6.6 recombinations per run,
10.9 M outcome rows over 111 k states). In the sceptre and regalia it is
checked at every state and forces 5–10 whole-graph re-solves. Regalia split:
exploring 0.34 s, values 0.78 s, recombination checks 1.03 s (mostly the first
two rounds, where all 35 k states are still open).

## Summary

- Each of these crafts explores 89–139 thousand states, but its optimal policy
  uses only about 8–11 thousand. For 70–80% of explored states the best action
  is "start over" or "reroll". The exact solver must still expand every action
  at each of them to prove that.
- The states come from a product: core item states × which mod is fractured ×
  whether an Eldritch implicit is present × whether a crafted blocker is
  present. Each of these "decorations" matters in only one phase of a real craft.
  Because rerolls keep fractured mods and Eldritch implicits, one decorated item
  seeds a full copy of the earlier craft.
- Recombination is not where the states come from (under 200 states each). It
  costs time through deferred-action checks and repeated whole-graph re-solves.
- A tolerance on optimality does not help (measured: no speedup at 0.5%, 1% or
  5%). Restricting individual mechanics helps 2–14× but does not reach one second.
- Splitting the craft at fracturing into separately solved phases is exact at
  the boundary (measured: same values to 0.1c). The plan below makes phases a
  planner feature and keeps the exact monolithic solve as an offline reference.

## What "within ε" would mean, and why it was dropped

An ε stop rule does not compare against the oracle. It compares against
ponder's own optimum, which can be certified without computing it. If every
action `a` at every state `s` satisfies

    V(s) ≤ (1 + ε)·c(s, a) + Σ P(t | s, a)·V(t)

then `V` is a sub-solution of the same problem with all costs scaled by `1 + ε`,
so `V(start) ≤ (1 + ε)·V*(start)`. The existing certificate is the case `ε = 0`,
with unknown states extended by zero. In practice this changes only the
improvement and admission thresholds in `ponder-solve` (`solve_component` and
the deferred-action check). Measured:

| Guide | ε | Solver (s) | Cost (c) | Explored |
|---|---:|---:|---:|---:|
| regalia | 0 | 15.9 | 1,252.8 | 88,813 |
| regalia | 0.005 | 17.1 | 1,252.8 | 88,813 |
| regalia | 0.05 | 20.0 | 1,254.7 | 88,813 |
| sceptre | 0 | 18.4 | 3,563.0 | 138,593 |
| sceptre | 0.05 | 18.9 | 3,563.2 | 138,593 |

The time goes into evaluating a graph that is explored eagerly whatever the
threshold. The policy is already within 0.2% after the first rounds. Do not
pursue this further.

## Where the states come from

`STATS=1` in the guide harness prints, per action, how many states it reached
first. With `solve_everywhere` (every explored state kept), the best action at
each explored state can be counted:

| | Regalia | Sceptre |
|---|---:|---:|
| Explored states | 88,813 | 138,593 |
| States the policy visits | 8,508 | 11,445 |
| Best action: start over (fractured items) | 41,114 | 27,057 |
| Best action: reroll (essence, scour, chaos) | 28,548 | 68,632 |
| Found first by fracturing | 51,806 | 29,071 |
| Found first by unveil | — | 49,136 |
| Outcomes stored | 11.1 M | 18.6 M |

Removing one decoration at a time from the policy's states (regalia: 8,500
items; 2,500 unfractured; 7,471 without crafted mods; 1,920 with all three
removed) shows the product directly. Across all explored states with fracture
outcomes collapsed, 86,406 items reduce to 3,888. The Eldritch flag exactly
doubles every bucket: lesser ichor is cheap, and essence rerolls keep the
implicit, so the whole craft reappears with it.

The useful regalia policy is small and human-shaped. Shrieking Woe spam sets up
the item, it fractures one of the three wanted ES prefixes, and then it runs a
159-state Shrieking Spite + exalt loop around the fractured mod (three
components, one per prefix). It costs 1,252.8c against the guide's 2,687.8c.

Per explored state, final solves cost roughly 100–200 µs including outcome rows.
A sub-second final solve therefore needs about 5,000 explored states or fewer,
or far fewer outcomes per state.

### Why smarter proofs did not help

At a junk state the true value is the restart worth `w`. A lazy search can skip
expanding it only with a lower bound close to `w` for every alternative's
successors. Admissible bounds here (relaxed lens, side bound, zero) are far
below `w`, so lazy search expands nearly everything anyway. That is the common
cause behind the LAO*, optimistic-placeholder and eager-bound regressions in
research.md. This conclusion depends on the craft having a free restart. It
holds for every multi-phase craft in the suite.

## Measured restrictions

These are policy restrictions inside the existing model: the cost is exact for
the restricted policy class, and the change in cost is the price of the
restriction. Rows marked † changed only the model, not the simulator. Their
real-item check fails because the simulator follows real items through states
the restricted model collapsed, so only their model cost is meaningful.

| Restriction | Regalia s / cost / explored | Sceptre s / cost / explored |
|---|---|---|
| none | 15.9 / 1,252.8 / 88,813 | 18.4 / 3,563.0 / 138,593 |
| no fracturing | 2.4 / 1,328.8 (+6.1%) / 14,937 | 2.5 / 3,718.6 (+4.4%) / 20,908 |
| fractured unneeded mod → start over † | 4.2 / 1,274.8 (+1.8%) / 35,481 | 6.4 / 3,661.1 (+2.8%) / 52,428 |
| after fracturing, only reroll or start over † | 14.8 / 1,274.8 (+1.8%) / 86,725 | 16.8 / 3,660.2 (+2.7%) / 137,953 |
| bench crafts only once a side is complete | 15.4 / 1,252.8 (0%) / 88,805 | **7.3 / 3,580.7 (+0.5%) / 49,942** |
| …plus Eldritch implicit only with ≤ 1 need missing, and Eldritch items take only finishing actions | **10.7 / 1,252.8 (0%) / 88,402**, outcomes 11.1 M → 4.8 M | (no Eldritch ops) |
| no fracturing + bench gate | 2.0 / 1,328.8 (+6.1%) / 14,830 | 1.25 / 3,740.0 (+5.0%) / 6,822 |
| reroll outcomes with no new need go back to the hub † | inconsistent, discard | 37.4 / 7,145 / 140,833 |

The bold rows pass the harness's real-item checks with costs unchanged or
within 0.5%. Even stacked, restrictions stop at 1.25 s and 2.0 s. The reroll row
came out *below* the unrestricted optimum on the regalia, so that hack was
mis-modelled. It also did not reduce explored states, so it is not a lever.

### The phase boundary is exact

A temporary guide started the regalia from a fractured T2 flat-ES (and %ES, and
hybrid) base, with the same goal and operations. Its best plans cost 550.7c,
588.1c and 616.0c. The monolithic solve gives those exact hub states 550.69c,
588.15c and 616.01c. The post-fracture craft is a self-contained subproblem; its
policy is 162 states.

That solve still explored 47,093 states, because the harness always offers
`buy:base` as well, so it re-explored the whole craft from an unfractured base.
Measuring a phase in isolation needs a start set without the plain base
(step 0 below).

## Plan: phased crafts

Make the planner's AND/OR subgoals (SPEC §6, "subgoal generators: fracturing
suggests '4-mod item with X'") real for fracturing and finishing. Each phase is
an ordinary `Craft` solved exactly; phases connect through values, not shared
states. Decorations cannot flow back into an earlier phase, so the mirror copies
disappear.

0. **Isolated starts in the harness.** Let a guide stage or request replace the
   default starts (`stocks` in `crates/ponder-plan/tests/guides.rs`, which always
   includes `base`). Re-run the three post-fracture regaliae. Expect the 162-state
   policy, and record explored states and time. This decides whether the plan
   reaches the budget, so do it first.
1. **Phase description in `ponder-plan`.** A phase has starts, an action filter
   `(state, op) -> bool`, and exits: outcome states mapped to a terminal value
   (another phase's solved worth plus an entry cost), `Restart`, or kept. The
   experiments hooked exactly two places, which is where this belongs:
   `Craft::actions` (`crates/ponder-plan/src/craft.rs`, `Node::Item` arm) and the
   outcome mapping (`Craft::nodes` / `Craft::outcomes`). `ponder-solve` needs no
   change: exits are fixed-worth states like `Goal` and `Restart`
   (`Entry::Fixed`).
2. **Fracture subgoals.** For each needed, natural, fracturable class X of the
   goal, solve F(X) from the reroll hub "rare, only X fractured", with all
   operations except fracturing. In the main craft, a fracturing outcome whose
   fractured class serves a need exits at
   `min over reroll ops r [price(r) + V_F(X)(hub_r)]`; any other fractured outcome
   exits at `Restart`. The main craft then holds no fractured states. Measured
   price of discarding the unfractured remainder at fracture: +1.8% regalia,
   +2.7% sceptre. A refinement keeps the remainder when it already holds
   needs (`missing_needs ≤ 1`). Measure it before adding it.
3. **Finishing phase.** Eldritch implicits and bench blocker/lock crafts are
   available only once the item is near done: Eldritch with at most one need
   missing; bench crafts (other than removing crafts) once one side holds all
   it needs. Inside the finishing phase, rerolls are not offered, so an Eldritch
   item never re-enters the early craft. Measured: 0% on the regalia,
   +0.5% on the sceptre.
4. **Recipes and simulation.** `Recipe`, `Craft::follow` and the real-item
   simulation must follow phase exits (enter F(X) after a fracture, abandon at
   `Restart`). The † rows failed their real-item checks for exactly this reason.
5. **Validation.** Keep the monolithic exact solve as an offline reference
   (a harness switch, nightly or on request, not in `cargo test`). For every guide
   with a phase split, report phased cost against exact cost. Accept at most
   +3% (measured gaps are 0–2.8%), with the existing guide and real-item checks
   still passing. Then run the full cohort cold, one guide at a time, under
   `ponder-tools bench --all`.

### Risks and open questions

- **Sceptre fracture with locks.** In research.md, 332 of 508 sceptre fractures
  happen with the suffix lock present, and it can prefer veiled chaos over
  fracturing when a fracture would lock a conflicting prefix. Its F(X) start is
  not just "X only": the lock and unveil state cross the boundary. Treat the
  lock as part of the phase-entry state, and measure before restricting more.
- **Recombination inside a phase** still costs `separation` time (regalia 5.8 s,
  1.02 M prepared checks) and 6–10 whole-graph re-solves per final. If phases
  alone are not enough, the next target is incremental re-solving after
  admissions, rather than restarting Newton over the whole graph each round.
- **The unveil phase (sceptre).** `unveil:fire-damage` first reaches 49 k states.
  Veiled chaos → unveil → finish is a natural third phase with the same shape as
  the finishing phase. It was not measured here.
- **Sub-second is not yet demonstrated.** What is shown: the boundary is exact,
  the restrictions cost 0–2.8%, and the remaining states sit in phases that
  never need to meet. Step 0 measures whether each phase fits the
  ~5,000-state budget.

## Reproducing

The experiment switches (`PONDER_EPS`, `PONDER_NO_FRACTURE`,
`PONDER_FRACTURE_NEED`, `PONDER_FRACTURE_RESET`, `PONDER_GATE`,
`PONDER_KEEP_ALL`) were a temporary patch of about 300 lines over `ponder-solve`,
`ponder-rules/src/lens.rs`, `ponder-plan/src/craft.rs` and the guide harness.
They are not on the branch. Re-creating any one of them is a few lines at the
hooks named in step 1. Run single guides as:

```sh
cargo build --release -p ponder-plan --tests
STATS=1 RAYON_NUM_THREADS=4 GUIDE_EXACT=cheap-high-es-regalia \
  target/release/deps/guides-<hash> --exact guides_cost_no_less_than_ponder --nocapture --test-threads=1
```
