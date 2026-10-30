# Ponder research and experiments

Updated 2026-10-29 against pinned RePoE 3.29.3.3 and `overhaul` at `dd8bb78`
plus local changes. This is the single record of research, measurements,
correctness findings and approaches already tried. Update this file for future
passes. [SPEC.md](SPEC.md) defines the target architecture.

The last completed guide sweep passed 42 active guides, with 24 inactive. All 84
printed costs and compared counters and seeded simulations remain unchanged
through the exact CPU optimizations and solver consolidation. The latest sweep
observes 39 final-item solves below one second; bow takes 5.518s, regalia 1.639s
and sceptre 1.365s. The universal one-second goal remains unmet.

Release measurements use a Ryzen 5 5600X unless explicitly labelled otherwise.
Fresh-process measurements retain ordinary OS file caches. Different goals,
policy restrictions, fixtures and pipeline scopes must not be compared as though
they were the same workload. Timings are observations, not latency guarantees.

- [Current architecture](#current-architecture)
- [Experiments not to repeat unchanged](#experiments-not-to-repeat-unchanged)
- [Retained solver improvements](#retained-solver-improvements)
- [Performance measurements](#performance-measurements)
- [Corpus and cache measurements](#corpus-and-cache-measurements)
- [Remaining work](#remaining-work)
- [Historical goal and rule corrections](#historical-goal-and-rule-corrections)
- [Crafting reference recipes](#crafting-reference-recipes)
- [Reusable developer tools](#reusable-developer-tools)

## Current architecture

```mermaid
flowchart LR
    D[Downloaded game and ladder data] --> M[Mod matching and demand mining]
    M --> P[Gear and cluster problems]
    P --> A[ponder-plan Session]
    A --> R[ponder-rules operations and lens]
    R --> S[ponder-solve]
    S --> C[Plan cache and reports]
    C --> U[CLI JSON and desktop job results]
    G[Exact guide planner: ponder-rules and ponder-plan] --> S
```

The application now uses the same planner, rules and solver as the guide harness.
The legacy model, donor trees, separate fracture and Awakener routes, protected
recovery, resale, pivots and partial-item finishing were removed at the user's
request. Reports retain native goal predicates, expected operation counts and
conditional craft costs. Totals and margins remain unknown until acquisition is
quoted. The native cache format is version 28. Application sessions have no fixed
state-count cutoff; optional state limits remain available to library, benchmark
and test callers. The version change prevents reusing the old cutoff failures.

The first native integration retained a 100,000-state cutoff. It was measured on
the downloaded 2,502-character snapshot:
18,121 gear items and 7,658 clusters merge to 439 selected candidates. With four
workers and an 8 GB per-process address-space limit, a release run with the plan
cache disabled takes 206.319s (819.837 CPU seconds, 2.105 GB peak RSS). A fresh
process using saved plans takes 1.324s (575 MB peak RSS). OS file caches were not
flushed. The two runs have identical goals, costs, operation counts and errors.
There are 139 successful plans, 80 with quoted acquisition costs; 212 candidates
hit the 100,000-state budget, 49 are unreachable with the available priced
operations, and 39 have unsupported bases, modifiers or lens contexts. These
timings include failures and do not establish complete ladder coverage.

This is not the earlier 123.461s batch: that run prepared 2,000 pinned input items
into 1,601 requests, solved 1,462, and still used the legacy crafting model with
the new numerical backend. The later scan selects products from a different
snapshot and changes the mechanics model too. It cannot establish a speed ratio
against the pinned corpus, and its poor coverage is a migration regression.
Preserving the guide oracle did not establish application performance parity.

After removing the cutoff, the same previously capped ring goal solves at
187,024 states, 2,531,023 actions and 6,148,574 outcomes in 14.598s total process
time (869 MB peak RSS). The previously capped ES body-armour goal solves at
190,833 states, 1,731,581 actions and 3,719,675 outcomes in 8.957s (781 MB RSS).
Both isolated release runs use one worker, a 4 GB address-space limit and external
deadlines. Removing the cutoff restores these plans but is not a speedup. Full
uncapped native batch runtime and coverage have not been measured; the capped
206s/1.3s observations must not be treated as current uncapped timings.
The optional-limit refactor passes the 194 workspace tests and separate
42-active-guide sweep; all 84 costs, compared counters and seeded simulation
summaries still exactly match the archived baseline. Strict release Clippy and
all-target builds pass. No new tests or dependencies were added for this change.

The subsequent enumeration audit found duplicate bench aliases, attack/caster
bans that suppressed no available modifiers on ES armour, unpriced operations
splitting lens classes, base-inappropriate ids in ranked targets (including abyss
jewel life accepted for a ring), and an Eldritch Chaos phase gate that made the
operation impossible to use. These are corrected. Unrestricted guide predicates
still describe transferred/historical modifiers; numeric tiers require a rank on
the actual base. The ring's trade minimum is now 100 life rather than 31.

An overly broad interpretation of the user's fracture restriction temporarily
removed Fracturing Orbs from application plans starting from white bases. That
was a mistake and is reverted: rolling an eligible rare, fracturing a target
modifier and finishing around it remains allowed. Version 28 invalidates the
restricted plans. Supplied fractured affixes remain fixed; the app does not
automatically shop for alternate fractured bases or construct recombination
donors. Explicit guide recipes retain their named mechanics. Temporary bench
blockers now select the highest total unwanted natural roll weight per side,
with price breaking ties; goal crafts and useful metamods remain separate.
This is a policy restriction, not a proof that every other blocker is dominated
under tagged draws or unveil offers. Required fossil-only modifiers now add their
specific fossil to the application catalogue, rather than all fossil combinations.

During the mistaken fracture exclusion, the ring product explored 78,651 states in 7.281s total
process time (690 MB RSS); the ES body product explores 59,663 states in 3.796s
(611 MB RSS), each one worker with a 4 GB address-space limit. Costs change from
10,520c to 154,392c and from 3,317c to 12,060c respectively: the former policies
used newly created fractures, so these are not unchanged-cost speedups. The
chest's new policy uses Eldritch Chaos. These restricted measurements are not
current planner performance. A full uncapped batch is still unmeasured.
Validation passes all 194 workspace tests (one existing ignored test), strict
release Clippy, all-target builds and the separate 42-active-guide sweep; all 84
printed guide costs still match the archived baseline exactly. Under the mistaken
fracture exclusion, an offline cold six-product smoke run took 6.550s total,
solving five products. After restoring normal fracturing, a fresh offline cold
smoke run again solves five products and confirms both ring and ES chest policies
use Fracturing Orbs. The gloves remain unreachable with the available priced
operations. The release CLI and startup example rebuild successfully.
Restricting entry to Eldritch finishing does not prove all recovery states can
be discarded; ordinary annuls and exalts can still move away from a near-goal
item, so do not prune those states solely because the current policy omits them.

The obsolete legacy plan cache was removed. The 432 entries for this ladder scan,
including failures, occupy 55,146 compressed payload bytes (about 1.8 MB allocated
on disk). They store summaries and expected usage instead of policy graphs.
Application goal extraction now uses the native goal compiler for tier and
source matching. Batch checks also exposed an inconsistency between streamed
fracture outcomes and the graph's restart semantics; those paths now agree.

Validation before cutoff removal: 194 workspace tests pass, with one existing ignored
throughput test. The separate sweep passes all 42 active guides: all 84 printed
costs, compared solver counters and seeded simulation summaries exactly match the
archived baseline. Strict release Clippy and all-target release builds pass. A
local PoB CLI run successfully costs its supported helmet goal, reports the boots
state limit, and explains the corrupted amulet exclusion. Desktop tests remain
headless; live authenticated trade was not exercised in this migration.

The backend-only migration described below and all corpus measurements predate
this application migration; they are historical evidence, not current timings.

### Historical backend-only consolidation

The adapter represents shared reroll distributions once, as continuations.
`PolicySystem` eliminates deterministic chains, retaining their rewards and
reconstructing their visit counts. Fixed-policy analysis uses shared direct
factorizations for its reward vectors, preserving operation counts and risk
estimates without repeatedly iterating ill-conditioned occupancy systems.
Negative costs are supported for proper policies, including legitimate resale
credits. Nonnegative-cost models retain their existing numerical checks.

The comparison also catches a legacy risk-reporting bug: its shared-distribution
correction handles only the first reward column. Objective cost is corrected,
but currency, operation and time moments can be understated. The new policy
analysis solves every reward vector. The shared-retry regression checks all four
means and variances against a geometric distribution. For the `gear_begin`
fixture, expected operations are approximately 2,239, consistent with action
usage; the old risk report incorrectly says one operation.

A difficult application case exposed a numerical regression during migration:
fixture item 192, a six-target craft with 54,814 states, took 0.488s in the old
backend but exceeded 20s in the initial adapter. The shared numerical backend
offers `Numerics::Hybrid`, which falls back to sparse LU from the existing
workspace `faer` dependency after 256 iterative steps. The application adapter
selects that strategy. The default `Numerics::Iterative` preserves the guide
planner's established numerical path and printed costs. Both strategies use the
same graph and policy solver. The direct solve eliminates deterministic rows
before factorization and reuses the factorization for subsequent right-hand sides.
That probe completes in 1.284s, with computed costs differing by 5.8e-12 relative
(259,089.8474197654c versus 259,089.8474212722c). It remains slower
than the old specialized backend. This consolidation is not a universal
performance improvement.

The application plan cache version is bumped from 22 to 23, so old plans and
cached failures cannot conceal behavior from the new engine.

The existing libraries remain appropriate: `ureq` for HTTP, Serde/Postcard and
`flate2` for data and plans, `regex` and `roxmltree` for parsing, Rayon for bounded
work, `faer` for sparse LU, and egui/eframe for desktop UI. Production solving
and developer execution paths are Rust.
The schema-less ladder decoder implements an external protocol, and the custom
small numerical kernels preserve established results. Replacing either solely
to reduce local lines is not justified by the measurements.

## Experiments not to repeat unchanged

| Approach | Result / lesson |
| --- | --- |
| Scalar single-RHS policy improvement | Improved sceptre but regressed bow/axe; extra evaluation and scheduling outweighed savings |
| Explicit prefix/suffix intermediate solver states | Fewer edges, more state/context and evaluation work; no general P×S -> P+S shortcut |
| Symmetric Gauss-Seidel preconditioning | Additional sweep did not improve total time |
| Complete unchanged-component cache | Most expensive components change; identical complete answers accounted for little time |
| Warm numerical seeds and carrying policy/value/slack across ceiling rounds | Small or mixed benefit; not an optimality certificate |
| Policy-only deferred admission | Incorrect, with the 11-versus-2 counterexample |
| Optimistic immediate-cost placeholders everywhere | Correct but 131 global solves on the axe; weak bounds cause sibling-by-sibling expansion |
| Eager one-side/zero lower-bound search | Repeated graph work and weak bounds caused severe regressions |
| Fixed guide-stage decomposition | Restricts opportunistic routes and recovery; can cost materially more |
| Early deterministic-chain/hub substitution | Preserved checked guide values but regressed runtime; the later application adapter separately benefits from shared reroll hubs and fixed-policy chain elimination |
| Repeated numeric support queries / long-lived kernel caches / packed keys | Overhead or no useful latency improvement |
| Broad pool merging | Can break fossil probabilities; only merge states when all relevant operations agree |
| Epsilon stopping at 0.5%, 1% or 5% | No useful speedup: eager exploration remains. Regalia 15.9s exact versus 17.1s at 0.5% and 20.0s at 5%; sceptre 18.4s versus 18.9s at 5% on the older Xeon cohort |
| Side-clean recombination rule | No speedup and +9.7% bow cost; violates the unchanged-cost objective |
| Expand deferred recombinations up front | 14–35 million outcomes and slower |
| Admit every violated recombination per state | Did not reduce solve count on the three slow guides |
| Conservative historical or exact mutable reverse dependency tracking | Avoided work did not repay tracking overhead; memory rose substantially |
| Exact per-item deterministic finishing bounds | Negligible graph reduction on this cohort and extra model work |
| Self-loop elimination during policy selection, extra Bellman backups, fewer improvements per admission | No useful overall improvement |
| Shared compiled recombination preparation across a separation pass | No useful latency gain for the retained memory/API complexity; prototype removed |
| Parallel affine right-hand-side solves | Did not improve matched guide timings |
| Limit ceiling increases to 4× | Older bow fell to about 5s at unchanged cost, but extra solves slowed other guides. Its former 10c→2858c jump admitted a 670c lock branch despite a 555c answer; that answer predates the unveiled-goal correction |
| Bench crafts only after one side completes | Not shipped: +11.5% on recombined bleed axe and +7% on chaos claw with free metamods; no regalia saving and only about 10% sceptre saving |
| Defer bench crafts instead of banning them | Restored exact costs and the original runtime |
| Reroll outcomes with no new need → hub | Inconsistent regalia cost below the unrestricted optimum, sceptre 37.4s/7145c, no state reduction; invalid model shortcut |
| No fracturing | Older regalia +6.1% cost, sceptre +4.4%; a policy restriction, not an exact speedup |
| After fracture, only reroll or restart | Nearly all states remained; little latency benefit |

These are results for their stated workloads. A materially different use needs a
new reason and a matched measurement, rather than repeating the same prototype.
Research forecasts of sixfold gains, a seven-million-edge result and over 90%
zero-savings matrix cells were not established. Known-state counts do not measure
matrix-cell sparsity.

Relevant primary references: [constraint generation / CG-iLAO*](https://arxiv.org/abs/2401.14636),
[topological value iteration](https://arxiv.org/abs/1401.3910),
[lazy expensive-transition search](https://arxiv.org/abs/2506.00285), and
[fixed-point certificates](https://arxiv.org/abs/2501.11467). The last is by
Chatterjee, Quatmann, Schäffeler, Weininger, Winkler and Zilken; earlier notes
misattributed it.

## Retained solver improvements

The earlier policy-only deferred-admission scheme was unsound: a four-state
counterexample returned 11 instead of 2 because an improvement lay behind an
unselected route. Its early subsecond claims are superseded. Omitted actions
must be checked outside the chosen policy as well.

For nonnegative costs and free restart, the solver evaluates a proper feasible
policy, checks global Bellman inequalities and admits violations. Unknown
continuations use zero or an admissible lower bound, never an unverified restart
upper value. Returned policies cannot depend on unexpanded tips. Warm numerical
seeds are not value certificates. Resale credits use the separate proper-policy
support in the shared backend; they do not justify applying nonnegative-cost
admission arguments to arbitrary negative cycles.

Retained work across the passes:

- Retain graphs across price ceilings and expand bounded batches of 256.
- Use contiguous component membership/offsets and reused DFS stacks instead of
  millions of singleton vectors. Resolve selected policy edges once; reuse visit
  indices for completed vertices and local positions for component membership.
- Store deterministic transitions inline; prune with `Vec::extract_if` to retain
  action allocations and order. Resolve policy leaves and self-loops before SCCs.
- Compare actions in parallel only for sufficiently large components (1024 states).
  Preserve each expectation's summation order and indexed policy ordering.
- Keep transition rows and ILU factors contiguous in `SparseRows`. `SparseSystem`
  shares factorization and Krylov scratch across affine right-hand sides;
  preconditioning and BiCGSTAB update existing buffers. Final residual checks
  and stopping tolerances remain unchanged.
- Check proper-exit reachability before expensive pruning. Keep exact singleton
  and single-action paths.
- Batch recombination expectations and Boolean successor support, preserving
  donor correlations, heir inheritance and global exclusivity. Aggregate compact
  selections before full item construction and link interned successors directly.
- Reuse ordinary selection samples across exclusivity modes and prepare
  independent donor/header tables concurrently. Settled deferred-action checks
  skip states only while their previous slack exceeds subsequent value decreases.
- Keep the one-side optimistic bound as a fallback after budget failure, rather
  than paying for its weak bound eagerly on ordinary cases.
- Cache full requests with prices, methods and source item details. This helps
  repeats; it does not make a new cold goal cheap.
- Stream `Craft::follow` operation counts and retain only the first compact trace.
  `follow_with_limit` and `Followed.truncated` make simulation limits explicit;
  default seeds, operation order and 200,000-iteration limit are unchanged.

The structural numerical refactor shortened the main solver module by 122 lines
and isolated its backend without adding tuning switches or dependencies. The
later application migration uses the existing workspace `faer` dependency.
Guide arithmetic/order and all 84 printed cost records remain checked exactly.

## Performance measurements

### Exact CPU passes

Starting point: `dd8bb78`, after merged rule and goal corrections. Four Rayon
workers, 200,000 states per attempt, 8 GB address space and 120s deadlines.
Solver time excludes fixture loading, reference validation and simulation.
The first three rows use three-run medians; other rows are complete-sweep samples.

| Guide | Baseline | First CPU pass | Structural pass, matched median |
| --- | ---: | ---: | ---: |
| cheap-ele-bow | 8.746s | 6.007s | 5.436s |
| ama-327-fire-dot-spell-crit-sceptre | 2.549s | 1.471s | 1.343s |
| cheap-high-es-regalia | 2.357s | 1.689s | 1.602s |
| one-hand-bleed-axe | 1.686s | 1.062s | 0.952s (single sweep) |
| block-fire-reduced-crit-shield | 1.317s | 0.718s | — |
| plus-two-minion-reservation-helmet | 1.277s | 0.736s | — |
| int-chaos-armour-boots | 1.005s | 0.751s | — |

The structural pass remeasured the preceding code at 5.904s/1.455s/1.668s;
its paired reductions were 7.9%/7.7%/4.0%. Complete-sweep summed final-item
solver time changed from 26.607s to 18.261s to 16.847s. Under-one-second counts
changed from 35 to 38 to 39. Costs, graph/admission/iteration counters match.
First-pass bow RSS fell 608→566 MB, sceptre 175→161 MB and axe 176→151 MB;
regalia rose 162→178 MB with parallel preparation.

Earlier contiguous-component-only comparisons on pre-phase T2 goals measured
sceptre 8.722→7.644s, regalia 15.175→14.141s and recombined axe 0.503→0.457s;
the cold cluster moved 4.959→5.694ms. Two baseline samples and one final sample
preserved costs and counters. These are different historical goals/policies.

### Fresh profile

Sampling with samply was blocked by `perf_event_paranoid = 2`. Temporary timers
were removed after collection; no system settings were changed.

| Guide | Solver | Explore | Prune | Values | Separation | Linear systems within values |
|---|---:|---:|---:|---:|---:|---:|
| cheap-ele-bow | 5.588s | 1.042s | 0.496s | 2.613s | 1.321s | 0.807s |
| ama-327-fire-dot-spell-crit-sceptre | 1.295s | 0.298s | 0.134s | 0.510s | 0.292s | 0.104s |
| cheap-high-es-regalia | 1.550s | 0.307s | 0.114s | 0.468s | 0.600s | 0.179s |

These are single instrumented observations with the same four-worker resource
limits. Linear-system time is included in values, not an additional phase.
Bow policy component traversal takes another 0.555s inside values. The bow
still performs 11 solves and 33 Newton steps. Linear algebra alone cannot
account for the remaining gap to one second; graph exploration, repeated
policy traversal and recombination separation all remain substantial.

Before the phase-rule merge, sceptre policy evaluation repeatedly processed
5.58 million rows and constructed 5.18 million components, 5.16 million of them
singletons, across 11 solves and 32 Newton steps. BiCGSTAB used 5773 iterations
across 970 calls. The older regalia constructed 3 million components and had
more substantial linear work. Slow convergence was not a universal diagnosis.

The old full-bow failure reached 200,000 states, solved a 31,817-state side bound,
then spent 36.34s/15 solves/80 Newton steps on a 110,797-state restricted graph
before exceeding the cap again. Its authored recipe evaluated in about 6ms over
821 states. A slow optimizer does not imply a slow-to-evaluate recipe. Returning
an incumbent on exhaustion would require an explicit incomplete-search contract.

### Ladder validation and simulation

The versioned `fixtures/solver-ladder.json` has 83 cases from the pinned 2000-item
fixture and downloaded 2502-character snapshot: affix subsets, recombination
partners, influences, 16 cluster cases, full six-affix goals, repairs and bench
finishes. These are affix predicates, not complete numerical item replicas.
Base/donor prices are 5c/50c proxies and methods are explicitly listed.

Fresh processes use four workers, 200,000 states, 8 GB and 120s. Each validation
follows 500 concrete policies with fixed seeds, tests every finish against its
goal and compares mean cost using four standard errors plus the existing 5%
guide margin. Case 49 explicitly uses a 20,000-iteration simulation budget.

- 75 validated cases, two simulation limits, five unsupported bases, one input
  lookup error; all 38,027 completed items satisfy their goals.
- 77 cases solve; 76 below one second. Six-affix ring repair is slowest at 1.207s.
- Influenced sceptre finishes only 28/500 simulations; six-affix body armour
  499/500. Censored means do not validate their expected costs.
- The Hunter Void Sceptre solves in 0.017s/1038 states at 1,033,280c. Its original
  timeout was in rare-success simulation, not solving.
- Special-base guards reject Simplex/Focused amulets and Manifold/Cogwork rings.
  The observed Ornate Quiver is `unique_only` in pinned data; no substitute is used.
- Streaming simulation cuts case-36 bow RSS 193→70 MB and Abyss jewel 153→70 MB;
  bow simulation time remains roughly 22s. This is a memory improvement.
- All 62 cases completed in the earlier sweep preserve costs/counters and seeded
  means/errors; the guide sweep preserves all 84 seeded reports and traces.

Earlier ladder-miner smoke tests passed 500 concrete runs each: Enduring
Composure (2.5ms), Assert Dominance/Towering Threat (2.7ms), chaos three-notable
cluster (3.8ms), cold three-notable cluster (4.5ms), Repeater/Streamlined (2.4ms).
Their observed enchant/level contexts were retained; passive count and real
market acquisition were not validated.

### GPU investigation parked

The user parked CUDA work. This host has an RTX 5070 Ti with about 16 GB VRAM
and a driver, but no `nvcc`, cuBLAS or cuSPARSE. No GPU timing was performed.
Dynamic graph discovery is not one large matrix multiply. The older sceptre's
0.32s linear phase within a 10.8s solve could not explain the whole latency;
the corresponding regalia phase was 2.55s within 15.45s. `f64` residuals near
1e-12 also preclude treating FP16/TF32 peak rates as applicable throughput.
Only a future measured resident-data experiment in Bellman scans, prepared
recombination contractions or large sparse solves would establish a case.
References: [large-state MDPs](https://arxiv.org/abs/2303.10672),
[sparse interval MDPs](https://arxiv.org/abs/2401.04068),
[cuSPARSE](https://docs.nvidia.com/cuda/cusparse/),
[NVIDIA best practices](https://docs.nvidia.com/cuda/cuda-c-best-practices-guide/).

## Corpus and cache measurements

### Full observed-affix corpus

The strict run accounts for all 2,000 items in
`fixtures/expected/items/ladder.json.gz`. It builds goals from all observed
explicit modifiers, accepts the requested natural/essence tiers or better,
and preserves existing fractures in the input. Other numerical rolls,
implicits, socket properties and cluster passive count are outside these goals.
Crafted and veiled goals use the current source/template matching semantics.
Input prices remain 5c fixture proxies, not market valuations.

Equivalent requests are grouped by base domain/class/tags/implicits, item
level, influences, requested modifier ids and fractures. Twelve cluster inputs
have multiple observed enchant contexts in the cached ladder data; they are
reported as ambiguous instead of choosing a pool. The remaining 1,988 inputs
merge into 1,975 distinct requests. Only thirteen solves disappear with this
strict grouping: most complete items really do request different crafts.

The initial full strict sweep took 1,181.95s (19m 42s), excluding the brief
manifest preparation, with one process at a time, four Rayon workers, an 8 GB
address-space limit and a ten-second deadline per process. Each request starts a
new process and reloads the game fixtures. Across 1,098 successful requests:

| Measurement | Time |
|---|---:|
| Median solver | 0.0129s |
| 95th percentile solver | 1.2251s |
| 99th percentile solver | 3.1258s |
| Maximum solver | 9.3681s |
| Sum of successful solver time | 258.70s |

1,034 successful requests are below one second. These percentiles exclude
failures, which are reported separately:

| Status | Requests | Original inputs |
|---|---:|---:|
| Solved | 1,098 | 1,106 |
| Unreachable under the tested methods | 707 | 709 |
| Unsupported base | 61 | 64 |
| Input lookup error | 85 | 85 |
| State limit | 24 | 24 |
| Ambiguous cluster context | 12 | 12 |

There are no process timeouts in this sweep. The methods are base currencies,
targeted catalogue essences/reforges, and applicable targeted/general bench or
veil methods drawn from the guide fixtures. There are no market-supplied donor
searches or promises of coverage across every game mechanic. Unreachable means
unreachable within this model and method set.

Failed reachability proofs consume another 507.71 solver seconds; state-limited
attempts consume 105.93s. Small median successful solves therefore do not imply
a fast or complete scan. Examples include unavailable beast crafts, special
modifiers, five-affix Abyss jewels, unsupported base implicits and modifiers
missing from the current game model.

The follow-up adds a conservative admission check to `Session`: a required mod
must already occur in an input/donor or be producible by an available primitive.
Roll and unveil sources are deliberately overapproximated; this is only a proof
of impossibility, not a restriction on viable policies. The check avoids building
a lens and graph solely to discover that a missing crafted or legacy modifier
has no producing operation. It preserves modifiers supplied on inputs and
recombination partners.

### Application extraction and grouping

The application run prepares serialized problems with the existing
`gear::build` and `cluster::problem` entry points, then solves those problems
through the migrated backend. Gear uses the existing resistance filler and
bench-finishing rules. Cluster goals use the existing notable extraction and
level-84 crafting context. This is intentionally a different demand definition
from complete observed-affix goals; their costs and coverage are not comparable.
It does not include trade requests, donor-market search or ranking every
alternative acquisition route.

The adapter also prioritizes a short unweighted route to a goal when ordering
its initial actions. This gives policy iteration a feasible starting route;
all available actions remain eligible for improvement.

Target order and fixed-mod order are canonicalized before grouping equivalent
problems by their fingerprint. Preparing the pinned corpus produces 1,601
unique application problems. Another 123 requests representing 126 inputs fail preparation,
and the twelve ambiguous cluster contexts remain explicit. Each timed child
loads a prepared problem instead of rebuilding the entire game database.

#### Completed application batch

The final batch accounts for every original input. Preparation takes 5.496s;
the supervised solving batch takes 117.965s, about **2m 03s combined**. Four
processes run concurrently, each with one Rayon worker, a 2 GB address-space
limit and a twenty-second deadline. The aggregate address-space budget is
8 GB. Each case starts a fresh process; OS file caches are not flushed. The
largest observed child uses 984 MB RSS.

The 1,862 inputs with prepared problems merge into 1,601 solves, avoiding 261
duplicate solves. Preparation errors also merge three duplicate inputs.

| Status | Requests | Original inputs |
|---|---:|---:|
| Solved | 1,462 | 1,723 |
| Unreachable under the application model | 87 | 87 |
| State/action pair limit | 52 | 52 |
| Input preparation error | 123 | 126 |
| Ambiguous cluster context | 12 | 12 |

There are **no process timeouts or runtime errors**. All 52 resource-limited
requests reach the existing one-million state/action pair limit. The maximum
state and transition budgets remain 100,000 and five million respectively.

Successful-request timings exclude problem decoding and price loading; batch
wall time includes process startup, decoding, failed attempts and supervision.

| Measurement | Solver time |
|---|---:|
| Median | 0.0555s |
| 95th percentile | 1.3983s |
| 99th percentile | 2.4010s |
| Maximum | 7.5294s |

1,333 of the 1,462 successful requests finish below one second. The earlier
serial application sweep took 373.994s plus 5.713s preparation and left four
timeouts. The final batch also includes the goal-route ordering and fixed-policy
analysis improvements, so that comparison does not isolate parallelism alone.
The four former timeouts now solve, and two former numerical failures reported
as unreachable now solve as well.

The existing application model marks 1,300 successful requests approximate.
Those flags are preserved: this batch is not evidence that every observed item
has an exact crafting plan. Extremely expensive, rare-event application policies
also show numerical sensitivity. Among 1,456 requests shared with the earlier
adapter sweep, the largest cost movement is 5.54e-5 relative for fixture item 87
(roughly 1.52 billion chaos). Its final cost differs from the saved legacy
backend by 1.69e-6 relative. Application results are not promised bit-identical
to the removed numerical backend; the exact guide cohort is checked separately.

#### Warm-cache follow-up

The 2m 03s result uses fresh solver processes and no saved plans. OS file caches
are already warm; it is not a measurement with filesystem caches flushed.

The application already has a persistent `PlanCache`. A follow-up loads all
1,601 prepared problems in one process with four Rayon workers, populates a
new cache, then reads it three times. The initial pass, including saving plans,
takes 105.856s. Subsequent passes take **1.414s, 1.396s and 1.399s**, including
problem decoding, cache-key calculation and decoding the full saved policies.
All three have zero solver calls. Costs, statuses, graph counters and
approximation flags exactly match the initial pass; the 139 unreachable or
resource-limited results are also cached. The cache occupies 74.8 MB.

These warm passes reuse the prepared problems and exclude request extraction.
Adding the earlier 5.496s preparation observation gives roughly **seven seconds**
for preparation plus cached retrieval, an estimate rather than a measured full
application startup. Trade requests and alternative acquisition routes remain
outside this corpus benchmark.

Recalculation with warm mechanics caches is a separate measurement. Six fixture
problems are each solved three times in one process, without using `PlanCache`,
with one Rayon worker and a 2 GB address-space budget. The same `Problem` retains
its transition caches; the graph and policy calculation are repeated. Every
printed cost is identical across its three solves.

| Fixture item | First solve | Mean of next two solves |
|---|---:|---:|
| 1807 | 2.69 ms | 1.48 ms |
| 236 | 47.80 ms | 28.27 ms |
| 742 | 244.03 ms | 83.62 ms |
| 452 | 1,004.82 ms | 444.72 ms |
| 991 | 1,803.60 ms | 755.46 ms |
| 1558 | 5,809.25 ms | 5,494.87 ms |

These are selected examples near the previous run's latency quantiles, not a
second full-corpus recalculation benchmark. A valid saved plan avoids solving;
an uncached request still pays for graph exploration and policy optimization.

### Cold estimate and durable data

With downloaded inputs local but empty plans and cold OS caches, allow roughly
2–3 minutes for the direct application corpus on this host with four workers.
This is an estimate; OS caches were not flushed. Network downloads add time.

The original 1462 version-23 plans total 74,779,284 bytes: median 9347.5 bytes,
p95 265,746 and maximum 487,140. Their 139 cached failures add 4667 bytes.
The later full route scan adds about 116 MB of useful plans; the normal cache
now holds approximately 192 MB of file content.

Downloaded game exports and ladder inputs live in `data/repoe/` and
`data/ladder/`; `PONDER_DATA` still overrides game data. User recovery quotes
live in `data/recovery-quotes.json`. Derived plans, prices and trade responses
remain in `.cache/`. All moved input files were SHA-256 verified. Cleanup
removed obsolete plans/binaries/build temporaries and reclaimed 2.76 GB.
The 1975-request strict manifest remains identical after moving the inputs.

### Compatibility

Across 22 pinned legacy fixtures, all 279 compared cost/time/action-usage fields
agree within 7.49e-13 relative or absolute at unit scale. Risk moments deliberately
use the corrected multi-column calculation. The probe totals 0.836s versus
0.391s in the old specialized backend, so consolidation is not a universal
performance win. The exact guide cohort remains bit-identical; extremely rare
application policies retain the numerical caveats above.

## Pipeline checks

### Wiring fixes

- Local pipeline runs use the league recorded with their saved prices. They
  previously queried the league API on every run, potentially waiting on the
  network and using a different league from the actual price snapshot.
- Successful ladder downloads send a typed completion event. The UI updates its
  readiness and can start its initial automatic run; previously a first download
  left the Run action disabled until restart.
- Explicit solver retries replace both the plan and error fields. A successful
  retry clears its previous error, and a failed retry clears its previous plan.
- Desktop job readiness uses one synchronized job list. Duplicate jobs are
  rejected centrally; a worker panic reports an error and clears its busy state.
  Jobs updating the same product are serialized, and old estimate results cannot
  replace the current run's status.
- Cluster listings are cached once in the price client, shared between clones
  and invalidated after a successful fetch. The pipeline delegates its cluster
  lookups to that implementation instead of maintaining a second copy.
- Pipeline phases share one Rayon pool helper. Cache loading and donor-route
  work respect the requested worker count. Ladder downloading also uses a
  separate bounded Rayon pool, replacing its manual atomic work queue and
  per-character result locks while preserving ladder order.
- User-entered recovery quotes live in `data/recovery-quotes.json`, alongside
  durable input data.
- CLI ladder output selects its rows before rendering their JSON. It previously
  rendered the complete report and then rendered the selected rows again,
  including when `--top` discarded almost every row. Report serialization now
  delegates directly to Serde's existing `Value` implementation.

No solver arithmetic, stopping tolerances, game rules or cohort costs are
changed by this audit.

### End-to-end checks

Release builds on the Ryzen 5 5600X, four workers, 8 GB address-space budget.
The startup example's `--offline` flag replaces every HTTP transport with a
request-denying transport. Each measurement starts a fresh process; filesystem
caches are warm.

| Scope | Pipeline elapsed | Outcome |
|---|---:|---|
| Six-product smoke, initial code | 3.819s | Six estimates, no unresolved products |
| Six-product smoke, current code, empty plan cache | 2.778s | Same six result snapshots |
| Six-product smoke, populated plan cache | 0.971s | Same six result snapshots |
| Full default scan, initial empty plan cache | Exceeded 120s | Interrupted, cache retained |
| Full default scan, resumed cache | 45.188s | 437 products, 433 craft estimates |
| Full default scan, populated cache | 10.883s | All 437 snapshots identical to the completed run |

The first and second smoke timings are single observations; they do not isolate
the performance effect of individual changes. The 45.188s run resumes an
interrupted cache fill and is not a cold-start measurement.

The completed default scan chooses 138 direct routes, 223 fractured routes,
43 recombination routes, 15 protected routes and 14 Awakener routes. Alternative
routes recover 52 of the 56 direct-solver failures. Four products remain
unresolved: three shield goals are unreachable under the current model and one
Crusader Vanguard Belt reaches the one-million state/action pair limit.

Peak RSS is 6.17 GB during completion and 5.66 GB on the cached scan. In the
cached run, input matching reaches candidate construction at 0.690s, route
planning begins at 1.616s and pivot discovery begins at 8.926s. Pivot discovery
replays up to 50,000 clicks per eligible product. Warm startup therefore still
does substantial route construction and simulation after plan-cache hits.

These numbers include more work than the earlier direct-only 2,000-item corpus
and the 1.4s prepared-plan-cache reads. They use the downloaded ladder sample
and the application's normal candidate selection and route planner.

The actual CLI also generates six complete JSON estimates with route, recipe
stage and risk fields. A full default CLI run with `--top 1` takes 18.522s
including output construction and process teardown, using the normal application
cache. This is a separate observation from the 10.883s library pipeline timing.
The public price-league and ladder-index endpoints return
HTTP 200 with the expected JSON shapes. Authenticated live trade searches were
not performed; trade parsing, failures and rate limits use injected transports
in the test suite. Desktop behavior is tested headlessly; no interactive window
test was performed.

335 workspace tests pass, with one existing ignored throughput test and the
long exact-guide cost sweep run separately in the preceding optimization work.
The final report-selection regression is checked with the pipeline integration
suite. Strict release Clippy passes across all workspace targets. The six-product
snapshots match before and after the wiring changes, and all 437 completed
snapshots match on cache reuse.

Validated full-scan plans were retained in the normal `.cache/plans/` directory
so the application can reuse them. This adds about 116 MB of useful plans to the
earlier direct-corpus cache; the resulting files total approximately 192 MB.

## Remaining work

The native application migration removes the duplicate model and compound-route
issues below. Missing base quotes now leave totals and margins unknown. Remaining
work is native batch performance and coverage, visible candidate exclusions,
provenance during merging, and readable recipes beyond policy operation counts.
Sessions perform one hybrid solve without a fixed state cutoff; expensive bound retries remain in the
guide harness. Lens-capacity errors are returned as unsupported contexts. Cached
failures include the same data, price, assumption and model-version identity as
successful estimates.

### Historical application audit

These findings describe the application before the native migration:

1. **Acquisition-price semantics.** `Estimate::total` and `protected_total` still
   treat a missing ordinary base quote as zero; only fractured bases require a
   quote. Outputs are therefore crafting estimates that can omit base acquisition
   cost. Unknown normal-base prices must remain explicit before totals or margins
   are treated as fully quoted economics.
2. **One mechanics model.** Candidate goals, supplies, recovery and rendering
   still need adaptation to `ponder-rules`/`ponder-plan`. This is the largest
   remaining duplication and the meaningful opportunity to remove substantial
   LoC. It requires preserving the oracle and owned-item behavior.
3. **Compound-route precision.** The top-level approximation flag and desktop
   badge inspect `cheapest_plan`, which covers direct and fractured routes.
   Compound routes can return a null precision flag even when component plans
   use estimated chances. Their component flags need aggregation. The existing
   `finishing_policy_risk` field describes only the finishing policy, not the
   total variance of every acquisition stage.
4. **Bounded route construction.** A full default scan is memory-heavy even with
   cached plans. Reuse or lazily construct donor-route metadata and optional
   pivot simulations; introduce cancellation between products for the desktop.
   A resource-aware scheduler is more useful here than swapping arithmetic
   libraries without profiling the route work.
5. **Coverage and provenance.** Diagnose the four unresolved goals, expose
   candidate-construction exclusions, and retain observation provenance when
   merging equivalent goals. The 437 products contain 436 distinct
   problem/title/base tuples; equal solver problems can still have different
   product evidence and should not be merged blindly.

Discovery still needs a shared AND/OR subgoal planner, mechanic-driven donor
generation and one executable recipe representation for rendering, replay and
recovery. Goal merging must preserve joint observations, compatible bases and
enchant pools, tier/source constraints and distinct-character support rather
than unioning popular modifiers. An owned starting item is available once;
restarts must pay for replacements. The session's replenishable priced supplies
are not an owned-item lifecycle.

A short main recipe may accept roughly 1% additional expected cost for comparable
risk and simpler steps, but exact solver optimizations must not silently make
that product tradeoff. Ordinary misses remain priced and worthwhile local
repairs remain available. Detailed salvage can be requested from the actual item.

The latest pipeline pass removes duplicated lookup/scheduling/serialization,
but tests and explicit error handling offset most production LoC savings.
The major remaining duplication is the two mechanics models.

## Historical phase experiments

These are older policy restrictions, not cost-preserving CPU optimizations.
The Xeon handoff used a 4-vCPU 2.1 GHz host and goals preceding the final merged
cohort. It had 44 active/22 inactive guides, versus today's 42/24. Local goal and
layout work (`cce1d63`) later merged with `c661042`/`7bac599` in `cbaf976`.
Do not reuse those historical prices as current acceptance values.

Shipped restrictions abandon a fracture serving no need and apply Eldritch
implicits only with at most one need missing, after which rerolls, rarity changes,
recombination, influence and fractures are unavailable. Model and concrete replay
both implement them. Settled recombination checks are a separate exact speedup.
The merged T2 cohort's cost increases included sceptre about 0.84%, regalia 3.45%
and Overlord 4.58%; these are policy changes, not representation-only wins.

Earlier regalia/sceptre graphs had 88,813/138,593 states but selected policies
only 8508/11,445. About 70–80% chose restart or reroll. Fracture identity,
Eldritch presence and crafted blockers multiply earlier states; recombination
itself first discovered fewer than 200 states per case, but forced repeated
whole-graph evaluation. Regalia's 86,406 fracture-collapsed items reduced to
3888 undecorated forms. At junk states, useful lower bounds would need to approach
the restart worth; relaxed/side/zero bounds were too weak, explaining LAO* and
optimistic-placeholder regressions.

The epsilon certificate considered was

    V(s) ≤ (1 + ε) c(s,a) + Σ P(t|s,a) V(t)

for every state/action, with a proper executable policy. It bounds relative
suboptimality; small observed improvement alone does not. Raising epsilon did
not avoid eager expansion, so it did not improve latency.

Post-fracture regalia continuation values 550.69c/588.15c/616.01c matched isolated
fractured-flat/%/hybrid-ES probes at 550.7c/588.1c/616.0c; selected policies had
162 states. Yet the probes explored 47,093 states because the harness also
supplied `buy:base`. An isolated phase must replace the start set.

The unshipped phase proposal connects ordinary exact crafts through fixed-value
exits, with separate fractured-mod subgoals and finishing phases. Discarding the
unfractured remainder measured +1.8% regalia/+2.7% sceptre; bench gating was later
rejected by broader comparisons. Exact continuation values do not prove that
restricting phase entry/exit preserves the original global optimum. Locks,
unveil states and useful remainders must cross boundaries where necessary.

Temporary switches `PONDER_EPS`, `PONDER_NO_FRACTURE`, `PONDER_FRACTURE_NEED`,
`PONDER_FRACTURE_RESET`, `PONDER_GATE` and `PONDER_KEEP_ALL` were experiments,
not supported current interfaces. Model-only outcome collapses failed real-item
checks until replay followed the same exits.

## Historical recovery-policy comparisons

The sceptre's largest chosen-policy cycle has 8,744 states and connects
preparation, recombination, fracturing and unveiling. A possible path is completed
suffixes -> veiled chaos -> awkward result -> annul loses a wanted suffix ->
recombination -> Scorn preparation. Freezing the guide's stages can exclude such
choices; earlier stage decomposition increased some costs by 29–38%.

The following observations predate the fractured-crafted-slot correction and
are historical diagnostics, not current legal policy recommendations.
Some distinctions were useful, not arbitrary: when fracturing can permanently
lock a prefix that conflicts with the required fire unveil, the preferred action
can change from fracturing to veiled chaos. In 500 reference runs, 332 of 508
fractures occur with the suffix lock present. This behavior followed the former
fractured-craft assumption; it was not solely an off-policy theoretical tail.

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

## Historical goal and rule corrections

These measurements record successive goal/rule corrections before the final CPU
passes. Their costs and timings belong to those earlier predicates and policies;
the current cohort is summarized at the top of this file.

### Adjustable T2 default

Omitted requirement tiers now deserialize as T2-or-better. A positive numeric
`tier` overrides the floor, including stricter T1 or looser T3. `tier = "any"`
explicitly removes the tier constraint (`ModReq.tier = None` in Rust). This applies
to final and intermediate predicates, so donor preparation and finish checks use
the same quality standard. Explicit tiers already present in guides are preserved.
Crafted, unveiled and fossil-only requirements still name their intended source;
the ordinary tier floor does not turn them into interchangeable natural mods.
Influenced modifiers are ranked with their own influence present in the quality
lookup, even if the base starts uninfluenced. This does not permit an illegal
influence action. A few historical, already-inactive fixtures explicitly retain
`"any"` for unsupported off-base or corrupted modifiers and disposable preparation
affixes that have no native tier ladder. They are not promoted as T2 finished items.

This is a quality-policy change, not a solver optimization. It groups fewer low
tiers as acceptable goal hits, but still considers their retry and repair outcomes.
It can change state counts, feasible policies and costs in either direction. No
subsecond claim follows from tightening the predicate. The domain-audit tables
below predate this default unless explicitly identified as new measurements.

The first corrected sceptre measurement passes reference and concrete-policy
simulation at 5,073.6c and 10.14s, versus 3,637.7c and 11.18s before the default.
This is a different quality target and a single timing observation, not a measured
algorithmic speedup. The same expensive cross-stage search remains.
Exploration actually grows from 126,311 to 181,749 states, while the retained
policy shrinks from 9,229 to 5,235 states. The new run scans 2.38m actions and
4.40m materialized outcomes; value work takes 5.67s and separation 2.17s across
11 global solves and 32 Newton steps. Tighter quality has not removed repeated
whole-graph work.

The complete rerun is `.cache/benchmarks/tier-default-cohort-20261027`: 41 pass,
24 inactive and the existing full-bow state limit, with no new failed guides.
All reference and concrete-policy checks pass for the solved guides. The
recombined bleed axe remains below one second at 0.63s and 10,041.8c; the
from-scratch physical axe is 0.71s and 4,988.0c. These costs supersede the earlier
any-tier endpoints. Existing explicit tiers, including T3 and T4, are unchanged.

### Domain audit before the default change

The initial full sweep passed 44 guides, with 22 inactive and six final solves
over one second. It exposed incomplete goals: the Contempt axe omitted flat
physical damage, the fractured Woe regalia omitted flat ES, the Loathing helmet
omitted reservation, and Hollow gloves omitted their abyssal socket. Both bow
goals also allowed a crafted hybrid in place of the guide's unveiled hybrid.
Those earlier low prices do not describe the now-requested finished items.

The user confirmed those ingredient stats must remain, with comparable natural
and essence alternatives allowed. Those requirements are now present. Explicit
essence-origin requirements across the guide corpus became T2-or-better quality
requirements; this does not force the optimizer to use an essence. Families with
no natural tier ladder rank within the base's essence modifiers. This is a tier
approximation, not a comparison of exact numerical rolls. Bench modifiers no
longer satisfy unqualified requirements, and both bows explicitly require an
unveiled hybrid. Explicit final bench crafts remain valid crafted requirements.

The pre-phase domain sweep, including the additional corrections below, gave:

| Problem | Final-item solve | Explored states | What the written guide does | Remaining search difficulty |
|---|---:|---:|---|---|
| Full elemental bow | State cap, 47.9s process | 200,000 cap per attempt | Make fire/cold and cold/lightning pairs, recombine three elemental prefixes, protect them, unveil a hybrid, craft the other speed/critical stat | Three-prefix acquisition couples to suffix room, locks, blockers and unveil failures. The old deterministic bench finish and its pruning bound are gone. |
| Fire-DoT / spell-crit sceptre | 11.18s | 126,311 | Build overlapping suffix pairs, combine three suffixes, lock suffixes, unveil fire damage, craft DoT | 16.24m outcomes; 7.28s in value work and 1.81s in separation, with ten global solves and 31 Newton steps |
| Recombined high-ES regalia | 15.62s | 106,405 | Recombine three ES prefixes, Eldritch-craft intelligence, craft increased attributes | 12.24m outcomes; 7.12s in value work and 5.12s checking deferred recombinations |
| Block/fire/reduced-crit shield | 2.85s | 67,752 | Make both block suffixes, protected critical reforge, add fire resistance, craft fire/chaos resistance, Crusader slam for damage per block | Six required affixes, suffix protection, acquisition of influence and prefix repairs |
| Essence bleed axe | 2.75s | 76,212 | Make an Elder base, Contempt until bleed multiplier, protect prefixes/unveil physical damage, protected physical reforges for culling, craft physical DoT | Mandatory flat physical damage plus legal influence acquisition expands relevant repairs; 2.09s in value work |
| Supplied-fracture elemental bow | 2.01s | 138,637 | Roll missing elemental prefixes on the fracture, protect them, unveil a hybrid and craft the other speed/critical stat | Only 177 retained policy states, but 1.84m actions and 2.93m outcomes explored |
| Life/int/chaos boots | 1.80s | 53,991 | Fracture life, roll intelligence and elemental resistance, lock/reforge chaos, Eldritch-finish movement speed | Five affix requirements across both sides, fracture acquisition, suffix protection and prefix repairs |
| +2 minion/reservation helmet | 1.60s | 53,989 | Fracture +2 minion, Loathing for reservation and resistance, protected chaos reforge, Eldritch-finish life | Coupled fracture, three-suffix completion and prefix repair |
| Chaos claw | 1.41s | 76,433 | Envy chaos damage, protect/unveil chaos penetration, protected critical reforge, craft the last suffix | The optimizer also considers fracturing an unveiled penetration prefix, then rerolling the remaining requirements |

The recombined bleed axe is not a major latency holdout: 0.40s, 13,104 states,
and 9,087c. It uses Contempt to make the flat-physical input, recombines with the
percentage-physical donor, adds Elder influence, and uses protected physical
reforges plus local repairs. Its goal differs from the essence-method axe.

Correcting goals changed these model costs; the table compares the former and
corrected targets rather than claiming optimizer regressions:

| Item | Former Ponder cost | Corrected Ponder cost | Corrected oracle cost |
|---|---:|---:|---:|
| Supplied-fracture bow | 118.8c | 1,888.0c | 2,401.8c |
| Supplied-fracture ES regalia | 79.5c | 737.1c | 2,575.9c |
| Hollow BV gloves | 4,406.3c | 37,259.6c | 138,341.0c |
| Reservation helmet | 3,148.9c | 5,258.8c | 10,994.3c |
| Essence bleed axe | 9,738.9c | 13,567.9c | 137,390.3c |
| Supplied-fracture two-handed physical axe | 1,276.6c | 8,748.6c | 10,040.4c |

The two-handed axe had the same omitted Contempt flat-damage requirement. Its
additional focused check passes after correction; its apparent 87% saving shrinks
to about 13%. The fractured ES regalia now uses Spite for intelligence while
rolling the remaining ES prefixes, with exalts on useful partial hits. These are
different quality predicates from the former 79.5c case.

Large remaining wins need different explanations. The snapshot wand falls from
442,585.5c to 21,866.1c by fracturing one fossil-only socketed-attribute-gem mod
and rerolling for the other, instead of requiring both on one unanchored fossil
roll. The physical DoT wand substitutes alteration/augmentation preparation for
four-fossil preparation (56,633.7c to 4,149.4c). These are plausible alternative
methods under the current rules, not independent in-game validations.

### Original-source comparison and additional corrections

The original [crafting workbook](https://docs.google.com/spreadsheets/d/12Q1N1E586NfLqoyzIHXU3Y-mCKtTISykeGBUv-HWi5w/edit)
was recovered as XLSX, including finished-item pictures. This provides stronger
evidence than inferring the goal from ingredient names alone.

- **Shield:** the missing prefix is
  [1% increased Damage per 1% Chance to Block Attack Damage](https://poedb.tw/us/Crusaders_Exalted_Orb),
  a Crusader prefix. At 75% attack block it supplies 75% increased damage. The
  target also needs both block suffixes, reduced extra damage from critical
  strikes, fire resistance and the final fire/chaos bench craft. The reference
  now starts uninfluenced and slams Crusader last. Runtime already rejected a
  second influence exalt; the reference had silently skipped its illegal action.
  Cleanup and restart branches now describe a legal, conservative reference,
  not every manual repair in the workbook. Current comparison is 102,288.9c
  versus Ponder's 13,460.2c; the former 4,797.6c Ponder result was incomplete.
- **Regeneration helmet and gloves:** chaos resistance is now mandatory at
  T2-or-better, without forcing Envy as its origin. Costs rise from about
  2,958c/2,641c to 5,411.7c/5,178.1c respectively.
- **Bow:** the screenshot has an unveiled speed/attribute hybrid and a plain
  crafted critical chance modifier. The goal now requires an unveiled speed/crit
  hybrid plus an explicit plain speed/crit craft; group conflicts force the other
  stat. It no longer requests two hybrids sharing an attribute group.
- **ES chests:** the finish is increased attributes, not flat all attributes.
  Both variants now require that craft. The full variant is 1,340.9c; the
  supplied-fracture variant is 737.1c.
- **Essence bleed axe:** Elder is a final influence requirement, not a free
  initial property. The reference now buys, alchemizes and Elder-slams a base
  before starting its essence loop.
- **Cheap elemental wand:** Hatred's cold damage is mandatory. The source permits
  natural critical chance or multiplier and crafts the opposite modifier. The
  reference now does that and annuls for multimod room. Requiring an essence
  rare to have only three affixes before cleanup had made the stricter reference
  unreachable. The focused corrected rerun passes: 2,463.4c reference versus
  2,378.4c Ponder, 62.8ms solve, 4,459 explored states.

Confirmed shared-rule fixes apply to both concrete items and compressed states:

- **Fractured bench crafts** retain crafted status, consume the crafted-mod slot
  and survive remove-crafted/full resets. The user confirmed the rule. The old
  sceptre policy's fractured-lock-plus-extra-craft branch was illegal; it occurred
  in 12.29% of the previous inspection's runs. Correcting this raises its model
  cost from 3,563.0c to 3,637.7c. Historical policy comparisons below predate the fix.
- **Veiled Exalted** removes a modifier before adding the veil, including on a
  six-affix item, as specified by the official
  [3.24 notes](https://www.pathofexile.com/forum/view-thread/3496784) and renamed
  in [3.26](https://www.pathofexile.com/forum/view-thread/3787013). Prefix/suffix
  locks apply; attack/caster blockers do not protect this removal. The Archmage
  wand's model cost rises from about 27,887c to 52,788.5c.
- **Annuls** now honor attack/caster protection, including metamods present only
  on a supplied input. This is separate from Veiled Exalted removal, following
  [Craft of Exile's operation-specific behavior](https://www.craftofexile.com/changelog).
- **Eldritch eligibility** is limited to uninfluenced body armour, boots, gloves
  and helmets. Influence slams reject existing Eldritch implicits as well as
  existing conventional influence and fractures.
- **Special bases** with implicit affix-limit/magnitude changes are rejected
  before planning: Focused, Simplex, Cogwork and Geodesic are not ordinary 3/3
  rares. Their earlier apparent bargains are unsupported. Admission is derived
  from the base's implicit modifier families, not a name blacklist.
- **Session input context** rejects mismatched base/item level, insufficient
  input quality and conventional influence combined with Eldritch implicits.
  Cross-base or lower-level donors must not be projected as the fixed context.
  The lower-level Craft API still assumes callers have performed admission.

Artifacts from that audit are `.cache/benchmarks/domain-audit-20261027` and the focused
`.cache/benchmarks/domain-wand-corrected-20261027`. Builds are outside measured
solves. Each passing guide still checks its reference and Ponder policy on real
items; shared simulator agreement does not independently prove a game rule.
At that stage the full bow still hit its state limit; later passes resolved it without raising the cap.

Final validation passes 50 release tests across rules, lens transitions,
recombination, goal matching, guide parsing, recipes and session admission, with
one ignored throughput test. Release Clippy passes all targets for the four new
crates with warnings denied. The complete-cohort results plus the corrected wand
rerun above cover reference and concrete-policy simulation separately.

## Remaining game-model risks

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

The domain sweep leaves the following open issues. They are not silently treated
as verified mechanics or strict finished-item comparisons.

### Final-item quality and reference fidelity

- The earlier sceptre, budget mana sceptre, +1 strength regalia, physical axe and
  several ring goals accepted any tier of important scalable stats. The T2 default
  now supplies a baseline for unspecified tiers. A particular finished candidate
  may still require different per-stat floors or numerical minima. Fixed-valued
  mods such as +1 gems and cluster notables do not need an invented quality floor.
- T2 natural/essence matching currently ranks required levels within a template;
  essence-only templates fall back to the applicable essence ladder. It does not
  compare numerical rolls or overall DPS/ES. This is an explicit approximation,
  not proof that every accepted essence is between natural T1 and T2 in strength.
- Guide pictures can include quality, implicits, enchantments and incidental
  affixes outside the encoded goal. The one-hand axe also uses a representative
  base rather than the pictured Reaver Axe. Optional finishes need explicit
  scope; a scoped partial goal must not be presented as the complete sale item.
- The essence-axe reference omits some source annul/attack-blocker repairs.
  Very large savings can reflect a conservative or weakened reference algorithm
  as well as a better planner. The shield's repaired reference is explicitly
  a conservative restart variant of the source's conditional repairs.
- Inactive chill-conversion gloves and the Cogwork fire ring still omit
  Envy chaos resistance and Anger fire damage respectively. Other inactive
  recipes put final influence on the initial base before later influence slams.
  They remain inactive and need endpoint/source review before promotion.

### Rules and costs

- Harvest add/remove uses the unverified `ADD_REMOVE_FACTOR = 10` times matching
  reforge lifeforce and omits Sacred Crystallised Lifeforce. Absolute costs and
  apparent wins involving these crafts remain provisional until the actual
  per-craft cost table replaces it. Reforge prices are a separate table.
- Generic additions apply active attack/caster bans. Harvest reforges need a
  separate policy: [Craft of Exile's changelog](https://www.craftofexile.com/changelog)
  records that they do not honor those blockers. The new annul filtering does
  not fix this separate draw-side behavior.
- The veil guard recognizes a veil placeholder, not an already-unveiled mod.
  Check whether a second veil can be introduced while preserving an unveiled
  affix, including protected Veiled Chaos. Current-version behavior needs an
  independent example before changing this rule.
- Recombination still uses empirical count probabilities and assumed candidate
  selection weights. Crafted-family exclusivity exceptions remain incomplete.
  Check cross-side group conflicts: the raw item logic filters globally, while
  the compressed recombination kernels resolve prefix/suffix pools separately.
  Donor products are sampled; exact solving of that declared distribution does
  not make it an exact model of all live-game outcomes.
- `influence-orb:*` adds influence without an affix and uses the corresponding
  exalt price. That abstraction is not an actual influence exalt. Two inactive
  fixtures use it; it must not become a production crafting method. Actual
  Shaper/Elder exalted orbs exist in this dataset and are a different matter.
- Plain bases use a 5c proxy; supplied fractures have rough or zero acquisition
  prices. Recombination dust and operation times also carry assumptions. These
  affect economics independently of solver correctness.

### Input and mechanic coverage

- Fixed-base/item-level donor handling and special-base limits now have session
  admission guards, not full implementations. Numerical rolls and special-base
  magnitude changes remain unrepresented.
- Synthesised, corrupted and mirrored flags are absent from Item. Bestiary
  imprints and swaps remain incomplete. Actual imported items with those facts
  need explicit representation and validation before planning is reliable.
- Eldritch state tracks only the dominant side, not implicit tiers. Applying a
  lesser implicit is currently represented as changing dominance; that is not
  reliable if a stronger opposing implicit already exists.
- The session API exposes replenishable priced supplies. An owned starting item
  still needs an explicit one-time lifecycle; it cannot be modelled as free stock.
- Agreement between simulator and compressed model cannot detect a shared game
  rule error. Independent analytic cases and primary-source checks remain needed.

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

## Crafting reference recipes

Reviewed 2026-10-26 against the 46 sheet recipes in [fixtures/guides](../fixtures/guides). These **19 supplemental references** replace the earlier mixed 60-entry research catalogue and now have **20 TOML equivalents** in the guide fixtures, including the Ghastly Eye fracture alternative. They cover the [3.29][ama329], [3.28][ama328], [3.27][ama327] and [3.26][ama326] AMAs, retaining ordinary crafting methods that add a distinct target, mod pool or operation sequence beyond the sheets.

Each reference links to its TOML fixture and to the original answer and request. The fixtures choose representative bases and item level 86 where the answer leaves those open. Unspecified tiered requirements now use the user-selected T2-or-better default; numeric tiers and `tier = "any"` override it. This baseline is a product choice, not a tier claim attributed to the source. Historical source prices, profit claims and attempt estimates are not test inputs. Supplied fractured starting bases have an explicit zero price, so those fixtures measure subsequent crafting rather than a guessed market acquisition cost. Ordinary, abyss and cluster-jewel domains are supported by the simulator, including cluster enchant tags. The Horror ring awaits Glyphic's corrupted-essence effect.

### Conventions

- P/S mean prefix/suffix. `1P + 1P → 2P` combines isolated prefixes; `AC + BC → ABC` combines overlapping two-mod donors. Rebuild or recycle donors according to what survives. No crafted-filler probability boost is assumed.
- Starting fractures, influences and synthesised implicits are explicit inputs. A bought prerequisite is not a recipe for manufacturing that prerequisite.
- Locks must be reapplied after they are removed. Wild Bristle Matron guarantees Suffixes Cannot Be Changed only with full suffixes and an available prefix; otherwise use the bench lock.
- A two-mod rare and a two-mod magic item are different inputs. Check rarity and available slots before multimodding or applying locks.
- The answer supplies the main operations. Retry notes use conservative failure handling: when a wanted mod is lost or no safe recovery is supplied, restart the affected stage/base. They do not claim an optimal salvage policy, free annuls or an undo.
- A targeted Harvest **reforge** is different from an **augment/add-remove**. The latter can remove another wanted modifier. A claimed forced result applies only to the stated base, influence, blocked groups and occupied slots; it does not guarantee a good tier.

### Jewels and clusters

#### 01. RF maximum-fire-resistance jewel

**Reference:** `ama-328-rf-max-fire-jewel` · [3.28 request and answer][rf-jewel] · [TOML](../fixtures/guides/ama-328-rf-max-fire-jewel.toml)

- **Start and target:** Ordinary jewel with fractured +2% maximum fire resistance; add increased maximum life and fire damage over time multiplier.
- **Recipe:** Buy the fractured jewel, or obtain the maximum-resistance roll, Harvest-swap it to fire and fracture it. On the fractured base, repeat Harvest reforge fire until both remaining targets appear. An Exalt on a spare prefix is optional and outside the target.
- **Retry:** A wrong fracture needs another candidate. A failed reforge stays in the reforge loop; the wanted fracture survives.
- **Adds beyond the sheets:** Ordinary-jewel affix limits, a fractured maximum-resistance suffix, and a two-target tagged-reforge loop. The sheet set has no ordinary-jewel recipes.
- **Fixture choices:** Fix the life/multiplier thresholds. If fracture production is tested too, prepare a legal fracture candidate rather than fracturing a one-mod jewel.

#### 02. Physical-minion Ghastly Eye Jewel

**Reference:** `ama-327-minion-physical-speed-jewel` · [3.27 request and answer][ghastly] · [TOML](../fixtures/guides/ama-327-minion-physical-speed-jewel.toml) · [fractured alternative](../fixtures/guides/ama-327-minion-physical-speed-jewel-fractured.toml)

- **Start and target:** Ghastly Eye Jewel; flat physical damage added to minion attacks and minion attack speed.
- **Recipe A:** Repeat Corroded + Jagged + Shuddering in a three-socket resonator until both modifiers appear.
- **Recipe B:** If T1 physical damage is required, buy or make that fractured prefix, then Harvest reforge speed until minion attack speed appears.
- **Retry:** Repeat the chosen fossil/reforge operation. Wrong fractures need another base. Neither route guarantees arbitrary extra affixes or good tiers on every attempt.
- **Adds beyond the sheets:** Abyss-jewel mod pools and a meaningful fossil-versus-fracture comparison. Keep the physical version explicit; the answer does not supply a separate elemental-minion fossil recipe.

#### 03. Doryani's Lesson cold cluster

**Reference:** `ama-327-cold-three-notable-cluster` · [3.27 request and answer][cold-cluster] · [TOML](../fixtures/guides/ama-327-cold-three-notable-cluster.toml)

- **Start and target:** Large cold-damage cluster with fractured Doryani's Lesson; add Blanketed Snow and Prismatic Heart.
- **Recipe:** Obtain the fractured notable, then repeat Dense + Prismatic + Pristine in a three-socket resonator until all three notables are present.
- **Retry:** Failed fracture preparation needs another candidate. Once the fracture is correct, fossil failures simply repeat.
- **Adds beyond the sheets:** A concrete three-notable cluster target and a fossil combination that removes competing filler pools. No cluster fixtures exist in the sheets.
- **Fixture choices:** Pin passive count and item level. The pinned mod data puts Doryani's Lesson in a suffix and the other two in prefixes; both Doryani's Lesson and Blanketed Snow require item level 68. An 8-passive choice would be a fixture decision, not a detail stated by the answer.

#### 04. Overlord staff cluster

**Reference:** `ama-326-overlord-staff-cluster` · [3.26 request and answer][overlord] · [TOML](../fixtures/guides/ama-326-overlord-staff-cluster.toml)

- **Start and target:** Mace/staff large cluster; Overlord and Fuel the Fight. Martial Prowess is the request's optional third notable.
- **Recipe:** Repeat Harvest reforge attack until the chosen notable combination appears. Use separate acceptance predicates for the two-notable and three-notable versions.
- **Retry:** Keep reforging any outcome that misses the selected predicate. Overlord alone is not success for the two-notable target.
- **Adds beyond the sheets:** Multiple notable requirements in one tagged-reforge outcome, including an optional stricter target without changing the recipe.
- **Fixture choices:** Pin passive count and base level. In the pinned data Overlord requires item level 75; Fuel the Fight is also a prefix and Martial Prowess a suffix. The author's estimate for hitting Overlord alone is not an estimate for either full target.

### Rings and belts

#### 05. Fractured Despair, DoT multiplier and life ring

**Reference:** `ama-329-despair-dot-ring` · [3.29 request and answer][despair-ring] · [TOML](../fixtures/guides/ama-329-despair-dot-ring.toml)

- **Start and target:** Non-influenced Delve ring with fractured Despair on hit; essence DoT multiplier, an elemental resistance, life and a crafted mana-cost prefix.
- **Recipe:** Use the DoT-multiplier essence until a satisfactory resistance appears with an open prefix. With the curse, multiplier and resistance filling suffixes, apply Matron/lock and Harvest reforge life. Repeat the lock/reforge stage until acceptable life and a spare prefix; craft mana cost.
- **Retry:** Lost ordinary suffixes return to essence rolling. If life reforging fills prefixes and no safe recovery is available, restart from the fractured base; do not assume a free prefix annul.
- **Adds beyond the sheets:** A bought drop-only curse fracture plus an essence suffix. The Cogwork sheet instead fractures DoT multiplier and has a different slot layout and life-finishing method.
- **Input check:** Use the Delve affix, not a Hunter-influenced Despair ring. The pinned data distinguishes `CurseOnHitLevelDespairDelve` from `CurseOnHitDespairInfluence2`; the influenced version is not a substitute for this fracture.

#### 06. Dual-Redeemer cold prefixes on a Cryonic Ring

**Reference:** `ama-329-redeemer-cold-ring-prefixes` · [3.29 request and answer][cryonic-ring] · [TOML](../fixtures/guides/ama-329-redeemer-cold-ring-prefixes.toml)

- **Start and target:** Redeemer Cryonic Rings; added cold damage against chilled/frozen enemies and added cold damage per frenzy charge on one output.
- **Recipe:** Prepare one donor with each wanted prefix isolated. Recombine `1P + 1P → 2P`, rebuilding or recycling donors until both survive on an eligible output.
- **Retry:** A result retaining just one target can become a donor again. Losing both requires new donors.
- **Adds beyond the sheets:** Two influenced ring prefixes with different conditionals, rather than jewellery suffix work or elemental weapon rolls.
- **Scope:** The exact terminal goal is the two-prefix base. The source leaves suffix preferences unspecified, so no particular suffix finish is inferred. In the pinned data the frenzy prefix starts at item level 75; its highest tier needs 80.

#### 07. Dexterity/resistance ring with unveiled life

**Reference:** `ama-327-dex-resist-veiled-life-ring` · [3.27 request and answer][dex-ring] · [TOML](../fixtures/guides/ama-327-dex-resist-veiled-life-ring.toml)

- **Start and target:** Ordinary ring, with Unset suggested in the request; dexterity, elemental resistance, chaos resistance and unveiled life.
- **Recipe:** Dexterity essence until the elemental resistance, with a free suffix and prefix. Bench Suffixes Cannot Be Changed, then reforge chaos. Once suffixes are complete, Matron/lock, Veiled Chaos, block mana if there is room and unveil life. Choose a compatible final prefix craft.
- **Retry:** Failed suffix preparation returns to essences. Failed veils can be cleared with a suffix lock and Scour when the lock fits; otherwise use a risky cleanup or restart. A full prefix set is not automatically repairable.
- **Adds beyond the sheets:** A jewellery prefix-unveil loop that cannot use Eldritch currency. The final bench choice and exact resistance tiers are fixture inputs, not supplied targets.

#### 08. Intelligence/strength/cast-speed ring with ES

**Reference:** `ama-326-attributes-cast-speed-es-ring` · [3.26 request and answer][attribute-ring] · [TOML](../fixtures/guides/ama-326-attributes-cast-speed-es-ring.toml)

- **Start and target:** Ordinary three-suffix ring; intelligence, strength, cast speed, energy shield and crafted channelled-skill mana cost.
- **Recipe:** Intelligence essence until satisfactory strength. Lock suffixes and reforge caster for cast speed. Lock suffixes and Scour unwanted prefixes; reapply the lock and Harvest augment defence for ES. Repeat prefix clearing and the augment if the ES tier is too low. Craft channelled mana cost.
- **Retry:** Bad suffix tiers return to the essence stage. Prefix retries preserve the completed suffixes through the lock; include the add/remove operation and any consumed lock in the state.
- **Adds beyond the sheets:** Caster-tag suffix completion followed by defence-tag prefix augmentation on jewellery.
- **Scope:** Use the ordinary-ring version the author explicitly allows. The optional random fourth suffix on a Cogwork Ring is omitted, avoiding another Cogwork-sheet variant.

#### 09. Strength/intelligence/Hunter-attributes belt

**Reference:** `ama-327-hunter-attribute-belt` · [3.27 answer][attribute-belt] · [TOML](../fixtures/guides/ama-327-hunter-attribute-belt.toml)

- **Start and target:** Non-influenced, non-fractured belt; intelligence, T1 strength, Hunter increased attributes, unveiled life and crafted elemental damage with attacks.
- **Recipe:** Intelligence essence until T1 strength, exactly two filled prefixes, one open prefix and one open suffix. Hunter Exalt, aiming for increased attributes. With the successful suffix set full, Matron/lock, Veiled Chaos, block mana when possible, unveil life, then craft elemental damage with attacks.
- **Retry:** A wrong Hunter suffix is a failed target; use another base rather than repeat the same pre-influence slam on it. Failed veils need a legal protected-prefix recovery or another attempt/base.
- **Adds beyond the sheets:** A belt target and an influence slam directed to a suffix by occupied slots. There are no belt fixtures in the sheet set.
- **Fixture choices:** The item image supplies no recoverable belt base or exact percentage-attributes tier. The TOML chooses Leather Belt and uses the T2 default for attributes; the influence slam does not guarantee the desired modifier.

#### 10. Horror frenzy-cold ring with attributes

**Reference:** `ama-327-horror-frenzy-cold-ring` · [3.27 request and answer][horror-ring] · [TOML](../fixtures/guides/ama-327-horror-frenzy-cold-ring.toml)

- **Start and target:** Purchased synthesised ring; Horror's cold damage per frenzy prefix, strength, intelligence, chaos resistance and unveiled life. Creating the implicit is outside this recipe.
- **Recipe:** Fundamental + Glyphic + Sanctified in a three-socket resonator until the wanted essence prefix and attributes appear. With a prefix and suffix available, lock suffixes and augment chaos. Reapply the suffix lock, Veiled Exalt, block mana where possible and unveil life. Select a compatible final craft.
- **Retry:** The augment or Veiled Exalt can remove the essence prefix. Losing it returns to fossils. A failed unveil requires a legal recovery or restart; the prefix is not protected merely because the suffixes are locked.
- **Adds beyond the sheets:** Glyphic's competing essence outcomes, attribute-biased fossil rolling, and two add/remove risks to a wanted prefix. This is different from the dual-influenced Redeemer prefix-pair recipe above.

### Weapons and quivers

#### 11. Holy Relic trigger sword with +1 socketed gems

**Reference:** `ama-328-holy-relic-trigger-sword` · [3.28 request and answer][holy-relic] · [TOML](../fixtures/guides/ama-328-holy-relic-trigger-sword.toml)

- **Start and target:** Non-influenced thrusting sword; essence minion damage, natural attack speed, +1 socketed gems, crafted trigger and Hits Cannot Be Evaded.
- **Recipe:** Fear essence until acceptable attack speed. Annul or split down to a rare retaining just those two affixes. Craft Multimod plus Cannot Roll Attack Modifiers, filling all three suffix slots. Exalt for +1 socketed gems. Remove crafts, then Multimod trigger and Hits Cannot Be Evaded.
- **Retry:** Losing minion damage or attack speed during isolation returns to essences. A split result must actually retain both before continuing.
- **Adds beyond the sheets:** A forced non-attack prefix on a melee weapon, with a complete six-affix finishing arrangement. The minion snapshot wand sheet targets socketed strength/intelligence/support gems instead.
- **Slot check:** Before the Exalt: one natural prefix, one natural suffix and two crafted suffixes. The claim about +1 applies to the stated non-influenced thrusting-sword pool.

#### 12. Crusader physical-DoT Citadel Bow

**Reference:** `ama-328-crusader-physical-dot-bow` · [3.28 request and answer][physical-bow] · [TOML](../fixtures/guides/ama-328-crusader-physical-dot-bow.toml)

- **Start and target:** High-level Crusader Citadel Bow; Contempt flat physical, generic DoT multiplier, Crusader physical/projectile-speed hybrid, crafted physical scaling and a chosen crafted suffix.
- **Recipe:** Contempt essence until DoT multiplier, then annul to exactly that prefix and suffix on a rare. Craft Multimod, Prefixes Cannot Be Changed and Suffixes Cannot Be Changed. Harvest reforge speed into the only open prefix. Remove crafts; Multimod increased physical damage and either attack speed or physical DoT multiplier.
- **Retry:** Failed isolation returns to essences. Reaching the protected slot arrangement is mandatory; do not apply this as an unrestricted reforge.
- **Adds beyond the sheets:** A physical DoT bow and simultaneous prefix/suffix protection. Existing bow sheets target elemental prefixes.
- **Data check:** The pinned `LocalIncreasedPhysicalDamagePercentAndProjSpeedInfluence*` modifiers remain Crusader bow prefixes; their tier requirements run from item level 68 to 80. Quality/Tempering is outside the affix test.

#### 13. Hunter double-DoT quiver

**Reference:** `ama-328-hunter-double-dot-quiver` · [3.28 request and answer][hunter-quiver] · [TOML](../fixtures/guides/ama-328-hunter-double-dot-quiver.toml)

- **Start and target:** Hunter Vile Quiver; chaos DoT multiplier with attack skills and generic DoT multiplier with attack skills, plus crafted attack speed. Prefix quality can be a separate stricter target.
- **Recipe:** Alteration-roll each suffix as a solo mod on separate Hunter donors. Recombine until both survive on a two-mod rare. Craft Cannot Roll Attack Modifiers and Exalt prefixes. If unacceptable, lock suffixes/Scour, restore the attack blocker and repeat. Fill prefixes, remove the blocker and craft attack speed.
- **Retry:** Recycle useful recombination failures. Protected prefix cleanup must preserve both suffixes and leave room for the blocker again.
- **Adds beyond the sheets:** No quiver guides exist; this exercises suffix recombination followed by blocked Exalts on an influenced item.
- **Target distinction:** The request mentions physical DoT, but the answer deliberately rolls the generic attack-DoT suffix. Hunter prefixes remain possible; the Exalts do not guarantee only life and bow damage or high tiers.

#### 14. Torment quiver with a final Hunter slam

**Reference:** `ama-326-torment-hunter-fire-dot-quiver` · [3.26 request and answer][torment-quiver] · [TOML](../fixtures/guides/ama-326-torment-hunter-fire-dot-quiver.toml)

- **Start and target:** Initially non-influenced quiver; Torment flat lightning, generic attack DoT multiplier, life, bow damage, crafted attack speed and Hunter fire DoT multiplier.
- **Recipe:** Torment essence until the generic multiplier; annul to those two mods. Craft Multimod plus Cannot Roll Attack Modifiers and Exalt twice for prefixes. Remove crafts, craft attack speed, then Hunter Exalt for the desired fire-DoT suffix.
- **Retry:** Losing either starting mod during isolation returns to essences. A wrong final influenced suffix fails the target and needs another base or a different project.
- **Adds beyond the sheets:** The non-influenced pool is used for prefix filling before influence is added. This is a different operation order and goal from the already-influenced double-DoT quiver.
- **Fixture choices:** Pin the starting quiver and tier thresholds, and confirm the non-attack prefix pool before treating both prefix types as forced. Neither their tiers nor the final Hunter result are guaranteed.

#### 15. Budget mana-RF sceptre

**Reference:** `ama-328-budget-mana-rf-sceptre` · [3.28 request and answer][mana-rf] · [TOML](../fixtures/guides/ama-328-budget-mana-rf-sceptre.toml)

- **Start and target:** Sceptre; natural mana prefix, natural generic DoT multiplier suffix, crafted fire DoT multiplier and crafted spell damage.
- **Recipe:** Roll solo mana on one magic donor and solo DoT multiplier on another. Recombine until both survive, then obtain the rare item state and room needed to Multimod fire DoT multiplier plus spell damage.
- **Retry:** Rebuild or recycle donors after failed combinations. A magic one-prefix/one-suffix result cannot be treated as already having rare-item crafting capacity.
- **Adds beyond the sheets:** A budget mixed-side recombination followed by crafted completion, with a different target from the +2 spell weapons. It is the author's explicit cheaper alternative to the requested four-natural-mod weapon.
- **Fixture choices:** The source does not specify sceptre base, mana/multiplier tiers or a rarity-conversion sequence. The TOML chooses Opal Sceptre with the T2 default and explicitly pays for Regal/annul normalisation when the mixed-side product is magic.

#### 16. Fire-DoT sceptre with three completed suffixes

**Reference:** `ama-327-fire-dot-spell-crit-sceptre` · [3.27 answer][fire-sceptre] · [TOML](../fixtures/guides/ama-327-fire-dot-spell-crit-sceptre.toml)

- **Start and target:** Sceptre; generic DoT multiplier, fire DoT multiplier and essence spell-critical chance suffixes, unveiled increased fire damage and crafted increased DoT damage.
- **Recipe:** Shrieking spell-critical essence until generic DoT multiplier for one pair. Recombine a separate spell-critical donor with isolated high fire DoT multiplier for the second pair. Combine the overlapping pairs until all three suffixes survive. Matron/lock, Veiled Chaos, block physical damage before unveiling, seek increased fire damage and craft DoT damage.
- **Retry:** Failed suffix combinations return to the relevant donor pairs. Failed prefix unveils repeat the protected veil stage only when cleanup is legal; otherwise restart.
- **Adds beyond the sheets:** A three-suffix recombination ladder followed by a rare prefix unveil. The budget mana sceptre instead stops at one natural prefix and one natural suffix.
- **Fixture choices:** Base and exact suffix tiers are not recoverable from the request image. The TOML chooses Opal Sceptre with the user-selected T2 default; the answer does not justify a T1 predicate.

#### 17. Archmage spell-damage/mana wand

**Reference:** `ama-326-archmage-mana-wand` · [3.26 answer][archmage] · [TOML](../fixtures/guides/ama-326-archmage-mana-wand.toml)

- **Start and target:** Wand with fractured spell-damage/mana hybrid; Woe spell damage, T3-or-better mana and cast speed, and unveiled double damage.
- **Recipe:** Woe essence until acceptable mana. Open a suffix if needed, risking a restart. Lock prefixes and reforge caster until acceptable cast speed as the sole suffix. Relock prefixes and Veiled Exalt, aiming to retain cast speed. Block damage-per-charge before unveiling, then seek double damage.
- **Retry:** Losing an ordinary prefix returns to essences. Losing cast speed or missing the unveil returns to protected suffix cleanup and the caster stage; a legal lock is required.
- **Adds beyond the sheets:** A fractured hybrid prefix, three completed caster prefixes and a suffix-add/remove risk. Existing caster wands focus on +2 gem levels.
- **Scope:** The source recommends T3-or-better mana/cast speed as the practical target. Stop the reference predicate at unveiled double damage. Its final “craft double damage” instruction does not name the variant, so do not invent a second compatible double-damage requirement.

### Shields

For the next two recipes, the common protected setup starts with **one natural prefix and one natural suffix on a rare**. Multimod is a suffix, Prefixes Cannot Be Changed is a suffix, and Suffixes Cannot Be Changed is a prefix. Together they give **2P/3S**, with exactly one open prefix. The targeted reforge must preserve both protected sides while adding that prefix; removing crafts then leaves the three wanted natural affixes.

#### 18. +1 chaos, block and ES recovery on block

**Reference:** `ama-328-chaos-block-es-recovery-shield` · [3.28 request and answer][chaos-shield] · [TOML](../fixtures/guides/ama-328-chaos-block-es-recovery-shield.toml)

- **Start and target:** Shaper intelligence shield; T1 increased block prefix, ES recovery on block suffix and +1 chaos spell gems prefix.
- **Recipe:** Alteration-roll recovery and block on separate eligible Shaper donors. Recombine into the two-mod rare. Apply the protected 2P/3S arrangement above, then Harvest reforge chaos. Remove crafts and choose compatible Multimod finishing affixes.
- **Retry:** Rebuild or recycle failed donor combinations. Incorrect slot counts must be repaired before the reforge; a different open slot arrangement is a different recipe.
- **Adds beyond the sheets:** The ES-block sheet obtains three defence/block prefixes before recovery. This recipe starts with one prefix/one suffix and uses both locks to add a gem prefix by tag.
- **Fixture choices:** Pin the shield base and recovery tier. Only the core three natural modifiers are required; the answer leaves its finishing crafts flexible.

#### 19. +1 minions, minion damage and life recovery on block

**Reference:** `ama-328-minion-life-recovery-shield` · [3.28 request and answer][minion-shield] · [TOML](../fixtures/guides/ama-328-minion-life-recovery-shield.toml)

- **Start and target:** Shaper minion-capable spirit shield; natural minion-damage prefix, life recovery on block suffix and +1 all minion gems prefix.
- **Recipe:** Roll/isolate the recovery suffix and minion-damage prefix on separate donors. Recombine into the two-mod rare. Apply the protected 2P/3S arrangement, then Harvest reforge minion. Remove crafts and select compatible finishing crafts.
- **Retry:** Failed donor isolation/recombination returns to donor preparation. Do not substitute a fractured influenced base or an unrestricted minion reforge.
- **Adds beyond the sheets:** An influenced minion offhand and minion-tag completion. It shares the lock arrangement of 18 but tests a different tag, native mod family and eligible base set.
- **Base check:** The pinned Fossilised and Bone Spirit Shields have `focus_can_roll_minion_modifiers`; Titanium Spirit Shield does not. The native damage and gem prefixes require the former tag. Choose an eligible base before compiling the goal.

### Deduplication and exclusions

The comparison was against the existing 46 TOML sheet recipes, not just item names. A changed resistance, defence type, gem element or destination base was not enough to add another entry unless it changed a meaningful goal or method.

- **Sheet overlap removed:** High-ES and high-armour variants; elemental bows/daggers/wands; the usual +2 spell/minion weapons; closely related suppression/minion helmets, strike-range gloves and elevated spectre boots. Examples already covered are [ES/block shield](../fixtures/guides/es-block-shield.toml), [+2 minion wand](../fixtures/guides/minion-plus-two-wand.toml), [physical DoT wand](../fixtures/guides/plus-two-physdot-wand.toml), [strike-range gloves](../fixtures/guides/strike-range-gloves.toml) and [spectre boots](../fixtures/guides/tailwind-plus-two-spectre-boots.toml).
- **League-dependent routes removed:** Allflame/Foulborn previews, sulphur/choice loops, Genesis Tree base production, Infamous or memory-strand crafts, Breach-mod transfers and special talisman rare-imprint routes. No such operation is part of the 19 recipes.
- **Insufficient answers removed:** Image-only targets, vague “recombine and pray” advice, unanswered requests, unexpanded redirects to a spreadsheet, and disputed or corrected guarantees without a clear final recipe. The three-T1-minion-stat Bone Ring request, for example, has no concrete author-supplied method worth promoting here.
- **No silent upgrades:** The Cryonic reference ends at two prefixes; the Hunter quiver uses the suffix the answer actually supplies; the Archmage reference leaves the ambiguous last bench variant unconstrained. None claims a complete original six-affix screenshot target from incomplete text.

### Oracle validation

All 66 fixtures have static source/template/base/tier/operation/price/dependency
checks. `curated_ama_references_have_equivalent_guides` checks the 19 references
below against their 20 TOMLs, including the fractured Ghastly Eye alternative.
Ordinary, Abyss and cluster domains are implemented; older reports of five
jewel/cluster recipes being inactive are superseded. Glyphic's corrupted-essence
effect remains unsupported. Runtime harness output is authoritative for active
versus inactive cases, separately from static validation.

Exact recipe evaluation is checked against concrete sampling without requiring
best-plan discovery to finish. Protected-reforge regressions test both locks,
the forced tagged family and existence of a finished-quality outcome: guaranteeing
a family does not guarantee T2. A bought legacy/elevated witness is an input,
not proof that its acquisition is modelled. Preserve purchased-start boundaries
when tightening goals and rerun template/source and real-item checks.

```sh
cargo test --release -p ponder-rules --test guides
GUIDE=ama-328-crusader-physical-dot-bow LIMIT=200000 RAYON_NUM_THREADS=4 \
  cargo test --release -p ponder-plan --test guides ama_recipes_match_real_items -- --nocapture
```

[ama329]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/ama_crafting_ama_for_329_curse_of_the_allflame/
[ama328]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/
[ama327]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/ama_crafting_ama_for_327_keepers_of_the_flame/
[ama326]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/ama_crafting_ama_for_326_secrets_of_the_atlas/
[rf-jewel]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/comment/oae1vel/
[ghastly]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnrhvlx/
[cold-cluster]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnyvk5l/
[overlord]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/myzpiul/
[despair-ring]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p11eqmy/
[cryonic-ring]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0yz4er/
[dex-ring]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnsez4n/
[attribute-ring]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/mz1o6ha/
[attribute-belt]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnxsv6n/
[horror-ring]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnxr9qu/
[holy-relic]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/comment/oajpqaz/
[physical-bow]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/comment/oaf49c4/
[hunter-quiver]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/comment/oaeuckp/
[torment-quiver]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/myzd2p1/
[mana-rf]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/comment/oaeq46w/
[fire-sceptre]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnsbrm6/
[archmage]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/mz4g4r2/
[chaos-shield]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/comment/oae6mtb/
[minion-shield]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/comment/oae5981/

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

### Corpus and pipeline commands

```sh
cargo build --release -p ponder-plan --example ladder
target/release/ponder-tools run --seconds 120 --workers 4 --out .cache/benchmarks/ladder-case-49 -- target/release/examples/ladder fixtures/solver-ladder.json 49
cargo build --release -p ponder --example startup
target/release/ponder-tools run --seconds 300 --workers 4 --memory-bytes 8000000000 --out .cache/benchmarks/pipeline -- target/release/examples/startup --offline --full --plan-cache .cache/plans
```

Output directories must be new. The historical corpus runner grouped by full
context, requirements and fractures and split its aggregate budgets among child
processes. Its Python implementation has been removed. Application preparation
canonicalized target/fixed order and grouped serialized problem fingerprints.
The legacy-model corpus example has also been removed.

`fixtures/solver-corpus-context.json` pins cluster enchant contexts to the
2000-item fixture's checksum, including 12 ambiguous cases. Historical batches
accounted for every original input. Successful latency percentiles exclude
separately counted failures; no concrete simulation ran in the corpus timing sweep.

The former corpus example supplied the repeated-solve and cache-read measurements
above. The retained native ladder example takes case indices
0–82 and emits `solved` before simulation; any non-`passed` outcome exits nonzero,
including unsupported inputs and simulation limits. The startup example's
`--smoke` selects six products; `--offline` rejects every HTTP request.

Historical backend-only validation: 335 workspace tests pass, one throughput test remains ignored,
and the long exact-guide sweep was checked separately. The final pipeline suite
has 28 passing integration tests. Strict release Clippy and CLI/app release builds
pass. For future solver changes, compare the full guide costs/counters and seeded
simulation output before accepting speedups; compile outside timed runs.

```sh
CARGO_BUILD_JOBS=2 RAYON_NUM_THREADS=4 cargo test --release --workspace -- --skip guides_cost_no_less_than_ponder
cargo test --release -p ponder --test pipeline
cargo clippy --release --workspace --all-targets -- -D warnings
```

## Artifacts

Historical reports, logs and discarded experiment source files are preserved in
`data/benchmarks/solver-history-20261028.tar.gz` (25.3 MB), SHA-256
`0f663c390570bb72279cd765e0455581786c8144897eb378779e33de9216973b`.
Former `.cache/benchmarks/<run>/...` references in this file mean archive members
`benchmarks/<run>/...`; frozen binaries and regenerable prepared problems were
removed. Every archived member was hash-verified. Cleanup accounting lives in
`data/benchmarks/cleanup-20261028.json`.

Key run families inside the archive:

- `merged-*`, `tier-default-cohort-*`, `domain-audit-*`, `domain-wand-corrected-*`:
  goal/rule changes and merged validation.
- `solver-code-profile-*`, `solver-layout-*`, `solver-speed-*`,
  `solver-structure-*`: profiles, matched CPU baselines, final comparisons and
  rejected prototypes.
- `solver-ladder-*`: 83-case validation, simulation/memory comparisons and the
  original influenced-sceptre simulation timeout.
- `solver-corpus-full-*`, `solver-corpus-app-*`, `solver-corpus-warm-*`:
  strict/application batches, statuses/budgets, warm reads and recalculation.
- `solver-unified-guides-numerics-*`, `solver-unification-work-*`: guide
  equivalence, legacy fixture comparisons and numerical migration probes.

Current end-to-end artifacts are in `data/benchmarks/wiring-audit-20261028/`:
`summary.json`, 437-product snapshots, CLI JSON, public-endpoint responses and
final test/Clippy/build logs. The interrupted full cold scan and resumed scan are
recorded separately; neither is a measured 45-second cold run.
