"""Cheapest crafting policy for a set of target mods.

The item is lumped into a small state: rarity, for each target whether it is
missing, on the item at a good tier, or on the item at a bad tier (which blocks
it, since mods of one group exclude each other), plus how many other ("junk")
mods sit on each side. Notables are counted apart from other junk because
cluster jewels cap them.

Every currency is an action mapping a state to a distribution over states.
Policy iteration finds, for every state, the action minimising the expected
total cost *to finish*, so the plan looks all the way ahead: it will pay for
an expensive action now when that saves more later. The policy is the recipe.

`simulate` replays a policy on real mods (exact mod groups, no lumping) to
check the lumped numbers.
"""

from __future__ import annotations

import itertools
import random
from dataclasses import dataclass, field
from functools import cached_property

import numpy as np

from . import data

# Assumptions not in the game data; adjust here if testing shows otherwise.
MAGIC_TWO_MOD_CHANCE = 0.5  # transmute/alteration roll 1 or 2 mods
# Mod count when an item becomes rare (alchemy, chaos, essence, fossil, Harvest).
# Gear follows a 1,000-chaos sample (4:5:6 = 9:3:1); jewels assume the same 3:1 step.
RARE_MOD_COUNTS = {"jewel": {3: 0.75, 4: 0.25}, "gear": {4: 9 / 13, 5: 3 / 13, 6: 1 / 13}}
RARE_AFFIX_LIMIT = {"jewel": 2, "gear": 3}  # per side
SIDE_SLOTS = {"prefix": (0, 1, 4), "suffix": (2, 3, 5)}  # State.junk slots on each side
JUNK_LABELS = ("other prefix notable", "other prefix", "other suffix notable", "other suffix", "filler prefix", "filler suffix")

NORMAL, MAGIC, RARE = 0, 1, 2
MISSING, GOOD, BLOCKED = 0, 1, 2


@dataclass(frozen=True)
class Mod:
    id: str
    side: str  # "prefix" or "suffix"
    weight: int
    groups: tuple[str, ...]
    notable: bool
    label: str
    tags: tuple[str, ...] = ()  # implicit tags, used by fossils and Harvest


@dataclass(frozen=True)
class Target:
    name: str
    side: str
    good: frozenset[str]  # mod ids that satisfy it
    groups: frozenset[str]  # other mods in these groups block it
    notable: bool = False


@dataclass(frozen=True)
class State:
    rarity: int
    status: tuple[int, ...]  # per target: MISSING, GOOD or BLOCKED
    # Mods that aren't targets, by slot: prefix notable, prefix other, suffix notable,
    # suffix other, prefix filler, suffix filler (filler: wanted, but any will do).
    junk: tuple[int, ...] = (0, 0, 0, 0, 0, 0)

    def __post_init__(self) -> None:
        if len(self.junk) < 6:
            object.__setattr__(self, "junk", tuple(self.junk) + (0,) * (6 - len(self.junk)))

    def __hash__(self) -> int:  # states are hashed millions of times while solving
        try:
            return self._hash
        except AttributeError:
            h = hash((self.rarity, self.status, self.junk))
            object.__setattr__(self, "_hash", h)
            return h


@dataclass(frozen=True)
class Action:
    """One currency use. `cost` is [(price id, quantity)]."""

    name: str
    kind: str  # transmute alt aug regal exalt annul reroll_magic reroll_rare keep_prefixes keep_suffixes
    cost: tuple[tuple[str, float], ...]
    from_rarity: tuple[int, ...]
    profile: str = "base"  # weight profile for rolled mods
    first_profile: str | None = None  # Harvest: the first mod comes from a tag-filtered pool
    forced: str | None = None  # essence: this mod is always added


@dataclass
class Problem:
    pool: list[Mod]
    targets: list[Target]
    need: int
    actions: list[Action]
    kind: str = "gear"  # "jewel" or "gear"
    notable_cap: int = 99
    fixed: list[Mod] = field(default_factory=list)  # fractured mods, never removed
    profiles: dict[str, dict[str, float]] = field(default_factory=dict)  # name -> mod id -> weight
    extra_mods: dict[str, Mod] = field(default_factory=dict)  # forced mods outside the pool
    reserve: dict[str, int] = field(default_factory=dict)  # side -> slots left open for bench crafts at the end
    filler: frozenset[str] = frozenset()  # mods that count as filler (e.g. any resistance at T3+)
    filler_need: int = 0  # how many filler mods the finished item needs, on top of the targets
    filler_sides: tuple[int, int] | None = None  # (prefix, suffix) filler each side needs; None: any mix of filler_need
    junk_cap: tuple[int, int] | None = None  # most other mods allowed (prefix, suffix): recombinator donors want clean items

    @cached_property
    def mods(self) -> dict[str, Mod]:
        return {**self.extra_mods, **{m.id: m for m in self.pool}, **{m.id: m for m in self.fixed}}

    @property
    def limit(self) -> int:
        return RARE_AFFIX_LIMIT[self.kind]

    @property
    def rare_counts(self) -> dict[int, float]:
        return RARE_MOD_COUNTS[self.kind]

    @property
    def start(self) -> State:
        return State(NORMAL, self.base.status, self.base.junk)

    def classify(self, mod_id: str) -> tuple[int, int]:
        """(target index, GOOD or BLOCKED), or (-1, junk slot)."""
        mod = self.mods[mod_id]
        for i, t in enumerate(self.targets):
            if mod_id in t.good:
                return i, GOOD
            if set(mod.groups) & t.groups:
                return i, BLOCKED
        if mod_id in self.filler:
            return -1, 4 if mod.side == "prefix" else 5
        return -1, (0 if mod.side == "prefix" else 2) + (0 if mod.notable else 1)

    @cached_property
    def base(self) -> State:
        """The fractured mods alone."""
        status, junk = [MISSING] * len(self.targets), [0] * 6
        for m in self.fixed:
            i, v = self.classify(m.id)
            if i >= 0:
                status[i] = v
            else:
                junk[v] += 1
        return State(NORMAL, tuple(status), tuple(junk))

    def weights(self, profile: str) -> dict[str, float]:
        return {m.id: m.weight for m in self.pool} if profile == "base" else self.profiles[profile]

    @cached_property
    def _lumped(self) -> dict[str, tuple[dict, dict]]:
        """profile -> ({(target, GOOD|BLOCKED): weight}, {junk slot: (weight, removed per draw)})."""
        out = {}
        for name in {"base", *self.profiles}:
            tw: dict[tuple[int, int], float] = {}
            groups: dict[int, dict[tuple, float]] = {slot: {} for slot in range(6)}
            for mod_id, w in self.weights(name).items():
                if w <= 0:
                    continue
                i, v = self.classify(mod_id)
                if i >= 0:
                    tw[(i, v)] = tw.get((i, v), 0.0) + w
                else:
                    g = self.mods[mod_id].groups
                    groups[v][g] = groups[v].get(g, 0.0) + w
            junk = {}
            for slot, g in groups.items():
                total = sum(g.values())
                # Drawing a junk mod removes its whole group; use the expected group weight.
                junk[slot] = (total, sum(w * w for w in g.values()) / total if total else 0.0)
            out[name] = (tw, junk)
        return out

    def count(self, s: State, side: str) -> int:
        n = sum(1 for i, v in enumerate(s.status) if v != MISSING and self.targets[i].side == side)
        return n + sum(s.junk[slot] for slot in SIDE_SLOTS[side])

    def notables(self, s: State) -> int:
        on = sum(1 for i, v in enumerate(s.status) if v != MISSING and self.targets[i].notable)
        return on + s.junk[0] + s.junk[2]

    def total(self, s: State) -> int:
        return self.count(s, "prefix") + self.count(s, "suffix")

    def done(self, s: State) -> bool:
        if sum(v == GOOD for v in s.status) < self.need or s.junk[4] + s.junk[5] < self.filler_need:
            return False
        if self.filler_sides and (s.junk[4] < self.filler_sides[0] or s.junk[5] < self.filler_sides[1]):
            return False
        if self.junk_cap and (s.junk[0] + s.junk[1] > self.junk_cap[0] or s.junk[2] + s.junk[3] > self.junk_cap[1]):
            return False
        limit = 1 if s.rarity == MAGIC else self.limit
        return all(self.count(s, side) + n <= limit for side, n in self.reserve.items())

    def keep(self, s: State, side: str) -> list[tuple[float, State]]:
        """Bench "<other side> Cannot Be Changed", then scour: only `side` (and fractured mods) stay.

        The metamod takes a slot on the other side, so that side needs room.
        """
        other = "suffix" if side == "prefix" else "prefix"
        if self.count(s, other) >= self.limit:
            return []
        status = tuple(v if self.targets[i].side == side else self.base.status[i] for i, v in enumerate(s.status))
        kept = SIDE_SLOTS[side]
        junk = tuple(n if slot in kept else self.base.junk[slot] for slot, n in enumerate(s.junk))
        out = State(RARE, status, junk)
        return [(1.0, out if self.total(out) else State(NORMAL, status, junk))]

    # -- mechanics ----------------------------------------------------------

    def add_mod(
        self, s: State, profile: str = "base", sides: tuple[str, ...] = ("prefix", "suffix"), taken: int = 0
    ) -> list[tuple[float, State]]:
        """Add one random mod: augment, regal, exalt, or one draw of a reroll (eldritch exalts: one side only).

        `taken` slots on each allowed side are already used by something the state doesn't track (a metamod).
        """
        tw, junk = self._lumped[profile]
        limit = 1 if s.rarity == MAGIC else self.limit
        cap_hit = self.notables(s) >= self.notable_cap
        options: list[tuple[float, State]] = []
        for side in sides:
            if self.count(s, side) + taken >= limit:
                continue
            for i, t in enumerate(self.targets):
                if t.side != side or s.status[i] != MISSING or (t.notable and cap_hit):
                    continue
                for v in (GOOD, BLOCKED):
                    if tw.get((i, v), 0.0) > 0:
                        status = list(s.status)
                        status[i] = v
                        options.append((tw[(i, v)], State(s.rarity, tuple(status), s.junk)))
            for slot in SIDE_SLOTS[side]:
                if slot in (0, 2) and cap_hit:
                    continue
                total, removed = junk[slot]
                w = max(total - removed * s.junk[slot], 0.0)
                if w > 0:
                    j = list(s.junk)
                    j[slot] += 1
                    options.append((w, State(s.rarity, s.status, tuple(j))))
        total = sum(w for w, _ in options)
        return [(w / total, st) for w, st in options] if total else []

    def force(self, s: State, mod_id: str) -> State:
        i, v = self.classify(mod_id)
        if i >= 0:
            status = list(s.status)
            status[i] = status[i] or v
            return State(s.rarity, tuple(status), s.junk)
        j = list(s.junk)
        j[v] += 1
        return State(s.rarity, s.status, tuple(j))

    def roll(self, rarity: int, profile: str = "base", first_profile: str | None = None, forced: str | None = None) -> list[tuple[float, State]]:
        """A fresh roll at `rarity`; fractured mods stay."""
        counts = {1: 1 - MAGIC_TWO_MOD_CHANCE, 2: MAGIC_TWO_MOD_CHANCE} if rarity == MAGIC else self.rare_counts
        state = State(rarity, self.base.status, self.base.junk)
        placed = len(self.fixed)
        if forced:
            state, placed = self.force(state, forced), placed + 1
        dist, result = {state: 1.0}, {}
        alive = 1.0  # chance the roll still has more mods to add
        for k in range(placed, max(counts) + 1):
            more = sum(p for n, p in counts.items() if n > k)
            stop = alive - more
            for s, p in dist.items():
                result[s] = result.get(s, 0.0) + p * stop
            alive = more
            if alive <= 1e-15:
                break
            prof = first_profile if (k == placed and first_profile and not forced) else profile
            nxt: dict[State, float] = {}
            for s, p in dist.items():
                for q, n in self.add_mod(s, prof) or [(1.0, s)]:
                    nxt[n] = nxt.get(n, 0.0) + p * q
            dist = nxt
        return [(p, s) for s, p in result.items() if p > 0]

    def lock_reroll(self, s: State, keep: str, profile: str = "base", first_profile: str | None = None) -> list[tuple[float, State]]:
        """Bench "<keep side> Cannot Be Changed", then reroll (chaos, Harvest): only the other side rolls again.

        The metamod takes one slot on the rerolled side (removed afterwards); the
        roll's mod count includes the kept mods, as a normal reroll's would.
        """
        other = "suffix" if keep == "prefix" else "prefix"
        if s.rarity != RARE or self.count(s, other) >= self.limit:
            return []
        status = tuple(v if self.targets[i].side == keep else self.base.status[i] for i, v in enumerate(s.status))
        junk = tuple(n if slot in SIDE_SLOTS[keep] else self.base.junk[slot] for slot, n in enumerate(s.junk))
        state = State(RARE, status, junk)
        # Only the kept side matters, so states that agree on it share one (identical) outcome list.
        key = (keep, profile, first_profile, state)
        cache = self.__dict__.setdefault("_lock_cache", {})
        if key not in cache:
            cache[key] = self._lock_reroll(state, other, profile, first_profile)
        return cache[key]

    def _lock_reroll(self, state: State, other: str, profile: str, first_profile: str | None) -> list[tuple[float, State]]:
        placed = self.total(state) + 1  # the metamod
        dist, result, alive = {state: 1.0}, {}, 1.0
        for k in range(placed, max(self.rare_counts) + 1):
            more = sum(p for n, p in self.rare_counts.items() if n > k)
            for st, p in dist.items():
                result[st] = result.get(st, 0.0) + p * (alive - more)
            alive = more
            if alive <= 1e-15:
                break
            prof = first_profile if (k == placed and first_profile) else profile
            nxt: dict[State, float] = {}
            for st, p in dist.items():
                for q, n in self.add_mod(st, prof, sides=(other,), taken=1) or [(1.0, st)]:
                    nxt[n] = nxt.get(n, 0.0) + p * q
            dist = nxt
        for st, p in dist.items():  # counts above the last k (if any) end here
            result[st] = result.get(st, 0.0) + p * alive
        return [(p, st) for st, p in result.items() if p > 0]

    def annul(self, s: State, side: str | None = None) -> list[tuple[float, State]]:
        """Remove one random mod that isn't fractured (eldritch annulment: from one side only)."""
        options: list[tuple[float, State]] = []
        for i, v in enumerate(s.status):
            if side and self.targets[i].side != side:
                continue
            if v != MISSING and self.base.status[i] == MISSING:
                status = list(s.status)
                status[i] = MISSING
                options.append((1.0, State(s.rarity, tuple(status), s.junk)))
        for slot in SIDE_SLOTS[side] if side else range(6):
            n = s.junk[slot] - self.base.junk[slot]
            if n > 0:
                j = list(s.junk)
                j[slot] -= 1
                options.append((float(n), State(s.rarity, s.status, tuple(j))))
        total = sum(w for w, _ in options)
        return [(w / total, st) for w, st in options] if total else []

    def roll_key(self, a: Action) -> tuple | None:
        """What a reroll's outcome depends on; None for actions that aren't rerolls.

        A forced (essence) mod matters only through what it counts as, so every
        essence forcing an unwanted prefix rolls the same distribution.
        """
        if a.kind in ("transmute", "alt", "reroll_magic"):
            return (MAGIC, a.profile)
        if a.kind == "reroll_rare":
            return (RARE, a.profile, a.first_profile, self.classify(a.forced) if a.forced else None)
        return None

    @cached_property
    def _rolls(self) -> dict[str, list[tuple[float, State]]]:
        out, by_key = {}, {}
        for a in self.actions:
            key = self.roll_key(a)
            if key is None:
                continue
            if key not in by_key:
                by_key[key] = self.roll(MAGIC, a.profile) if key[0] == MAGIC else self.roll(RARE, a.profile, a.first_profile, a.forced)
            out[a.name] = by_key[key]
        return out

    def outcomes(self, s: State, a: Action) -> list[tuple[float, State]]:
        if s.rarity not in a.from_rarity:
            return []
        if a.name in self._rolls:
            return self._rolls[a.name]
        if a.kind == "aug":
            return self.add_mod(s) if self.total(s) < 2 else []
        if a.kind == "regal":
            return self.add_mod(State(RARE, s.status, s.junk))
        if a.kind == "exalt":
            return self.add_mod(s) if self.total(s) < 2 * self.limit else []
        if a.kind == "annul":
            return self.annul(s)
        if a.kind in ("exalt_prefix", "exalt_suffix"):
            return self.add_mod(s, sides=(a.kind[6:],)) if self.total(s) < 2 * self.limit else []
        if a.kind in ("annul_prefix", "annul_suffix"):
            return self.annul(s, a.kind[6:])
        if a.kind in ("keep_prefixes", "keep_suffixes"):
            return self.keep(s, a.kind[5:-2])
        if a.kind in ("lockreroll_prefixes", "lockreroll_suffixes"):
            return self.lock_reroll(s, a.kind[11:-2], a.profile, a.first_profile)
        return []


# -- solving ------------------------------------------------------------------


@dataclass
class Plan:
    cost: float  # expected chaos from a normal base, excluding the base itself, clicking time included
    policy: dict[State, str]
    usage: dict[str, float]  # expected uses of each action
    visits: dict[State, float]  # expected times each state is acted on
    states: int
    unpriced: list[str] = field(default_factory=list)  # actions left out for lack of a price
    hours: float = 0.0  # expected time spent clicking

    @property
    def currency(self) -> float:
        """Chaos spent on currency alone (`cost` less the clicking time)."""
        return self.cost - self.hours * CHAOS_PER_HOUR


# What an hour of clicking is worth, in chaos. Charging for time stops plans
# that spam a cheap orb ten thousand times for a long shot.
CHAOS_PER_HOUR = float(data.env("PONDER_CHAOS_PER_HOUR") or 1500)

# Seconds per use, including reading the result.
SECONDS = {"transmute": 1.0, "alt": 1.0, "aug": 1.0, "regal": 2.0, "exalt": 2.0, "annul": 2.0, "reroll_magic": 3.0}
HARVEST_SECONDS = 2.0  # one click, like a chaos orb
FOSSIL_SECONDS = 2.0  # socketing each fossil into the resonator
LOCK_SECONDS = 5.0  # a bench metamod, on top of the reroll after it


def seconds(a: Action) -> float:
    if a.kind in SECONDS:
        return SECONDS[a.kind]
    if a.kind.startswith(("keep_", "lockreroll_")):
        reroll = a.name.split("+", 1)[1]
        return LOCK_SECONDS + (1.0 if reroll == "scour" else _reroll_seconds(reroll, a.cost))
    if a.kind == "reroll_rare":
        return _reroll_seconds(a.name, a.cost)
    return 2.0  # eldritch currency


def _reroll_seconds(name: str, cost) -> float:
    if name.startswith("harvest"):
        return HARVEST_SECONDS
    t = 2.0 + (1.0 if name.startswith("scour+") else 0.0)
    if "fossil" in name:
        t += FOSSIL_SECONDS * sum(1 for pid, _ in cost if pid.endswith("-fossil"))
    return t


def action_cost(a: Action, prices: dict[str, float]) -> float:
    """Chaos of currency one use takes."""
    return sum(prices.get(pid, 0.0) * qty for pid, qty in a.cost)


def step_cost(a: Action, prices: dict[str, float]) -> float:
    """Currency plus the clicking time, in chaos: what the solver minimises."""
    return action_cost(a, prices) + seconds(a) * CHAOS_PER_HOUR / 3600


def solve(problem: Problem, prices: dict[str, float], max_rounds: int = 60) -> Plan:
    """Optimal policy by policy iteration, evaluating each policy exactly.

    Actions with any unpriced input are left out rather than treated as free.
    """
    unpriced = [a.name for a in problem.actions if any(pid not in prices for pid, _ in a.cost)]
    problem.actions = _cheapest_of_equals([a for a in problem.actions if a.name not in unpriced], problem, prices)
    problem.__dict__.pop("_rolls", None)
    problem.__dict__.pop("_lock_cache", None)
    costs = {a.name: step_cost(a, prices) for a in problem.actions}
    if problem.done(problem.start):
        return Plan(0.0, {}, {}, {}, 1, unpriced)
    states, edges = _explore(problem)
    g = _Graph(problem, edges, costs)
    policy = g.proper_policy()
    if (policy < 0).any():
        raise ValueError("no sequence of the available currencies can reach this target")

    for _ in range(max_rounds):
        value = g.evaluate(policy)
        better = g.improve(g.q(value), policy)
        if (better == policy).all():
            break
        policy = better

    visits = g.visits(policy, g.index[problem.start])
    usage = {a.name: 0.0 for a in problem.actions}
    for i, n in enumerate(visits):
        usage[g.action[policy[i]]] += float(n)
    hours = sum(n * seconds(a) for a in problem.actions if (n := usage[a.name])) / 3600
    return Plan(
        float(value[g.index[problem.start]]),
        {s: g.action[policy[i]] for i, s in enumerate(g.states)},
        usage,
        {s: float(visits[i]) for i, s in enumerate(g.states)},
        len(states),
        unpriced,
        hours,
    )


FRACTURE_ORB = "fracturing-orb"
FRACTURE_MIN_MODS = 4  # a Fracturing Orb needs a rare with at least this many mods
FRACTURE_SECONDS = 3.0


@dataclass
class SelfFracture:
    """Make your own fractured base: reroll until the mod lands with few enough mods, then fracture.

    A Fracturing Orb fixes one random mod of `k`, so it hits the wanted one
    1 time in k; a miss ruins the base and the loop starts over on a new one.
    """

    target: int  # index into the problem's targets
    action: str  # the reroll to spam
    max_mods: int  # fracture only when the item has at most this many mods
    hit: float  # chance one reroll lands the mod with few enough mods
    fracture_chance: float  # chance the orb then picks it
    cost: float  # expected chaos (currency and clicking) to a base with the mod fractured
    hours: float

    @property
    def rerolls(self) -> float:
        return 1 / (self.hit * self.fracture_chance)

    @property
    def orbs(self) -> float:
        return 1 / self.fracture_chance


def self_fracture(problem: Problem, target: int, prices: dict[str, float]) -> SelfFracture | None:
    """The cheapest way to roll and fracture `problem.targets[target]` yourself (gear only).

    Each rare reroll (chaos, scour+alch, essences, fossils, Harvest) gives,
    from its exact roll distribution, the chance P_k of the mod landing at a
    good tier on an item with k mods. Fracturing only at k <= K costs
    (reroll / P(k <= K) + orb) / E[1/k | k <= K]; the best reroll and K win.
    """
    if problem.kind != "gear" or problem.fixed or FRACTURE_ORB not in prices:
        return None
    orb = prices[FRACTURE_ORB] + FRACTURE_SECONDS * CHAOS_PER_HOUR / 3600
    best = None
    for a in problem.actions:
        if a.kind != "reroll_rare" or RARE not in a.from_rarity or any(pid not in prices for pid, _ in a.cost):
            continue
        by_k: dict[int, float] = {}
        for p, s in problem.roll(RARE, a.profile, a.first_profile, a.forced):
            if s.status[target] == GOOD and (k := problem.total(s)) >= FRACTURE_MIN_MODS:
                by_k[k] = by_k.get(k, 0.0) + p
        step = step_cost(a, prices)
        hit = chance = 0.0
        for k in sorted(by_k):
            hit += by_k[k]
            chance = sum(p / j for j, p in by_k.items() if j <= k) / hit
            cost = (step / hit + orb) / chance
            if best is None or cost < best.cost:
                hours = (seconds(a) / hit + FRACTURE_SECONDS) / chance / 3600
                best = SelfFracture(target, a.name, k, hit, chance, cost, hours)
    return best


def _cheapest_of_equals(acts: list[Action], problem: Problem, prices: dict[str, float]) -> list[Action]:
    """Rerolls with the same outcomes from the same rarities: only the cheapest can be optimal."""
    best: dict[tuple, Action] = {}
    out = []
    for a in acts:
        key = problem.roll_key(a)
        if key is None:
            out.append(a)
            continue
        key = (key, a.from_rarity)
        if key not in best or step_cost(a, prices) < step_cost(best[key], prices):
            best[key] = a
    return out + [a for a in acts if problem.roll_key(a) is not None and best[(problem.roll_key(a), a.from_rarity)] is a]


def evaluate(problem: Problem, policy: dict[State, str], prices: dict[str, float]) -> float:
    """Exact expected cost of following `policy` (state -> action name) from the start."""
    costs = {a.name: step_cost(a, prices) for a in problem.actions}
    g = _Graph(problem, _explore(problem)[1], costs)
    chosen = np.array([g.pair_of(i, policy[s]) for i, s in enumerate(g.states)])
    return float(g.evaluate(chosen)[g.index[problem.start]])


def _explore(problem: Problem):
    """Every state reachable from the normal base, with each action's outcomes."""
    frontier, seen = [problem.start], {problem.start}
    edges: dict[State, dict[str, list]] = {}
    checked: set[int] = set()  # rerolls share one outcome list across many states
    while frontier:
        s = frontier.pop()
        if problem.done(s):
            continue
        edges[s] = {}
        for a in problem.actions:
            out = problem.outcomes(s, a)
            if out:
                edges[s][a.name] = out
                if id(out) in checked:
                    continue
                checked.add(id(out))
                for _, n in out:
                    if n not in seen:
                        seen.add(n)
                        frontier.append(n)
    return list(seen), edges


class _Graph:
    """The explored states as flat arrays, so policy iteration is a few numpy calls.

    Open (unfinished) states are numbered 0..n-1. Every (state, action) pair is
    a row; pairs of one state are contiguous, in action order. Each pair's
    outcomes are entries (pair, next state, probability), with next = -1 for a
    finished item, whose remaining cost is 0.
    """

    def __init__(self, problem: Problem, edges: dict[State, dict[str, list]], costs: dict[str, float]):
        self.states = [s for s in edges]  # edges holds exactly the open states
        self.index = {s: i for i, s in enumerate(self.states)}
        self.n = len(self.states)
        pair_state, self.action, pair_cost = [], [], []
        nexts, probs = [], []
        converted: dict[int, tuple[np.ndarray, np.ndarray]] = {}  # shared reroll lists convert once
        state_ptr = [0]
        for i, s in enumerate(self.states):
            for name, outcomes in edges[s].items():
                key = id(outcomes)
                if key not in converted:
                    converted[key] = (
                        np.array([self.index.get(n, -1) for _, n in outcomes], dtype=np.int64),
                        np.array([p for p, _ in outcomes], dtype=float),
                    )
                nxt, prob = converted[key]
                pair_state.append(i)
                self.action.append(name)
                pair_cost.append(costs[name])
                nexts.append(nxt)
                probs.append(prob)
            state_ptr.append(len(pair_state))
        self.pair_state = np.array(pair_state, dtype=np.int64)
        self.pair_cost = np.array(pair_cost, dtype=float)
        self.state_ptr = np.array(state_ptr, dtype=np.int64)
        lengths = [len(x) for x in nexts]
        self.e_pair = np.repeat(np.arange(len(pair_state)), lengths)
        self.entry_ptr = np.concatenate(([0], np.cumsum(lengths)))[:-1]  # each pair's first entry
        self.e_next = np.concatenate(nexts)
        self.e_prob = np.concatenate(probs)
        self.e_open = self.e_next >= 0

    def pair_of(self, i: int, name: str) -> int:
        lo, hi = self.state_ptr[i], self.state_ptr[i + 1]
        return lo + self.action[lo:hi].index(name)

    def q(self, value: np.ndarray) -> np.ndarray:
        """Expected cost to finish after taking each pair's action."""
        ext = np.append(value, 0.0)  # index -1: finished
        return self.pair_cost + np.bincount(self.e_pair, self.e_prob * ext[self.e_next], minlength=len(self.pair_cost))

    def improve(self, q: np.ndarray, policy: np.ndarray) -> np.ndarray:
        """Each state's cheapest pair (first in action order on ties), kept unless clearly better."""
        best = np.minimum.reduceat(q, self.state_ptr[:-1])
        is_best = np.flatnonzero(q <= best[self.pair_state])
        _, first = np.unique(self.pair_state[is_best], return_index=True)
        candidate = is_best[first]
        current = q[policy]
        switch = best < current - 1e-9 * np.maximum(1.0, current)
        return np.where(switch, candidate, policy)

    def proper_policy(self) -> np.ndarray:
        """A starting policy that surely finishes: always step toward fewer moves-to-done.

        Policy iteration needs one; "always reroll" isn't enough when no single
        reroll can hit every target (e.g. three notables need alt, regal, exalt).
        Returns -1 for states that can't finish.
        """
        dist = np.full(self.n, np.inf)
        policy = np.full(self.n, -1, dtype=np.int64)
        live = self.e_prob > 0
        while True:
            d = np.where(self.e_open, dist[self.e_next], 0.0)
            d[~live] = np.inf
            step = np.minimum.reduceat(d, self.entry_ptr) + 1  # entries of a pair are contiguous
            best = np.minimum.reduceat(step, self.state_ptr[:-1])
            improved = best < dist
            if not improved.any():
                return policy
            is_best = np.flatnonzero(step <= best[self.pair_state])
            _, first = np.unique(self.pair_state[is_best], return_index=True)
            policy = np.where(improved, is_best[first], policy)
            dist = np.where(improved, best, dist)

    def _matrix(self, policy: np.ndarray) -> np.ndarray:
        chosen = np.zeros(len(self.pair_cost), dtype=bool)
        chosen[policy] = True
        m = chosen[self.e_pair] & self.e_open
        P = np.zeros((self.n, self.n))
        np.add.at(P, (self.pair_state[self.e_pair[m]], self.e_next[m]), self.e_prob[m])
        return np.eye(self.n) - P

    def evaluate(self, policy: np.ndarray) -> np.ndarray:
        return np.linalg.solve(self._matrix(policy), self.pair_cost[policy])

    def visits(self, policy: np.ndarray, start: int) -> np.ndarray:
        """Expected times each state is acted on, starting from state `start`."""
        s0 = np.zeros(self.n)
        s0[start] = 1.0
        return np.linalg.solve(self._matrix(policy).T, s0)


def describe(problem: Problem, s: State) -> str:
    """E.g. `rare: Prismatic Heart + Life (blocked) + 1 other suffix`.

    "blocked" means a mod sharing the target's group (a worse tier, or an
    exclusive notable) is on the item, so the target can't roll until it goes.
    """
    parts = []
    for i, v in enumerate(s.status):
        if v == GOOD:
            parts.append(problem.targets[i].name)
        elif v == BLOCKED:
            parts.append(f"{problem.targets[i].name} (blocked)")
    parts += [f"{n} {label}" for n, label in zip(s.junk, JUNK_LABELS) if n]
    return f"{('normal', 'magic', 'rare')[s.rarity]}: {' + '.join(parts) or 'nothing useful'}"


def describe_parts(problem: Problem, s: State) -> tuple[str, list[tuple[str, str]]]:
    """`describe` in pieces for colouring: (rarity, [(text, "good" | "blocked" | "other")])."""
    parts = []
    for i, v in enumerate(s.status):
        if v == GOOD:
            parts.append((problem.targets[i].name, "good"))
        elif v == BLOCKED:
            parts.append((f"{problem.targets[i].name} (blocked)", "blocked"))
    parts += [(f"{n} {label}", "filler" if slot >= 4 else "other") for slot, (n, label) in enumerate(zip(s.junk, JUNK_LABELS)) if n]
    return ("normal", "magic", "rare")[s.rarity], parts


def recipe_states(plan: Plan, min_share: float = 0.01) -> list[tuple[State, str, float]]:
    """Like `recipe`, but with the states themselves."""
    total = sum(plan.visits.values()) or 1.0
    rows = [(s, plan.policy[s], n / total) for s, n in plan.visits.items() if n / total >= min_share]
    return sorted(rows, key=lambda r: -r[2])


def recipe(problem: Problem, plan: Plan, min_share: float = 0.01) -> list[tuple[str, str, float]]:
    """The policy for the states you actually meet: (state, action, share of all actions)."""
    total = sum(plan.visits.values()) or 1.0
    rows = [(describe(problem, s), plan.policy[s], n / total) for s, n in plan.visits.items() if n / total >= min_share]
    return sorted(rows, key=lambda r: -r[2])


@dataclass
class Exit:
    chance: float  # per time you reach the step
    to: int | None  # step number; None when the item is done or the branch is too rare to list
    what: str  # e.g. "+Life", "lost Life", "done"
    kind: str  # "done", "win", "back" (lost a target), "side" (same targets, something else changed), "rare" (unlisted)
    via: str = ""  # the action that mostly leads here


@dataclass
class Step:
    """One stretch of a recipe: every situation with the same rarity and the same targets on the item."""

    number: int
    rarity: int
    have: tuple[str, ...]  # target names on the item
    actions: list[tuple[str, str, float]]  # (action, what else is on the item when you use it, uses per reach), most used first
    reached: float  # expected times you get here per finished item
    uses: float  # expected actions here per finished item
    exits: list[Exit]

    @property
    def per_reach(self) -> float:
        return self.uses / self.reached if self.reached else 0.0


def steps(problem: Problem, plan: Plan, min_reach: float = 0.02) -> list[Step]:
    """The policy as numbered steps ("spam X until Y: a. ... → 3, b. ... → back to 1").

    States are grouped by progress: rarity plus which targets are on at a good
    tier. Following the policy's flow between groups gives, per step, how
    often you reach it, how many actions it takes and where each outcome
    leads. Steps reached less than `min_reach` times per item are folded
    into their parent's "rare" exit.
    """
    by_name = {a.name: a for a in problem.actions}

    def key(st: State) -> tuple[int, frozenset[int]]:
        return st.rarity, frozenset(i for i, v in enumerate(st.status) if v == GOOD)

    open_visits = {st: v for st, v in plan.visits.items() if v > 1e-9 and st in plan.policy}
    groups: dict[tuple, list[State]] = {}
    for st in open_visits:
        groups.setdefault(key(st), []).append(st)
    reached = {k: 0.0 for k in groups}
    if key(problem.start) in reached:
        reached[key(problem.start)] += 1.0
    flow: dict[tuple, dict] = {k: {} for k in groups}  # group -> next group (or "done") -> expected moves
    via: dict[tuple, dict] = {}  # (group, next) -> action -> expected moves
    for st, v in open_visits.items():
        k = key(st)
        a = plan.policy[st]
        for p, nxt in problem.outcomes(st, by_name[a]):
            to = "done" if problem.done(nxt) else key(nxt)
            if to == k:
                continue
            flow[k][to] = flow[k].get(to, 0.0) + v * p
            by = via.setdefault((k, to), {})
            by[a] = by.get(a, 0.0) + v * p
            if to != "done" and to in reached:
                reached[to] += v * p
    kept = sorted((k for k in groups if reached[k] >= min_reach or k == key(problem.start)), key=lambda k: (len(k[1]), k[0], -reached[k]))
    number = {k: i + 1 for i, k in enumerate(kept)}
    names = [t.name for t in problem.targets]
    out = []
    for k in kept:
        rarity, have = k
        acts: dict[str, list] = {}
        for st in groups[k]:
            a = plan.policy[st]
            acts.setdefault(a, [0.0, st, 0.0])
            acts[a][0] += open_visits[st]
            if open_visits[st] > acts[a][2]:
                acts[a][1], acts[a][2] = st, open_visits[st]
        uses = sum(v for v, _, _ in acts.values())
        actions = [(a, _else(problem, st), v / reached[k]) for a, (v, st, _) in sorted(acts.items(), key=lambda x: -x[1][0])]
        exits, rare = [], 0.0
        for to, n in sorted(flow[k].items(), key=lambda x: -x[1]):
            chance = n / reached[k]
            by = max(via[(k, to)].items(), key=lambda x: x[1])[0]
            if to == "done":
                rest = [names[i] for i in range(len(names)) if i not in have]
                what = f"got {', '.join(rest)}" if rest and problem.need == len(names) else "done"
                exits.append(Exit(chance, None, what, "done", by))
            elif to in number:
                gained, lost = to[1] - have, have - to[1]
                what = ", ".join([f"got {names[i]}" for i in sorted(gained)] + [f"lost {names[i]}" for i in sorted(lost)])
                if not what:
                    what = f"now {('normal', 'magic', 'rare')[to[0]]}"
                kind = "back" if lost else "win" if gained else "side"
                exits.append(Exit(chance, number[to], what, kind, by))
            else:
                rare += chance
        if rare > 1e-6:
            exits.append(Exit(rare, None, "something rarer", "rare"))
        out.append(Step(number[k], rarity, tuple(names[i] for i in sorted(have)), actions, reached[k], uses, exits))
    return out


def _else(problem: Problem, s: State) -> str:
    """What's on the item besides good targets, e.g. "2 other prefix + 1 other suffix"."""
    _, parts = describe_parts(problem, s)
    return " + ".join(text for text, kind in parts if kind != "good") or "nothing else"


# -- simulation on real mods ----------------------------------------------------


def simulate(problem: Problem, plan: Plan, prices: dict[str, float], runs: int = 2000, seed: int = 0, max_steps: int = 10**5) -> float:
    """Average chaos spent replaying `plan` with real mods and exact group exclusion."""
    rng = random.Random(seed)
    by_name = {a.name: a for a in problem.actions}
    costs = {a.name: step_cost(a, prices) for a in problem.actions}
    cache: dict = {}
    total = 0.0
    for _ in range(runs):
        rarity, mods = NORMAL, list(problem.fixed)
        for _ in range(max_steps):
            state = _lump(problem, rarity, mods)
            if problem.done(state):
                break
            a = by_name[plan.policy[state]]
            total += costs[a.name]
            rarity, mods = _apply(problem, a, rarity, mods, rng, cache)
    return total / runs


def _apply(problem: Problem, a: Action, rarity: int, mods: list[Mod], rng, cache):
    if a.kind in ("transmute", "alt", "reroll_magic"):
        return MAGIC, _sim_roll(problem, MAGIC, a, rng, cache)
    if a.kind == "reroll_rare":
        return RARE, _sim_roll(problem, RARE, a, rng, cache)
    if a.kind in ("aug", "regal", "exalt"):
        rarity = MAGIC if a.kind == "aug" else RARE
        m = _draw(problem, rarity, mods, "base", rng, cache)
        return rarity, mods + [m] if m else mods
    if a.kind in ("exalt_prefix", "exalt_suffix"):
        m = _draw(problem, RARE, mods, "base", rng, cache, side=a.kind[6:])
        return rarity, mods + [m] if m else mods
    if a.kind in ("annul", "annul_prefix", "annul_suffix"):
        removable = [m for m in mods if m not in problem.fixed and (a.kind == "annul" or m.side == a.kind[6:])]
        if removable:
            gone = rng.choice(removable)
            mods = [m for m in mods if m is not gone]
        return rarity, mods
    if a.kind in ("keep_prefixes", "keep_suffixes"):
        side = a.kind[5:-2]
        kept = [m for m in mods if m.side == side or m in problem.fixed]
        return (RARE if kept else NORMAL), kept
    if a.kind in ("lockreroll_prefixes", "lockreroll_suffixes"):
        keep = a.kind[11:-2]
        other = "suffix" if keep == "prefix" else "prefix"
        kept = [m for m in mods if m.side == keep or m in problem.fixed]
        counts = problem.rare_counts
        n = rng.choices(list(counts), weights=list(counts.values()))[0]
        first = a.first_profile
        while len(kept) + 1 < n:  # + the metamod
            m = _draw(problem, RARE, kept, first or a.profile, rng, cache, side=other, occupied=1)
            first = None
            if m is None:
                break
            kept = kept + [m]
        return RARE, kept
    raise ValueError(a.kind)


def _sim_roll(problem: Problem, rarity: int, a: Action, rng, cache) -> list[Mod]:
    counts = {1: 1 - MAGIC_TWO_MOD_CHANCE, 2: MAGIC_TWO_MOD_CHANCE} if rarity == MAGIC else problem.rare_counts
    n = rng.choices(list(counts), weights=list(counts.values()))[0]
    mods = list(problem.fixed)
    if a.forced:
        mods.append(problem.mods[a.forced])
    first = None if a.forced else a.first_profile
    while len(mods) < n:
        m = _draw(problem, rarity, mods, first or a.profile, rng, cache)
        first = None
        if m is None:
            break
        mods.append(m)
    return mods


def _draw(problem: Problem, rarity: int, mods: list[Mod], profile: str, rng, cache: dict, side: str | None = None, occupied: int = 0) -> Mod | None:
    key = (rarity, profile, frozenset(m.id for m in mods), side, occupied)
    if key not in cache:
        limit = 1 if rarity == MAGIC else problem.limit
        taken = {g for m in mods for g in m.groups}
        notables = sum(m.notable for m in mods)
        weights = problem.weights(profile)
        pool = [
            m
            for m in problem.pool
            if weights.get(m.id, 0) > 0
            and (side is None or m.side == side)
            and sum(x.side == m.side for x in mods) + occupied < limit
            and not set(m.groups) & taken
            and not (m.notable and notables >= problem.notable_cap)
        ]
        cache[key] = (pool, list(itertools.accumulate(weights[m.id] for m in pool)))
    pool, cumulative = cache[key]
    return rng.choices(pool, cum_weights=cumulative)[0] if pool else None


def _lump(problem: Problem, rarity: int, mods: list[Mod]) -> State:
    status, junk = [MISSING] * len(problem.targets), [0] * 6
    for m in mods:
        i, v = problem.classify(m.id)
        if i >= 0:
            status[i] = status[i] or v
        else:
            junk[v] += 1
    return State(rarity, tuple(status), tuple(junk))
