# Ponder: research results and remaining work

Updated 2026-10-27 against pinned RePoE 3.29.3.3. This is the consolidated
record of solver research, correctness audits, recipe experiments and candidate
generation. It replaces the separate research reports, comparison JSON files,
performance diary and optimization plan. [SPEC.md](../SPEC.md) defines the
architecture; [the oracle index](crafting-ama-recipes.md) preserves the authored
reference recipes.

## Current position

- The new rules, solver and planner can solve the bow within the normal
  200,000-state limit. The remaining large crafting problems are slow, not all
  broken: bow about 16s, sceptre about 10s, recombined regalia about 9s.
- Common cluster-notable candidates mined from the cached ladder solve in
  milliseconds. Candidate mining already exists in the older implementation;
  its connection to the new planner is not yet a production feature.
- Unrestricted optimization spends considerable work on alternative and recovery
  policies that do not belong in a short main recipe. Local repairs remain
  valuable. Detailed recovery can be requested for the actual item instead.
- The newer planner returns a policy, not a complete human-readable guide.
  Compact recipe discovery and rendering remain substantial work.
- The all-guides-under-one-second target is incomplete. The latest full cohort
  predates the successful bow fallback and has not been repeated on every
  subsequent working-tree experiment.

## Candidate, start, state graph and recipe

The intended flow is:

```text
ladder items -> candidate goal B -> methods, donors and subgoals
                                      |
supplied item A or market purchases -> planner / solver -> recipe
                                      |
                         actual unexpected item -> recovery request
```

B is a goal predicate: base/enchant, level, acceptable modifier tiers, sources,
influences and slot requirements. It describes a set of acceptable finished
items, rather than a single exact roll. A is either the actual owned item or a
market boundary where buying a base, partial item or finished item competes with
crafting. No supplied item does not mean free starting material.

The state graph contains item facts relevant to the goal and available methods,
including failures, retries, restarts and dead ends. It is not necessary or
desirable to enumerate every possible state between A and B. The chosen policy
selects an action in each represented situation; a recipe must express its
important loops, stop conditions and fallbacks.

An owned item is available once. Restarting must acquire a replacement from the
market; treating the owned item as an endlessly replenishable zero-price supply
would create a loophole. The new session API currently exposes priced supplies;
the owned-item lifecycle needs explicit integration. The older pipeline already
has finishing-from-an-item functionality.

There are two candidate-generation problems: choosing useful final goals from
demand, and choosing useful intermediate/donor goals from crafting mechanics.
Neither is the low-level solver's job. A reliable solver plus a candidate miner
provides the core of the product, with market integration and readable recipes
still required. A cheap craft alone is not evidence that it will sell profitably.

## Fresh bow and cluster checks

Release builds run serially with four Rayon workers, an 8 GB address-space
guard, a 120-second invocation limit and the normal 200,000-state cap. Game data
is loaded before per-goal timing; preparation and solving are included, while
simulation and network are excluded. These are single screens, not latency
guarantees.

| Existing guide | Explored states | Retained policy states | Best-plan time | Model cost |
| --- | ---: | ---: | ---: | ---: |
| Elemental bow | 77,699 | 1,451 | 16.372s | 555.00c |
| Supplied fractured elemental bow | 12,886 | 86 | 0.130s | 118.80c |
| Overlord + Fuel the Fight large cluster | 106 | 30 | 0.004s | 1,403.39c |
| Supplied fractured Doryani cold cluster | 225 | 32 | 0.004s | 960.33c |

All four pass their guide checks and 500 concrete-item runs with zero lost runs.
Supplied-fracture fixtures price the prerequisite at zero and measure subsequent
crafting, not acquisition of that starting item.

The full bow first exceeds the budget under eager expansion. A one-side
optimistic bound then solves an abstract problem of 30,844 states, followed by
the bounded full search shown above. Each solve has the normal cap; total work
across attempts is greater than the printed final search count. The result
matches the earlier larger-budget reference within numerical tolerance.
This works by avoiding expansions, not by raising the limit or enumerating the
entire full state space. The fallback overhead still contributes to latency.

The bow policy uses qualifying ordinary-tier Anger/Wrath modifiers, a
cold/lightning donor, recombination and bench finishing. Tiered essence-forced
modifiers are not automatically essence-exclusive. The goal has three elemental
prefixes plus a hybrid suffix; it never requires four prefixes.

### Actual cached ladder candidates

The cached 2026-10-23 snapshot contains 2,502 characters and 7,658 recognized
cluster items. The older miner produces 503 cluster archetypes at its existing
five-character support setting. A temporary adapter passed the five leading
notable combinations to the new planner, using observed size, enchant and modal
item level. No guide step sequence was supplied.

| Notables | Context: size, enchant, passives, level | Characters | States | Time | Model cost |
| --- | --- | ---: | ---: | ---: | ---: |
| Enduring Composure | Small, armour, 2, 83 | 149 | 20 | 2.5ms | 37.87c |
| Assert Dominance + Towering Threat | Medium, area damage, 4, 83 | 77 | 96 | 2.7ms | 454.16c |
| Unholy Grace + Unwaveringly Evil + Wicked Pall | Large, chaos damage, 8, 83 | 75 | 241 | 3.8ms | 862.83c |
| Blanketed Snow + Prismatic Heart + Widespread Destruction | Large, cold damage, 8, 83 | 58 | 229 | 4.5ms | 1,601.71c |
| Repeater + Streamlined | Medium, projectile damage, 5, 84 | 55 | 38 | 2.4ms | 30.36c |

Each finishes 500 concrete-item simulations with zero lost runs and passes the
existing model-versus-simulation cost check. These are notable-combination smoke
tests with fixture prices and a 5c base proxy, not live market valuations.
Passive count is recorded context, not an additional affix constraint or a
verified acquisition price. High-tier small-passive combinations were not tested
in this sample.

## What is implemented, and what remains disconnected

| Layer | Current implementation | Remaining work |
| --- | --- | --- |
| Ladder ingestion | Cached character items with modifier IDs in `ponder-net` | Version and sampling-quality controls |
| Candidate mining/merging | `ponder::mine`, `ponder::demand`; observed combinations, tier demand, character support and deduplication | Port/adapt goals to new types without combining incompatible observations |
| Methods and subgoals | Explicit currencies/catalogue operations, supplies and recombination partners in the new planner | General mechanic-driven intermediate/donor generation and shared AND/OR planning |
| Rules and solving | `ponder-rules`, `ponder-solve`, `ponder-plan` | Remaining rule gaps, scale and compact strategy discovery |
| Recipe output | New planner has policies, traces and fixed-guide evaluation; older core has a grouped renderer | Verified short recipes and explicit recovery boundaries |
| Product pipeline | CLI/app still use the older planner | Connect candidate generation, new planning, market inputs and rendering |

The merger should retain joint observed combinations, compatible bases and
enchant pools, tier/source requirements and distinct-character support. It must
not union every popular modifier into an impossible item. Affix limits, modifier
groups, influence restrictions and special origins need feasibility checks.
Generated goals should carry their evidence and provenance.

Guide fixtures provide goals, available mechanics and named input/donor products;
the optimizer chooses its operations and failure handling. Their written step
sequences are comparison policies, not instructions imposed on the optimizer.
Independent donor/subgoal discovery is still a separate planner responsibility.

## Solver correctness and retained improvements

The earlier policy-only admission scheme was unsound: a four-state example
returned cost 11 instead of 2 because an improvement sat behind an unselected
route. Its early subsecond claims are superseded. The exact path now checks
omitted actions outside the chosen policy too.

With nonnegative costs and free restart, the solver evaluates a proper feasible
policy, checks global Bellman inequalities and admits violated actions. Unknown
continuations use zero or an admissible lower bound, never an unverified restart
upper value. The accepted policy must be executable without unexpanded tips.
Admissible boundary values need not be consistent if expanded-domain inequalities
and the stopped-policy boundary argument are maintained. Warm numerical seeds
are not validated value certificates.

Useful improvements currently in the working tree include:

- Retaining graphs across price ceilings and expanding bounded batches of 256.
- Iterator-based Tarjan, dense local positions, direct singleton evaluation and
  single-action fast paths.
- Forward-triangular sparse preconditioning; the later ILU(0) trial has mixed
  timings and is not established as a suite-wide win. Accuracy thresholds remain
  unchanged, including final Gauss-Seidel checks.
- Proper-exit reachability witnesses before expensive global pruning.
- Exact batched recombination expectations, Boolean missing-successor support,
  and direct compact linking. Donor correlations, both heirs and global
  exclusivity are preserved.
- Compact aggregation before full item construction; three policy improvements
  per admission batch and ten on complete graphs.
- A one-side optimistic fallback after a state-budget failure. Eager use of this
  bound regressed ordinary cases severely and was removed.
- Full-request session caching, including exact prices, methods and source item
  details. This accelerates repeat requests, not new cold goals.

Historical unchanged-cost/state-count improvements include mana wand
822ms -> 308ms, dex/resist ring 1,285ms -> 309ms, and minion wand
2,194ms -> 766ms. The last complete cohort before the current bow fallback had
43 passing active guides, 22 inactive and one bow failure; 38 passing plans were
below one second including donors, 18 final solves below 100ms and 15 below
50ms. Targeted bow success does not substitute for a new complete sweep.

### Current bottleneck diagnosis

Instrumented sceptre measurements put 2.04s in policy-component reconstruction,
1.46s in singleton evaluation, 1.92s in action rescanning and 1.43s in sparse
numerical solving. Instrumentation adds overhead. The sparse solves take at most
23 iterations: numerical convergence is not the dominant remaining issue.
Ten global solves and 31 Newton steps process 5.23 million repeated state rows.
The final 308 admissions trigger 1.86s of value work for a retrospective
0.0029c improvement. That observed delta is not a valid stopping certificate.

The regalia and bow spend about 2.58s and 3.12s constructing expectation tables;
bow sparse numerical work is only 0.22s. Their bottlenecks are not interchangeable.

## Discovery, local repair and recovery on demand

The agreed product direction is a short main recipe with worthwhile local
repairs, plus recovery guidance from an actual unexpected item. Roughly 1% more
expected cost is acceptable for a much simpler recipe with comparable spread.
There is no fixed 99/1 coverage target or per-roll rarity cutoff.

Ordinary misses and retries remain priced. Expected benefit of a special repair
depends on its encounter frequency and savings over an ordinary fallback.
Continuation bounds can limit the value of further repair search; they do not
automatically prove global optimality over every alternative recipe.

The sceptre's largest chosen-policy cycle has 8,744 states and connects
preparation, recombination, fracturing and unveiling. A possible path is completed
suffixes -> veiled chaos -> awkward result -> annul loses a wanted suffix ->
recombination -> Scorn preparation. Freezing the guide's stages can exclude such
choices; earlier stage decomposition increased some costs by 29–38%.

Some distinctions are useful, not arbitrary: when fracturing can permanently
lock a prefix that conflicts with the required fire unveil, the preferred action
can change from fracturing to veiled chaos. In 500 reference runs, 332 of 508
fractures occur with the suffix lock present. This behavior follows the current
fractured-craft assumption; it is not solely an off-policy theoretical tail.

### Bounded sceptre comparisons

| Strategy family | Model mean | Mean change | Cost SD change | Operations | Explored states | Solver screen |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Unrestricted reference | 3,563.00c | — | — | 230.58 | 138,593 | 9.83s |
| Exclude fractured-item recombination | 3,575.63c | +0.35% | +0.54% | 240.32 | 138,560 | 5.20s |
| Fracture after required natural mods; no fractured recombination | 3,617.62c | +1.53% | +2.03% | 249.25 | 108,613 | 4.13s |
| Exclude fracturing | 3,718.64c | +4.37% | +10.86% | 254.66 | 20,908 | 1.51s |

These are diagnostic strategy restrictions, not production mechanic bans.
Timings are single screens; the reference is an earlier clean screen. All
candidates retain the same goal, prices, donors and numerical tolerances.
Each has numerical first/second-moment evaluation, 100,000 model simulations and
500 concrete-item runs with no loss.

The best restricted family retains exalts, annuls, locks, unveiling and useful
fractures. Its variance rises 1.08%; sampled p95 cost is about 8,098c versus
8,044c, with both sampled p99 values about 11,558c. Its selected policy shrinks
to 4,211 states, but discovery still enumerates almost the whole original graph.

Simply restarting where the original policy would recombine a fractured item
costs 3,586.44c (+0.66%), retaining 4,558 policy states and 99,295 outcomes versus
796,177. This is an evaluation of an already known policy, not fast discovery.
Restarting on every unwanted fracture instead costs 3,921.04c (+10.05%): many
misses allow valuable ordinary continuation and should not be abandoned.

Lossless diagnostic decision trees have 586 leaves for the original policy,
317 without fractured recombination, 259 with the fracture checkpoint and 174
without fracturing. These are not minimum-size proofs or finished human guides.
The next architectural prototype should discover compact recipe programs before
constructing all bespoke recovery variants, rather than solve everything and
hide branches afterward.

## Correctness audits and remaining rule risks

Implemented corrections include the global one-exclusive recombination cap,
ordinary-tier versus essence-only distinction, exclusive count-slot behavior,
chosen-heir fracture inheritance, native/influence eligibility and magic/rare
input requirements. Adding influence to fractured items is rejected. Fracturing
ignores both locks and includes crafted modifiers, excluding veil placeholders.

Small declared-model checks give 1/3 for two isolated natural prefixes both
surviving, 10% for fire plus cold/lightning retaining all three, and 30% for
fire/cold plus cold/lightning retaining the three distinct prefixes. Two
exclusive essence modifiers cannot both survive. These probabilities use the
empirical count table, not verified exact live-game constants.

Remaining priorities:

- Veiled Exalted currently only adds a veil. The remove-then-add behavior in
  [GGG's 3.24 notes](https://www.pathofexile.com/forum/view-thread/3496784) and
  [3.26 notes](https://www.pathofexile.com/forum/view-thread/3787013) needs its
  own independent correction and regression.
- Attack/caster metacraft protection during annulment needs an independent rule
  test; current generic removal does not implement that filter.
- Check cross-side modifier-group conflicts, crafted-family exclusivity
  exceptions and inferred recombination weighting against legal concrete cases.
- Bestiary/imprints, cross-base donor handling, expanded-affix bases and
  numerical rolls within a tier remain incomplete. Synthesised/corrupted state
  restrictions need explicit representation before accepting those inputs.
- Agreement between simulator and compressed model cannot detect a shared game
  rule error. Keep independent analytic examples and external source checks.

## Experiments not to repeat unchanged

| Approach | Result / lesson |
| --- | --- |
| Scalar single-RHS policy improvement | Improved sceptre but regressed bow/axe; extra evaluation and scheduling outweighed savings |
| Explicit prefix/suffix intermediate solver states | Fewer edges, more state/context and evaluation work; no general P×S -> P+S shortcut |
| Symmetric Gauss-Seidel preconditioning | Additional sweep did not improve total time |
| Complete unchanged-component cache | Most expensive components change; identical complete answers accounted for little time |
| Warm numerical seeds | Small or mixed benefit; not an optimality certificate |
| Policy-only deferred admission | Incorrect, with the 11-versus-2 counterexample |
| Optimistic immediate-cost placeholders everywhere | Correct but 131 global solves on the axe; weak bounds cause sibling-by-sibling expansion |
| Eager one-side/zero lower-bound search | Repeated graph work and weak bounds caused severe regressions |
| Fixed guide-stage decomposition | Restricts opportunistic routes and recovery; can cost materially more |
| Deterministic-chain/hub substitution | Preserved checked values but regressed runtime |
| Repeated numeric support queries / long-lived kernel caches / packed keys | Overhead or no useful latency improvement |
| Broad pool merging | Can break fossil probabilities; only merge states when all relevant operations agree |

Research forecasts of sixfold gains, a seven-million-edge result, and over 90%
zero-savings matrix cells were not established. Known-state counts are not
matrix-cell sparsity measurements. Published solver speedups are not forecasts
for these crafting graphs.

Useful primary references are [constraint generation / CG-iLAO*](https://arxiv.org/abs/2401.14636),
[topological value iteration](https://arxiv.org/abs/1401.3910),
[lazy expensive-transition search](https://arxiv.org/abs/2506.00285), and
[fixed-point certificates](https://arxiv.org/abs/2501.11467).
The latter is by Chatterjee, Quatmann, Schäffeler, Weininger, Winkler and Zilken;
earlier notes misattributed it.

## Independent oracle leads

The corpus has 66 TOMLs: 46 sheet methods and 20 AMA equivalents for 19 authored
references. The following independently sourced leads have not been promoted
to new fixtures; historical steps need checking against the pinned/current
rules. Source prices and reported attempt rates are not test inputs.

| Lead | Distinct coverage and source |
| --- | --- |
| Level-82 fractured-speed chaos dagger | Level-82/83 chaos-prefix pool boundary; [authored process](https://www.reddit.com/r/PathOfExileBuilds/comments/1lp1sad/comment/n0rlkw3/) |
| Fractured-speed +3 bow | Dread, isolation, attack block and forced gem prefix; [authored answer](https://www.reddit.com/r/pathofexile/comments/108j9f8/comment/j3uz389/) |
| Dense-fossil fractured-intelligence ES chest | Fossils plus suffix-only Eldritch cleanup; [completed craft](https://www.reddit.com/r/PathOfExileBuilds/comments/1lz06eh/crafting_1200_es_fross_chest/) |
| Faceted body armour | Two gem suffixes and fractured start; [process](https://www.reddit.com/r/pathofexile/comments/12v9ue4/comment/jha5j7k/) |
| Fracturing a metacraft | Negative lock-protection case; [first-hand result](https://www.reddit.com/r/pathofexile/comments/1ggydcv/first_time_using_a_fracturing_orb/) |
| Veiled Exalted dagger | Remove/add risk, blockers and removal of temporary crafts; [worked answer](https://www.reddit.com/r/PathOfExileBuilds/comments/1lnyq4s/comment/n0j39zx/) |
| Caster-protected annul | Protecting +1 spell while removing another affix; [worked answer](https://www.reddit.com/r/PathOfExileBuilds/comments/1mtucgr/comment/n9e7om4/) and [in-game test](https://www.reddit.com/r/pathofexile/comments/rs36f4/) |
| Bone Ring swap | Trade a bad prefix for a suffix; [item and answer](https://www.reddit.com/r/pathofexile/comments/18iy8ta/comment/kdg9tsj/) |
| Imprint/regal physical foil | Bestiary and imprint rollback; [completed process](https://www.reddit.com/r/pathofexile/comments/fb0ms7/my_first_big_craft_following_mathils_steps/) |
| Failed-fracture recombination | A separate on-demand recovery case; [first-hand report](https://www.reddit.com/r/pathofexile/comments/1er2dxl/psa_you_can_use_recombinator_to_salvage_failed/) |
| Four-suffix Cogwork ring | Expanded slots, Bestiary insertion and Hunter finish; [worked recipe](https://www.reddit.com/r/PathOfExileBuilds/comments/1fabdzo/comment/llsdzk7/) |

Do not import old split/synthesis acquisition, essence-plus-veiled recombination
or exclusive-padding probability claims without current verification. The
existing fractured ES, physical caster and elemental bow fixtures already cover
many superficially renamed recipes.

## Next work and verification

The sceptre/regalia diagnosis and the phased-craft plan are in
[the phased-crafts handoff](phased-crafts-handoff.md).

1. Audit remaining rule gaps and independent examples before promoting new
   mechanics or judging profitable loopholes.
2. Discover compact methods with priced ordinary fallbacks and worthwhile local
   repairs; detailed salvage is a separate request. Preserve the exact solver
   as a quality reference where practical.
3. Adapt the existing ladder miner into typed candidate goals with joint support,
   feasibility and provenance; add mechanic-driven intermediate/donor generation.
4. Build one executable recipe representation for rendering, replay and dynamic
   recovery. Verify its actions or explicitly measure simplification cost/risk.
5. Run the complete cohort after a substantive solver change; distinguish
   inactive recipes from passing solves and final goals from cheap donors.

This consolidation rechecked the bow, both existing cluster guides and five
ladder-derived goals. The focused release suites pass 63 tests with one ignored
throughput benchmark; three candidate-miner tests also pass. Those checks do
not replace a new full-cohort latency sweep. Build before timing; do not compile
or run other tests alongside benchmark cases.

After cleanup, planner tests were repeated, the normal guide harness rebuilt,
and release clippy passes for the three new crates and all their targets with
warnings denied. Local documentation links and whitespace checks pass.
Temporary audit programs, benchmark serialization hooks, debug output and
research scratch artifacts were removed. Pinned game/test inputs and runtime
data caches remain project inputs rather than research reports.

## Reusable developer tools

`ponder-tools` is a Linux Rust tool for process budgets, repeatable guide timings
and policy inspection. Build the tool once:

```sh
cargo build --release -p ponder-tools
target/release/ponder-tools --help
target/release/ponder-tools list --filter cluster
```

### Bounded processes

```sh
target/release/ponder-tools run --seconds 30 --memory-bytes 2000000000 --workers 4 -- cargo test --release -p ponder-solve
```

`run` accepts an arbitrary executable and arguments, with no shell interpolation.
It records stdout, stderr, command, limits, exit status, elapsed wall time, CPU time
and peak RSS in a new directory. Use `--out PATH` to name that directory; existing
directories are rejected. Defaults are 120 seconds, 8,000,000,000 bytes and four
workers. Rayon, OpenMP, OpenBLAS, Cargo and Rust test worker environment variables are set; programs
which ignore those settings can create more threads.

Linux `RLIMIT_AS` enforces inherited **per-process virtual address space**, not an
aggregate descendant RSS cap. Linux `wait4` supplies peak RSS and CPU accounting;
peak RSS is a high-water mark, not a sum of simultaneously resident processes.
The deadline kills the process group, and normal exit also cleans up descendants.
SIGINT/SIGTERM request the same cleanup. Processes deliberately escaping their
process group are outside this runner's contract. Deadline checks have a 5 ms
polling interval plus operating-system scheduling and reaping time.

`run` preserves ordinary exit codes; timeout returns 124 and interruption returns
130. Signals, failed allocations and ordinary failures are recorded without
claiming that every crash is an out-of-memory failure. The runner API is
`ponder_tools::process::{run, Limits, Usage}`; the CLI installs its signal handler
and uses the library's explicit cancellation hook.

### Guide benchmarks

```sh
target/release/ponder-tools bench --guide cheap-ele-bow --repeat 3 --out .cache/benchmarks/before
target/release/ponder-tools bench --filter cluster --repeat 5
target/release/ponder-tools bench --all --exclude cheap-ele-bow
target/release/ponder-tools compare .cache/benchmarks/before/results.tsv .cache/benchmarks/after/results.tsv
```

`--guide` selects an exact fixture name and can be repeated. `--filter` explicitly
selects a substring. `--all` must be explicit. Unknown exact names and empty
selections fail before building. The guide harness is built once in release mode,
outside benchmark timing, with a separate 600-second build deadline. Build memory
and worker limits match the requested budget. `--build-seconds` changes that
deadline; `--test-binary PATH` reuses a previously built guide test executable.

Every case and repetition runs in a fresh process with the normal acceptance
checks, serially. Case order rotates across repetitions. This measures cold solver
processes with ordinary OS file caching; it does not flush machine caches. Avoid
compiling or running other CPU-heavy work concurrently. The state cap defaults to
200,000 **per solver attempt**, changeable with `--state-limit`; fallback attempts
can do additional cumulative work, so attempts and phase statistics are recorded.

`results.tsv` retains every stage, explicitly marking the final item. The terminal
table and `compare` use the final item, never a cheap donor. Solver time includes
lens/model construction and optimization, excluding data loading, guide recipe
validation and simulation. Process wall time includes all those checks. Reports
also include exact cost, explored/retained states, actions, outcomes and solver
phase counters. Inactive recipes, state limits, timeouts, signals, failed checks
and missing benchmark records have distinct statuses. Inactive cases are reported
as skipped, and are excluded from passed timing comparisons. Any failed case makes
the benchmark command fail. Per-run logs and flushed results survive failed cases.

`environment.md` records the executable path and BLAKE3 hash, Git revision, CPU,
budgets, and hashes of selected guide fixtures, pinned game files, prices and Cargo.lock. The binary
hash identifies compiled local edits as well as committed builds. `compare` shows
paired median times, sample counts and exact-cost changes; it does not assert
statistical significance or equivalence of different game datasets. No research
JSON reports or frozen donor distributions are produced.

### Policy inspection

```sh
target/release/ponder-tools inspect --guide ama-327-fire-dot-spell-crit-sceptre --samples 10000 --out .cache/benchmarks/sceptre-policy
target/release/ponder-tools inspect --guide cheap-ele-bow --item main --top 30 --state 42
```

Inspection reuses the same guide builder, solver and concrete acceptance checks.
The final item is selected by default; `--item` selects a named intermediate item.
It writes `policy.md` and `states.tsv`, after the timed solve. The process budget
still covers inspection, so a slow or broken inspector cannot run indefinitely.
Use `--samples`, `--max-steps` and `--top` to control diagnostic work and report size.
The seed is fixed. State IDs are local to that solution, not stable item identities.

The report gives selected action frequency and sampled spend, retry-component
sizes, common states, locks/fractures, class membership, selected-action Bellman
residuals and optional complete outcomes and available actions for one state.
Alternative actions with missing successor values show their unknown probability
and leave Q unevaluated. Internal continuations
are labeled separately from operations to avoid counting them as extra clicks.
Restart edges are omitted when counting retry components: redirecting them to the
market would otherwise manufacture one large component. Simulation does redirect
restarts to the initial distribution and includes repeated purchase costs.

Reachability is structural. Visits and runs reaching a state are seeded Monte
Carlo estimates. Unobserved states are not declared impossible or irrelevant.
The report distinguishes finished from truncated runs; cost mean and standard
deviation use completed runs only, so they must not be trusted if truncation is
material. This is analysis of the abstract selected policy, alongside the
harness's existing concrete simulation. It is not a human recipe generator or a
proof that a simplification preserves expected cost and variance.

Generic analysis lives in `ponder_solve::inspect`, taking any `Model`, `Solution`,
start distribution, sampling limits and uniform random source. Craft-specific
rendering lives in `ponder_plan::inspect`. Missing positive-probability successors,
unavailable selected actions and malformed probability distributions are errors;
they are never treated as free finishes. The small permanent harness interface is
`GUIDE_EXACT`, `PONDER_BENCHMARK`, and `PONDER_INSPECT_*`; normal guide runs produce
none of these extra files unless inspection is requested.

Tooling validation on 2026-10-27 passed 42 focused release tests, including actual
allocation refusal, deadlines, descendant cleanup, interruption, status handling,
final-item comparisons and generic policy inspection. Release Clippy passes for
`ponder-tools`, `ponder-plan` and `ponder-solve`, including all targets with warnings
denied. Six real guide cases also
passed their ordinary oracle and concrete-simulation checks under the default
resource limits. These are individual observations, not a new full-cohort sweep:

| Guide | Final-item solve (s) | Process wall (s) | Peak RSS (MB) |
|---|---:|---:|---:|
| Overlord staff cluster | 0.003854 | 0.956 | 70.6 |
| Cold three-notable cluster | 0.004303 | 0.351 | 70.6 |
| Fire DoT / spell crit sceptre | 10.307195 | 12.101 | 820.3 |
| Cheap elemental bow | 15.888790 | 16.640 | 473.6 |
| Cheap high-ES regalia | 11.231342 | 11.987 | 563.2 |
| Recombined bleed axe | 0.384994 | 1.375 | 70.7 |

The sceptre inspector additionally completed 10,000/10,000 sampled runs, reporting
10 retry components, a largest component of 8,744 states, 796,402 selected
transitions and a maximum selected-action Bellman residual of `1.501e-11`.
The completed-run mean was 3,576.57c versus the exact 3,563.00c, with sampled
standard deviation 2,259.47c. Internal essence continuations are reported
separately from essence uses. These diagnostics describe the current policy;
they do not remove the large retry component or change the solver's objective.
