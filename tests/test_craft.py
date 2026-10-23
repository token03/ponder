"""Crafting solver: toy pools need no data; cluster checks run once data is fetched."""

import pytest

from ponder import craft, data
from ponder.craft import Mod, Problem, State, solve

PRICES = {"Orb of Alteration": 1.0, "Orb of Augmentation": 0.2, "Regal Orb": 3.0, "Exalted Orb": 50.0,
          "Orb of Scouring": 1.0, "Orb of Transmutation": 0.1}


def toy(need=2, cap=99):
    pool = [
        Mod("A", "prefix", 100, ("A",), True, "A"),
        Mod("B", "suffix", 100, ("B",), True, "B"),
        Mod("C", "prefix", 100, ("C",), True, "C"),
        Mod("x1", "prefix", 300, ("x",), False, "x1"),
        Mod("x2", "prefix", 300, ("x",), False, "x2"),  # same group as x1
        Mod("y", "suffix", 500, ("y",), False, "y"),
    ]
    return Problem(pool=pool, targets=frozenset({"A", "B"}), need=need, notable_cap=cap)


def test_reroll_is_a_distribution():
    outcomes = toy().reroll()
    assert abs(sum(p for p, _ in outcomes) - 1) < 1e-9
    # magic items never hold two prefixes
    assert all(s.count("prefix", toy()) <= 1 for _, s in outcomes)


def test_solver_matches_simulation():
    problem = toy()
    plan = solve(problem, PRICES)
    simulated = craft.simulate(problem, plan, PRICES, runs=4000, seed=1)
    assert simulated == pytest.approx(plan.cost, rel=0.08)


def test_policy_beats_alt_spam():
    problem = toy()
    plan = solve(problem, PRICES)
    alt_only = craft.Plan(0, {s: ("alt" if not s.rare else "scour") for s in plan.policy}, {}, {})
    assert plan.cost < craft.simulate(problem, alt_only, PRICES, runs=2000)


def test_magic_with_one_target_augments():
    problem = toy()
    plan = solve(problem, PRICES)
    assert plan.policy[State(False, frozenset({"A"}))] == "aug"


needs_data = pytest.mark.skipif(
    not (data.available() and (data.data_dir() / "cluster_jewels.json").is_file()),
    reason="run `ponder data fetch` first",
)


@needs_data
def test_cluster_two_prefix_notables_need_a_regal():
    from ponder import cluster

    problem, plan = cluster.cost("medium", "area damage", ["Vast Power", "Assert Dominance"], PRICES)
    assert {problem.mods[t].side for t in problem.targets} == {"prefix"}
    assert plan.usage["regal"] > 0
    assert craft.simulate(problem, plan, PRICES, runs=1500, seed=2) == pytest.approx(plan.cost, rel=0.12)


@needs_data
def test_cluster_notable_cap():
    from ponder import cluster

    with pytest.raises(ValueError):
        cluster.cost("medium", "area damage", ["Vast Power", "Assert Dominance", "Magnifier"], PRICES)
