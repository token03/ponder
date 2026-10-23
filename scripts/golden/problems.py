"""Export problem-building and ladder-demand goldens for the Rust port.

    uv run python scripts/golden/problems.py [OUT_DIR]   # default rust/goldens/problems

- archetypes.json.gz: demand.archetypes and cluster_archetypes on the cached ladder (Counters as dicts).
- gear.json.gz: for the top archetypes, archetype_match (mods by id, finish spec) and the
  gear.build result (problem as in solver.py, rows, bench, skipped, filler families, item level),
  plus the estimate's key fields (title, base, bases, trade query).
- clusters.json.gz: estimate_cluster problems for the top cluster archetypes.
- fractures.json.gz: add_fractures on a few gear estimates (target, mod id, fixed problem, self-made cost).
"""

from __future__ import annotations

import gzip
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from solver import problem  # noqa: E402

from ponder import demand, gear, ladder, pipeline, prices  # noqa: E402
from ponder.mods import ModDB  # noqa: E402


def plain(x):
    if isinstance(x, Counter) or isinstance(x, dict):
        return {str(k) if not isinstance(k, tuple) else "|".join(map(str, k)): plain(v) for k, v in x.items()}
    if isinstance(x, (list, tuple, set, frozenset)):
        items = [plain(v) for v in x]
        return sorted(items, key=json.dumps) if isinstance(x, (set, frozenset)) else items
    return x


def match(m):
    return {
        "base": m.item.base, "base_id": m.base_id, "item_class": m.item_class, "item_level": m.item.item_level,
        "influences": sorted(m.item.influences), "tags": sorted(m.tags),
        "mods": [{"mod_id": x.mod_id, "kind": x.kind, "family": x.family, "source": x.source, "tier": x.tier, "fractured": x.fractured} for x in m.mods],
        "finish": plain(m.finish),
    }  # fmt: skip


def save(out: Path, name: str, doc) -> None:
    with gzip.open(out / f"{name}.json.gz", "wt") as f:
        json.dump(doc, f)
    print(f"{name}: {len(doc)} entries")


def main() -> None:
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "rust/goldens/problems")
    out.mkdir(parents=True, exist_ok=True)
    db = ModDB()
    table, _ = prices.load()
    chars = list(ladder.cached())
    worn = demand.wear(db, chars)
    found = demand.archetypes(worn, 5)
    found_cl = demand.cluster_archetypes(demand.wear_clusters(chars), 5)
    save(out, "archetypes", {
        "characters": len(chars), "worn": len(worn),
        "gear": [{"group": a.group, "families": list(a.families), "characters": a.characters, "items": a.items, "lift": a.lift, "niche": a.niche,
                  "skills": plain(a.skills), "bases": plain(a.bases), "tiers": plain(a.tiers), "resistances": plain(a.resistances),
                  "others": plain(a.others), "fillers": plain(a.fillers), "filler_tiers": plain(a.filler_tiers), "filled": plain(a.filled)} for a in found],
        "clusters": [{"size": a.size, "enchant": a.enchant, "tag": a.tag, "notables": list(a.notables), "characters": a.characters, "items": a.items,
                      "passives": plain(a.passives), "item_levels": plain(a.item_levels), "full": a.full, "tiers": plain(a.tiers)} for a in found_cl],
    })  # fmt: skip
    rows, ests = [], []
    for a in found[:120]:
        m = demand.archetype_match(db, a)
        if m is None:
            rows.append({"group": a.group, "families": list(a.families), "match": None})
            continue
        gp = gear.build(db, m)
        e = pipeline.estimate_gear(db, m, table, solve=False)
        ests.append(e)
        rows.append({
            "group": a.group, "families": list(a.families), "match": match(m),
            "build": {"problem": problem(gp.problem), "item_level": gp.item_level, "skipped": gp.skipped, "bench": [[l, [list(c) for c in cost]] for l, cost in gp.bench],
                      "rows": [[r.role, r.side, r.text, r.tier] for r in gp.rows], "bench_ids": gp.bench_ids, "filler_families": gp.filler_families},
            "estimate": {"title": e.title, "base": e.base, "bases": e.bases, "targets": e.targets, "error": e.error, "query": e.query, "search": e.search},
        })  # fmt: skip
    save(out, "gear", rows)
    cl = []
    for a in found_cl[:60]:
        c = pipeline.estimate_cluster(a.size, a.enchant, list(a.notables), table, "Standard", passives=a.usual_passives, solve=False)
        cl.append({"size": a.size, "enchant": a.enchant, "notables": list(a.notables), "passives": a.usual_passives, "title": c.title, "targets": c.targets,
                   "error": c.error, "problem": problem(c.problem) if c.problem else None, "query": c.query, "search": c.search})  # fmt: skip
    save(out, "clusters", cl)
    fr = []
    for e in [e for e in ests if not e.error][:15]:
        pipeline.add_fractures(e, "Standard", table, filler=False)
        fr.append({"title": e.title, "targets": e.targets, "fractures": [
            {"target": f.target, "mod_id": f.mod_id, "problem": problem(f.problem), "query": f.query, "made": None if f.made is None else
             {"action": f.made.action, "max_mods": f.made.max_mods, "hit": f.made.hit, "fracture_chance": f.made.fracture_chance, "cost": f.made.cost}}
            for f in e.fractures]})  # fmt: skip
    save(out, "fractures", fr)
    save(out, "prices", table)


if __name__ == "__main__":
    main()
