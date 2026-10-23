import pytest

from ponder import data, recomb


def test_side_counts_are_distributions():
    for n, row in recomb.SIDE_COUNTS.items():
        assert sum(row.values()) == pytest.approx(1.0), n
        assert max(row) <= min(n, 3)


def test_nnn_padding_raises_the_chance_to_keep_everything():
    # Two wanted prefixes, one per donor: a pool of 2 keeps both a third of the time.
    plain = recomb.chance({"prefix": (1, 0)}, {"prefix": (1, 0)}, same_base=True)
    assert plain == pytest.approx(0.33)
    # Two NNN prefixes on a partner base make a pool of 4: kept 2+ 90% of the time, but the base is a coin flip.
    padded = recomb.chance({"prefix": (1, 0)}, {"prefix": (1, 2)}, same_base=False)
    assert padded == pytest.approx(0.5 * 0.9)
    # Sides multiply; a side with nothing wanted doesn't matter.
    both = recomb.chance({"prefix": (1, 0), "suffix": (1, 0)}, {"prefix": (1, 0), "suffix": (0, 0)}, same_base=True)
    assert both == pytest.approx(0.33 * 0.59)


def test_step_cost_divides_by_the_chance():
    a = recomb.Donor("A", "a", ("x",))
    b = recomb.Donor("B", "b", ("y",))
    a.plan = type("P", (), {"cost": 10.0})()
    b.plan = type("P", (), {"cost": 20.0})()
    step = recomb.Step(a, b, 0.25)
    assert step.cost == pytest.approx(120.0)
    assert [s[0] for s in step.steps()] == ["craft", "craft", "recombine"]


@pytest.mark.skipif(not data.available(), reason="run `ponder data fetch` first")
def test_planner_builds_padded_leaves_and_a_tree():
    from ponder import gear
    from ponder.craft import Target
    from ponder.mods import ModDB

    db = ModDB()
    base = db.find_base("Warlock Gloves")
    tags = db.item_tags(base, set())
    pool = gear.pool(db, base, tags, 84)

    def target(family, name):
        ids = frozenset(m.id for m in pool if db.mods[m.id]["type"] == family and db.tier(m.id, tags) <= 2)
        side = next(m.side for m in pool if m.id in ids)
        return Target(name, side, ids, frozenset(g for m in pool if m.id in ids for g in m.groups))

    problem = gear.problem_on(db, base, [target("IncreasedLife", "life"), target("Intelligence", "int")], 84)
    planner = recomb.Planner(db, problem, base, 84)
    leaves = planner.leaves()
    assert any(not d.on_target and d.padding for d in leaves)  # partner bases with NNN pads
    assert all(d.problem.junk_cap == (0, 0) for d in leaves)  # donors must be clean
    for d in leaves:  # stand-in costs: padding costs more
        d.plan = type("P", (), {"cost": 5.0 + 3.0 * sum(d.padding.values())})()
    best = planner.best()
    assert best is not None and sorted(best.carries) == ["int", "life"]
    plain = 10.0 / (0.59 * 0.59)  # life on one donor, int on the other, both on the target base
    assert best.cost <= plain + 1e-9


@pytest.mark.skipif(not data.available(), reason="run `ponder data fetch` first")
def test_filler_slots_get_distinct_resistance_families():
    from dataclasses import replace

    from ponder import gear
    from ponder.mods import ModDB

    db = ModDB()
    base = db.find_base("Warlock Gloves")
    problem = gear.problem_on(db, base, [], 84)
    res = frozenset(m for m in problem.mods if db.mods[m]["type"] in gear.FILLER_FAMILIES)
    problem = replace(problem, filler=res, filler_need=3)
    slots = [w.target.good for w in recomb.wants(problem)]
    assert len(slots) == 3 and all(slots)
    groups = [{g for m in ids for g in problem.mods[m].groups} for ids in slots]
    assert not (groups[0] & groups[1] or groups[0] & groups[2] or groups[1] & groups[2])  # two donors can't both bring fire


def test_chance_matches_a_simulation_with_nnn_and_junk_free_donors():
    a = {"prefix": ["life"], "suffix": ["res"]}
    b = {"prefix": ["es", "nnn1", "nnn2"], "suffix": ["nnn3"]}
    legal = {"life", "res", "es"}
    sim = recomb.simulate(a, b, legal, same_base=False, wanted=legal, trials=40000)
    exact = recomb.chance({"prefix": (1, 0), "suffix": (1, 0)}, {"prefix": (1, 2), "suffix": (0, 1)}, same_base=False)
    assert sim == pytest.approx(exact, abs=0.01)


def test_failures_that_hand_back_an_input_cut_the_cost():
    # life (prefix) + int (suffix) on two target-base donors: each side keeps its mod 59% of the time.
    a, b = {"prefix": (1, 0)}, {"suffix": (1, 0)}
    assert recomb.salvage(a, b, same_base=True) == pytest.approx(0.59 * 0.41)  # life kept, int lost
    # Exact salvage chances plus success and the rest add up over the four outcomes.
    total = recomb.chance(a, b, True) + recomb.salvage(a, b, True) + recomb.salvage(b, a, True) + 0.41 * 0.41
    assert total == pytest.approx(1.0)
    x = recomb.Donor("A", "a", ("life",))
    y = recomb.Donor("A", "a", ("int",))
    x.plan = y.plan = type("P", (), {"cost": 10.0})()
    step = recomb.Step(x, y, 0.59 * 0.59, 0.59 * 0.41, 0.59 * 0.41)
    assert step.cost == pytest.approx(20 * (1 - 0.59 * 0.41) / (0.59 * 0.59))


def test_salvage_matches_a_simulation():
    import random

    a = {"prefix": ["life", "es"], "suffix": ["res"]}
    b = {"prefix": ["nnn1"], "suffix": ["int", "nnn2"]}
    legal = {"life", "es", "res", "int"}
    rng, hits, trials = random.Random(1), 0, 40000
    for _ in range(trials):
        if rng.random() >= 0.5:
            continue
        kept = set()
        for side in recomb.SIDES:
            pool = a.get(side, []) + b.get(side, [])
            row = recomb.SIDE_COUNTS[min(len(pool), 6)]
            k = rng.choices(list(row), weights=list(row.values()))[0]
            eligible = [m for m in pool if m in legal]
            kept |= set(rng.sample(eligible, min(k, len(eligible))))
        hits += kept == {"life", "es", "res"}
    exact = recomb.salvage({"prefix": (2, 0), "suffix": (1, 0)}, {"prefix": (0, 1), "suffix": (1, 1)}, same_base=False)
    assert hits / trials == pytest.approx(exact, abs=0.01)
