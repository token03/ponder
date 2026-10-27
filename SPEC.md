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
2. **Recipes.** The cheapest method rendered as steps, stop conditions, fallbacks and per-step
   costs, readable like the crafting guides.
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
5. **Fractured mods** are never removed or changed. Fracturing needs a rare with at least four
   mods, on a base that isn't influenced or synthesised.
6. **Crafted mods** obey the bench limit (one, or three with the multimod metamod). They are
   removed by "remove crafted mods" or a full reset.
7. **Metamods** protect exactly what Craft of Exile's table says, operation by operation.
   Essences and fossils cannot be applied with a metamod present.
8. **Information order.** You block before the outcome is drawn, then choose after seeing it.
   Unveils and Awakener previews are an observation followed by a choice, never one combined step.

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
5. **Recombinator failures lose both inputs (for now).** Guides often reuse a failed output
   ("if it kept the donor's mods, recombine again"). Salvage is the first refinement after
   recombinators land; until then their costs are upper bounds on that point.
6. **Purchases are methods.** Buying a base, a partial item or the finished item competes with
   crafting it on equal terms. A missing market quote means "unknown", never "cheap".
7. **Selling ends a branch.** Selling an intermediate is a terminal method with negative cost.
   No cycle may earn money.

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
enum Step { Clear, Rarity, Settle, RollCount, Force, Craft, Draw, Fill, Remove, Swap,
            RemoveCrafted, Influence, Eldritch, Fracture, Veil, Unveil }
```

A mechanic is a composition of primitive steps. `ops::run` samples them on real items (the
simulator); `Lens::run` computes their exact distribution on solver states. A mechanic is
therefore written once, and a new mechanic needs no solver change. Only a new primitive does,
and it gets both interpretations plus the agreement test. The **lens** derives the state from
the goals and operations (SPEC §5.3).

Lens classes split mods only where it changes an outcome: by goal need, tracked group, a tag
that a draw, removal or ban filters on, influence and unveil preference. Fossil and veiled mods
join the natural mods they share these with, since they differ only in their weights. Tracked
mods key on all their tracked groups (a hybrid's conflicts are not its parts'); untracked mods
also split by the draw pools that can roll them, so a mod a pool cannot roll (a fracture, an
earlier fossil's) never counts as one of that pool's draws.

Still to come in the same shape: observation steps (Awakener preview: draw offers, then choose), operations on two items (recombinator, Awakener), and **subgoal generators**:
the intermediates a mechanic makes useful (a recombinator suggests "only these mods on this
side"; fracturing suggests "4-mod item with X").

**Planner.** An AND/OR graph over subgoals. OR: alternative methods for one subgoal (buy, craft,
join). AND: a join needs both inputs. Subgoals are shared across every goal in a run, so a
thousand goals on the same base reuse the same intermediate solves. This is where donor bases,
starting points and recombinators live.

**Solver** (`ponder-solve`, no PoE types). For one method ("from states in S, reach subgoal P
by repeating operations"), it finds the cheapest policy over the derived states: a stochastic
shortest-path problem. It enumerates reachable states, drops actions that risk an unreachable
goal, and solves strongly connected components from the goal backwards with exact policy
evaluation inside each; evaluation splits again by the components of the chosen policy's graph,
so each linear solve is only as large as one loop: dense elimination up to 1500 states,
Gauss-Seidel with self-loops eliminated beyond. A fixed policy (a guide recipe) may give up:
states it can never finish restart instead, as a crafter abandons a bricked item. Starting over is a restart state worth the start's own value, found by
Newton steps on the start's affine value, so buying fresh does not merge every state into one
component. Rerolls (chaos, fossils, reforges, essences on rares) pass through a hub state, the
item after its reset, so their thousands of outcomes are stored once per hub rather than once
per item. Any focused search (iLAO\* and friends) is a later swap behind the same
`Model` trait, adopted only if it wins on the full workload.

**Simulator.** Runs a plan (ponder's policy or a guide recipe) on real items for validation and
recipe rendering.

Crates: `ponder-rules` (items, goals, operations, lens, simulator), `ponder-solve`,
`ponder-plan` (market, craft models, recipes; grows into the planner), plus `ponder-net`,
`ponder-cli` and `ponder-app`. `ponder-core`, `ponder-game` and `ponder` are the previous
implementation, deleted as their responsibilities move.

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
| Recombinator mods kept per side | by pool size; table in assumptions file | [poewiki](https://www.poewiki.net/wiki/Recombinator) / Butsicles. Medium; unmeasured above 6. |
| Recombinator dust cost | priced, not zero | Guide note: "dust for recombs still has price". To source. |
| Time per operation | 1–5 s by type; recombine 15 s | Current code; rough. |
| Chaos per hour | 1,500 | User preference. |
| Plain base price | 5c at any item level | Bases are not in the price file. Low. |
| Bought stage prices | `price` on the guide stage | Rough market estimates, marked in each guide. Low. |
| Recombinator price | 10 chaos each kind, when the price file has none | Guess. Low. |
| Recombinator kept affixes | Per side, by pooled count n: n=1 keeps 0/1 at 1:2; 2 keeps 1/2 at 2:1; 3 keeps 1/2/3 at 3:5:2; 4 keeps 1/2/3 at 2:11:7; 5 keeps 2/3 at 1:1; 6+ keeps 2/3 at 3:7. Uniform picks; a pick sharing a group with a kept one is discarded | Craft of Exile's simulator. Medium. |
| Recombinator bonus mod | 25% when the result has room: an exalt on the side with room (a magic result's empty side) | Craft of Exile's simulator; its rarer recombinator-only mods are not modelled. Medium. |
| Recombinator heir | Base, influences, quality and Eldritch implicits from either item at even odds; item level the floor of the mean; influenced mods need the heir's influence | Craft of Exile's simulator. Medium. |
| Fracturing | One uniform pick among all affixes but veiled ones, crafted included; a fractured craft is permanent | User (3.29). High. |
| "Cannot Be Changed" refill | New mods may land on either side when it has room | Game behaviour as commonly reported. Medium. |
| Veiled side | Prefix or suffix, even odds | Unverified. Low. |
| Unveil | 3 offers drawn by weight without repeats (mods of one class share its mean weight); the wanted offer is kept, else the first | Approximation. Medium. |
| Harvest add/remove cost | 10 × the matching reforge's lifeforce | Unverified (`ADD_REMOVE_FACTOR`). Low. |
| Influence orb price | Priced as the matching conqueror exalted orb | Proxy; not in the price file. Low. |
| Eldritch implicits | Only which side is dominant (Exarch prefixes, Eater suffixes); implicit tiers are not modelled | Game rule, simplified. Medium. |
| Fracturing | Rare, at least 4 mods, not influenced, none fractured; fractures one random mod | Game rule. High. |
| Starting over | Any item can be abandoned for nothing and a new start bought | Modelling choice. |
| Guide recipe at the goal | A recipe stops the moment the item meets the goal, even partway through an attempt (no annul after the exalt that finished it) | How crafters play. High. |
| Essence tiers | An essence mod's tier is where its level falls among the base's natural tiers of the same template | Matches how guides name essence tiers. Medium. |
| Guide recipe without a lock | When a step's "Cannot Be Changed" craft cannot be made, the attempt goes to the step's `else` (step 1 if none) | Reading of how guides are followed. Medium. |

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

**Later:** unveiling, Awakener, beastcrafting, imprints, elevation, influence application,
catalysts, synthesis implicits, Eldritch implicits.
