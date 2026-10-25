# Finished item goals and automatic crafting recipes

Status: the goal, automatic route, protected-stage, recovery and reporting changes below
are implemented. This document also retains the broader research roadmap; advanced systems
listed as remaining are not implemented.

Implemented: observed joint goals, explicit useful stats, equivalent single elemental
resistances, independent donor continuations, automatic Awakener/recombination comparisons,
validated natural Focused/Simplex joins, four-fossil component candidates, bench blockers,
terminal bench crafts, protected finishing, priced annul failures and repeated acquisition,
explicit sell/replacement quotes with break-even premiums, and selected-route reporting.

Remaining: general concurrent intermediate bench crafts, influence application/elevation,
imprint/restore, preview locks, Memory Strand routes, influenced/exclusive recombination and
full hybrid-stat mining identities. T1 remains preferred by the user; the current optimizer
minimizes cost to an acceptable floor and does not price a separate T1 quality premium.

Replace generic gear filler with explicit finished-item requirements. Automatically
compare eligible crafting routes against the same complete goal. Compose those
routes from reusable stages, using the existing solver for local item decisions.

Success means implementing the mechanics, connecting them to route generation,
and verifying that complete technical recipes win naturally when they are cheaper.
Adding recipe names, changing the displayed action counts, or artificially
penalizing alterations and augmentations does not satisfy this objective.

## Original causes addressed by this change

- `demand.rs::finished_keys` selects a small core and promotes other stats only
  when they clear a frequency threshold, stopping at `MAX_KEYS = 4`.
  `archetype_match` puts other commonly worn families into one interchangeable
  filler set. Suppression can therefore become an alternative to a resistance or
  attribute rather than a requirement.
- Resistances are folded into one token before mining. The finished goal loses
  distinctions such as required chaos resistance versus sufficient elemental
  resistance. `GearOpts::max_targets = 4` also rejects larger explicit goals.
- `gear.rs::build` emits `any mod` rows and sets `min_mods` to full capacity.
  `Problem::done` can count arbitrary junk toward that capacity. An ANY prefix
  currently means an unconstrained rolled prefix, not necessarily an empty slot.
  Filling that slot does not establish that the item is a useful finished product.
- Archetypes use individual stat frequencies and tiers, rather than preserving
  complete combinations seen on the same items. A mod's first stat is also used
  as its mining identity, weakening distinctions between hybrid modifiers.
- Both donor methods sit behind `opts.recombs`. The recombinator planner accepts
  only ordinary 3/3, unfractured, uninfluenced items. Awakener planning requires an
  existing two-influence goal and an ordinary finishing plan.
- Donor results are costed through the original plan's value table. A missing
  completion state can discard a route instead of triggering a finishing solve.
  Candidate-level errors can prevent donor planning before it is considered.
- Component catalogues exclude metamods and Harvest add/remove, and consider at
  most two fossils. Ordinary bench blockers and concurrent bench crafts are not
  represented as general intermediate actions. The compact state supports one
  metamod at a time.
- `Estimate::recipe`, `steps`, and `usage` still use the ordinary plan directly.
  The desktop has some route-aware rendering, but exported recipe/currency data
  can describe a different route from the cheapest reported total.

## Research and recipe examples

The supplied [Crafting Guide's workbook](https://docs.google.com/spreadsheets/d/12Q1N1E586NfLqoyzIHXU3Y-mCKtTISykeGBUv-HWi5w/edit)
was read through public CSV exports, including its index, currency notes, fracture
bases, cheap ES armour, influenced boots, minion wand, Focused amulet, and armour
finishing examples. Its recurring structures suggest these initial benchmarks:

- ES armour: cheap one-mod donors, prefix recombination, then Eldritch suffix
  finishing; compare a fractured-base and essence alternative.
- Minion wand: a four-fossil prefix stage, protected suffix cleanup, unveil
  blocking, and a named final craft.
- Influenced boots: prepare or buy influenced donors, elevate where required,
  Awakener join, then protected Harvest and unveil finishing.
- Focused amulet: targeted Harvest with bench blockers, recombination between
  different base capacities, and reuse of suitable unsuccessful results.

Alts are legitimate in the first example's donor preparation. A large alteration
count alone does not prove a route is wrong. The complete route, including donor
acquisition, recovery, and finishing, must be compared.

The workbook's index contains 3.29 updates while some recipe pages still contain
split-beast steps the index marks as removed. Local game exports report
`3.29.3.3`. Import recipe ideas with per-operation version checks; do not import
all instructions or stated guarantees as current mechanics.

[Craft of Exile's advanced guide](https://www.craftofexile.com/advanced) explains
blocking through modifier groups and using item level to restrict outcomes.
Its [metamod reference](https://www.craftofexile.com/metamods) distinguishes
operation-specific interactions. These motivate explicit blocker and protection
rules instead of a generic interpretation of every reroll.

[Sirgog's historical minion-wand analysis](https://www.sirgog.com/wp-content/uploads/2023/10/plus-2-minion-wands-322.pdf)
illustrates why eliminating competing modifier groups can matter more than
boosting individual desired weights. Its 3.22 probabilities must be re-evaluated
against current exports rather than reused as current numbers.

## 1. Define one complete item goal

Introduce a typed `ItemGoal` shared by mining, goal validation, planning, trade
queries, pricing, and presentation. It contains:

- Required modifier/stat bundles and their minimum values or tiers.
- One quality preference: T1 is preferred, with T2/T3 accepted when the chosen
  product's quality floor allows them. Stronger rolls always satisfy that floor.
  Do not manufacture a requirement for a deliberate mixture of T1 and T3 rolls.
- Explicit alternatives where substitutes actually satisfy the same product.
- Single elemental resistance rolls normalized by their eligible strength/tier:
  fire, cold and lightning are roughly equivalent because Harvest can convert
  between them. Count the required resistance rolls and their strength rather
  than demanding a particular element throughout preparation. Chaos resistance,
  all-elemental resistance and hybrid modifiers are separate requirements.
- Base, affix capacities, level, influence, fracture, quality and other relevant
  source constraints.
- Named bench finishing and any deliberately open slot.
- The character/build cohort and supporting observed complete items.

Gear should have no generic filler completion rule. Arbitrary junk remains part
of the probability model, but cannot substitute for suppression, resistance, a
defence prefix, or another required feature. Display required features and named
finishes; do not advertise ANY affixes as product features.

Do not require six occupied affixes universally. Some valuable finished items
have fewer affixes or an intentionally open craft slot, and asymmetric bases
have different capacities. Completion means satisfying the declared useful
requirements. Reservations for an intermediate craft are not finished affixes.

Keep cluster-jewel small-passive requirements distinct from gear completion.

Harvest resistance conversion is an actual finishing operation. Use the small
lifeforce cost, preserve the documented similar-tier behavior, and account for
the resulting value range rather than promising an identical numerical roll.
Keep the actual element when it affects intermediate action eligibility. Only
collapse states where legal conversion makes them equivalent for the remaining
goal; observe destination mod-group conflicts and protection restrictions.

The current [Horticrafting listing](https://poedb.tw/us/Horticrafting) lists
single-element conversions at 500 of the destination's lifeforce type. The
[metamod reference](https://www.craftofexile.com/metamods) notes that resistance
swaps cannot be used with suffixes locked. Conversion pricing and eligibility
therefore belong in the operation model, even when their economic effect is small.

## 2. Generate goals from complete observed combinations

Mine complete modifier bundles within relevant build and defence cohorts, then
cluster nearby variants. Preserve co-occurrence, hybrid modifier identity, and
source restrictions. Avoid combining independently common stats into a product
that no supporting item actually carries.

Suppression should become required for a cohort whose item pattern requires it;
do not hardcode that every evasion item needs it. A different build can have a
different legitimate goal on the same base.

Select representative attainable combinations and tier thresholds from their
supporting items. Retain provenance for every requirement. Use the same goal for
the trade comparison so a weaker craft cannot be priced against stronger items.

Remove the four-key definition of a finished product. Represent the full goal
and reduce the dimensions of each planning stage instead of dropping requirements.

## 3. Make route evaluation part of every estimate

All entry points should automatically enumerate eligible direct, bought-start,
fractured, recombinator, and Awakener routes. Remove the requirement to press
Plan donors or enable a shared recombination checkbox.

Whether a method is appropriate depends on the goal, current item, legal base
properties and prices. Do not choose a method solely from the base name, or
force recombination when a cheaper complete alternative exists.

Give each route its own result. Failure to solve a direct craft must not prevent
a donor route. For a join outcome absent from an existing value table, solve the
remaining goal from that explicit starting item and memoize the result. A missing
table entry is not evidence that completion is impossible.

Share donor acquisition and preparation infrastructure while keeping different
join mechanics: recombination inherits selected properties without an Awakener
reforge; Awakener transfers its influenced modifiers and then rerolls the tail.
Track receiver and expendable donor base requirements separately.

Extend recombination eligibility for influenced and asymmetric bases only with
validated rules for base selection, native eligibility, exclusives and capacities.
The Focused-amulet example is an explicit acceptance case; changing only the
current boolean eligibility check would be insufficient.

## 4. Complete the operations needed for technical crafting

First implement reusable operations that make the already available systems
useful together:

- Craft/remove a named temporary blocker, preserving its modifier groups.
- Multiple legal bench crafts, including multimod plus protection/blocking.
- Keep a completed side, clean the other side, then target the missing feature.
- Block before a targeted add or unveil, then remove or replace the blocker.
- Convert eligible single elemental resistance rolls during finishing instead
  of rerolling an otherwise suitable item for a particular resistance element.
- Generate fossil candidates using group elimination as well as desired weight;
  evaluate known four-fossil recipes against current pools.
- Make component action availability depend on its preparation goal, rather than
  globally excluding advanced operations from all donors.

Next add the distinct systems required by higher-end examples: influence
application, Orb of Dominance/elevation, imprint and restore, and the applicable
preview-lock and Memory Strand preparation/recovery operations. Validate current
behavior and prices separately for each. Preview decisions require a different
transition from blindly consuming an Awakener Orb.

Audit final-slam currencies and current league crafting as additional route
providers. Preserve base quality and other preparation value across success,
salvage and restart branches. Non-native natural modifiers, exclusive modifiers,
and temporary crafted modifiers need different recombination semantics.

## 5. Plan stages without a giant inventory search

Use a recipe graph for acquisition, donor preparation, joining, protected-side
completion and final finishing. Each stage declares its preconditions, retained
features, operation, outcome distribution and recovery options. Keep the current
single-item solver for local decisions within a stage.

Represent the identities needed for protection, blocking and special operations
explicitly. Forget irrelevant junk identity where the existing abstraction is
valid. Share donor and finishing subproblems across goals and recipes.

A named macro is not automatically a better policy than the existing solver:
that solver already chains its supported actions. The gains must come from
correct goals, missing operations, broader legal routes and reusable decomposition.

Cost every outcome, including failed joins, salvage, imprint restoration and
restarting. Use `V(s) = min_r [cost(r, s) + sum_o P(o | r, s) V(o)]` over the
relevant stage states. Dividing an attempt cost by success probability is valid
only when every failure returns to the same already-priced restart condition.

Include consumed donor bases, influence premiums, currency, dust and clicking
time consistently. Keep non-tradeable resources visible rather than silently
assigning them a market price. Report uncertainty where probabilities remain
empirical; do not present heuristic route coverage as a proof over all crafting.

## 6. Return one selected recipe everywhere

Produce one recipe result containing the complete goal, chosen route, stages,
costs, expected action counts and failure/recovery branches. The desktop, CLI,
JSON, trade comparison and currency totals should consume that result.

Show donor preparation separately from recipient finishing. Large alt counts
should identify which donor they prepare. Include the cheaper complete route
when it beats direct crafting; never attach a donor-route price to the direct
route's currency list.

## Delivery order and acceptance

1. Replace goal/filler semantics, complete candidate mining and trade matching.
   Confirm suppression cannot be substituted by resistance or attributes and
   that ANY occupancy cannot make an incomplete goal pass.
2. Integrate route evaluation and explicit finishing solves into all entry points.
   Confirm a valid donor route remains available when direct crafting fails.
3. Implement bench blockers, concurrent crafts and stage composition, then
   validate the ES armour, wand and protected armour examples.
4. Extend influenced/asymmetric joins and advanced preparation operations.
   Validate the influenced boots and Focused amulet examples against the active
   ruleset, including their recovery branches.
5. Unify reporting and benchmark the complete automatic pipeline.

Use small exact models for transition tests and real-modifier replay for complete
recipes. Check final useful requirements, costs and reported action counts from
every supported route. Test blocked additions, unveil choices, legal capacities,
protection combinations, base retention and partial-success reuse.

Acceptance includes price fixtures where protected finishing, a four-fossil
recipe, recombination or Awakener preparation is cheaper than direct alt/aug
crafting. The solver must select the complete cheaper route automatically and
report its actual action counts. Measure the reduction against the old direct
plan for the same finished goal. Test cases where direct preparation remains
cheaper as well, so route selection is not a hardcoded preference for a method.

Resistance regressions must accept eligible single fire/cold/lightning rolls at
the accepted quality floor or better and include any necessary final conversion. They must not
let elemental resistance replace required suppression, chaos resistance or an
all-resistance modifier, or let a weaker tier pass as an equivalent roll.

Benchmark the same cached ladder and price inputs with all automatic routes
enabled. Compare warm and cold caches separately and report peak memory. Retain
the existing sub-10-second warm-start target; do not recover it by silently
weakening item goals or omitting expensive route families. Invalidate caches when
the goal contract, mechanics profile or recipe representation changes.
