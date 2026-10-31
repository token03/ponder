# ponder specification

This file is the source of truth for what ponder is and how it is built. If code or another
document disagrees with it, the spec wins or the spec is changed first. Keep it short and true:
no benchmark logs, handoffs or paper reviews here.

## 1. Purpose

At its core, ponder is a **fast, modular stochastic model of the Path of Exile crafting
space**: what every operation does to an item, with what probability, at what cost, and the
cheapest way to get from any item (or nothing) to any goal.

Applications are built on that core, not baked into it:

1. **Cost of an item.** The expected cost to make any finished or partial item, compared with
   buying it. This is the first application and the main acceptance test.
2. **Recipes.** Competitive methods rendered as short steps, stop conditions, ordinary
   fallbacks and per-step costs, readable like the crafting guides. Prefer a simpler recipe
   when its measured cost and risk are comparable; detailed recovery can be requested from
   the actual item.
3. **Profit.** Craft cost against sale price over thousands of candidate items from the ladder.
4. **Anything else the model answers**: selling intermediates, buying partial items, finishing
   an item someone owns, comparing two recipes, how a patch changes a craft.

Reference game version: **PoE 1, 3.29**. PoE 2 is out of scope, but nothing in the solver may
assume PoE 1.

## 2. Glossary

| Term | Meaning |
|---|---|
| **Item** | A concrete item: base, item level, quality, influences, rarity, implicits, and affixes (mod id, side, group, tier, crafted/fractured/veiled flags). |
| **Goal** | A predicate over items: required mods with minimum tiers, interchangeable groups ("any T2+ elemental resistance"), counts, base and item level constraints, slots kept open for a final craft. |
| **Operation** | One game action on one or more items: a currency, essence, fossil set, bench craft, Harvest craft, recombination, fracture, purchase. |
| **State** | What the solver sees: an item reduced to the facts the current goal and operations can tell apart. |
| **Subgoal** | A goal used as an intermediate, e.g. "velour gloves, 26% quality, +2 minion fractured, nothing else on prefixes". |
| **Method** | A way to obtain a subgoal: buy it, craft it from another subgoal, join two subgoals (recombinator), or sell it. |
| **Plan** | The chosen methods for a goal, together with each loop's policy. It renders as a recipe. |
| **Cold run** | Game data, prices and candidate items are on disk; the plan cache is empty. Network time is excluded. |
| **Warm run** | Same as cold, but the plan cache was filled by a previous run with identical data, prices, assumptions and code version. |

Goals describe finished quality, not ingredients. A requirement defaults to T2-or-better;
`tier = N` sets the floor and `tier = "any"` removes it. Floors apply to natural, influenced and
essence mods, which can satisfy each other; tiers compare by mod level, and essence-only
families rank among that base's essences. Crafted, unveiled and fossil-only mods keep their
source: bench mods satisfy only requirements marked crafted, and an unveiled requirement is not
met by its bench counterpart.

## 3. Project immutables

These hold for the life of the project. Changing one is a deliberate decision recorded here.

1. **The model comes first, applications second.** The rules and solver answer "what happens and
   what does it cost". Profit, ladder scans and the app are consumers; none of them may leak
   special cases into the core. The default objective is expected currency at market prices +
   time × the user's chaos-per-hour rate. Other objectives (risk, click count) are explicit
   alternatives, never silently mixed in.
2. **New mechanics are modules, not edits.** Adding a crafting mechanic means adding one module
   that implements the operation contract (§6), plus its data. If adding it requires changing the
   solver, state layout, or another mechanic, the design is wrong and gets fixed first.
3. **The solver knows nothing about PoE.** It solves an abstract cost-minimization problem. All
   game knowledge lives in the rules layer. The complexity of the project belongs in translating
   mechanics, not in the solver.
4. **One implementation per rule.** Each mechanic is written once, against real items. Exact
   distributions, sampling, replay and rendering all use that one implementation. No second
   "lumped" copy of a mechanic.
5. **The simulator is the ground truth.** Any faster model (exact enumeration, compressed states,
   cached kernels) must agree with Monte Carlo of the real-item simulator within tolerance.
6. **Guides are the oracle.** Every human recipe in the guide suite must be expressible as a
   plan, and ponder's best plan must cost no more than the guide's plan under the same prices
   and assumptions, within the margin of error that immutable 5 also allows. Beating a guide is
   fine; losing to one is a bug.
7. **Assumptions live in one place.** Every number that does not come from game data or market
   prices is a named entry in the assumptions file (§10). No magic numbers in code. Assumptions
   are part of every cache key.
8. **Names never change behavior.** Behavior comes from data (mod ids, groups, tags, weights)
   and typed operation definitions, never from matching display strings.
9. **Caches never change meaning.** Cold and warm runs give identical results. Cache keys cover
   every semantic input.
10. **Deterministic output.** Same inputs give the same plans: seeded randomness, stable ordering,
    deterministic tie-breaks.
11. **Slim codebase.** Replace, don't add alongside. When a new path lands, the old path and its
    tests are deleted in the same change. No compatibility shims, no dead modes, no "comparison
    algorithm" kept around without a dated reason to exist.

## 4. Game rules (3.29)

Facts about the game. Each is implemented once in the rules layer and checked automatically for
every operation.

1. **Affix limits.** Magic: 1 prefix, 1 suffix. Rare: 3 prefixes, 3 suffixes (jewels 2 and 2).
2. **One mod per group.** An item never carries two mods sharing a mod group.
3. **Spawning.** A mod can be added only if its spawn tags match the item's base, its level is at
   or below the item level, its side has room, and its group is absent. Its weight comes from
   game data, changed only by declared modifiers (fossils, essences, metamods, Harvest, influence).
4. **Random draws.** Every random addition is a weighted draw without replacement from the
   eligible pool, re-evaluated after each draw.
5. **Fractured mods** survive rerolls and removals. Recombination only pools the fractured
   affixes belonging to the selected input base. Fracturing needs a rare with at least four
   mods and none fractured, on a base that isn't influenced or synthesised. Fracturing ignores "Prefixes Cannot
   Be Changed" and "Suffixes Cannot Be Changed"; neither lock protects an affix from its pick.
6. **Crafted mods** obey the bench limit (one, or three with the multimod metamod). They are
   removed by "remove crafted mods" or a full reset unless fractured. A fractured bench
   modifier remains crafted, consumes its craft slot and cannot be removed.
7. **Metamods** protect exactly what Craft of Exile's table says, operation by operation.
   Essences and fossils cannot be applied with a metamod present.
8. **Information order.** You block before the outcome is drawn, then choose after seeing it.
   Unveils and Awakener previews are an observation followed by a choice, never one combined step.
9. **Unpredictable recombination** accepts magic or rare inputs and retains at most one
   exclusive modifier across the entire output. A natural tiered modifier forced by an
   essence is not essence-exclusive. Influence exalted orbs require an uninfluenced,
   unfractured rare without Eldritch implicits. Eldritch implicits require uninfluenced
   body armour, boots, gloves or helmets.
10. **Veiled Exalted** removes a modifier before adding a veil. The removal respects
    prefix/suffix locks; attack/caster roll blockers do not protect against this removal.

## 5. Modelling rules

1. **Goals are monotone.** A stronger roll or a better tier of a wanted mod always satisfies
   the goal.
2. **Stop conditions are part of the plan.** A loop step carries its condition ("until T1 mana")
   and its fallback ("else go back to step 2"). Plans are policies, not fixed sequences.
3. **The state is derived, not designed.** The goal plus the active operations decide which
   item facts the solver distinguishes. Operations declare what they read (groups, tags, sides,
   flags); everything else is summarized. No per-mechanic hand-written state fields.
   Untracked mods count per class; a class holding k mods keeps the weight expected after k
   weight-proportional draws without replacement over its groups. The simulator check (immutable 5) catches the cases where that estimate is too rough,
   and the fix is to track more facts, not to tune the estimate.
4. **Exact where cheap, sampled where not.** An operation may provide an exact outcome
   distribution. Otherwise its outcomes are estimated by sampling, and the plan is marked
   estimated.
5. **Unpredictable recombination consumes both inputs.** It produces a new item that may be
   reused even when the goal was missed. A discovery recipe may use a simple priced fallback
   and leave elaborate salvage to a recovery request; failure probability is still accounted.
6. **Purchases are methods.** Buying a base, a partial item or the finished item competes with
   crafting it on equal terms. A missing market quote means "unknown", never "cheap".
   An owned starting item is available once; restarting does not replenish it for free.
7. **Selling ends a branch.** Selling an intermediate is a terminal method with negative cost.
   No cycle may earn money.
8. **Unsupported contexts are rejected.** The session planner requires inputs and donors
   to share its fixed base and item level, and already meet any quality requirement.
   Bases with implicit changes to affix limits or explicit-modifier magnitudes are not
   admitted until those properties are represented. Lower-level craft APIs assume this
   preflight has happened.

## 6. Architecture

```
game data ─► rules (Item, Operation, mechanics/*) ─► planner (goals, subgoals, methods) ─► solver
                         ▲                                    │
                     simulator ◄──────── recipe rendering ◄───┘
```

**Rules layer** (`ponder-rules`). Items, goals and the operation contract, one module per
mechanic family under `ops/`:

```rust
trait Operation {
    fn name(&self) -> String;
    fn applies(&self, ctx: &Ctx, item: &dyn View) -> bool; // real items and solver states alike
    fn steps(&self) -> &[Step];                             // what it does, as data
    fn cost(&self) -> Cost;                                 // price ids + seconds
}
```

A mechanic is a composition of primitive `Step`s. `ops::run` samples them on real items (the
simulator); `Lens::run` computes their exact distribution on solver states. Only a new
primitive touches the solver side, and it gets both interpretations plus the agreement test.

The **lens** derives the state (§5.3). Classes split mods only where it changes an outcome: goal
need, tracked group, a tag a draw, removal or ban filters on, influence and unveil preference.
Fossil and veiled mods join the natural mods they share these with. Tracked mods key on all
their tracked groups (a hybrid's conflicts are not its parts'); untracked mods also split by the
draw pools that can roll them.

Still to come in the same shape: observation steps (Awakener), two-item operations, and
**subgoal generators** (a recombinator suggests "only these mods on this side"; fracturing
suggests "4-mod item with X").

**Planner.** An AND/OR graph over subgoals: OR over methods (buy, craft, join), AND over a
join's inputs. Subgoals are shared across every goal in a run. Donor bases, starting points and
recombinators live here.

**Solver** (`ponder-solve`, no PoE types). For one method it finds the cheapest policy over the
derived states, a stochastic shortest-path problem:

- Enumerate reachable states, drop actions that risk an unreachable goal, and solve strongly
  connected components from the goal backwards with exact policy evaluation (dense elimination
  for small components, preconditioned BiCGSTAB / Gauss-Seidel for large ones).
- Starting over is a restart state worth the start's value, found by Newton steps on the start's
  affine value, so restarts do not merge every state into one component. A fixed policy (a guide
  recipe) restarts from states it can never finish.
- Rerolls pass through a hub state (the item after its reset), so their outcomes are stored once.
- Admission: with free restart and nonnegative costs, solve a restricted policy, then check every
  omitted action's Bellman inequality at every known state; admit the best violated action per
  state and resolve. Passing every check certifies the start value. Unexpanded actions are
  optimistic placeholders priced at their immediate cost; the returned policy uses none.
- Recombination expectations are batched in the rules layer with conditional prefix/suffix
  operators, preserving donor correlation, fractured inheritance and global exclusivity. Recombining
  an item that holds junk can be deferred.

Wider focused search (iLAO\* and friends) is a later swap behind the same `Model` trait, adopted
only if it wins on the full workload.

**Simulator.** Runs a plan (ponder's policy or a guide recipe) on real items for validation and
recipe rendering.

Crates: `ponder-rules`, `ponder-solve`, `ponder-plan` (market, craft models, recipes; grows into
the planner), `ponder-net`, `ponder-cli`, `ponder-app`. `ponder-game` and `ponder` are the
previous implementation, deleted as their responsibilities move.

## 7. Performance targets

Measured with a release build limited to 4 worker threads, network excluded. Single-goal
latency must also hold on 1 thread.

| Workload | Cold | Warm |
|---|---|---|
| One goal, price check | < 1 s | < 50 ms |
| 1,000 goals | < 60 s | < 5 s |

Scaling should follow the number of distinct subgoals, not the number of goals. Benchmarks are a
separate command that reports time and work counts (states, transitions, subgoals). They are never
part of `cargo test`.

## 8. Tests

1. **Rule properties.** For every registered operation, a property test generates random legal
   items and checks §4 on the outputs. A new mechanic gets these for free.
2. **Exact agrees with sample.** For every operation that provides `exact`, its distribution
   matches sampling within tolerance.
3. **Guide acceptance.** Every recipe sheet in the guide workbook (42 sheets; the index and
   reference sheets excluded) is encoded in `fixtures/guides/*.toml`, one file per method (46).
   A guide has a goal, named intermediate stages (goals too), and steps: operations
   (`mechanic:argument`) on a named item, repeated `until` a stage holds, with `max` attempts and
   an `else` step to fall back to. `cargo test -p ponder-rules --test guides -- --nocapture`
   checks every reference against game data and prints how many recipes need each mechanic.
   A case runs once all its mechanics exist, so the suite doubles as the progress meter. An
   active case passes when the guide's recipe is expressible and ponder's best plan costs at most
   the guide's within the margin of error (immutable 6).
4. **Small mechanic tests** only for rules with a non-obvious contract.

No timing assertions, no snapshot tests of internal structures, no tests that pin a particular
algorithm's work counts.

## 9. Code style

- Types over flags: enums instead of integer codes, newtypes for ids, interned ids
  (`u16`/`u32`) instead of `String` on hot paths.
- Pure rules, I/O only at the edges (`ponder-net`, CLI, app).
- Small public surfaces. A module exposes what its callers need.
- Comments explain why, not what. Docs stay short and current.
- New dependencies need a reason.
- Release builds only for performance claims.

## 10. Assumptions

Loaded from one file. Each entry has a value, a source and a confidence. Changing one invalidates
caches.

| Name | Value | Source / confidence |
|---|---|---|
| Rare mod count (chaos, alchemy, essence, fossil, Harvest) | 4 / 5 / 6 mods = 9 / 3 / 1 | 1,000-chaos sample (674 / 239 / 87), [Reddit 2020](https://lr.us.psf.lt/r/pathofexile/comments/ex2c5o/comment/fg8je7u). Medium. |
| Rare jewel mod count | 3 / 4 = 3 / 1 | Current code; unverified. Low. |
| Magic mod count (transmute, alteration) | 1 / 2 = 1 / 1 | Current code; unverified. Low. |
| Recombinator dust cost | priced, not zero | Guide note: "dust for recombs still has price". To source. |
| Time per operation | 1–5 s by type; recombine 15 s | Current code; rough. |
| Chaos per hour | 1,500 | User preference. |
| Plain base price | 5c at any item level | Bases are not in the price file. Low. |
| Bought stage prices | `price` on the guide stage | Rough market estimates, marked in each guide. Low. |
| Recombinator price | 10 chaos each kind, when the price file has none | Guess. Low. |
| Recombinator kept affixes | Per side, by pooled count n: n=1 keeps 0/1 at 41:59; 2 keeps 1/2 at 2:1; 3 keeps 1/2/3 at 4:5:1; 4 keeps 1/2/3 at 1:6:3; 5 keeps 2/3 at 43:57; 6+ keeps 2/3 at 3:7 | [poewiki](https://www.poewiki.net/wiki/Recombinator) empirical estimates. Medium; unmeasured above 6. |
| Recombinator selection | Uniform selection within eligible ordinary candidates, skipping conflicting groups. Non-native ordinary modifiers count before selection but cannot survive on an ineligible heir. | Community model; modifier weighting in the current game is not fully known. Medium/low. |
| Recombinator exclusives | At most one globally. Each side's exclusive candidates form one count slot, with one candidate selected uniformly among those pooled; either side is processed first at 50/50. | Global cap: user-supplied wiki and original empirical research, high. Count collapse: [post-3.26 experiments](https://www.reddit.com/r/pathofexile/comments/1ldc3lz/326_recomb_psas_a_few_useful_tipsconfirmations/), medium. Side-order and candidate weighting remain modelling assumptions after 3.26. |
| Recombinator crafted classification | A crafted modifier with an ordinary natural/influenced counterpart in the same data family is non-exclusive; other crafted families are exclusive. Essence-only, veiled, fossil/drop-only and elevated modifiers are exclusive. | Data-derived classification; the full crafted exception list remains incompletely verified. Medium/low. |
| Recombinator bonus mod | Disabled | Unpredictable Recombination has no Sentinel bonus roll. |
| Recombinator heir | Base, influences, quality and Eldritch implicits from either item at even odds; item level two above the floor of the mean, at most the higher input's; influenced mods need the heir's influence | Craft of Exile's simulator; item level from [poewiki](https://www.poewiki.net/wiki/Recombinator). Medium. |
| Fracturing | One uniform pick among all affixes but veiled ones, crafted included; a fractured craft is permanent and still consumes the crafted-mod slot | User (3.29). High. |
| "Cannot Be Changed" refill | New mods may land on either side when it has room | Game behaviour as commonly reported. Medium. |
| Veiled side | Prefix or suffix, even odds | Unverified. Low. |
| Remove-and-veil planning phase | Offered with at most one unmet requirement that cannot be bench-finished; all item requirements remain hard goals. | Search restriction, not a game rule. Preserve Oracle prices when changing it. |
| Unveil | 3 offers drawn by weight without repeats (mods of one class share its mean weight); the wanted offer is kept, else the first | Approximation. Medium. |
| Harvest add/remove cost | 10 × the matching reforge's lifeforce | Unverified (`ADD_REMOVE_FACTOR`). Low. |
| Influence orb price | Priced as the matching conqueror exalted orb | Proxy; not in the price file. Low. |
| Eldritch implicits | Only which side is dominant (Exarch prefixes, Eater suffixes); implicit tiers are not modelled | Game rule, simplified. Medium. |
| Starting over | Any item can be abandoned for nothing and a new start bought | Modelling choice. |
| Guide recipe at the goal | A recipe stops the moment the item meets the goal, even partway through an attempt (no annul after the exalt that finished it) | How crafters play. High. |
| Essence tiers | An essence mod's tier is where its level falls among the base's natural tiers of the same template | Matches how guides name essence tiers. Medium. |
| Guide recipe without a lock | When a step's "Cannot Be Changed" craft cannot be made, the attempt goes to the step's `else` (step 1 if none) | Reading of how guides are followed. Medium. |
| Memory strands | A family rolls its top `floor(100/(2.5s+147.5)·(T−1))+1` tiers (all at 0 strands) and keeps its weight; a currency uses 0 to its most strands evenly (exalt by mods held 0–5: 10/16/22/26/32/34); Remembrance sets 1–100 evenly | [poewiki](https://www.poewiki.net/wiki/Memory_Strand); the even spreads and the exalt onto a bare item are assumed. Medium/low. |
| Strands in solver states | Rounded down to the fewest strands with the same class weights | Abstraction; overstates cost slightly, never understates it. |
| Orb of Dominance | Remove a random influenced mod no lock protects, then upgrade a random influenced mod, locked or not: a tier up, the best tier to its elevated mod | Order and lock reach inferred from guides. Low. |
| Wild Bristle Matron | Adds a "Cannot Be Changed" lock that fits, either as likely | Guess. Low. |
| Imprints in plans | An imprint serves the one operation after it; then the plan restores it or lets it go | Search restriction, not a game rule. |
| Hinekora's Lock | A Craicic Croaker and a Black Mórrígan lock a magic item: the next currency's result shows before it is used, and any currency uses the lock up (a chromatic, to decline) | [poewiki](https://www.poewiki.net/wiki/Hinekora%27s_Lock). Medium. |
| Affix limits from implicits | Cogwork, Simplex and Focused limits change every rarity's, never below none | Unverified. Low. |

## 11. Scope

**v1 (rebuild order):**
1. Base currency: transmute, alteration, augment, regal, chaos, exalt, annul, scour
2. Essences
3. Fossils and resonators
4. Bench crafts and metamods
5. Harvest reforges
6. Eldritch currency
7. Fracturing
8. Purchases and starting items

Then **recombinators** (including the NNN technique), with donor bases. These are a hard priority,
and the planner must support them from day one, even before the mechanic lands.

**Inputs:** ladder demand mining stays as the candidate-item source. Sale prices come from the
trade API, good enough for now and not a focus.

**Later:** synthesis implicits, corruption.
