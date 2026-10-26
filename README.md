# ponder

Find Path of Exile market opportunities from ladder items, practical crafting routes and
trade prices. `ponder` prints JSON; `ponder-app` is the egui desktop client. Both use the
same Rust library.

This is the single project guide: setup, current implementation, invariants, refactor plan
and validation. Fixture READMEs describe only the data they accompany.

## Setup

Requires Rust 1.99 or newer.

```sh
cargo build
./target/debug/ponder data fetch
./target/debug/ponder prices fetch
./target/debug/ponder ladder fetch
cargo run -p ponder-app
cargo test --workspace
```

Run from the project directory, or pass that directory to `ponder-app`. Runtime caches live
under `.cache/`. Game exports come from [repoe-fork](https://github.com/repoe-fork/repoe).
`PONDER_DATA` selects another game-data directory. Tests use the pinned snapshot in
`fixtures/repoe`, currently 3.29.3.3, and never download it.

Development builds optimize the solver and dependencies while keeping desktop rebuilds
short. Use `cargo check` for compiler feedback and release builds for performance measurements.
`cargo build -p ponder-app --timings` reports compilation time separately from solve time.
Keep `target/` and compiler settings stable between comparisons.

Linux needs OpenGL, X11 or Wayland, and `libxkbcommon-x11`. On Windows install the Visual
Studio Build Tools with Desktop development with C++, then Rust's MSVC toolchain. Use a
Windows-side checkout rather than building through `\\wsl$`. The app writes crashes to
`ponder-crash.log`. From WSL, a Windows cross-build uses the `x86_64-pc-windows-gnu` Rust
target and `gcc-mingw-w64-x86-64`.

## Use

Every CLI command prints one JSON document. Progress goes to stderr, `-q` hides it and
`--pretty` indents JSON. Errors return exit code 1.

```sh
ponder ladder fetch
ponder ladder archetypes --group Bow
ponder ladder tiers --group "Gloves (ES)"
ponder ladder prices --group Bow --top 3
ponder ladder crafts
ponder ladder crafts --trade
ponder ladder crafts --starts 10 --top 20
ponder ladder finish item.txt
ponder run https://pobb.in/XXXX build.txt
ponder item item.txt
ponder prices show
ponder data version
```

Build sources can be PoB codes, XML, pobb.in/pastebin links, directories or text files of
links. Use a site's exported PoB code when its page blocks automated access.

The desktop runs pipeline work in the background. Crafts and Overview select products;
Item shows costs, operations, market comparisons and recovery quotes; Recipe shows the
selected route. Fractured bases, Products & starts and Recombinator show alternatives.
Paste an in-game item to find possible finishes. Ctrl+R runs the pipeline. Layout,
filters, sorting and run settings persist.

Goals retain observed combinations of useful stats and acceptable floors. Stronger rolls
qualify. Single elemental resistances can be interchangeable; suppression, chaos resistance,
hybrid rolls and all-resistance remain separate. `--exact` requests the item's specific
modifiers. Base groups share native pools; trade and craft comparisons use the same goal.

Trade uses `POESESSID` from the environment or ignored `.env`. The client follows the
server's rate-limit headers, caches responses and applies `--trade-budget`. Price downloads
include currency, essences, fossils, resonators, beasts and cluster bases.

## Current behavior and ownership

Costs are expected currency plus operation time priced by `Settings::chaos_per_hour`.
`PONDER_CHAOS_PER_HOUR` sets the default, currently 1500c/h; zero disables time pricing.
This preference is explicit and adjustable. There is no variance premium or currency-count
cutoff. A buy or fracture becomes attractive through its complete acquisition/finishing cost.
An unquoted acquisition is not evidence that a cheap listing exists.

Craft of Exile is the mechanics reference for this refactor. Its
[fossil rules](https://www.craftofexile.com/how-to-use) add boosts, multiply reductions and
respect blocks. The implementation retains boosts and reductions separately, including
when one fossil supplies both. Its [metamod reference](https://www.craftofexile.com/metamods)
is operation-specific: protecting one side does not mean every currency becomes a reset.

| Responsibility | Current source |
|---|---|
| Game exports, item/PoB parsing, modifier matching | `ponder-game` |
| HTTP, ladder input, market prices and trade | `ponder-net` |
| Candidate goals and available operations | `ponder/src/{demand,gear,actions}` |
| Goal definition, item projection and identity | `ponder-core/src/problem.rs` |
| Pools, rolls, effects and unveil outcomes | `ponder-core/src/problem/` |
| Distributions and probability arithmetic | `ponder-core/src/transition.rs` |
| Single-item search and policy evaluation | `ponder-core/src/{search,linear,solve}.rs` |
| Real-mod replay and rendering | `ponder-core/src/{mechanics,simulate,steps}.rs` |
| Donor joins and self-fracturing | `ponder/src/{recomb,awakener}.rs`, `ponder-core/src/fracture.rs` |
| Route orchestration and shared solves | `ponder/src/pipeline/solving.rs` |
| Reporting, market queries and finishing listings | `ponder/src/pipeline/` |
| Entry points | `ponder-cli`, `ponder-app` |

Probability arithmetic, goal projection, fingerprint helpers, compound effects, block
factorization and route orchestration have been consolidated into their owning units.
File consolidation reduces navigation; it does not remove the underlying search work.

Current search still constructs the reachable compressed graph before policy iteration.
Fresh rerolls share distributions. Refinement patches known lists when possible and
rebuilds when their support changes. Some transitions remain estimated because the state
forgets junk identities. `Plan::approximate` is not a bound on the error, and exact roll
probabilities do not establish that every useful action or route was considered.

General temporary bench crafts before slams and Harvest add/remove are absent. Unveil
blockers remain special cases; surviving metamods can now be removed before reveal. Multiple
intermediate crafts, elevation, imprint/restore, preview locks and influenced/exclusive
recombination are not generally supported. Protected route generation excludes fractured
items. These are capability gaps, not proof that such crafts are impossible.

## Why uncached planning is expensive

A plan cache skips computation, not just reads. One product generates direct and fracture
problems, donor goals, joins, finishing continuations and refinements. An earlier instrumented
437-item scan recorded approximately 14,000 policy evaluations, including refinement passes.

Most expensive work is paid before a policy is selected. A five-target diagnostic explored
75,063 states although its final policy had only 824 states with expected visits above
1e-9. That route never unveiled, but enumeration still attempted over half a million unveil
choices. More Rust optimization cannot remove that multiplicative formulation cost.

Adding a mechanic can also change the state representation. Unveil blocker groups currently
share tracked status slots with actual targets. Each extra independent status can introduce
up to three cases throughout the graph. A new weight profile can split formerly identical
junk classes and enlarge exact-roll dynamic programming. Donor variants multiply those
costs across products. Parallelism then raises aggregate allocation and memory pressure.

This explains recurring regressions: eligibility, candidate generation, compression, prices
and search breadth are coupled. Many local tests can still pass while a full workload grows
substantially. Old contradictory descriptions of certificates compounded the problem.
The current solver does not return lower/upper cost certificates or prove global PoE
optimality.

## Invariants for the refactor

1. **The goal does not move.** Tier floors, required stats, base constraints, fixed mods and
   finishing reservations remain identical across competing routes. A faster solve may not
   silently weaken the item.
2. **Mechanics have one authority.** Legality, eligible groups, slot limits and protection
   follow the pinned data and Craft of Exile. Enumeration and replay use the same rule
   predicates. A currency name or display label never changes behavior.
3. **Item facts belong to the item.** Crafted/fractured provenance, occupied groups and
   pending observations are facts; which recipes happen to be offered is not. A new operation
   can require a finer projection when it reads previously ignored facts. That dependency
   and resulting split must be explicit; listing a blocker must not manufacture goal slots.
4. **Compression preserves future behavior or declares approximation.** States may be merged
   exactly only if goal status, available actions, costs and probabilities into every retained
   class agree. Similar target counts or similar weights alone are insufficient.
5. **Every discarded choice has a reason.** Record impossibility, equivalence, proven
   dominance or a lower-bound comparison. A top-N list and a repetition cap are heuristics,
   not proofs. Resource limits report incomplete work rather than redefining legal crafts.
6. **Information arrives in order.** Block before offers are drawn; choose after observing
   them. In general `E[min V]` differs from `min E[V]`. Collapsing observations must preserve
   that ordering and the executable decisions needed by replay.
7. **Complete economics compose.** Include preparation, bases, cleanup, joins, retries,
   salvage and time exactly once. Stage costs can be added independently only when their
   hand-off distributions and recovery contracts justify it. Cheapest preparation alone need
   not produce the cheapest finished item.
8. **Caches cannot determine meaning.** Cold and warm runs must agree on goals, coverage and
   costs within declared numerical tolerance. Keys cover semantic inputs. An incomplete
   attempt can be memoized with its search configuration; it is not an impossibility proof.
9. **Search optimization does not change legality.** Selecting an optimal policy may discard
   dominated actions. Evaluating or replaying a supplied legal policy must still allow them.
10. **Cost assumptions are explicit.** Nonnegative-cost pruning is disabled for sale/recovery
    contexts with negative credits. No repeatable profitable cycle may be assigned a fake
    finite finishing cost. Prices rank choices; they do not change physical outcomes. Time
    value is a declared user preference, not something derivable from currency mechanics.

These invariants constrain behavior. They do not prescribe a particular matrix library,
search algorithm, number of modules or inventory of hardcoded human recipes.

## Search reductions and their proof obligations

- **Exact action equivalence.** For the same state, identical successor distributions and
  availability with no greater cost allow the dearer action to be removed. Matching action
  names or fossil ingredients is insufficient. Uniform rescaling of all eligible draw
  weights preserves a draw kernel; rounding-dependent approximations require care.
- **Zero-cost completion.** If an available action finishes with certainty for zero cost,
  and all future costs are nonnegative, its value is zero and alternatives cannot improve
  it. This reduction is implemented for observed unveil choices. It is disabled for negative
  credit contexts and does not restrict explicit policy evaluation.
- **Backward relevance and exchangeability.** Start with facts read by the goal and legal
  future effects. Close that set over group conflicts, slot occupancy, tags, protection and
  draw weights. Forget only facts outside this dependency closure. Groups may be counted
  together only when they are exchangeable under every remaining operation. A protected
  side can still affect another side through shared groups; retain those dependencies.
- **Bounds from relaxed problems.** Let a relaxation remove constraints, preserve all real
  options and avoid increasing any real cost. Its optimum is a lower bound. A complete
  feasible policy supplies an upper bound under the same probability model. Prune only when
  the candidate lower bound is strictly above the incumbent upper bound, allowing numerical
  tolerance. A currently expensive policy is not a lower bound. Estimated probabilities and
  missing acquisition prices cannot silently become certificates.
- **Dominance of richer starts.** A start with extra wanted mods is not automatically better:
  it can consume slots or prevent a needed craft. Prove a legal simulation from the richer
  state to the poorer state's choices with no extra cost before using this reduction.
- **Commuting effects.** Combine or reorder operations only when their preconditions, costs
  and complete outcome distributions commute. Paying for a lock twice or drawing offers
  before blocking violates this requirement.
- **Acquisition crossover.** For two fixed feasible recipes with currency costs `C1,C2` and
  times `T1,T2` in hours, prefer the first when `C1 + h*T1 < C2 + h*T2`, where `h` is the
  user's time value. If `T1 > T2`, their break-even rate is `(C2-C1)/(T1-T2)`. A recipe that
  costs no more currency and takes no more time dominates independently of that preference,
  provided its goal, outcome and recovery contracts agree. This derives buy/fracture signals
  from actual alternatives rather than an operation-count threshold. Replanning may change
  the policies, so one pair's crossover is not a universal threshold for all crafts.

[Stochastic bisimulation and model minimization](https://cs.brown.edu/people/tdean/publications/archive/GivanetalAIJ-03.pdf)
provides the exact state-merging criterion. It is a specification for equivalence, not a
reason to enumerate the entire game and minimize it afterward.

[LAO*](https://ai.dmi.unibas.ch/research/reading_group/hansen-zilberstein-aij2001.pdf)
expands a partial solution graph from the origin and supports loops. It can retain policy
iteration as its local evaluator. Its guarantees require admissible estimates and the
appropriate stochastic-shortest-path assumptions.
[Labeled RTDP](https://bonetblai.github.io/reports/lrtdp.pdf) also avoids exhaustive evaluation,
but sampling alone is a poor initial fit for extremely rare crafting outcomes. These papers
motivate the search boundary; their guarantees do not automatically apply to our estimated
kernels, zero-cost observation cycles or negative sale credits.

[CG-iLAO*, Artificial Intelligence 2026](https://arxiv.org/abs/2604.01855) extends iLAO*
with partial action expansion. Deferred actions can return when value changes make them
competitive. Its separation oracle tracks potentially violated Bellman constraints instead
of repeatedly checking every action. The reported average speedup over iLAO* is 2.8 times
on the paper's benchmarks; it is not a prediction for this project. Its ordinary one-step
expansion still scores all applicable actions initially. Expensive outcome construction
therefore also needs shared kernels and sound cheap bounds. Internal values can decrease
when omitted actions return; they are not automatically admissible pruning bounds.

Use iLAO* as the first comparison mode of one partial-graph engine, then add CG-iLAO*'s
action expansion and dependency tracking in that engine. Compare states, generated kernels,
Q-value computations, memory and wall time before choosing the production mode. Preserve
proper-policy handling, deterministic ties and possible successor support. Do not copy a
fixed dead-end penalty into crafting economics or assume a small Bellman residual gives the
same small origin-cost error across very long retry loops.

## Objectives and optional learning

Keep expected currency, per-currency usage, operation counts and time as separate measures
of a policy. The current objective is expected currency plus a declared time value.
Weighted currency/click objectives remain ordinary SSPs; the weights express preferences.
Several weight settings can expose useful tradeoffs, but do not enumerate every Pareto
policy. [CARL](https://schmalz.cc/carl/) is relevant if expected click/time budgets become
constraints instead. Such constraints are expectations, not guarantees for an individual
craft, and optimal constrained policies can require randomization.

Evaluate variance and budget-exceedance probabilities on complete candidate policies before
considering risk optimization. For a fixed finite proper policy with deterministic immediate
cost vector c and transient transition matrix P, its mean m satisfies `(I-P)m=c` and second
moment u satisfies `(I-P)u=c*c+2*c*(P*m)`, with elementwise multiplication. Variance is
`u-m*m`. Reuse the same factorization. Random preparation costs, stage handoffs and recovery
need their actual joint moments; replacing them by their means destroys tail information.
Optimizing variance is a different problem from evaluating it: total-cost variance is not
an additive action cost. [Variance-sensitive SSP research](https://arxiv.org/abs/2204.12280)
shows that general variants can require memory and substantially harder optimization.
[CVaR SSP research](https://arxiv.org/abs/2203.01640) addresses bad spending tails directly,
but needs its own treatment rather than another coefficient in the ordinary Bellman backup.
No variance objective or new risk default is currently enabled.

Optional neural guidance can rank expansions or propose complete policies, trained from
verified solves and simulator experience conditioned on goals, prices and preferences.
The rules layer still determines legality and probabilities; verified policy evaluation
determines incumbent costs. An unverified learned value is not an admissible lower bound
and cannot justify discarding alternatives. This is single-agent learning through simulation;
no opponent is needed. Introduce it only after a deterministic focused-search baseline shows
that inference saves more work than it adds.

## Replacement plan and deletion boundaries

The remaining overhaul is a replacement of responsibilities, not another parallel solver
framework. Target units below are ownership boundaries; their file count follows cohesion.

1. **Separate rules, item facts and goals.** Build eligibility and deterministic edits from
   one rules layer. Compile goal-relevant, exchangeable classes for each local stage. Keep
   real modifier IDs where group identity affects future draws. Acquisition/recovery now use
   explicit state kinds. Remove target-index-dependent
   crafted/blocker coordinates after general crafted state replaces them.
2. **Make rolls and observations reusable kernels.** One draw/remove/apply implementation
   supports enumeration and sampling. Cache a fresh reset once per retained context, not once
   per irrelevant item state. A locked or side-only reroll retains a context and is not a
   universal reset. Evaluate unveil offers as local observation/choice nodes; preserve the
   post-veil blocker decision and every legal continuation. Retire duplicated mechanics and
   special unveil blocker plumbing as their replacements become complete.
3. **Replace full BFS with focused expansion.** Implement an iLAO* frontier on `Model`, then
   CG-iLAO* action expansion in the same engine, initially reusing the present policy evaluator.
   Seed complete guide methods as incumbents. Expand only unresolved states on competitive partial policies. Generate
   candidates lazily; use relaxed bounds to discard them. Preserve the eager implementation
   temporarily as a comparison mode, then delete it after the acceptance cases and full
   workload pass. Handle negative-credit contexts separately until their bounds are sound.
4. **Use one component/stage planner.** Direct crafts, fractured starts, bought components,
   protected sides, recombination and Awakener joins become method providers over shared
   components. A hand-off carries its retained facts, possible outcomes and recovery cost.
   If two preparations leave different tails, retain those differences or price their
   continuations before discarding one. Remove separate single-item solving paths and
   repeated per-route catalogue construction. Add fractured staging through this contract,
   not by deleting a guard and assuming the recovery semantics still work.
5. **Retire heuristics and unused machinery.** The current fossil shortlist of ten, the 2.3
   affixes-per-side scoring approximation, donor padding caps and exact-roll work limits are
   not game laws. Replace candidate cutoffs with lazy generation plus equivalence/bounds;
   keep numerical work limits as explicit incomplete-result budgets. Remove interval vectors
   only after preserving exactness and possible successor support, including outcomes with
   zero nominal but nonzero possible probability. Replace the specialized matrix machinery
   only if local kernels and the focused search make its cyclic systems small in measurements.

A practical destination is four core units: **item/goal**, **rules/kernels**, **search**,
**plan/replay**; and three planning units: **components/methods**, **market**, **reporting**.
These are proposed replacements, not a claim that the current code already has these seams.
A module is removed when its responsibility has moved; a forwarding file is unnecessary.

## Validation and change discipline

`fixtures/crafting-guides.json` records source, date, pinned patch, item goals, reference
methods and exact scope. The current cases cover a three-mod Focused amulet (T1 mana needs
item level 86), minion-wand gem-prefix preparation, and fractured-life boots through Spite
suffix preparation. They omit the wand's unveiling/Trigger finish and the boots' chaos,
movement-speed and final craft. The four-fossil reference is built from its ingredients,
independently of the production shortlist. Reference costs use our model; they are not
externally measured game odds. A 5% comparison tolerance is a regression guard for estimated
probabilities, not an optimality certificate.

Guide cases should expand to complete recipes as missing mechanics are implemented. Prefer
legal operation traces, complete goals, price perturbations, buy/craft crossover checks and
operation counts over a second universal solver. Keep small mechanics/numerical checks for
specific contracts such as group exclusion, retry accounting and policy-evaluation legality.

For each semantic or search change:

- Retain one baseline binary, pinned inputs and a minimal slow/failing case.
- Separate mechanics changes from search reductions when comparing outputs.
- Report coverage, costs, time and work counts together. Cache mode, objective, worker count,
  compiler profile and inputs belong to the measurement. Track graph builds, states, action
  pairs, generated outcomes, refinements and peak memory as the benchmark grows.
- When a case regresses, identify the first growing count and the responsible new dependency.
  Repeated broad runs, raising limits and changing tolerances do not explain the regression.
- A reproduction is the input plus its semantic configuration, never an unrelated cached plan.
  After replacing a path, delete the obsolete implementation and its architecture-specific
  expectations. Keep observable acceptance requirements.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p ponder --test crafting_guides -- --nocapture
cargo run --release -p ponder --example startup -- --full --cold --budget-seconds 10
```

`--cold` disables the plan cache while retaining cached game, ladder and price inputs.
It excludes fresh network acquisition. To measure cache fill and warm startup, use
`--plan-cache /tmp/ponder-benchmark-plans` twice instead. The ten-second full cold target is
still unmet; passing the small guide suite does not satisfy it.

On 2026-10-25 a matched comparison before the fossil correction took 70.048s before the
zero-cost unveil reduction and 67.506s afterward. All 437 route selections and coverage
results were identical, with zero cost differences. This is a modest timing change within
ordinary system variation; the 75,063-state diagnostic still explored 75,063 states.
The result supports the reduction's preservation claim and confirms that it does not solve
the eager-enumeration bottleneck.

After additive fossil weighting and consolidation, a fresh plan-cache fill took 59.217s;
the subsequent warm startup took 8.061s. Both produced 437 estimates with 433 priced routes,
and all 437 identities, selected costs and routes agreed between cold and warm runs. The
physics change means this is not a semantics-preserving speedup comparison with the older
fossil model. The full cold ten-second target remains unmet.

## Desktop visual verification in WSL

WSLg supplies an X11 server even when the terminal has no `DISPLAY` variable.
Check that `/mnt/wslg/.X11-unix/X0` exists. The capture script connects to `:0`
automatically in that case. It needs Python 3, `libX11`, and FFmpeg with `x11grab`;
clicking and scrolling also need `libXtst`. No Python packages are required.

Build and launch the native app with X11 and a separate settings directory so the
verification session has its own layout and filters:

```sh
cargo build --release -p ponder-app
mkdir -p target/visual-check/data
DISPLAY=:0 WAYLAND_DISPLAY= XDG_DATA_HOME="$PWD/target/visual-check/data" \
  target/release/ponder-app > target/visual-check/app.log 2>&1 &
app_pid=$!
```

Wait for game data to load, click **Run ladder**, and wait for results. The app uses
the project's existing data, ladder, price, and plan caches. Keep trade pricing
disabled for a local UI check.

Capture only that process's window, resize it, or send a click or scroll before
capturing. Coordinates are pixels relative to the app window; positive scroll
counts move right/down and negative counts move left/up. Input brings the app
window to the foreground and moves the pointer.

```sh
uv run scripts/capture-window.py --pid "$app_pid" \
  --output target/visual-check/wide.png --size 1500 900
uv run scripts/capture-window.py --pid "$app_pid" \
  --output target/visual-check/narrow.png --size 1000 650
uv run scripts/capture-window.py --pid "$app_pid" \
  --output target/visual-check/narrow-right.png --scroll 400 300 12 0
uv run scripts/capture-window.py --pid "$app_pid" \
  --output target/visual-check/narrow-bottom.png --scroll 400 300 0 12
uv run scripts/capture-window.py --pid "$app_pid" \
  --output target/visual-check/selected.png --click 400 150
```

Inspect the PNGs with the agent's image viewer. For table scrolling, compare the leftmost and rightmost
columns, header/body alignment, the last visible row, selection, and sorting.
Screenshots and settings stay under the ignored `target/` directory. Close the
verification app when finished.

Without WSLg, run the same workflow under an Xvfb server with GLX/Mesa support.
Set `DISPLAY` to that server and keep it running for both the app and capture
commands; a running X server is required.

## Handoff: 2026-10-25

- Checkpoint validation: all 232 workspace tests and strict Clippy pass.
- Production defaults to eager policy iteration. iLAO* and CG-iLAO* are implemented comparison
  modes in `ponder-core/src/search.rs`; they are not ready to replace the default. Zero-bound
  experiments expanded about 42,000 states on the 75,063-state baseline and were slower.
- Added explicit acquisition/recovery state kinds, shared observation legality, removal of a
  surviving metamod before unveiling, and structural proper-policy checks before evaluation.
  Existing junk compression and special unveil blockers still need the larger mechanics refactor.
- Currency plus time remains the objective. `Plan::risk` and `finishing_policy_risk` report
  point-model moments for the remaining policy. Summarized random recovery lacks joint cost
  information and returns no risk estimate. Operations count action applications, not literal
  mouse clicks. No variance penalty or repetition cutoff is enabled.
- Recombination pruning now separates feasible finishing costs from optimistic preparation
  bounds. Missing finishing policies cannot supply a free feasible incumbent.
- Resume with `cargo test --workspace` and the fixture comparison in `tests/simulator.rs`.
  Benchmark the saved slow case using `startup --solve-case PATH --algorithm eager|ilao|cg`.
  The next priority is an admissible crafting relaxation and accurate total-work counters;
  keep the eager default until focused search wins the cold workload. Full-file consolidation,
  general bench blockers and complete acquisition-risk composition remain unfinished.
