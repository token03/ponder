# Solver design: exact mechanics, certified planning, priced options

Status: the single-item solver and module boundaries in sections 2 and 4 are built. Sections 3, 5 and 6 describe future work.

## 1. What "optimal" can honestly mean

A crafting plan is a policy for a stochastic shortest-path problem: from each item, the action
with the least expected cost to finish. Policy iteration solves that exactly, *for the model
it is given*. Three things decide whether the answer means anything:

- **The mechanics model.** The mod pool and weights, the mod-count chances
  (`RARE_MOD_COUNTS_*`, `MAGIC_TWO_MOD_CHANCE`), and how each currency acts. Everything below
  is relative to it; it is validated by replaying plans on real mods (`simulate`) and by an
  exact item-level model of small pools (`ponder-core/tests/exact.rs`).
- **The state abstraction.** Planning on real items (which mod ids are on it) is far too big for
  a 6-mod rare (millions of states). The solver plans on a lumped state: rarity, each target
  missing/good/blocked, and how many other mods per slot. That forgets which groups the other
  mods are, and the groups decide what the next mod can be.
- **The actions searched.** Shortlists (fossil combos kept, donor shapes, base partners) cut
  options before solving. Unlike the abstraction, they have no bound attached.

## 2. The certified single-item solver (built)

**Bounds on every chance.** Every transition from a lumped state carries, beside its point
estimate, an interval that holds for every real item the state stands for. Rolling one more
mod into a slot holding `k` other mods: the weight those `k` mods keep out lies between the
`k` lightest and the `k` heaviest groups that may be there, so each outcome's chance lies
between its weight at its least against the others at their most, and the reverse.
Annulments are exact on counts (uniform over mods), except under a "cannot roll" metamod,
where which mods are protected is bounded. A junk group whose tiers fall in two slots (low
life is filler, high life other junk) blocks both.

**Exact fresh rolls.** A fresh roll (chaos, alchemy, essence, fossil, Harvest reforge) doesn't
depend on the item, so it can be worked out on real groups: groups of equal weights are
interchangeable, so they are pooled into classes and the roll carries which classes were
drawn. A full side's classes stop mattering and are dropped, and the last draw is projected
straight into the result. A pool too big for that (more than 30k partial items per draw)
falls back on the lumped roll with bounds. Distinct group lists that overlap also use this
conservative path: they cannot be treated as independent interchangeable groups.

**Certified bounds on the plan** (`Plan::bounds`). A lumped model with interval chances is a
bounded-parameter MDP (Givan, Leach and Dean, AIJ 2000); its interval value iteration brackets
the value of every real MDP consistent with the intervals. Done by policy iteration here:

- `lower`: crafter and nature both minimise. No way of crafting, even one that reads every mod
  on the item, expects to spend less using the enabled actions under the supplied mechanics model.
- `upper`: nature maximises against the plan. Following the plan expects to spend at most this.

`tests/exact.rs` checks `lower <= exact optimum <= plan on real items <= upper` on 24 generated
pools under two price sets, plus an overlapping-group regression. Fixture replays check the
ranges and point estimates with sampling error accounted for. These tests validate specific
mechanics and input shapes; they do not prove every future mechanic or pool is supported.

**Lazy refinement.** Most of the ~100 rerolls on offer are never worth using. Rolls start out
lumped (quick, bounded); the ones the plan or the lower bound's policy use where they act are
then worked out exactly and the problem solved again, until none are left. This is
constraint generation applied to model fidelity rather than to actions (compare CG-iLAO*,
Schmalz and Trevizan, AAAI 2024 / AIJ 2026). The fixtures give the same plans as making every
roll exact.

**Dead ends are avoided, not fatal.** An action that may land where nothing can finish is cut
(with what leads only there), as in SSPs with avoidable dead ends (Kolobov, Mausam and Weld,
UAI 2012). The target is unreachable only if the origin itself can't finish.

**Measured costs.** Cold solves are 3-4x slower than lumped-only (bench: 0.68 s vs 0.19 s for
the fixtures; a cold `ladder crafts` 54 s vs 21 s; warm runs read the plan cache as before).
The abstraction's regret is real: on adversarial toy pools (weights 400x apart), a crafter
reading every mod beats the lumped plan by up to ~25%; the bounds report it.

**Modularization check (2026-10-24).** Against the uncommitted tree immediately before this
refactor, all 22 fixture policies, costs and bounds are identical. Clicking hours differ by
at most `4.5e-16` from summation order. A sequential local benchmark used five fresh solves
per fixture and three replays of 200 items with seed 11, comparing sums of per-fixture
medians: cold solves 0.865 s before / 0.829 s after; replay 1.875 s / 1.738 s. Cold timing is
close to run-to-run variance; replay improved by about 7%. These are fixture measurements,
not an end-to-end ladder benchmark. The comparison harness and initial tree were retained
under `/tmp/ponder-modularize-baseline-20261024` during the review.

## 3. Tightening and refinement (next)

- **Split junk slots by weight class** (heavy / light other mods). Bounds shrink with the
  ratio of weights inside a slot, so a split where the gap is wide buys most of the precision.
  Refine only where `upper - lower` is large: bounds say where.
- **Bounded focused search.** With certified bounds per state, Bounded RTDP (McMahan,
  Likhachev and Gordon, ICML 2005) or CG-iLAO* can stop exploring once `upper - lower` at the
  origin is within a tolerance the user picks, instead of exploring every reachable state.
  Feasible recipes give upper bounds; the optimistic model gives lower ones.

## 4. Implemented boundaries and extension rules

The existing crates remain. The module boundaries are:

| Module | Owns | Must not own |
|---|---|---|
| `ponder/actions/{essence,fossils,harvest,bench}.rs` | Available actions, profiles, costs and timing metadata | Search or state compression |
| `ponder/gear/catalog.rs::configure_actions` | One catalogue assembly path for products, components and relevelled items; explicit capabilities and fossil shortlist limits | Currency transition implementations |
| `ponder-core/action.rs` | Typed `ActionKind` and shared `Effect` definitions; duration and currency cost | Search policy |
| `ponder-core/mechanics.rs` | Real-mod draws and effects; shared mod-count, affix-limit and removal rules | Goal classification or policy selection |
| `ponder-core/problem/projection.rs` | Per-goal classification and completion | Market value |
| `ponder-core/problem/{pool,rolls,effects,interval}.rs` | Compiled pools, exact or conservative compressed transitions, and refinement caches | Prices or policy iteration |
| `ponder-core/search/` | Numeric decisions, transitions, dead-end pruning, policy evaluation, bounds and refinement scheduling | Currency kinds, prices or metamod rules |
| `ponder-core/solve.rs` | Adapter from a priced crafting problem to the numerical model and back to a readable plan | Physical currency effects |
| `ponder-core/fracture.rs` | Fracturing retry economics using exact roll distributions | General search internals |
| `ponder/pipeline/` and `ponder/recomb.rs` | Products, listings, assembly, reporting and opportunities | A second single-item solver |

`ProblemSpec` is editable configuration. Calling `compile()` produces an immutable `Problem`.
Clones share its specification and derived caches. To change a craft, call `to_spec()`, edit
that specification and compile it. There is no manual cache-reset API. Solving selects priced
and equivalent actions by stable catalogue indices without modifying the problem; changing
prices and solving again cannot lose previously excluded actions.

Actions carry typed kinds and explicit seconds. The current JSON format requires timing;
fixtures have been converted instead of retaining an old-format deserializer. Display names
never determine behavior or timing. Unknown kinds fail to load. Fossil combinations supply
their socketing time explicitly.

`Distribution` distinguishes exact probabilities from bounded probabilities. Public roll
queries request refinement and retain the returned bounds when exact computation cannot be
used. Fracturing cost estimates require exact rolls. A failed refinement attempt is cached,
so an oversized or overlapping pool cannot cause an endless retry loop.

`search::Model` supplies an origin, goal predicate, numeric decisions and transitions. It can
optionally refine selected transitions. `TransitionBuffer` keeps buffered probabilities and
ranges aligned; shared distributions preserve the existing reuse of reroll outcomes. A
small non-crafting model in `tests/search.rs` exercises retry loops, avoidable dead ends,
interval values and refinement through this interface. The current backend still explores
the reachable graph; this seam makes a focused backend possible without rewriting mechanics.

Model action IDs stay stable across refinement. Buffered transitions start with a cleared
buffer and contain a normalized distribution with feasible probability intervals. Refinement
returns true only when its transitions change and must preserve their soundness. The SSP backend supports market credits and requires a finite proper finishing policy.
Negative net costs are valid; a singular or non-finite policy is reported. `solve_with` takes one `SolveOptions` value for
backend, iteration and resource limits; `solve` uses its defaults.

The exact mechanics engine and compressed mathematics deliberately have different item
representations. They consume the same typed effects and shared rules. The independent
small-pool test oracle remains separate so shared implementation mistakes can be detected.

Extension checklist:

- An essence or Harvest recipe using an existing effect changes its catalogue module and
  data-specific tests. It does not change the search kernel.
- A new physical effect adds a typed effect, its real-item implementation, and its compressed
  exact or conservative transition implementation. It does not add another solver.
- A goal or junk-class refinement changes projection and compressed transitions. It does not
  change what a currency physically does.
- A new search algorithm consumes `search::Model`. It does not decode currency names.
- Assembly and future value exits consume component plans and transition contracts. Their
  market assumptions must remain outside the mechanics layer.

Plan cache version 17 covers current goal, continuation, bench and recovery semantics. Older
plans are invalidated. Every solved plan includes bounds; there is no pre-bounds plan format.

## 5. Pivots and opportunities, priced in

Spamming a fresh roll for product A sometimes lands a start worth more to product B (or a
donor, or a sale). That is an option with a value, and it belongs in the plan's cost:

- **Exit with credit.** When a roll lands real item `x`, the crafter may take it, crediting
  `worth(x)`, and start A over on a new base: an outcome's cost to go becomes
  `min(V_A(x), V_A(origin) + base - worth(x))`. `worth(x)` is the best of: another product's
  cheapest cost from scratch less its cost to finish from `x` (today's `pivots.rs` saving), a
  component's value, or a market price.
- **Only fresh rolls need it.** A fresh roll's real outcomes don't depend on the item, so the
  joint distribution of (A's lumped outcome, `worth(x)`) is worked out once per roll: by the
  exact class roll for small pools, else by sampling real rolls (credit rounded down keeps the
  cost an upper bound). Pivots off a half-done item are rare and can be left out (cost stays
  an upper bound).
- **Breadth stays cheap.** Index products and components by the mods they need; a rolled
  item is read only by those whose mods it carries. Most rolls touch none.
- **Report both.** The plan's cost with credits, the credits on their own ("1 in 40 alts
  lands a T1 life + T1 res start for Boots (ES), worth ~120c: take it"), and per step the
  starts to watch for. A credit larger than the craft's own cost means the spam pays for
  itself: say so rather than loop.
- **Credit once per demand.** A start for a product you'll make once is worth its saving
  once; a market price can be credited per item. Don't credit the same demand twice.

## 6. Components: donors and throwaway items

Fracture bases, Awakener's donors and recombinator leaves don't want 6 mods. They want 1-2
mods on a given base, often magic or with few other mods. Those are small goals with small
state spaces (a handful of targets, junk capped), so their plans are exact and cheap. Each
component has sources:

- craft it (its own plan and bounds),
- buy it (trade: price and listing count),
- byproduct (a pivot credit from another craft's spam, section 5).

Its cost is the cheapest source. Assembly then works over components: recombinators (kernel
per side pool size, NNN padding relative to the output base), Awakener's Orb (influence
pairs), fracturing (1 in k mods). Recipes become pairs of (stage, hand-off): acquire a core,
protect a side, finish. Each stage is a small solved problem whose value at its hand-off
states is the next stage's cost to go, so a stage never finishes into a state the next one
hates. Recipe libraries give upper bounds without forbidding other routes.

Still to model (from the earlier plan, unchanged): the 3.26 special-mod penalty in
recombinators, exclusive-mod rules, beast lock + Awakener preview as observation nodes
(`E[max_d V(preview(s, d))]`), and influence as its own stage. Every community-derived
number lives in one versioned table so a patch is a data update.

## 7. Implemented recipe stages and recovery

Gear completion uses named useful requirements, plus interchangeable single elemental
resistance counts. Suppression and chaos resistance cannot be exchanged for elemental
resistance. Each requirement has one acceptable floor; stronger rolls qualify without adding
separate T1/T3 state dimensions. Accepted substitutes retain the same conflict-group footprint,
so a hybrid armour/ES mod cannot silently block an otherwise independent armour target.

Eligible complete natural sides are prepared as small component goals, then handed to an
independent finisher. Lock/scour and full-suffix Matron/scour are exact compound operations.
The finisher can annul; losing its preserved targets returns to a priced acquisition node.
Awakener acquisition preserves its transferred pair and includes its random reforge. Its
recovery repeats both donor preparation and transfer. Recombination retains its separate join
kernel and never receives that reforge. Supported natural asymmetric joins include Focused
and Simplex; side capacities above three and special/exclusive or influenced recombination
remain outside the validated kernel.

The repeated-acquisition node solves the regenerative equation rather than pricing only the
first donor pair. Its value includes all expected retries, currency and clicking time. It adds
no physical inventory dimension. A full blocked item retaining the quoted core also offers a
sale/replacement action. Its toll is replacement preparation plus base and trade time minus
explicit sale proceeds. Lost-core outcomes cannot claim that quote. Without a quote, a
baseline policy supplies state-specific break-even premiums. Quotes are scoped to both the
goal and its preserved core. Negative net replacement credits are supported, including finite negative expected finishing
costs. Singular policies are reported rather than assigned a fake cost.

A single observed bench craft becomes a normal stat requirement: stronger natural rolls
can satisfy it, or a terminal bench action can finish it. It allows the source to be chosen by the recipe. Full blocked results remain
possible, and the terminal bench action checks the actual free slot. Ordinary bench crafts
can finish a goal as absorbing actions. Temporary unveil blockers are
modeled before the three offers are drawn. Multiple concurrent intermediate bench crafts,
elevation, imprint/restore and preview-lock stages are not yet general route providers.
Route decomposition produces a feasible-policy estimate, not a proof of global optimality
over systems the model does not represent. Initial unquoted donor-base prices remain excluded.

Upper-bound certification follows only states reachable under the selected policy, using
positive probability upper limits for reachability. Uncertainty in an unused route cannot
invalidate that policy's bound. Adversarial choices keep their prior probabilities on equal
expected value; repeated numerical policies and unresolved convergence leave conservative
bounds rather than producing a finite certificate from unstable arithmetic. Closed interval
classes are detected before attempting a singular worst-case solve.

When optimistic bounds identify uncertainty, all competing actions that could improve the
incumbent at the optimistic policy's visited states are refined together. This preserves the
previous refinement candidates and avoids rebuilding the whole graph once per fossil or
Harvest profile. The six-goal amulet diagnostic in `target/craft-cases` dropped from an
unfinished run exceeding six minutes, through 43 seconds after policy projection, to 6.3
seconds after batching. Its nominal cost and 40,631 discovered states were unchanged.

With shared modifier pools and profiles, failed-shape reuse and block substitution,
the full local workload takes 11.2 seconds in the cached benchmark and 175.2 seconds
to fill an empty plan directory, compared with
14.1 and 290.2 seconds for the previous binary on the same inputs. Cold peak resident
memory remains high at 6,404 MiB; the cached run peaks at 5,172 MiB. It finds priced
routes for 433 of 437 crafts. Recipe decomposition improves coverage without making
the entire portfolio a proof of optimality, and the ten-second full-startup target
remains open.

## 8. Competitive route evaluation

Policy matrices retain their exact stochastic equations. The sparse part of the
Woodbury decomposition is factored by strongly connected components: cycles remain
inside their own LU factors, while dependencies between components use substitution.
The transpose solve processes components in the opposite order. Dense and sparse
oracles cover multiple right-hand sides, shared resets between cyclic blocks,
singular cycles, and both positive costs and finite negative credits.

The block factors retain only external component dependencies after construction.
Consecutive scalar components share a column-wise substitution pass. These changes
preserve the stochastic equations and their evaluation order within each column.
Shared pools and weight profiles change ownership, not mechanics or serialized
problem contents. Failed-roll shape reuse preserves the existing bounded fallback;
it does not impose a new search cutoff or discard a crafting method.

A complete policy supplies an upper bound for its route. Comparing a different
candidate uses its own certified lower bound, including acquisition and final bench
costs on both sides of the comparison. Search stops only when the lower bound is
strictly greater than the incumbent, with the numerical comparison tolerance; ties
remain eligible. Probability intervals that straddle the incumbent do not justify
pruning. Lower certificates are cached separately from plans and failures.

Protected recovery can certify an incumbent only when its preparation price is
already an upper bound: an uncertain preparation price cannot be multiplied by the
nominal retry count and called a certificate. Recombination adds each donor's
upper-cost premium multiplied by its expected copies, since the join kernel is
exact in the supported model. The independent finishing policy contributes its
own upper bound. Uncertified recipe costs remain useful estimates but cannot stop
another search.

Promising fracture variants run before the direct craft so their complete costs
can provide incumbents. A later trade quote or removal of a quoted sale recipe can
make an alternative competitive again; the pipeline retries it rather than treating
the earlier comparison as a permanent failure.
