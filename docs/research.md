# Ponder: research results and remaining work

Updated 2026-10-27 against pinned RePoE 3.29.3.3. This is the consolidated
record of solver research, correctness audits, recipe experiments and candidate
generation. It replaces the separate research reports, comparison JSON files,
performance diary and optimization plan. [SPEC.md](../SPEC.md) defines the
architecture; [the oracle index](crafting-ama-recipes.md) preserves the authored
reference recipes.

## Current position

- The complete T2-default cohort before the component-layout change has 41 passing guides, 24 inactive and one bow
  state-budget failure. Thirty passing final-item solves are under one second,
  13 under 100ms and eight under 50ms. All use the usual 200,000-state, 8 GB
  address-space, 120-second and four-worker limits. These are single observations,
  including borderline timings, not established latency guarantees.
- Requiring the bow's unveiled suffix invalidates the previous cheap bench finish.
  The full bow now hits the state cap after 51.7s; the supplied-fracture bow solves
  in 2.58s. Sceptre and recombined regalia take 10.14s and 17.67s. Recombined bleed
  axe is 0.63s; the corrected essence axe is 2.67s.
- Common cluster-notable candidates mined from the cached ladder solve in
  milliseconds. Candidate mining already exists in the older implementation;
  its connection to the new planner is not yet a production feature.
- Unrestricted optimization spends considerable work on alternative and recovery
  policies that do not belong in a short main recipe. Local repairs remain
  valuable. Detailed recovery can be requested for the actual item instead.
- The newer planner returns a policy, not a complete human-readable guide.
  Compact recipe discovery and rendering remain substantial work.
- The all-guides-under-one-second target is incomplete. Goal fidelity and shared
  mechanic errors must be audited before attributing large cost wins to planning.

## Code-level performance audit

The 2026-10-27 code audit measured the current T2 targets using temporary Rust
timers/counters. Sampling with samply was blocked by `perf_event_paranoid = 2`;
no system setting was changed. Temporary instrumentation was removed afterward.
Profiles are in `.cache/benchmarks/solver-code-profile-20261027`. Four workers,
200,000 states per attempt, 8 GB address space and 120s process deadlines remain
in force. The times below are instrumented single observations, not estimates
of instruction-level CPU utilization or stable latency distributions.

### What the bow failure actually means

The reference bow recipe itself solves in about 6ms with 821 states, at a model
cost of 5,260.5c. Its concrete simulation also passes. The optimizer's error does
not mean that this item is impossible or that executing its recipe is expensive
to compute. It fails while searching a much larger set of alternative policies.

The traced optimizer does the following:

1. The ordinary search reaches 200,000 states in 3.93s. Its first-discovery
   counters attribute about 69k states to fracturing, 47k to unveiling and 20k
   to the prefix lock. These counts depend on expansion order; an item can be
   reachable through more than one method. They are not exclusive causes.
2. It constructs a certified side-only lower bound in another 0.65s. The bound
   model has 31,817 states. It relaxes the opposite side, making its continuation
   cheaper than the real craft; this is safe but insufficiently selective.
3. Retrying with this bound spends 36.34s solving a restricted graph of 110,797
   states. This involves 1,287 expansion rounds, 15 global solves and 80 Newton
   steps. Many bound-valued frontier states are discovered and expanded between
   global solves; expansion rounds are not the same as global solves.
4. The restricted answer raises the action-cost ceiling from about 337c to
   5,406c. Another 4.38s reaches 200,000 states again. This is the returned error.
   `Craft::solve` returns that side-bound retry directly, so it does not then
   attempt the other generic relaxation after this failure.

The 200,000 cap counts solver nodes, including operation continuations and
frontier nodes, not just distinct finished items. Peak RSS is about 554 MB:
the observed failure is the explicit state-count guard, not allocation refusal.
The second cap still includes about 27k unexpanded frontier tips.

The expensive combinations join three elemental prefixes to the suffix-side
facts needed for a legal protected unveil and the final opposite speed/critical
craft. Fracture choices, temporary crafted blockers, occupied suffix slots,
failed unveils and annul repairs all change the continuation. The operation set
also permits exploring fracture-first and reroll alternatives to recombination.
Rare affixes are handled by probabilities; the solver is not literally sampling
millions of essence rolls. Enumerating distinct continuations is the costly part.

The normal error API discards failure-phase statistics and any earlier incumbent
solution. Returning a verified feasible policy plus an incomplete-search status
at the budget boundary would be useful for discovery, but would not certify
global optimality or make discovery itself subsecond. No such API change is
included in this audit.

### Where the sceptre and regalia spend time

| Instrumented phase | Sceptre | Full ES regalia |
|---|---:|---:|
| Model exploration and linking | 1.33s | 2.43s |
| Graph pruning and outer component ordering | 0.83s | 0.77s |
| Policy value work, total | 6.27s | 7.33s |
| Deferred-action separation, total | 2.18s | 4.73s |
| Full final-item solver measurement | 10.80s | 15.45s |

Within policy value work, the sceptre spends 2.21s rebuilding policy components,
2.11s scanning actions and only 0.32s in dense/iterative linear solves. The
corresponding regalia numbers are 1.53s, 1.69s and 2.55s. These are nested timings
and must not be added to the total policy value work again.

The sceptre evaluates 5.58m state rows and constructs 5.18m policy components,
of which 5.16m are singletons, across 11 global solves and 32 Newton steps. The
graph has 181,749 explored nodes, 2.38m actions and a selected policy with 5,235
nodes. Most repeated component construction produces tiny acyclic pieces rather
than solving one enormous numerical system. Its 970 BiCGSTAB calls execute 5,773
loop iterations in total; slow numerical convergence is not the main bottleneck.

The regalia evaluates 3.60m rows and constructs 3.00m policy components, with
2.99m singletons. Its 106,405-node graph has 12.24m outcomes and more costly
linear work. It is not interchangeable with the sceptre as a benchmark.

### Concrete Rust findings

The implementation already uses release optimization, fixed-size state arrays,
integer node IDs, FxHashMap, Rayon expansion, operation continuations and exact
singleton arithmetic. Those are useful choices. The remaining issues are in
allocation, representation and scheduling:

- `components` allocated a separate `Vec<usize>` for each output component and
  a new DFS stack per root. Millions of singleton components turn this into
  millions of small heap allocations. The retained change stores membership in
  one vector with component offsets and reuses the DFS stack. Traversal order,
  component membership, numerical equations and policy choices are unchanged.
- Every `evaluate` still rebuilds the selected-policy component partition,
  scratch positions and cyclic rows. Affine values are restarted at each global
  solve. Workspaces and dependency-aware invalidation are more targeted than
  caching only complete unchanged large components, which was previously tried.
- Greedy improvement scans every action at each participating state, including
  many whose relevant successors did not change. This remains a measured cost.
- Graph actions/outcomes and linear rows use nested vectors/boxed slices.
  Contiguous arenas and sparse row offsets could reduce allocations and pointer
  chasing. Their construction/update costs need measuring before a wider rewrite.
- `recombination_kernel` is rebuilt by expectation, support and transition-table
  preparation. The sceptre's 40 expectation tables spend 1.08s constructing
  kernels, 0.12s obtaining value grids and 0.28s in right/left contractions. The
  regalia's 51 tables spend 1.49s, 0.16s and 1.60s respectively. This distinguishes
  expensive structure preparation from arithmetic that could be offloaded.
- BiCGSTAB allocates several vectors per iteration and ILU application returns
  a fresh vector. Reusing these buffers is sensible, but accelerating a 0.32s
  phase cannot resolve a 10.8s sceptre solve. The regalia gives it more headroom.

The initial instrumented flat-layout comparison preserves exact costs, state
counts, policy sizes and iteration counts: sceptre 10.80s to 7.73s, regalia
15.45s to 14.35s. This is one screen, not a statistical claim. The clean matched
binary comparisons below exclude temporary profiling overhead.

The clean comparison uses the same four cases and resource limits, two baseline
samples and one final sample per case. Additional interrupted-run data is not
combined with this table. These observations support retaining the change but
do not establish statistical significance or new latency guarantees.

| Guide | Baseline median | Final sample | Time change |
|---|---:|---:|---:|
| T2 fire-DoT sceptre | 8.722s | 7.644s | -12.4% |
| Full ES regalia | 15.175s | 14.141s | -6.8% |
| Recombined bleed axe | 0.503s | 0.457s | -9.2% |
| Cold three-notable cluster | 4.959ms | 5.694ms | +0.735ms |

All four preserve exact costs, explored/retained counts, actions, outcomes and
iteration/admission counters. The cluster difference is a small absolute change
on a millisecond case; more samples would be needed to attribute it. The final
source passes 42 focused release tests, strict Clippy for all targets of the four
new crates, formatting and whitespace checks. No new full-cohort sweep or final
bow timing was performed for this representation-only change. Baseline and final
artifacts are `.cache/benchmarks/solver-layout-baseline-20261027` and
`.cache/benchmarks/solver-layout-cpu-final-20261027`.

### CPU priorities after the layout change

The user parked CUDA work. Further optimization should be measured against the
Rust CPU implementation, without changing goals, prices, probability rules or
resource caps. The next candidates are:

1. **Reuse prepared recombination structure within a separation pass.** Expectation,
   successor-support and transition preparation currently reconstruct related
   side kernels. A typed preparation context can share those immutable kernels
   while keeping values and actual source-pair support distinct. This is narrower
   than the previously unsuccessful long-lived cache and avoids session-wide
   cache invalidation. The measured kernel-construction cost justifies a prototype.
2. **Reduce repeated evaluation by dependency, not by whole-component identity.**
   A small policy change currently makes a large component rebuild its entire
   selected-policy partition. Fine-grained dirty propagation could retain clean
   subcomponents and their affine values. External exits, policy changes and
   changes in restart worth must invalidate the appropriate decisions; simply
   reusing old numeric values would be incorrect. The complete-component cache
   already tried is not evidence for this finer approach.
3. **Use local deterministic completion ceilings.** `Craft::finish_cost` already
   prices deterministic bench finishes when constructing a global repeat bound,
   but `Craft::actions` does not use that per-item ceiling. With nonnegative
   future costs, a state that can finish for 1c need not explore a 240c fracture.
   This is exact cost dominance, not a rarity cutoff or recipe-stage restriction.
   Avoid evaluating expensive finish predicates on every state blindly: the
   overhead of obtaining the bound needs measuring alongside expansion saved.
4. **Compact hot adjacency and numerical rows.** The current nested vectors and
   separately boxed outcomes require many allocations and indirect reads. Sparse
   row offsets with contiguous outcomes can make greedy scans and matrix-vector
   products cheaper. Dynamic action admission needs an append-friendly arena,
   not repeated rebuilding of a whole compact graph after every insertion.
5. **Reuse numerical and policy scratch storage.** Chosen/improved policy vectors,
   evaluation positions, BiCGSTAB work vectors and ILU output buffers can retain
   capacity. This is clean safe Rust, but numerical-only savings are limited by
   the small sceptre linear phase; prioritize the regalia for that experiment.
6. **Give the bow stronger search boundaries and a budget-result contract.**
   Layout changes improve the cost per search step but do not remove the 200k
   logical cap. Better admissible continuation bounds, compact recipe discovery
   and retaining a verified incumbent on exhaustion address different parts of
   that problem. No extra speculative recovery branches should be added merely
   because cheaper allocation makes room for them.

These are ranked experiments, not promised speedups. Keep a change only after
matching costs and graph/policy checks and measuring the same bounded cases.
No unsafe code, new allocator or additional dependency is needed for the retained
component-layout change.

### CUDA investigation parked

The material below records the completed feasibility assessment. No further GPU
work is planned following the user's instruction to focus on the Rust CPU code.

This host exposes an RTX 5070 Ti with about 16 GB VRAM. The CUDA driver is present;
`nvcc`, cuBLAS and cuSPARSE are not installed in the current environment. No GPU
runtime benchmark was performed. GPU value iteration is a real approach, including
[large-state MDP work](https://arxiv.org/abs/2303.10672) and
[sparse interval-MDP implementations](https://arxiv.org/abs/2401.04068); those
results do not establish a speedup for these crafting graphs.

A direct line-by-line port would retain dynamic hash insertion, variable-length
outcomes, branching operations, graph discovery and sequential dependencies.
The current implementation is not one large matrix multiply. Better GPU targets
are batched Bellman action evaluations over resident sparse arrays, recombination
contractions over prepared kernels, and sufficiently large sparse policy solves.
[cuSPARSE](https://docs.nvidia.com/cuda/cusparse/) supplies relevant primitives.

Infinite acceleration of the sceptre's measured 0.32s linear phase alone would
leave about 10.48s. The regalia's 2.55s phase would leave about 12.90s. These are
optimistic arithmetic ceilings with no transfer, allocation or launch cost, not
predicted GPU times. Accelerating action scans and table contractions too would
increase coverage, but requires changing the representation and keeping data
resident across iterations. NVIDIA's
[best-practices guide](https://docs.nvidia.com/cuda/cuda-c-best-practices-guide/)
explicitly emphasizes end-to-end parallel coverage and transfer minimization.

Current numerical tolerances use `f64` around 1e-12; Tensor Core FP16/TF32 peak
rates are not directly applicable. The RTX's compute-capability-12.0 FP64
arithmetic throughput is much lower than FP32 throughput according to NVIDIA's
instruction table. Sparse work may still benefit from memory bandwidth, so this
does not rule out a useful GPU backend. Mixed precision would need explicit
residual/certificate checks rather than silently weakening the solver.

The next GPU experiment should benchmark a frozen real graph's Bellman scans and
table contractions, including preparation, upload, repeated evaluation and
download. Meanwhile, reducing repeated graph work and allocation helps the CPU
implementation and creates the contiguous representation a GPU backend needs.

## Oracle fidelity audit and corrected cohort

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

The latest complete sweep, including the additional domain corrections below, gives:

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

Latest artifacts are `.cache/benchmarks/domain-audit-20261027` and the focused
`.cache/benchmarks/domain-wand-corrected-20261027`. Builds are outside measured
solves. Each passing guide still checks its reference and Ponder policy on real
items; shared simulator agreement does not independently prove a game rule.
The full bow remains a state-limit failure; no resource cap was raised.

Final validation passes 50 release tests across rules, lens transitions,
recombination, goal matching, guide parsing, recipes and session admission, with
one ignored throughput test. Release Clippy passes all targets for the four new
crates with warnings denied. The complete-cohort results plus the corrected wand
rerun above cover reference and concrete-policy simulation separately.

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

## Earlier bow and cluster checks

The bow prices and success below precede the unveiled-source correction above.
They are retained as experiment history, not current acceptance results. Cluster
goals and their conclusions are unchanged.

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

Implemented improvements include:

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
separately from essence uses. These diagnostics describe the earlier policy;
they do not remove the large retry component or change the solver's objective.
