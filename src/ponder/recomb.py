"""Recombinator plans: build two clean donors, recombine them into the target.

How the recombinator works (PoE1, 3.25+, community-measured; see SIDE_COUNTS):

- The output takes the base of one input, 50/50.
- Prefixes and suffixes are pooled separately from both inputs. How many the
  output keeps on a side is drawn from a table that depends only on the
  side's pool size (the total mods on that side across both inputs).
- The kept mods are drawn from the pool, but a mod that can't roll on the
  output's base is removed from the draw *after* the pool size was counted.
  That is non-native natural (NNN) padding: a "+% Evasion" prefix on an
  evasion donor counts towards the pool size, so the output keeps more
  prefixes, yet it can never land on an ES base, so it never displaces a
  wanted mod. With clean donors (wanted mods plus NNN padding, no junk) every
  kept mod is a wanted one, and the output is the target exactly when the
  kept count reaches the wanted count on both sides.

So a plan is: donor A on the target base carries some wanted mods; donor B
carries the rest, either on the target base too (no base coin flip) or on a
partner base whose native mods are NNN for the target (a 50% base flip, but
padding on both sides). Each donor is an ordinary craft with a "no other
mods" constraint, solved like any other item; the plan's expected cost is
(donor A + donor B + fee) / chance of success, every attempt from scratch.

Fractured mods can't move between bases; influenced, veiled and essence-only
"special" mods are left out (3.26 nerfed recombining those).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from itertools import combinations, product

from . import gear
from .craft import Plan, Problem, Target
from .mods import ModDB

# Chance the output keeps k mods on a side, by that side's pool size (both inputs).
# Rows 1-4 are community measurements (3.19 guides, 3.26 re-tests); 5 and 6 are
# extrapolated. Edit here when a patch changes them: everything else derives.
SIDE_COUNTS: dict[int, dict[int, float]] = {
    0: {0: 1.0},
    1: {0: 0.41, 1: 0.59},
    2: {1: 0.67, 2: 0.33},
    3: {1: 0.40, 2: 0.50, 3: 0.10},
    4: {1: 0.10, 2: 0.60, 3: 0.30},
    5: {2: 0.50, 3: 0.50},
    6: {2: 0.30, 3: 0.70},
}
TABLE_VERSION = "3.25 community (rows 5-6 extrapolated)"
RECOMB_FEE = 0.0  # chaos per recombination, on top of the donors
SIDES = ("prefix", "suffix")


def keep_at_least(pool_size: int, wanted: int) -> float:
    """Chance a side with `pool_size` pooled mods keeps at least `wanted` of them."""
    row = SIDE_COUNTS[min(pool_size, 6)]
    return sum(p for k, p in row.items() if k >= wanted)


def chance(a: dict[str, tuple[int, int]], b: dict[str, tuple[int, int]], same_base: bool) -> float:
    """Success chance for clean donors: per side, (wanted mods, NNN padding) on each donor.

    Wanted mods are all distinct and legal on the target base; padding is
    illegal there. Success: the target base, and every wanted mod kept.
    """
    p = 1.0 if same_base else 0.5
    for side in SIDES:
        (wa, na), (wb, nb) = a.get(side, (0, 0)), b.get(side, (0, 0))
        if wa + wb:
            p *= keep_at_least(wa + na + wb + nb, wa + wb)
    return p


@dataclass
class Donor:
    """One recombinator input: a clean craft of some wanted mods (plus NNN padding) on a base."""

    base: str
    base_id: str
    carries: tuple[str, ...]  # wanted target names
    padding: dict[str, int] = field(default_factory=dict)  # side -> NNN mods
    problem: Problem | None = None
    plan: Plan | None = None
    error: str | None = None
    _on_target: bool = True

    @property
    def cost(self) -> float | None:
        return self.plan.cost if self.plan else None

    @property
    def on_target(self) -> bool:
        return self._on_target

    def counts(self, side: str, wants: dict[str, str]) -> tuple[int, int]:
        """(wanted mods, NNN pads) on `side`, for `chance`."""
        return sum(1 for n in self.carries if wants[n] == side), self.padding.get(side, 0)

    def describe(self) -> str:
        pad = " + ".join(f"{n} NNN {side}" for side, n in self.padding.items() if n)
        return f"{self.base}: " + " + ".join(self.carries) + (f" + {pad}" if pad else "")


@dataclass(frozen=True)
class Want:
    name: str
    side: str
    target: Target | None  # None: one filler mod (any resistance)


def wants(problem: Problem) -> list[Want]:
    """What the finished item needs, one entry per mod: targets, then each filler mod."""
    out = [Want(t.name, t.side, t) for t in problem.targets]
    filler_side = {problem.mods[m].side for m in problem.filler if m in problem.mods}
    side = "suffix" if filler_side != {"prefix"} else "prefix"
    out += [Want(f"res {i + 1}", side, None) for i in range(problem.filler_need)]
    return out


def partners(db: ModDB, base_id: str, item_level: int, top: int = 2) -> list[tuple[str, dict[str, frozenset[str]]]]:
    """Other bases of the same class whose native mods can't roll on `base_id`: (base id, side -> NNN mod ids).

    One base per mod pool (the best), most NNN weight first.
    """
    base = db.bases[base_id]
    used = gear.spawn_tags()
    mine = {m.id for m in gear.pool(db, base_id, db.item_tags(base_id, set()), item_level)}
    sig = frozenset(db.item_tags(base_id, set()) & used)
    seen, ranked = set(), []
    for other_id, other in db.bases.items():
        if other.get("item_class") != base["item_class"] or other.get("domain") != base.get("domain") or other.get("release_state") != "released":
            continue
        osig = frozenset(db.item_tags(other_id, set()) & used)
        if osig == sig or osig in seen:
            continue
        seen.add(osig)
        best = _best_of(db, base["item_class"], base.get("domain"), osig)
        nnn: dict[str, set[str]] = {s: set() for s in SIDES}
        weight = 0
        for m in gear.pool(db, best, db.item_tags(best, set()), item_level):
            if m.id not in mine:
                nnn[m.side].add(m.id)
                weight += m.weight
        if any(nnn.values()):
            ranked.append((weight, best, {s: frozenset(v) for s, v in nnn.items()}))
    ranked.sort(key=lambda r: -r[0])
    return [(b, n) for _, b, n in ranked[:top]]


def _best_of(db: ModDB, item_class: str, domain, signature: frozenset[str]) -> str:
    used = gear.spawn_tags()
    ranked = []
    for bid, b in db.bases.items():
        same = b.get("item_class") == item_class and b.get("domain") == domain and b.get("release_state") == "released"
        if same and frozenset(db.item_tags(bid, set()) & used) == signature:
            tags = set(b.get("tags", []))
            ranked.append(("experimental_base" not in tags, "top_tier_base_item_type" in tags, b.get("drop_level", 0), bid))
    return max(ranked)[-1]


def essence_nnn(db: ModDB, base_id: str, item_level: int) -> dict[str, frozenset[str]]:
    """Essence-forced mods that can't roll on `base_id` but are ordinary ("named") mods elsewhere.

    The recombinator counts them towards the pool size and then drops them, so
    they pad a donor on the target base itself, with no base coin flip. Essence-only
    "of the Essence" mods are special and stay in the draw; they're left out.
    """
    from . import actions

    item_class = db.bases[base_id]["item_class"]
    mine = {m.id for m in gear.pool(db, base_id, db.item_tags(base_id, set()), item_level)}
    out: dict[str, set[str]] = {s: set() for s in SIDES}
    for mod_id in actions.essences(item_class, item_level)[1].values():
        mod = db.mods.get(mod_id)
        if mod_id in mine or not mod or mod["generation_type"] not in SIDES:
            continue
        if any(w["weight"] > 0 for w in mod["spawn_weights"]):  # rolls naturally somewhere: a named mod
            out[mod["generation_type"]].add(mod_id)
    return {k: frozenset(v) for k, v in out.items()}


def _bins(db: ModDB, ids: frozenset[str], n: int) -> list[frozenset[str]]:
    """Split NNN mods into `n` sets of whole families, heaviest families spread first,
    so `n` pads are `n` distinct targets."""
    fams: dict[str, set[str]] = {}
    for m in ids:
        fams.setdefault(db.mods[m]["type"], set()).add(m)
    bins: list[set[str]] = [set() for _ in range(n)]
    for i, fam in enumerate(sorted(fams, key=lambda f: (-len(fams[f]), f))):
        bins[i % n] |= fams[fam]
    return [frozenset(b) for b in bins if b]


@dataclass
class Step:
    """Recombine two clean items (donors or earlier steps) into one carrying both sets of mods."""

    a: Donor | Step
    b: Donor | Step
    chance: float

    @property
    def carries(self) -> tuple[str, ...]:
        return self.a.carries + self.b.carries

    @property
    def padding(self) -> dict[str, int]:
        return {}  # NNN pads can't land on the target base: the output is clean

    @property
    def on_target(self) -> bool:
        return True

    @property
    def cost(self) -> float | None:
        if self.a.cost is None or self.b.cost is None or self.chance <= 0:
            return None
        return (self.a.cost + self.b.cost + RECOMB_FEE) / self.chance

    def counts(self, side: str, wants: dict[str, str]) -> tuple[int, int]:
        return sum(1 for n in self.carries if wants[n] == side), 0

    def donors(self) -> list[Donor]:
        return [d for x in (self.a, self.b) for d in (x.donors() if isinstance(x, Step) else [x])]

    def steps(self) -> list[tuple[str, str, float | None]]:
        """The plan in order, as (what, detail, expected chaos): donors first, then each recombination."""
        out: list[tuple[str, str, float | None]] = []
        self._walk(out)
        return out

    def _walk(self, out: list) -> int:
        refs = []
        for x in (self.a, self.b):
            if isinstance(x, Step):
                refs.append(x._walk(out))
            else:
                out.append(("craft", x.describe(), x.cost))
                refs.append(len(out))
        out.append(("recombine", f"#{refs[0]} + #{refs[1]} → " + " + ".join(self.carries) + f"  ({self.chance:.0%})", self.cost))
        return len(out)

    def to_dict(self) -> dict:
        return {"cost": self.cost, "chance": self.chance, "table": TABLE_VERSION, "steps": [{"do": d, "what": w, "chaos": c} for d, w, c in self.steps()]}


class Planner:
    """Recombination trees for one item: clean leaf donors, then the cheapest way to merge them.

    Leaves carry at most `leaf_size` wanted mods (cheap, often magic: alt spam
    or one essence), on the target base (optionally with an essence NNN pad) or
    on a partner base padded with its native NNN mods. Bigger sets are built
    only by recombining smaller ones, cheapest first (a DP over subsets), so a
    4-mod item can come from two 2-mod recombinations. Solve `leaves()` (they
    are ordinary Problems), then call `best()`.
    """

    def __init__(self, db: ModDB, problem: Problem, base_id: str, item_level: int = gear.DEFAULT_ITEM_LEVEL, leaf_size: int = 2, max_wants: int = 6):
        self.db, self.problem, self.base_id, self.item_level = db, problem, base_id, item_level
        self.want = wants(problem)
        self.side = {w.name: w.side for w in self.want}
        self.ok = not problem.fixed and 2 <= len(self.want) <= max_wants
        self._leaves: dict[frozenset[str], list[Donor]] = {}
        self._pools: dict[str, set[str]] = {}
        self._keys: dict[tuple, Donor] = {}
        if self.ok:
            self._build(leaf_size)

    def leaves(self) -> list[Donor]:
        return list(self._keys.values())

    def _pool(self, bid: str) -> set[str]:
        if bid not in self._pools:
            self._pools[bid] = {m.id for m in gear.pool(self.db, bid, self.db.item_tags(bid, set()), self.item_level)}
        return self._pools[bid]

    def _build(self, leaf_size: int) -> None:
        db = self.db
        ess = essence_nnn(db, self.base_id, self.item_level)
        parts = partners(db, self.base_id, self.item_level)

        def room(carried, side):
            return 3 - sum(1 for w in carried if w.side == side)

        def free(ids, carried):
            """Pads that don't share a mod group with what the donor carries (they'd block it)."""
            taken = {g for w in carried if w.target is not None for g in w.target.groups}
            return frozenset(m for m in ids if not taken & set(db.mods[m]["groups"]))

        for size in range(1, leaf_size + 1):
            for carried in combinations(self.want, size):
                if any(room(carried, s) < 0 for s in SIDES):
                    continue
                key = frozenset(w.name for w in carried)
                opts = [self._donor(self.base_id, carried, {})]
                for s in SIDES:  # one essence pad on the target base
                    if room(carried, s) > 0 and (ids := free(ess[s], carried)):
                        opts.append(self._donor(self.base_id, carried, {s: [ids]}))
                for bid, nnn in parts:  # partner bases, 0 / 1 / every free slot padded per side
                    choices = [sorted({0, min(1, room(carried, s)), room(carried, s)}) if free(nnn[s], carried) else [0] for s in SIDES]
                    for counts in product(*choices):
                        pads = {s: _bins(db, free(nnn[s], carried), n) for s, n in zip(SIDES, counts) if n}
                        if pads:
                            opts.append(self._donor(bid, carried, pads, on_target=False))
                self._leaves[key] = [d for d in opts if d is not None]

    def _donor(self, bid: str, carried, pads, on_target: bool = True) -> Donor | None:
        db, pool = self.db, self._pool(bid)
        targets = []
        for w in carried:
            if w.target is not None:
                good = w.target.good & pool
                if not good:
                    return None  # this mod can't roll on the donor's base
                targets.append(Target(w.target.name, w.target.side, frozenset(good), w.target.groups, w.target.notable))
        padding = {}
        for side, bins in pads.items():
            for i, ids in enumerate(bins):
                targets.append(Target(f"NNN {side} {i + 1}", side, ids, frozenset(g for m in ids for g in db.mods[m]["groups"])))
            padding[side] = len(bins)
        key = (bid, tuple(sorted(w.name for w in carried)), tuple(sorted((s, tuple(sorted(tuple(sorted(x)) for x in b))) for s, b in pads.items())))
        if key not in self._keys:
            p = gear.problem_on(db, bid, targets, self.item_level, junk_cap=(0, 0))
            if fillers := sum(1 for w in carried if w.target is None):
                p.filler = frozenset(m for m in self.problem.filler if m in pool)
                p.filler_need = fillers
            self._keys[key] = Donor(db.bases[bid]["name"], bid, tuple(w.name for w in carried), padding, p, _on_target=on_target)
        return self._keys[key]

    def best(self) -> Step | None:
        """The cheapest tree that yields every wanted mod, once the leaves are solved."""
        if not self.ok:
            return None
        names = [w.name for w in self.want]
        # options[S]: clean items carrying exactly S (target-base leaves, partner leaves, steps)
        options: dict[frozenset[str], list] = {k: [d for d in v if d.cost is not None] for k, v in self._leaves.items()}
        best_step: dict[frozenset[str], Step] = {}
        for size in range(2, len(names) + 1):
            for combo in combinations(names, size):
                full = frozenset(combo)
                if any(sum(1 for n in full if self.side[n] == s) > 3 for s in SIDES):
                    continue
                top = None
                members = sorted(full)
                for r in range(1, size):
                    for left in combinations(members, r):
                        s1 = frozenset(left)
                        s2 = full - s1
                        for x in options.get(s1, []):
                            if not x.on_target:
                                continue  # the first input sits on the target base
                            for y in options.get(s2, []):
                                c = chance({s: x.counts(s, self.side) for s in SIDES}, {s: y.counts(s, self.side) for s in SIDES}, y.on_target)
                                cost = (x.cost + y.cost + RECOMB_FEE) / c if c > 0 else None
                                if cost is not None and (top is None or cost < top.cost):
                                    top = Step(x, y, c)
                if top is not None:
                    best_step[full] = top
                    options.setdefault(full, []).append(top)
        return best_step.get(frozenset(names))


def plan(db: ModDB, problem: Problem, base_id: str, table: dict[str, float], item_level: int = gear.DEFAULT_ITEM_LEVEL, workers: int | None = None) -> Step | None:
    """Solve the leaves (in parallel, plan-cached) and return the cheapest tree."""
    from .pipeline import solve_all

    planner = Planner(db, problem, base_id, item_level)
    solve_all(planner.leaves(), table, workers)
    return planner.best()


def simulate(a: dict[str, list[str]], b: dict[str, list[str]], legal: set[str], same_base: bool, wanted: set[str], trials: int = 20000, seed: int = 0) -> float:
    """Monte Carlo of the recombinator as modelled above, on explicit mod lists, to check `chance`.

    `a`/`b`: side -> mod ids on each input; `legal`: ids that can roll on the
    target base (the rest are NNN); success: the target base and every
    `wanted` id kept.
    """
    import random

    rng = random.Random(seed)
    wins = 0
    for _ in range(trials):
        if not same_base and rng.random() >= 0.5:
            continue
        kept: set[str] = set()
        for side in SIDES:
            pool = a.get(side, []) + b.get(side, [])
            row = SIDE_COUNTS[min(len(pool), 6)]
            k = rng.choices(list(row), weights=list(row.values()))[0]
            eligible = [m for m in pool if m in legal]
            kept |= set(rng.sample(eligible, min(k, len(eligible))))
        wins += wanted <= kept
    return wins / trials
