"""Export solver goldens for the Rust port: problems as JSON plus what the Python solver makes of them.

    uv run python scripts/golden/solver.py [OUT_DIR]   # default rust/goldens/solver

Each file is {"problem": ..., "prices": ..., "chaos_per_hour": ..., "plan": ..., "steps": ..., "self_fracture": ...}.
The problem is written *before* solving (solve filters actions in place). Plans carry the
expected cost to finish from every open state, so a port can be checked state by state.
Ties between equally cheap actions may break differently; compare values, not policies.
"""

from __future__ import annotations

import copy
import dataclasses
import gzip
import json
import sys
from pathlib import Path

from ponder import craft, demand, ladder, pipeline, prices
from ponder.craft import Action, Mod, Problem, State, Target
from ponder.mods import ModDB


def state(s: State | None):
    return None if s is None else {"rarity": s.rarity, "status": list(s.status), "junk": list(s.junk)}


def mod(m: Mod):
    return {"id": m.id, "side": m.side, "weight": m.weight, "groups": list(m.groups), "notable": m.notable, "label": m.label, "tags": list(m.tags)}


def target(t: Target):
    return {"name": t.name, "side": t.side, "good": sorted(t.good), "groups": sorted(t.groups), "notable": t.notable}


def action(a: Action):
    sale = None
    if a.sale:
        need, (p, q, n), spare = a.sale
        sale = {"need": sorted(need), "prefix": p, "suffix": q, "any": n, "spare": sorted(spare)}
    return {
        "name": a.name, "kind": a.kind, "cost": [[pid, qty] for pid, qty in a.cost], "from_rarity": list(a.from_rarity),
        "profile": a.profile, "first_profile": a.first_profile, "forced": a.forced, "credit": a.credit, "sale": sale,
    }  # fmt: skip


def problem(p: Problem):
    return {
        "pool": [mod(m) for m in p.pool],
        "targets": [target(t) for t in p.targets],
        "need": p.need,
        "actions": [action(a) for a in p.actions],
        "kind": p.kind,
        "notable_cap": p.notable_cap,
        "fixed": [mod(m) for m in p.fixed],
        "profiles": {k: dict(v) for k, v in p.profiles.items()},
        "extra_mods": {k: mod(m) for k, m in p.extra_mods.items()},
        "reserve": dict(p.reserve),
        "filler": sorted(p.filler),
        "filler_need": p.filler_need,
        "filler_sides": list(p.filler_sides) if p.filler_sides else None,
        "junk_cap": list(p.junk_cap) if p.junk_cap else None,
        "begin": state(p.begin),
    }


def export(name: str, p: Problem, table: dict[str, float], out: Path) -> None:
    before = problem(p)
    used = sorted({pid for a in p.actions for pid, _ in a.cost} | {craft.FRACTURE_ORB})
    priced = {pid: table[pid] for pid in used if pid in table}
    work = copy.deepcopy(p)
    made = [dataclasses.asdict(f) if (f := craft.self_fracture(work, i, priced)) else None for i in range(len(work.targets))]
    try:
        plan = craft.solve(work, priced)
        result = {
            "cost": plan.cost, "hours": plan.hours, "currency": plan.currency, "states": plan.states, "unpriced": plan.unpriced,
            "usage": plan.usage,
            "values": [[state(s), v] for s, v in plan.values.items()],
            "policy": [[state(s), a] for s, a in plan.policy.items()],
            "visits": [[state(s), v] for s, v in plan.visits.items()],
            "actions_after": [a.name for a in work.actions],
        }  # fmt: skip
        steps = [dataclasses.asdict(st) for st in craft.steps(work, plan)]
    except (ValueError, ArithmeticError) as e:
        result, steps = {"error": str(e)}, []
    doc = {"name": name, "problem": before, "prices": priced, "chaos_per_hour": craft.CHAOS_PER_HOUR, "plan": result, "steps": steps, "self_fracture": made}
    with gzip.open(out / f"{name}.json.gz", "wt") as f:
        json.dump(doc, f)
    print(f"{name}: {result.get('states', '-')} states, cost {result.get('cost', result.get('error'))}")


def toys() -> list[tuple[str, Problem]]:
    pool = [
        Mod("A", "prefix", 100, ("A",), True, "A", ("fire",)),
        Mod("B", "suffix", 100, ("B",), True, "B", ("cold",)),
        Mod("C", "prefix", 100, ("C",), True, "C"),
        Mod("x1", "prefix", 300, ("x",), False, "x1"),
        Mod("x2", "prefix", 300, ("x",), False, "x2"),
        Mod("y", "suffix", 500, ("y",), False, "y"),
    ]

    def tgt(i):
        m = next(m for m in pool if m.id == i)
        return Target(m.label, m.side, frozenset({m.id}), frozenset(m.groups), m.notable)

    from ponder import actions

    def toy(extra=(), profiles=None, targets=("A", "B"), **kw):
        return Problem(pool=pool, targets=[tgt(t) for t in targets], need=len(targets), actions=list(actions.BASIC) + list(extra), kind="jewel", profiles=profiles or {}, **kw)

    harvest = Action("harvest fire", "reroll_rare", (("lifeforce", 30),), (craft.RARE,), first_profile="fire")
    fossil = Action("fire fossil", "reroll_rare", (("fossil", 1),), (craft.NORMAL, craft.RARE), profile="fossil")
    profiles = {"fire": {"A": 100.0}, "fossil": {m.id: m.weight * (10 if "fire" in m.tags else 1) for m in pool}}
    only_a = craft.solve(toy(targets=("A",)), TOY_PRICES).cost
    sale = Action("sell as A", "sell", (), (craft.MAGIC, craft.RARE), credit=0.7 * only_a, sale=(frozenset({0}), (0, 0, 0), frozenset()))
    begun = toy()
    begun.begin = begun.state_of(craft.RARE, [pool[0], pool[5]])
    return [
        ("toy_basic", toy()),
        ("toy_harvest_fossil", toy([harvest, fossil], profiles)),
        ("toy_three", toy(targets=("A", "B", "C"))),
        ("toy_sell", toy([sale])),
        ("toy_begin", begun),
        ("toy_fixed", toy(fixed=[pool[0]], targets=("B",))),
    ]


TOY_PRICES = {"transmute": 0.1, "alt": 1.0, "aug": 0.2, "regal": 3.0, "exalted": 50.0, "annul": 20.0, "alch": 0.5, "chaos": 2.0, "scour": 1.0, "lifeforce": 0.1, "fossil": 5.0}


def main() -> None:
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "rust/goldens/solver")
    out.mkdir(parents=True, exist_ok=True)
    saved, craft.CHAOS_PER_HOUR = craft.CHAOS_PER_HOUR, 0.0  # toys: currency only, as in the Python tests
    for name, p in toys():
        export(name, p, TOY_PRICES, out)
    craft.CHAOS_PER_HOUR = saved
    table, _ = prices.load()
    db = ModDB()
    found = demand.archetypes(demand.wear(db, list(ladder.cached())), 5)
    seen: set = set()
    picks = []
    for a in found:  # one of each base group with 2-4 targets, the first dozen groups
        if a.group in seen or len(picks) >= 12:
            continue
        m = demand.archetype_match(db, a)
        if m is None:
            continue
        e = pipeline.estimate_gear(db, m, table, solve=False)
        if e.error or not e.problem or not 2 <= len(e.problem.targets) <= 4:
            continue
        seen.add(a.group)
        picks.append(e)
    for e in picks:
        slug = "".join(c if c.isalnum() else "_" for c in e.title).strip("_").lower()
        export(f"gear_{slug}", e.problem, table, out)
    e = picks[0]
    pipeline.add_fractures(e, "x", table, filler=False)
    if e.fractures:
        export("gear_fractured", e.fractures[0].problem, table, out)
    half = copy.deepcopy(picks[1].problem)
    half.begin = craft.State(craft.RARE, tuple(craft.GOOD if i == 0 else craft.MISSING for i in range(len(half.targets))), (0, 1, 0, 1, 0, 0))
    export("gear_begin", half, table, out)
    for i, a in enumerate(demand.cluster_archetypes(demand.wear_clusters(list(ladder.cached())), 5)[:3]):
        c = pipeline.estimate_cluster(a.size, a.enchant, list(a.notables), table, "x", passives=a.usual_passives, solve=False)
        if c.problem and not c.error:
            export(f"cluster_{i}", c.problem, table, out)


if __name__ == "__main__":
    main()
