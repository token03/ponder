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

# Assumptions not in the game data; adjust here if testing shows otherwise.
MAGIC_TWO_MOD_CHANCE = 0.5  # transmute/alteration roll 1 or 2 mods
# Mod count when an item becomes rare (alchemy, chaos, essence, fossil, Harvest).
# Gear follows a 1,000-chaos sample (4:5:6 = 9:3:1); jewels assume the same 3:1 step.
RARE_MOD_COUNTS = {"jewel": {3: 0.75, 4: 0.25}, "gear": {4: 9 / 13, 5: 3 / 13, 6: 1 / 13}}
RARE_AFFIX_LIMIT = {"jewel": 2, "gear": 3}  # per side

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
    junk: tuple[int, int, int, int] = (0, 0, 0, 0)  # prefix notable/other, suffix notable/other

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
        return -1, (0 if mod.side == "prefix" else 2) + (0 if mod.notable else 1)

    @cached_property
    def base(self) -> State:
        """The fractured mods alone."""
        status, junk = [MISSING] * len(self.targets), [0, 0, 0, 0]
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
            groups: dict[int, dict[tuple, float]] = {0: {}, 1: {}, 2: {}, 3: {}}
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
        return n + (s.junk[0] + s.junk[1] if side == "prefix" else s.junk[2] + s.junk[3])

    def notables(self, s: State) -> int:
        on = sum(1 for i, v in enumerate(s.status) if v != MISSING and self.targets[i].notable)
        return on + s.junk[0] + s.junk[2]

    def total(self, s: State) -> int:
        return self.count(s, "prefix") + self.count(s, "suffix")

    def done(self, s: State) -> bool:
        if sum(v == GOOD for v in s.status) < self.need:
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
        kept = (0, 1) if side == "prefix" else (2, 3)
        junk = tuple(n if slot in kept else self.base.junk[slot] for slot, n in enumerate(s.junk))
        out = State(RARE, status, junk)
        return [(1.0, out if self.total(out) else State(NORMAL, status, junk))]

    # -- mechanics ----------------------------------------------------------

    def add_mod(self, s: State, profile: str = "base") -> list[tuple[float, State]]:
        """Add one random mod: augment, regal, exalt, or one draw of a reroll."""
        tw, junk = self._lumped[profile]
        limit = 1 if s.rarity == MAGIC else self.limit
        cap_hit = self.notables(s) >= self.notable_cap
        options: list[tuple[float, State]] = []
        for side in ("prefix", "suffix"):
            if self.count(s, side) >= limit:
                continue
            for i, t in enumerate(self.targets):
                if t.side != side or s.status[i] != MISSING or (t.notable and cap_hit):
                    continue
                for v in (GOOD, BLOCKED):
                    if tw.get((i, v), 0.0) > 0:
                        status = list(s.status)
                        status[i] = v
                        options.append((tw[(i, v)], State(s.rarity, tuple(status), s.junk)))
            for slot in (0, 1) if side == "prefix" else (2, 3):
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

    def annul(self, s: State) -> list[tuple[float, State]]:
        """Remove one random mod that isn't fractured."""
        options: list[tuple[float, State]] = []
        for i, v in enumerate(s.status):
            if v != MISSING and self.base.status[i] == MISSING:
                status = list(s.status)
                status[i] = MISSING
                options.append((1.0, State(s.rarity, tuple(status), s.junk)))
        for slot in range(4):
            n = s.junk[slot] - self.base.junk[slot]
            if n > 0:
                j = list(s.junk)
                j[slot] -= 1
                options.append((float(n), State(s.rarity, s.status, tuple(j))))
        total = sum(w for w, _ in options)
        return [(w / total, st) for w, st in options] if total else []

    @cached_property
    def _rolls(self) -> dict[str, list[tuple[float, State]]]:
        out = {}
        for a in self.actions:
            if a.kind in ("transmute", "alt", "reroll_magic"):
                out[a.name] = self.roll(MAGIC, a.profile)
            elif a.kind == "reroll_rare":
                out[a.name] = self.roll(RARE, a.profile, a.first_profile, a.forced)
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
        if a.kind in ("keep_prefixes", "keep_suffixes"):
            return self.keep(s, a.kind[5:-2])
        return []


# -- solving ------------------------------------------------------------------


@dataclass
class Plan:
    cost: float  # expected chaos from a normal base, excluding the base itself
    policy: dict[State, str]
    usage: dict[str, float]  # expected uses of each action
    visits: dict[State, float]  # expected times each state is acted on
    states: int
    unpriced: list[str] = field(default_factory=list)  # actions left out for lack of a price


def action_cost(a: Action, prices: dict[str, float]) -> float:
    return sum(prices.get(pid, 0.0) * qty for pid, qty in a.cost)


def solve(problem: Problem, prices: dict[str, float], max_rounds: int = 60) -> Plan:
    """Optimal policy by policy iteration, evaluating each policy exactly.

    Actions with any unpriced input are left out rather than treated as free.
    """
    unpriced = [a.name for a in problem.actions if any(pid not in prices for pid, _ in a.cost)]
    problem.actions = [a for a in problem.actions if a.name not in unpriced]
    problem.__dict__.pop("_rolls", None)
    costs = {a.name: action_cost(a, prices) for a in problem.actions}
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
    return Plan(
        float(value[g.index[problem.start]]),
        {s: g.action[policy[i]] for i, s in enumerate(g.states)},
        usage,
        {s: float(visits[i]) for i, s in enumerate(g.states)},
        len(states),
        unpriced,
    )


def evaluate(problem: Problem, policy: dict[State, str], prices: dict[str, float]) -> float:
    """Exact expected cost of following `policy` (state -> action name) from the start."""
    costs = {a.name: action_cost(a, prices) for a in problem.actions}
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
        self.e_pair = np.repeat(np.arange(len(pair_state)), [len(x) for x in nexts])
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
        n_pairs = len(self.pair_cost)
        live = self.e_prob > 0
        while True:
            d = np.where(self.e_open, dist[self.e_next], 0.0)
            d[~live] = np.inf
            pair_best = np.full(n_pairs, np.inf)
            np.minimum.at(pair_best, self.e_pair, d)
            step = pair_best + 1
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
    labels = ("other prefix notable", "other prefix", "other suffix notable", "other suffix")
    parts += [f"{n} {label}" for n, label in zip(s.junk, labels) if n]
    return f"{('normal', 'magic', 'rare')[s.rarity]}: {' + '.join(parts) or 'nothing useful'}"


def describe_parts(problem: Problem, s: State) -> tuple[str, list[tuple[str, str]]]:
    """`describe` in pieces for colouring: (rarity, [(text, "good" | "blocked" | "other")])."""
    parts = []
    for i, v in enumerate(s.status):
        if v == GOOD:
            parts.append((problem.targets[i].name, "good"))
        elif v == BLOCKED:
            parts.append((f"{problem.targets[i].name} (blocked)", "blocked"))
    labels = ("other prefix notable", "other prefix", "other suffix notable", "other suffix")
    parts += [(f"{n} {label}", "other") for n, label in zip(s.junk, labels) if n]
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


# -- simulation on real mods ----------------------------------------------------


def simulate(problem: Problem, plan: Plan, prices: dict[str, float], runs: int = 2000, seed: int = 0, max_steps: int = 10**5) -> float:
    """Average chaos spent replaying `plan` with real mods and exact group exclusion."""
    rng = random.Random(seed)
    by_name = {a.name: a for a in problem.actions}
    costs = {a.name: action_cost(a, prices) for a in problem.actions}
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
    if a.kind == "annul":
        removable = [m for m in mods if m not in problem.fixed]
        if removable:
            gone = rng.choice(removable)
            mods = [m for m in mods if m is not gone]
        return rarity, mods
    if a.kind in ("keep_prefixes", "keep_suffixes"):
        side = a.kind[5:-2]
        kept = [m for m in mods if m.side == side or m in problem.fixed]
        return (RARE if kept else NORMAL), kept
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


def _draw(problem: Problem, rarity: int, mods: list[Mod], profile: str, rng, cache: dict) -> Mod | None:
    key = (rarity, profile, frozenset(m.id for m in mods))
    if key not in cache:
        limit = 1 if rarity == MAGIC else problem.limit
        taken = {g for m in mods for g in m.groups}
        notables = sum(m.notable for m in mods)
        weights = problem.weights(profile)
        pool = [
            m
            for m in problem.pool
            if weights.get(m.id, 0) > 0
            and sum(x.side == m.side for x in mods) < limit
            and not set(m.groups) & taken
            and not (m.notable and notables >= problem.notable_cap)
        ]
        cache[key] = (pool, list(itertools.accumulate(weights[m.id] for m in pool)))
    pool, cumulative = cache[key]
    return rng.choices(pool, cum_weights=cumulative)[0] if pool else None


def _lump(problem: Problem, rarity: int, mods: list[Mod]) -> State:
    status, junk = [MISSING] * len(problem.targets), [0, 0, 0, 0]
    for m in mods:
        i, v = problem.classify(m.id)
        if i >= 0:
            status[i] = status[i] or v
        else:
            junk[v] += 1
    return State(rarity, tuple(status), tuple(junk))
