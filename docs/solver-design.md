# Solver design: staged crafting with exact mechanics and a value layer

Status: proposal. What exists today is one stage of this (single-item MDP over
currency actions); this document is where it goes next and why.

## The problem with "search the whole graph"

Crafting is procedural. A real craft is a short sequence of *phases*
(acquire a core mod, isolate it, combine, protect a side, finish), and inside
each phase only a handful of tools make sense. Searching every currency at
every step is both too big and pointless: nobody alt-spams a 5-mod rare, and
nobody recombinates a normal item.

So the search is constrained by **structure**, not by forbidding options:

1. **Recipe grammars fix the topology, the solver fills in the actions.** A
   grammar is a small graph of stages. Each stage has an entry condition, an
   exit condition (the hand-off), the actions it may use, and invariants it
   must keep (e.g. "completed suffixes stay"). The solver picks *which*
   essence, fossil, Harvest tag, donor, tier to settle on, and when to stop.
2. **Each stage is an exact MDP over a small projected state** (what we
   already solve: rarity, per-target status, counts of filler and junk per
   side). Stages are chained by backward induction: the value of a stage's
   exit states is the next stage's cost-to-go from them, so a stage never
   "finishes" into a state the next stage hates.
3. **Hand-off states are few.** "Magic item with core mod X only", "rare with
   suffixes done and 1+ open prefix", "item with N desired mods and an open
   slot". That keeps cross-stage composition cheap even when each stage's
   inner MDP is big.

Options aren't lost because the grammar library is open-ended (a new trick is
a new grammar or a new action in a stage), and within a stage the solver still
considers every legal action.

## Two layers that must not be merged

- **Mechanics layer (exact).** Every mod is a real mod id with its family,
  groups, tier, item level, spawn tags, mod tags, source (native natural,
  non-native natural relative to a *specific* base, essence-only, crafted,
  veiled, fractured, influenced, exclusive). Legality and weights are always
  computed from this. Fire res and cold res are different here (Harvest and
  fossils treat them differently).
- **Value layer (projected).** A target spec maps an exact item to utility:
  core mods, preferred tiers with settle curves, interchangeable groups
  ("any 2 resistances, 80% total"), optional mods with marginal value, and
  the value of open affixes.

The planner's projected state (today's `State`) is a sufficient statistic for
the actions in a stage only because the profile weights are computed from the
exact pool; the exact simulator (`craft.simulate`) replays plans on real mods
to check that. Keep both: plan on the projection, verify on exact mods.

## Settling falls out of a STOP action

Today a plan must reach "done". With a value layer, every state gets a STOP
action whose reward is the item's value there (sell price, or utility). Policy
iteration then maximises `value - expected cost`, and "keep the T2 life, don't
risk the +1/+1 for T1" is simply where STOP beats every continuation. This is
a small change to the solver (one more action per state) and the biggest
change to what it recommends.

## Binary actions are a higher-level planner

Recombinators and Awakener's Orbs combine two items, so they don't fit a
single-item MDP. They live one level up, in an **assembly planner** over
intermediates:

- an intermediate is "item with these desired mods (+ these fillers) on this
  base", produced by a single-item stage, with an expected cost;
- a combination is a kernel `(A, B) -> distribution over outputs`;
- the planner searches small trees: which base is the output, how the core
  mods are split across donors, what filler each donor carries.

That search is enumerable: at most six desired mods, two inputs per node, and
a few candidate bases.

### Recombinator kernel

Community-reported (3.26+, unverified): prefixes and suffixes are treated
separately; the output count on a side depends on the combined pool size on
that side (e.g. a pool of 3: 40% / 50% / 10% for 1 / 2 / 3 mods; a pool of
4: 10% / 60% / 30%); then that many mods are drawn from the pool, and only
mods legal on the output base can land. Since 3.26, inputs leaning on mods
that "cannot normally roll" (essence-only, veiled) give fewer mods on average.

Model it as a **versioned empirical table** (counts by pool size, plus a
penalty rule), not as derived mechanics, so a patch is a data update.

### NNN is a relation, not a property

A mod is non-native natural *with respect to an output base*:
`nnn(mod, base) = natural(mod) and not legal(mod, base)`. NNN mods inflate the
pool size (more output slots) without competing for those slots on the output
base. So donor design is: maximise pool size while keeping the pool, restricted
to the output base, close to "only the desired mods". With the legality engine
this is a search over which cheap natural mods to put on which donor base.

## Information actions (beast lock + Awakener preview)

A lock turns one stochastic transition into "observe the outcome, then choose
whether to commit". With a donor portfolio `D`, the relevant value is
`E[max_d V(preview(s, d))]` rather than `E[V(awaken(s, d))]`, and re-locking
gives repeated draws. In the MDP this is an observation node: from a locked
state, the action set is "commit with donor d" for each previewed d, or
re-lock. Community testing suggests previews are deterministic per donor
"signature" under one lock; treat that as an empirical, versioned rule.

## Heuristics that prune without losing real options

- **Dominance:** rerolls with identical outcome distributions keep only the
  cheapest (done); a reroll that can't touch a wanted slot is dropped (done
  for Harvest+lock).
- **Anchor order:** secure first the mod with the highest
  `marginal value x reacquire cost / acquire chance`; stages after it must
  preserve it (fracture, lock, or isolate-and-combine).
- **Stage gates:** recombinators and Awakener only on items whose desired mods
  are isolated (magic, or few mods); alts only while magic; exalt-style slams
  only when open slots exist on the side they can hit.
- **Base choice as a variable:** the best base for selling, but donor bases
  chosen for NNN.

## Order of work

1. Value layer: target spec (core, settle curves, interchangeable groups,
   optional, open-affix value) and the STOP action. Today's filler is a first,
   hard-coded instance of an interchangeable group.
2. Stage grammars for single items: anchor -> protect -> finish, with
   fracture, essence, fossil, Harvest, metamods and eldritch as stage actions,
   and backward induction between stages.
3. Legality engine with source classification per base (native, NNN,
   essence-only, veiled, exclusive) as an explicit API.
4. Assembly planner with a recombinator kernel table.
5. Influence: Dominance, Awakener, lock/preview observation nodes.
6. Prices and market search on top (exists in part).

3.30 may change recombinator odds, beastcraft recipes and Harvest; every
community-derived number above lives in one versioned table so it can be
updated without touching the solver.
