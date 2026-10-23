"""Expected cost of hitting target mods with "add a mod" currencies.

Covers the alteration / augmentation / regal / exalt / scour loop. The item is
lumped into a small state: which target mods it has, plus how many other
("junk") mods sit on each side, split into notables and the rest because
notables share a cap. Value iteration over those states gives the cheapest
policy, which is the recipe: for each state, which currency to use next.

`simulate` replays a policy on real mods (exact group exclusion, no lumping)
to check the lumped numbers.
"""

from __future__ import annotations

import itertools
import random
from dataclasses import dataclass, field
from functools import cached_property

import numpy as np

# Assumptions not in the game data; adjust here if testing shows otherwise.
MAGIC_TWO_MOD_CHANCE = 0.5  # alteration/transmutation roll 1 or 2 mods
JEWEL_AFFIX_LIMIT = {"magic": 1, "rare": 2}  # per side

ACTIONS = ("alt", "aug", "regal", "exalt", "scour")
CURRENCY = {
    "alt": ["Orb of Alteration"],
    "aug": ["Orb of Augmentation"],
    "regal": ["Regal Orb"],
    "exalt": ["Exalted Orb"],
    "scour": ["Orb of Scouring", "Orb of Transmutation"],
}


@dataclass(frozen=True)
class Mod:
    id: str
    side: str  # "prefix" or "suffix"
    weight: int
    groups: tuple[str, ...]
    notable: bool
    label: str


@dataclass(frozen=True)
class State:
    rare: bool
    targets: frozenset[str]
    # junk counts: (prefix notables, prefix others, suffix notables, suffix others)
    junk: tuple[int, int, int, int] = (0, 0, 0, 0)

    def count(self, side: str, problem: "Problem") -> int:
        n = sum(1 for t in self.targets if problem.mods[t].side == side)
        return n + (self.junk[0] + self.junk[1] if side == "prefix" else self.junk[2] + self.junk[3])

    def notables(self, problem: "Problem") -> int:
        return sum(1 for t in self.targets if problem.mods[t].notable) + self.junk[0] + self.junk[2]


@dataclass
class Problem:
    pool: list[Mod]
    targets: frozenset[str]  # acceptable mod ids
    need: int  # how many of `targets` must be on the item
    notable_cap: int = 99
    affix_limit: dict = field(default_factory=lambda: dict(JEWEL_AFFIX_LIMIT))

    @cached_property
    def mods(self) -> dict[str, Mod]:
        return {m.id: m for m in self.pool}

    @cached_property
    def _junk(self) -> dict[tuple[str, bool], tuple[float, float]]:
        """(side, notable) -> (total weight, expected weight removed per draw)."""
        out = {}
        for side in ("prefix", "suffix"):
            for notable in (True, False):
                groups: dict[tuple, float] = {}
                for m in self.pool:
                    if m.side == side and m.notable == notable and m.id not in self.targets:
                        groups[m.groups] = groups.get(m.groups, 0) + m.weight
                total = sum(groups.values())
                removed = sum(w * w for w in groups.values()) / total if total else 0.0
                out[(side, notable)] = (total, removed)
        return out

    def done(self, s: State) -> bool:
        return len(s.targets) >= self.need

    def add_mod(self, s: State) -> list[tuple[float, State]]:
        """Outcomes of adding one random mod (augment, regal, exalt)."""
        limit = self.affix_limit["rare" if s.rare else "magic"]
        cap_hit = s.notables(self) >= self.notable_cap
        taken = {g for t in s.targets for g in self.mods[t].groups}
        options: list[tuple[float, State]] = []
        for side in ("prefix", "suffix"):
            if s.count(side, self) >= limit:
                continue
            for t in self.targets - s.targets:
                m = self.mods[t]
                if m.side == side and not (m.notable and cap_hit) and not set(m.groups) & taken:
                    options.append((m.weight, State(s.rare, s.targets | {t}, s.junk)))
            for notable in (True, False):
                if notable and cap_hit:
                    continue
                idx = (0 if side == "prefix" else 2) + (0 if notable else 1)
                total, removed = self._junk[(side, notable)]
                weight = max(total - removed * s.junk[idx], 0.0)
                if weight > 0:
                    junk = list(s.junk)
                    junk[idx] += 1
                    options.append((weight, State(s.rare, s.targets, tuple(junk))))
        total = sum(w for w, _ in options)
        return [(w / total, st) for w, st in options] if total else []

    def reroll(self) -> list[tuple[float, State]]:
        """Outcomes of an alteration (or transmutation on a normal item)."""
        empty = State(False, frozenset())
        out: dict[State, float] = {}
        for p1, one in self.add_mod(empty):
            out[one] = out.get(one, 0) + p1 * (1 - MAGIC_TWO_MOD_CHANCE)
            for p2, two in self.add_mod(one):
                out[two] = out.get(two, 0) + p1 * MAGIC_TWO_MOD_CHANCE * p2
        return list((p, s) for s, p in out.items())

    def actions(self, s: State) -> dict[str, list[tuple[float, State]]]:
        acts: dict[str, list[tuple[float, State]]] = {}
        mods = s.count("prefix", self) + s.count("suffix", self)
        if not s.rare:
            acts["alt"] = self.reroll()
            if mods < 2:
                acts["aug"] = self.add_mod(s)
            acts["regal"] = [(p, State(True, n.targets, n.junk)) for p, n in self.add_mod(State(True, s.targets, s.junk))]
        else:
            if mods < 2 * self.affix_limit["rare"]:
                acts["exalt"] = self.add_mod(s)
            acts["scour"] = self.reroll()
        return {a: o for a, o in acts.items() if o}


@dataclass
class Plan:
    cost: float  # expected chaos from a fresh (transmuted) item
    policy: dict[State, str]
    usage: dict[str, float]  # expected number of each action
    visits: dict[State, float]  # expected times each state is acted on


def solve(problem: Problem, prices: dict[str, float], max_rounds: int = 100) -> Plan:
    """Cheapest policy by policy iteration, evaluating each policy exactly."""
    action_cost = {a: sum(prices.get(c, 0.0) for c in CURRENCY[a]) for a in ACTIONS}
    states, edges = _explore(problem)
    open_states = [s for s in states if not problem.done(s)]
    index = {s: i for i, s in enumerate(open_states)}
    # Start from "reroll everything", which always works eventually.
    policy = {s: ("alt" if "alt" in edges[s] else "scour") for s in open_states}

    for _ in range(max_rounds):
        value = _evaluate(open_states, index, edges, policy, action_cost)
        changed = False
        for s in open_states:
            q = {
                a: action_cost[a] + sum(p * value[index[n]] for p, n in outcomes if n in index)
                for a, outcomes in edges[s].items()
            }
            best = min(q, key=q.get)
            if q[best] < q[policy[s]] - 1e-9 * max(1.0, q[policy[s]]):
                policy[s], changed = best, True
        if not changed:
            break

    start = problem.reroll()  # transmute the fresh base
    cost = prices.get("Orb of Transmutation", 0.0) + sum(p * value[index[s]] for p, s in start if s in index)
    visits = _visits(open_states, index, edges, policy, start)
    usage = {a: 0.0 for a in ACTIONS}
    for s, n in visits.items():
        usage[policy[s]] += n
    return Plan(cost=cost, policy=policy, usage=usage, visits=visits)


def _explore(problem: Problem):
    """Every state reachable from a fresh item, and the outcomes of each action."""
    frontier = [s for _, s in problem.reroll()]
    states: set[State] = set(frontier)
    edges: dict[State, dict] = {}
    while frontier:
        s = frontier.pop()
        if problem.done(s):
            continue
        edges[s] = problem.actions(s)
        for outcomes in edges[s].values():
            for _, n in outcomes:
                if n not in states:
                    states.add(n)
                    frontier.append(n)
    return sorted(states, key=lambda s: (s.rare, len(s.targets), s.junk)), edges


def _transitions(open_states, index, edges, policy) -> np.ndarray:
    """P[i, j]: chance policy moves open state i to open state j."""
    P = np.zeros((len(open_states), len(open_states)))
    for s in open_states:
        for p, n in edges[s][policy[s]]:
            if n in index:
                P[index[s], index[n]] += p
    return P


def _evaluate(open_states, index, edges, policy, action_cost) -> np.ndarray:
    P = _transitions(open_states, index, edges, policy)
    c = np.array([action_cost[policy[s]] for s in open_states])
    try:
        return np.linalg.solve(np.eye(len(open_states)) - P, c)
    except np.linalg.LinAlgError:  # policy never finishes from some state
        return np.full(len(open_states), np.inf)


def _visits(open_states, index, edges, policy, start) -> dict[State, float]:
    """Expected number of times each state is acted on before success."""
    P = _transitions(open_states, index, edges, policy)
    s0 = np.zeros(len(open_states))
    for p, s in start:
        if s in index:
            s0[index[s]] += p
    n = np.linalg.solve((np.eye(len(open_states)) - P).T, s0)
    return {s: float(n[index[s]]) for s in open_states}


def describe(problem: Problem, s: State) -> str:
    """E.g. `magic: Prismatic Heart + 1 other suffix`."""
    parts = sorted(problem.mods[t].label for t in s.targets)
    labels = ("other prefix notable", "other prefix", "other suffix notable", "other suffix")
    for n, label in zip(s.junk, labels):
        if n:
            parts.append(f"{n} {label}")
    return f"{'rare' if s.rare else 'magic'}: {' + '.join(parts) or 'nothing useful'}"


def recipe(problem: Problem, plan: Plan, min_share: float = 0.01) -> list[tuple[str, str, float]]:
    """The policy for the states you actually meet: (state, action, share of all actions)."""
    total = sum(plan.visits.values()) or 1.0
    rows = [(describe(problem, s), plan.policy[s], n / total) for s, n in plan.visits.items() if n / total >= min_share]
    return sorted(rows, key=lambda r: -r[2])


def simulate(problem: Problem, plan: Plan, prices: dict[str, float], runs: int = 2000, seed: int = 0) -> float:
    """Average chaos spent replaying `plan` on real mods."""
    rng = random.Random(seed)
    cache: dict = {}
    action_cost = {a: sum(prices.get(c, 0.0) for c in CURRENCY[a]) for a in ACTIONS}
    total = 0.0
    for _ in range(runs):
        total += prices.get("Orb of Transmutation", 0.0)
        rare, mods = False, _roll_magic(problem, rng, cache)
        for _ in range(10**6):
            state = _lump(problem, rare, mods)
            if problem.done(state):
                break
            action = plan.policy.get(state) or "alt"
            total += action_cost[action]
            if action in ("alt", "scour"):
                rare, mods = False, _roll_magic(problem, rng, cache)
            else:
                rare = rare or action == "regal"
                mod = _draw(problem, rare, mods, rng, cache)
                if mod:
                    mods = mods + [mod]
    return total / runs


def _roll_magic(problem, rng, cache) -> list[Mod]:
    mods = [_draw(problem, False, [], rng, cache)]
    if rng.random() < MAGIC_TWO_MOD_CHANCE:
        second = _draw(problem, False, mods, rng, cache)
        if second:
            mods.append(second)
    return [m for m in mods if m]


def _draw(problem: Problem, rare: bool, mods: list[Mod], rng, cache: dict) -> Mod | None:
    key = (rare, frozenset(m.id for m in mods))
    if key not in cache:
        limit = problem.affix_limit["rare" if rare else "magic"]
        taken = {g for m in mods for g in m.groups}
        notables = sum(m.notable for m in mods)
        pool = [
            m
            for m in problem.pool
            if sum(x.side == m.side for x in mods) < limit
            and not set(m.groups) & taken
            and not (m.notable and notables >= problem.notable_cap)
        ]
        cache[key] = (pool, list(itertools.accumulate(m.weight for m in pool)))
    pool, cumulative = cache[key]
    if not pool:
        return None
    return rng.choices(pool, cum_weights=cumulative)[0]


def _lump(problem: Problem, rare: bool, mods: list[Mod]) -> State:
    targets = frozenset(m.id for m in mods if m.id in problem.targets)
    junk = [0, 0, 0, 0]
    for m in mods:
        if m.id not in problem.targets:
            junk[(0 if m.side == "prefix" else 2) + (0 if m.notable else 1)] += 1
    return State(rare, targets, tuple(junk))
