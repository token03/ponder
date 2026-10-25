# Code map and crafting extensions

The two entry points are `crates/ponder-cli/src/main.rs` and
`crates/ponder-app/src/state.rs`. They assemble a `pipeline::Context`, load a
`ModDB`, and call the same pipeline. The desktop jobs keep this work off the UI
thread.

## Files and ownership

- `ponder-game/src/data.rs` owns the typed repoe exports and their process caches.
  `mods.rs` turns text or known modifier IDs into matched items. `item.rs` and
  `pob.rs` parse their respective input formats.
- `ponder-net` owns HTTP, trade queries, ladder downloads, currency and beast
  prices. Its transport and clock are injectable for offline tests.
- `ponder/src/demand.rs` mines products from matched ladder items. The common
  cluster subset collector handles notable and small-passive products.
- `ponder/src/gear.rs` turns a matched item into a goal. `gear/bases.rs` owns base
  grouping, pools and implicit-derived affix limits. `gear/catalog.rs` assembles
  actions, weight profiles and special modifiers that can satisfy a target.
- `ponder/src/actions/` owns recipe availability, prices and metadata. Adding an
  action here does not implement its transition.
- `ponder-core/src/action.rs` maps public action names to typed effects.
  `problem/projection.rs` classifies modifiers and checks completion.
  `problem/pool.rs` compiles weights and group exclusions. `problem/rolls.rs`
  calculates fresh roll distributions. `problem/effects.rs`, `recipes.rs` and
  `unveil.rs` apply effects to compressed states.
- `ponder-core/src/search/` discovers the state graph, improves policies and
  computes probability-bound certificates. `linear.rs` solves the resulting
  systems with faer. `linear/blocks.rs` separates cyclic dependencies into
  strongly connected components and substitutes between blocks. `mechanics.rs`
  and `simulate.rs` independently replay real
  modifier draws to check the compressed model.
- `ponder/src/recomb.rs` and `awakener.rs` own two different donor joins.
  `pipeline/recombs.rs` prepares and solves their shared donor batch.
- `ponder/src/pipeline/` separates building, solving, market queries, donor
  searches, finishing existing items, pivots and report presentation.

The important seam is `ProblemSpec::compile()`: the editable goal and catalogue
become an immutable `Problem` with shared compiled data and transition caches.
Unveil choice actions are regenerated here, so removing a target for a fractured
variant cannot leave stale target indexes in its recipe.

## Existing libraries worth using

This code already uses serde, rayon, faer, regex, postcard, blake3, indexmap and
flate2. The cleanup reuses those libraries and the standard library instead of
adding another general framework. In particular:

- Trade lookups and essence catalogues consume the same typed exports as
  crafting. They no longer parse a second raw modifier/translation/essence tree.
- `Memo` uses `OnceLock` and `Mutex` to cache equivalent pools, donor
  problems and partner searches. Initializers run outside the cache lock, and
  the first completed value is published; nested parallel jobs cannot wait on
  their own unfinished initializer. Database keys retain their source data,
  preventing pointer reuse from confusing caches built from different databases.
- Problems memoize a streaming blake3 fingerprint. Pricing keys add only the
  currencies that the problem uses.
- Self-fracture price results are stored alongside plans, keyed by the problem,
  exact relevant currency prices (including the Fracturing Orb), clicking-time
  price and cache version. Repeated startup no longer reruns every candidate
  roll distribution solely to recover those prices.
- Equivalent donor entries share both their compiled problem and their solved
  `Arc<Plan>`; they do not each read and retain a separate plan.
- Editable problem variants share modifier pools and weight profiles through
  `Arc`. Editing those catalogues uses `Arc::make_mut`; serialized inputs retain
  the same shape.

Report JSON is a presentation boundary. Its total, recipe stages, currency and
clicking time follow the selected route. Internal formats are current-only; old
plans are invalidated, and stored solver fixtures now carry explicit action timing.

## Recipe composition

The reusable operations are clearing unlocked modifiers, setting rarity, adding
weighted modifiers, removing an eligible modifier, and filling to a sampled rare
modifier count. Eldritch Chaos, lock reforges, Awakener tails and Veiled Chaos use
these shared operations. Fresh rerolls keep their existing class-based exact
calculation and shared-transition fast path.

Composition still needs explicit rules. Beast swaps remove first and then draw
on the other side; their removal ignores attack/caster tag protection. Harvest
augment adds a modifier and removes a different, pre-existing modifier. It must
retain its paired-delta operation. Veiling requires a decision after randomness:
three distinct modifier groups are offered, and the policy chooses one. Treating
that choice as a random single modifier would price the craft incorrectly.

Unveil offers use interchangeable weight classes and cached offer masks. The
state stores an offer mask and the class of its existing unveiled modifier.
Ordinary item identity is still compressed. If unknown junk groups can block
unveil options, probability intervals remain conservative and can be wide.
Member-specific unveils are excluded from generic veiled currency.

## Asymmetric jewellery

Affix limits come from the base's implicit stats, including the rarity adjustment
for magic items. Rare limits are Simplex 1/2, Focused 2/1, Helical 1/4, Cogwork 2/4,
Geodesic 4/2, Manifold 4/1, Composite 6/0 and Ratcheting 0/6. Simplex and Focused
cannot roll magic affixes. Rare count distributions are capped by total capacity.

Experimented jewellery gets a separate base group, mining key and trade search;
it is not interchangeable with an ordinary ring or amulet sharing its mod pool.
Bench reservations, filler construction and completion use the same side limits.
Text matching retains the pinned fixture behavior; this pass does not change how
modifier magnitude scaling is inferred from pasted item text.

## Donor joins

An Awakener recipe prepares two clean, singly influenced donors, each carrying
one transferable modifier. The transfers are retained while the remaining
explicit modifiers are reforged. Its reported cost includes both donor crafts,
the orb, clicking time and the expected finish from the reforge outcomes. Donor
trade links require the appropriate influence. Base purchase prices remain
additional to the crafting estimate, as with existing recombinator donors.

Recombination uses its own modifier-count, base-selection, duplication and salvage
model. It does not get an Awakener-style random reforge afterward. The validated natural-modifier kernel also supports Focused/Simplex capacities
up to three per side. Influenced/exclusive inputs and larger-side asymmetric
joins remain excluded until their transfer rules have a validated kernel.

Bristle Matron includes both side locks, both tag restrictions and multimod.
With full suffixes and an open prefix it deterministically produces Suffixes
Cannot Be Changed. The inner state still supports one bench metamod at a time;
multimod is a removable outcome, not a search over concurrent bench crafts.

## Performance checks

Run `cargo run --release -p ponder --example startup` for a small local
workload, `-- --full` for the default full ladder/donor workload, and `-- --builds`
to separate catalogue building from self-fracture calculation. Inputs come from
existing local game, ladder and price caches. A cold plan cache includes solving;
a warm plan cache measures repeated startup. Do not compare them as the same test.
Pass `-- --cold` to disable the plan cache without deleting existing plans.
Use `-- --full --plan-cache target/new-plans` with a new directory to measure an
actual cache fill, and `--snapshot path.json` to save compact price/route results.

Fresh-roll group checks consider modifiers with positive weight in the active
profiles. Unrollable unveiled groups no longer force exact natural rolls into
the conservative fallback, and fracture targets share the same roll caches.
An unsuccessful exact refinement no longer requests a graph rebuild when its
distribution and probability bounds are unchanged. Dead-end analysis checks
shared outcome lists once per pass instead of rescanning them for every action.
Exact draws stop as soon as a layer crosses the existing fallback threshold;
they previously enumerated the rest of that layer before discarding it.
The linear solver only uses the shared-row Woodbury decomposition for long lists
actually reused by multiple policy rows; unique long transitions stay sparse.
Independent crafts use Rayon, while faer is built with `std` and sequential SIMD
matrix operations. The previous nested matrix thread pool stalled in a GEMM
barrier during the full ladder scan; a debugger thread dump showed the barrier
worker spinning while the surrounding Rayon workers slept.
Fossil catalogue scoring also runs within each craft rather than spawning nested
parallel jobs while a shared donor problem is being initialized.

Veiled Chaos caches by the item after clearing its unlocked affixes, so states
with the same retained modifiers share one transition distribution. The repeated
full distribution per starting item was the main source of transition growth.
Each pipeline solve owns its state-specific lock and veil caches, which are
released after solving. Compiled modifier data and existing exact reroll tables
are retained: discarding exact tables would repeat refinement and graph builds.
Pivot replay uses the same temporary-cache lifetime. Its real-modifier draw cache
shares tiers with identical groups, side and notable status, which have identical
draw eligibility despite their different modifier IDs.

`pipeline/protected.rs` owns complete-side preparation and regenerative finishing.
`pipeline/resale.rs` supplies explicit sale quotes and state-specific break-even
premiums; `Context` persists quotes by problem and preserved core. `solve.rs`
connects these recipes to virtual acquisition/recovery decisions. They charge
repeated preparation without adding a multi-item inventory to the search state.

Use the startup example for current performance measurements. Earlier 8–9 second
warm results used looser goals and are not comparable to the complete-goal run.

Bound certification projects the chosen policy onto its own reachable states. Its
interval-policy evaluation preserves probability choices on ties and checks closed
classes. Optimistic refinements batch competing uncertain actions before a graph
rebuild, rather than discovering and refining one profile on each full solve.

Stable direct-search failures (unreachable goals or exhausted existing graph limits)
are cached under the same problem, prices, settings and solver version. Their
incomplete status remains visible and alternative recipes are evaluated. A changed
input retries the solve; singular/numerical failures are not persisted.

Policy action names are shared `Arc<str>` values, including after cache decoding.
Each plan's action dictionary is allocated once; states hold shared references
instead of copying the same currency/action name for every state.

`Pricing` fingerprints its immutable price table once. Component memo keys contain
that fingerprint and a hash of the goal, rather than sorting, serializing and
retaining the whole currency table for every donor variant.

After separating shared reroll rows, the remaining sparse system is decomposed
into strongly connected components. Singleton blocks use scalar substitution;
small cyclic blocks use dense faer LU; large blocks use sparse faer LU. Components
are solved in dependency order, reversed for the transpose system used for visit
counts. This is an exact block triangular solve, including cycles within blocks
and the existing Woodbury correction between shared rerolls.

Internal component edges are discarded after factorization; subsequent substitution
reads only dependencies between components. Consecutive singleton components are
processed together, one matrix column at a time, to keep right-hand-side access local.

Gear and donor specifications retain the shared natural modifier pool. Harvest,
essence and selected fossil weight maps are immutable shared profiles. Fossil
combinations cache their price-independent weights; current currency and clicking
costs are applied when ranking them. Changing a price does not duplicate the weight
tables. Component requirements are ordered canonically before fingerprinting.

Exact roll generation remembers combinatorial shapes that already exceeded its
existing enumeration threshold. Its shape key records the start state, capacities,
notable restrictions, first/main target support and junk-class multiplicities and
support. Changing positive numeric weights changes probabilities but not the states
the enumeration must construct. Known failures take the same bounded fallback without
repeating the failed enumeration. Successful exact shapes remain eligible.

Unveil offer cache keys track occupied groups relevant to that offer side. Good and
blocked tiers exclude the same group; unrelated target statuses do not change offer
odds. Cross-side group conflicts, junk uncertainty and explicit blockers remain in
the key. Three completed offers return before searching for a fourth candidate.

Each explored graph retains the most recent sparse symbolic factorization. Numeric
probabilities may change while dimensions, column pointers and row indices remain
identical; those evaluations reuse faer's symbolic analysis and compute a fresh
numeric factorization. Changed structure replaces the analysis. The cache is local
to that graph and does not retain every policy's matrix.

Optimistic bound evaluation also retains its last numeric factorization. If the
next greedy policy uses the same actions and probabilities, its values have already
been solved exactly; certification reuses those values and that factorization for
the visit counts instead of factoring the identical system again.

The pipeline loads existing plans, prices complete donor recipes, then compares
fractures before the direct craft. A candidate can stop when its certified lower
bound exceeds a complete recipe's certified upper bound. A zero finishing-cost
lower bound also rules out a fractured base that already costs too much. Uncertain
nominal costs alone cannot prune a search. Donor finishing recipes run independently
on the outer Rayon pool; matrix operations remain sequential within each solve.

Pruned candidates retain a lower bound in their estimate and in a `.bound` cache
record. They are neither failed searches nor completed plans. Price and solver
inputs participate in the normal cache key. A changed incumbent or base quote
rechecks the inequality and resumes the search when the stored bound is no longer
enough. JSON exposes the bound and the native UI displays it with `≥`.

Donor sets use ordered maps before constructing continuation distributions. Their
order participates in cache keys, so an unordered map previously caused identical
inputs to generate different continuation entries between runs.

The final local full scan with the current beast quotes produced 437 estimates:
139 direct, 223 fracture, 43 recombination, 14 Awakener and 14 protected recipes.
Four had no priced route: three unreachable shield goals and one belt whose search
reached an existing graph limit. Direct search failed for 55 crafts, but alternative
recipes supplied 51 of those. These failures remain visible.

The controlled comparison uses the same game data (3.29.3.3), ladder and beast
prices, with a completely empty plan directory for each binary. The previous
binary filled it in 290.233 seconds; the block solver took 214.797 seconds, a 26%
reduction. Peak resident memory was 7,642 MiB before and 7,842 MiB after. Cached
startup changed from 14.127 seconds and 7,137 MiB to 11.839 seconds and 6,530 MiB.
The first cached run after the new fill took 12.355 seconds.
Native verification reported 13.5 seconds with the existing full cache and
12.8–13.6 seconds with the newly filled cache; the `≥` comparisons were checked
in the native item pane and craft table.

The subsequent sharing and repeated-work pass took 175.244 seconds with an empty
plan directory and 6,404 MiB peak resident memory. The cached run took 11.177 seconds
and 5,172 MiB. All 437 selected prices still agree with the baseline within 1.4e-10
relative error; the same two numerically tied fracture selections differ. Search
coverage, route counts and the four unresolved goals are unchanged. Donor pricing
contains 9,832 unique problems after canonical ordering, down from 9,905. The full
empty-cache workload still exceeds the ten-second target substantially; whole-item
comparisons consume about 93 seconds and prior preparation/recipe pricing about 81.

All 437 selected prices agree within 1.4e-10 relative error. Two equal-cost fracture
choices selected a different modifier; route category counts stayed unchanged.
The final run pruned 174 direct and 401 fracture alternatives using certificates.
The current six-goal diagnostic retains 47,996 states and fell from 9.0 to 5.1
seconds. Workspace tests (231), dense-reference comparisons and strict Clippy pass.

Cold filling still takes about 3.6 minutes, and the ten-second full-startup target
remains unmet. The solver still builds and explores thousands of donor and native
candidate graphs. Earlier 305-second price-update fills reused some existing plans;
earlier looser-goal or older-price runs are not direct comparisons.
