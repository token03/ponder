"""Crafting solver: toy pools need no data; cluster and gear checks run once data is fetched."""

import pytest

from ponder import actions, craft, data
from ponder.craft import GOOD, MAGIC, MISSING, NORMAL, RARE, Action, Mod, Problem, State, Target, solve

PRICES = {
    "transmute": 0.1, "alt": 1.0, "aug": 0.2, "regal": 3.0, "exalted": 50.0, "annul": 20.0,
    "alch": 0.5, "chaos": 2.0, "scour": 1.0, "lifeforce": 0.1, "fossil": 5.0,
}  # fmt: skip

POOL = [
    Mod("A", "prefix", 100, ("A",), True, "A", ("fire",)),
    Mod("B", "suffix", 100, ("B",), True, "B", ("cold",)),
    Mod("C", "prefix", 100, ("C",), True, "C"),
    Mod("x1", "prefix", 300, ("x",), False, "x1"),
    Mod("x2", "prefix", 300, ("x",), False, "x2"),  # same group as x1
    Mod("y", "suffix", 500, ("y",), False, "y"),
]


def target(mod_id):
    m = next(m for m in POOL if m.id == mod_id)
    return Target(m.label, m.side, frozenset({m.id}), frozenset(m.groups), m.notable)


def toy(extra_actions=(), profiles=None, fixed=(), targets=("A", "B")):
    return Problem(
        pool=POOL,
        targets=[target(t) for t in targets],
        need=len(targets),
        actions=list(actions.BASIC) + list(extra_actions),
        kind="jewel",
        fixed=[m for m in POOL if m.id in fixed],
        profiles=profiles or {},
    )


def test_rolls_are_distributions():
    problem = toy()
    for rarity in (MAGIC, RARE):
        outcomes = problem.roll(rarity)
        assert abs(sum(p for p, _ in outcomes) - 1) < 1e-9
    assert all(problem.count(s, "prefix") <= 1 for _, s in problem.roll(MAGIC))


def test_solver_matches_simulation():
    problem = toy()
    plan = solve(problem, PRICES)
    assert craft.simulate(problem, plan, PRICES, runs=4000, seed=1) == pytest.approx(plan.cost, rel=0.08)


def test_harvest_and_fossil_match_simulation():
    harvest = Action("harvest fire", "reroll_rare", (("lifeforce", 30),), (RARE,), first_profile="fire")
    fossil = Action("fire fossil", "reroll_rare", (("fossil", 1),), (NORMAL, RARE), profile="fossil")
    profiles = {"fire": {"A": 100.0}, "fossil": {m.id: m.weight * (10 if "fire" in m.tags else 1) for m in POOL}}
    problem = toy([harvest, fossil], profiles)
    plan = solve(problem, PRICES)
    assert craft.simulate(problem, plan, PRICES, runs=4000, seed=3) == pytest.approx(plan.cost, rel=0.08)


def test_policy_beats_alt_spam():
    problem = toy()
    plan = solve(problem, PRICES)
    alt_only = craft.Plan(0, {s: ("scour+transmute" if s.rarity == RARE else "alt" if s.rarity == MAGIC else "transmute") for s in plan.policy}, {}, {}, 0)
    assert plan.cost < craft.simulate(problem, alt_only, PRICES, runs=2000)


def test_magic_with_one_target_augments():
    problem = toy()
    plan = solve(problem, PRICES)
    assert plan.policy[State(MAGIC, (GOOD, MISSING))] == "aug"


def test_fractured_mod_survives_and_lowers_cost():
    plain = solve(toy(), PRICES).cost
    fractured = toy(fixed=("A",))
    assert fractured.start.status == (GOOD, MISSING)
    assert all(s.status[0] == GOOD for _, s in fractured.roll(RARE))
    assert solve(fractured, PRICES).cost < plain


def test_annul_never_removes_fractured():
    problem = toy(fixed=("A",))
    s = State(RARE, (GOOD, MISSING), (0, 0, 0, 1))
    assert all(n.status[0] == GOOD for _, n in problem.annul(s))


def test_unpriced_actions_are_dropped_not_free():
    free_lunch = Action("mystery", "reroll_rare", (("unknown-orb", 1),), (NORMAL, RARE))
    plan = solve(toy([free_lunch]), PRICES)
    assert "mystery" in plan.unpriced and "mystery" not in plan.usage


needs_data = pytest.mark.skipif(
    not (data.available() and (data.data_dir() / "cluster_jewels.json").is_file()),
    reason="run `ponder data fetch` first",
)


@needs_data
def test_cluster_two_prefix_notables_need_a_rare():
    from ponder import cluster

    prices = {**PRICES, "exalted": 50.0}
    problem, plan = cluster.cost("medium", "area damage", ["Vast Power", "Assert Dominance"], prices)
    assert {t.side for t in problem.targets} == {"prefix"}
    # a magic jewel holds one prefix, so every plan must make the item rare
    assert any(plan.usage[a] > 0 for a in ("regal", "alch", "chaos", "scour+alch"))
    assert craft.simulate(problem, plan, prices, runs=1500, seed=2) == pytest.approx(plan.cost, rel=0.12)


@needs_data
def test_cluster_notable_cap():
    from ponder import cluster

    with pytest.raises(ValueError):
        cluster.cost("medium", "area damage", ["Vast Power", "Assert Dominance", "Magnifier"], PRICES)


@needs_data
def test_gear_targets_respect_tier_and_essences():
    from ponder import gear
    from ponder.item import parse_item
    from ponder.mods import ModDB

    db = ModDB()
    item = parse_item(
        "Rarity: RARE\nX\nTwo-Toned Boots\nItem Level: 84\nImplicits: 0\n"
        "+117 to maximum Life\n30% increased Movement Speed\n"
    )
    gp = gear.build(db, db.match(item), settle=0)
    life = next(t for t in gp.problem.targets if "Life" in t.name)
    assert all(db.mods[m]["required_level"] >= 54 for m in life.good)  # T1 only on boots
    assert any("essence" in a.name for a in gp.problem.actions)
    plan = solve(gp.problem, {**PRICES})  # essences unpriced here, so left out
    assert plan.unpriced and plan.cost > 0


def test_metamod_keeps_one_side_and_matches_simulation():
    lock = Action("lock prefixes+scour", "keep_prefixes", (("divine", 1), ("scour", 1)), (RARE,))
    problem = toy([lock])
    [(_, kept)] = problem.outcomes(State(RARE, (GOOD, MISSING), (0, 1, 0, 1)), lock)
    assert kept == State(RARE, (GOOD, MISSING), (0, 1, 0, 0))
    assert problem.outcomes(State(RARE, (GOOD, MISSING), (0, 0, 0, 2)), lock) == []  # no suffix room for the metamod
    # The toy pool is too small for the lock to pay off, so force it wherever it applies
    # and check the solver's exact value for that policy against a replay.
    prices = {**PRICES, "divine": 0.5}
    plan = solve(problem, prices)
    states, edges = craft._explore(problem)
    for s in plan.policy:
        if lock.name in edges[s] and s.status[1] == MISSING and s.junk[3]:
            plan.policy[s] = lock.name
    exact = craft.evaluate(problem, plan.policy, prices)
    assert exact > plan.cost  # the solver was right to skip it here
    assert craft.simulate(problem, plan, prices, runs=4000, seed=4) == pytest.approx(exact, rel=0.08)


def test_reserved_bench_slot_must_stay_open():
    problem = toy(targets=("A",))
    problem.reserve = {"prefix": 1}
    assert not problem.done(State(RARE, (GOOD,), (0, 1, 0, 0)))  # jewel: two prefixes, none left
    assert problem.done(State(RARE, (GOOD,), (0, 0, 0, 2)))
    assert solve(problem, PRICES).cost > solve(toy(targets=("A",)), PRICES).cost
