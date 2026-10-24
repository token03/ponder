"""Export recombinator and pricing goldens for the Rust port.

    uv run python scripts/golden/recomb.py [OUT_DIR [PART ...]]   # default rust/goldens/recomb, every part

PART is any of kernel, planner (with solved), pricing. Nothing here needs the network.

Inputs are what `pipeline.add_recombs` feeds the planner: the gear estimates of the cached
ladder's top archetypes (the same ones `problems.py` exports, so the Rust test takes their
problems from rust/goldens/problems/gear.json.gz by row index).

- kernel.json: SIDE_COUNTS, keep_at_least / keep_exactly over a grid, chance and salvage over
  the one-sided grid and a seeded sample of two-sided cases.
- planner.json.gz: for every estimate with a planner (`ok`): the wanted mods, every leaf donor
  (key fields plus a CRC fingerprint of its problem), and `best().to_dict()` once the leaves are
  given stand-in costs (a fixed function of the leaf's position, some left unsolved).
- solved.json.gz: the same for the first few planners with the leaves *really* solved
  (`pipeline.solve_all`): leaf costs, errors, and the cheapest tree.
- pricing.json.gz: `pricing.archetype_query` for every archetype, and `pricing.fit` on synthetic
  trade listings (generated here from repoe, no network) with the matched mods of each listing.

Python's set iteration order varies between runs and `recomb.wants` breaks ties between equally
heavy resistance families by it; the filler sets are iterated sorted here so the export is
reproducible (the Rust port sorts too).
"""

from __future__ import annotations

import gzip
import json
import random
import re
import sys
import zlib
from itertools import product
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from solver import problem as problem_json  # noqa: E402

from ponder import craft, demand, gear, ladder, pipeline, prices, pricing, recomb  # noqa: E402
from ponder.demand import RES, stat_key  # noqa: E402
from ponder.mods import ModDB  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
SOLVED_PLANNERS = 10  # planners whose leaves are really solved


class SortedFrozenset(frozenset):
    """A frozenset that iterates in sorted order (Python's own order varies per run)."""

    def __iter__(self):
        return iter(sorted(frozenset.__iter__(self)))


# `recomb.wants` takes a filler slot's side from `next(iter(frozenset_of_ids))`, and a slot can mix
# sides (resistances share a slot with a defence family), so Python's answer depends on hash order.
# Make every frozenset the module builds iterate sorted; the Rust port takes the first sorted id.
recomb.frozenset = SortedFrozenset


def save(out: Path, name: str, doc, gz: bool = True) -> None:
    if gz:
        with gzip.open(out / f"{name}.json.gz", "wt") as f:
            json.dump(doc, f)
    else:
        (out / f"{name}.json").write_text(json.dumps(doc, indent=1))
    print(f"{name}: {len(doc)} entries")


def crc(text: str) -> int:
    return zlib.crc32(text.encode())


def fingerprint(p: craft.Problem) -> dict:
    """What Rust checks of a leaf's problem: the strings below, hashed with CRC-32."""
    pool = "".join(f"{m.id}|{m.weight}|{m.side};" for m in p.pool)
    acts = "".join(f"{a.name}|{a.kind}|{a.profile}|{a.first_profile}|{a.forced}|{','.join(map(str, a.from_rarity))};" for a in p.actions)
    profs = "".join(f"{k}:{','.join(sorted(v))};" for k, v in sorted(p.profiles.items()))
    extra = ",".join(sorted(p.extra_mods))
    return {
        "pool": crc(pool), "actions": crc(acts), "profiles": crc(profs), "extra": crc(extra), "n_pool": len(p.pool), "n_actions": len(p.actions),
        "cost_sum": sum(q for a in p.actions for _, q in a.cost),
        "targets": [[t.name, t.side, sorted(t.good), sorted(t.groups), t.notable] for t in p.targets],
        "junk_cap": list(p.junk_cap) if p.junk_cap else None, "filler": sorted(p.filler), "filler_need": p.filler_need,
        "kind": p.kind, "need": p.need,
    }  # fmt: skip


def leaf_doc(d: recomb.Donor) -> dict:
    return {
        "base": d.base, "base_id": d.base_id, "carries": list(d.carries), "padding": dict(d.padding), "on_target": d.on_target,
        "problem": fingerprint(d.problem),
    }  # fmt: skip


def want_doc(w: recomb.Want) -> dict:
    t = w.target
    return {"name": w.name, "side": w.side, "target": None if t is None else {"good": sorted(t.good), "groups": sorted(t.groups)}}


class Stub:
    def __init__(self, cost):
        self.cost = cost


def stand_in(planner: recomb.Planner) -> list:
    """Costs that depend only on the leaf's place and shape; every 17th is left unsolved."""
    costs = []
    for i, d in enumerate(planner.leaves()):
        if i % 17 == 5:
            d.plan, cost = None, None
        else:
            cost = 20.0 + 7.0 * len(d.carries) + 9.0 * sum(d.padding.values()) + (3.0 if not d.on_target else 0.0) + (i * 37 % 11) * 0.5
            d.plan = Stub(cost)
        costs.append(cost)
    return costs


def best_doc(planner: recomb.Planner) -> dict | None:
    b = planner.best()
    if b is None:
        return None
    return {**b.to_dict(), "carries": list(b.carries), "doubled": list(b.doubled), "donors": [d.describe() for d in b.donors()]}


# -- kernel ------------------------------------------------------------------------------------


def kernel() -> dict:
    rng = random.Random(7)
    pools = range(0, 9)
    out = {
        "side_counts": {str(n): {str(k): p for k, p in row.items()} for n, row in recomb.SIDE_COUNTS.items()},
        "table_version": recomb.TABLE_VERSION,
        "keep_at_least": [[n, w, recomb.keep_at_least(n, w)] for n in pools for w in range(0, 5)],
        "keep_exactly": [[n, w, s, recomb.keep_exactly(n, w, s)] for n in pools for w in range(0, 5) for s in range(0, w + 1)],
    }
    grid = list(product(range(0, 4), range(0, 4), range(0, 4), range(0, 4)))  # wa, na, wb, nb on one side
    one = []
    for (wa, na, wb, nb), side, same in product(grid, ("prefix", "suffix"), (True, False)):
        a, b = {side: (wa, na)}, {side: (wb, nb)}
        one.append([side, [wa, na], [wb, nb], same, recomb.chance(a, b, same), recomb.salvage(a, b, same), recomb.salvage(b, a, same)])
    out["one_side"] = one
    two = []
    for _ in range(3000):
        a = {s: (rng.randint(0, 3), rng.randint(0, 3)) for s in recomb.SIDES}
        b = {s: (rng.randint(0, 3), rng.randint(0, 3)) for s in recomb.SIDES}
        same = rng.random() < 0.5
        two.append([[list(a[s]) for s in recomb.SIDES], [list(b[s]) for s in recomb.SIDES], same, recomb.chance(a, b, same), recomb.salvage(a, b, same), recomb.salvage(b, a, same)])
    out["two_sides"] = two
    return out


# -- pricing -----------------------------------------------------------------------------------

RATES = {"chaos": 1.0, "divine": 150.0, "exalted": 12.0}
RANGE = re.compile(r"\((-?\d+(?:\.\d+)?)-(-?\d+(?:\.\d+)?)\)")


def roll_lines(db: ModDB, mod_id: str, high: bool = False) -> list[str]:
    """The item text of a mod at the bottom (or top) of its roll ranges."""
    text = db.mods[mod_id]["text"]
    return [RANGE.sub(lambda m: m.group(2 if high else 1), line) for line in text.split("\n")]


def synth_listings(db: ModDB, a, m, rng: random.Random, n: int = 90) -> list[dict]:
    grp = gear.group(db, m)
    pool = gear.pool(db, m.base_id, m.tags, 86)
    core = [k for k in a.families if k != RES]
    by_key = {k: sorted((x.id for x in pool if stat_key(x.id) == k and db.tier(x.id, m.tags)), key=lambda i: db.tier(i, m.tags)) for k in core}
    res = [x.id for x in pool if db.mods[x.id]["type"] in gear.FILLER_FAMILIES and db.tier(x.id, m.tags)]
    worth = {k: (rng.uniform(1.5, 4.0), rng.uniform(1.1, 2.0)) for k in core}
    out = []
    for i in range(n):
        lines, price = [], 40.0
        for k in core:
            ids = by_key[k]
            if not ids:
                continue
            pick = min(int(rng.expovariate(0.7)), len(ids) - 1)
            tier = db.tier(ids[pick], m.tags)
            price *= worth[k][0] if tier == 1 else worth[k][1] if tier == 2 else 1.0
            lines += roll_lines(db, ids[pick], high=rng.random() < 0.3)
        picked = rng.sample(res, min(len(res), rng.randint(0, 3))) if res else []
        price *= 1.15 ** len(picked)
        for r in picked:
            lines += roll_lines(db, r)
        price *= rng.uniform(0.85, 1.15)
        item = {"name": "", "baseType": rng.choice(grp.bases), "ilvl": rng.choice([80, 84, 86]), "explicitMods": lines, "influences": {"shaper": False}}
        if rng.random() < 0.15 and len(lines) > 1:
            item["fracturedMods"], item["explicitMods"] = [lines[0]], lines[1:]
        if rng.random() < 0.2:
            item["craftedMods"] = ["+(10-12)% to Fire Resistance".replace("(10-12)", "11")]
        cur = rng.choice(["chaos", "chaos", "divine", "exalted"])
        amount = round(price / RATES[cur], 2) or 0.01
        listing = {"item": item, "listing": {"price": {"amount": amount, "currency": cur}}}
        if i % 29 == 7:
            listing["listing"]["price"]["currency"] = "mirror"  # no rate: skipped
        if i % 31 == 3:
            item["baseType"] = "Not A Real Base"  # no match: skipped
        out.append(listing)
    return out


def model_doc(model) -> dict | None:
    if model is None:
        return None
    return {
        "group": model.group, "listings": model.listings, "base_price": model.base_price, "per_res": model.per_res, "per_open_affix": model.per_open_affix,
        "r2": model.r2,
        "tiers": [{"key": t.key, "text": t.text, "t1": t.t1, "t2": t.t2, "n_t1": t.n_t1, "n_t2": t.n_t2} for t in model.tiers],
    }  # fmt: skip


def pricing_goldens(db: ModDB, found: list, matches: dict) -> dict:
    queries, fits = [], []
    rng = random.Random(11)
    for i, a in enumerate(found):
        m = matches.get(i)
        if m is None:
            continue
        q = pricing.archetype_query(db, a.examples[0], a.families)
        ex = a.examples[0]
        example = {"base_id": ex.base_id, "item_class": ex.item_class, "base": ex.item.base, "influences": sorted(ex.item.influences), "tags": sorted(ex.tags)}
        queries.append({"row": i, "group": a.group, "families": list(a.families), "example": example, "query": q})
        if len(fits) < 12 and any(k != RES for k in a.families):
            listings = synth_listings(db, a, m, rng)
            model = pricing.fit(db, a.group, a.families, listings, RATES)
            matched = []
            for lst in listings:
                mm = pricing.listing_match(db, lst)
                matched.append(None if mm is None else [[x.mod_id, x.kind, x.source, x.tier, x.family] for x in mm.mods])
            fits.append({"row": i, "group": a.group, "families": list(a.families), "listings": listings, "matched": matched, "model": model_doc(model)})
    # too few listings: no model
    few = fits[0]["listings"][:5] if fits else []
    first = found[fits[0]["row"]] if fits else None
    short = pricing.fit(db, first.group, first.families, few, RATES) if first else None
    return {"rates": RATES, "queries": queries, "fits": fits, "too_few": {"n": len(few), "model": model_doc(short)}}


# -- main --------------------------------------------------------------------------------------


def main() -> None:
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "rust/goldens/recomb")
    out.mkdir(parents=True, exist_ok=True)
    assert craft.CHAOS_PER_HOUR == 1500.0, "goldens assume the default 1500 chaos per hour"
    parts = set(sys.argv[2:]) or {"kernel", "planner", "pricing"}
    if "kernel" in parts:
        save(out, "kernel", kernel(), gz=False)
    db = ModDB()
    table, _ = prices.load()
    chars = list(ladder.cached())
    worn = demand.wear(db, chars)
    found = demand.archetypes(worn, 5)[:120]
    gear_golden = REPO / "rust/goldens/problems/gear.json.gz"
    rows = json.load(gzip.open(gear_golden, "rt")) if gear_golden.exists() else None
    planners, matches = [], {}
    for i, a in enumerate(found):
        m = demand.archetype_match(db, a)
        if m is None:
            continue
        matches[i] = m
        if "planner" not in parts:
            continue
        e = pipeline.estimate_gear(db, m, table, solve=False)
        if e.error or not (bid := db.find_base(e.base)):
            continue
        e.problem.filler = SortedFrozenset(e.problem.filler)
        if rows is not None:
            assert rows[i]["group"] == a.group and problem_json(e.problem) == rows[i]["build"]["problem"], f"row {i} differs from problems/gear.json.gz"
        planner = recomb.Planner(db, e.problem, bid, e.item_level)
        planners.append((i, e, bid, planner))
        print(f"{i}: {e.title} ok={planner.ok} leaves={len(planner.leaves())}", flush=True)
    stand, solved = [], []
    if "pricing" in parts:
        save(out, "pricing", pricing_goldens(db, found, matches))
    for i, e, bid, planner in planners:
        head = {"row": i, "title": e.title, "base": e.base, "base_id": bid, "item_level": e.item_level, "ok": planner.ok}
        if not planner.ok:
            stand.append({**head, "wants": [want_doc(w) for w in planner.want]})
            continue
        leaves = [leaf_doc(d) for d in planner.leaves()]
        keys = [sorted(k) for k in planner._leaves]
        costs = stand_in(planner)
        stand.append({**head, "wants": [want_doc(w) for w in planner.want], "leaves": leaves, "leaf_sets": [[k, [planner.leaves().index(d) for d in v]] for k, v in
                      zip(keys, planner._leaves.values())], "costs": costs, "best": best_doc(planner)})  # fmt: skip
    if "planner" in parts:
        save(out, "planner", stand)
    # Real solves, for the first planners.
    for i, e, bid, planner in [p for p in planners if p[3].ok][:SOLVED_PLANNERS]:
        for d in planner.leaves():
            d.plan = None
        pipeline.solve_all(planner.leaves(), table)
        solved.append({
            "row": i, "title": e.title,
            "costs": [d.cost for d in planner.leaves()], "errors": [d.error for d in planner.leaves()], "best": best_doc(planner),
        })  # fmt: skip
        print(f"solved {i}: {e.title}", flush=True)
    if "planner" in parts:
        save(out, "solved", solved)


if __name__ == "__main__":
    main()
