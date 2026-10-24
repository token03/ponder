"""Export pipeline goldens for the Rust port (rust/crates/ponder/tests/pipeline.rs).

    uv run python scripts/golden/pipeline.py [OUT_DIR]   # default rust/goldens/pipeline

Run from the repository root. It uses the cached ladder, the saved prices, the repoe data and
Python's plan cache (`.cache/plans`); nothing touches the network: `current_league` is fixed and
`trade.price` is replaced by a pure function of the query, as tests/test_pipeline.py replaces
`trade.listings`.

- tree.json.gz: a mid-sized `ladder()` run (the top gear archetypes and a few clusters), recorded
  in the order the pipeline does it. The estimates are written as they are *before* the product
  tree (problems, fractures, base prices, trade queries), then the tree (`covers` for every pair
  of one base group, `smaller`/`bigger`), the sale actions each wave adds, and the plan costs.
  Compare the problems and links exactly, the sale actions exactly (their credits to 3 figures),
  and the costs to the solver's tolerance. `ladder` holds the run's `summary`, `to_dict` a few
  estimates' JSON, and `prices` the table and clicking rate everything was solved at.
- finish.json.gz: `finish` and `finishes` for the rare items of tests/fixtures/real against the
  estimates of those same builds.
- ladder_full.json.gz: the summary of a default `ladder()` run (counts and the cheapest cost per
  archetype). Python's own runs drift a little from run to run (sale credits depend on the order
  plans finish in, e.g. 1111 vs 1113 fractures), so compare it with tolerances.
- trade.json.gz: `price_on_trade` on the mid run's estimates with a fake `trade.price`: which
  searches were made in what order, and what each estimate and fracture ended up with.
"""

from __future__ import annotations

import dataclasses
import gzip
import hashlib
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from solver import problem  # noqa: E402

from ponder import craft, demand, ladder as lad, pipeline, pob, prices, trade  # noqa: E402
from ponder.item import parse_item  # noqa: E402
from ponder.mods import ModDB  # noqa: E402

LEAGUE = "Standard"
MID = dict(gear_top=60, per_group=0, niche=0, cluster_top=8, min_characters=5, workers=1)
TRADE_BUDGET = 40
FIXTURES = Path("tests/fixtures/real")

pipeline.current_league = lambda: LEAGUE


def save(out: Path, name: str, doc) -> None:
    with gzip.open(out / f"{name}.json.gz", "wt") as f:
        json.dump(doc, f)
    print(f"{name}: written")


def sale(a) -> dict:
    need, (p, q, n), spare = a.sale
    return {"name": a.name, "credit": a.credit, "sale": {"need": sorted(need), "prefix": p, "suffix": q, "any": n, "spare": sorted(spare)}}


def sells(problem_) -> list:
    return [sale(a) for a in problem_.actions if a.kind == "sell"]


def cost_of(x):
    return {"cost": x.plan.cost, "hours": x.plan.hours} if x.plan else {"cost": None, "error": x.error}


def summary(report) -> dict:
    """What an end-to-end run is compared on: counts, and per estimate the cheapest way to make it."""
    ests = report.estimates
    return {
        "characters": report.characters, "items": report.items, "clusters": report.clusters,
        "archetypes": report.archetypes, "cluster_archetypes": report.cluster_archetypes,
        "estimates": len(ests), "gear": sum(e.kind == "gear" for e in ests), "cluster": sum(e.kind == "cluster" for e in ests),
        "fractures": sum(len(e.fractures) for e in ests), "planned": sum(e.plan is not None for e in ests),
        "planned_fractures": sum(f.plan is not None for e in ests for f in e.fractures),
        "linked": sum(bool(e.smaller) for e in ests), "sale_actions": sum(len(sells(e.problem)) for e in ests if e.problem),
        "per_estimate": [
            {"kind": e.kind, "title": e.title, "targets": e.targets, "demand": e.demand, "cheapest": e.cheapest, "total": e.total,
             "craft_cost": e.craft_cost, "fractures": len(e.fractures), "smaller": len(e.smaller), "bigger": len(e.bigger),
             "sales": len(sells(e.problem)) if e.problem else 0, "error": e.error}
            for e in ests
        ],
    }  # fmt: skip


def mid_run(chars, table):
    """`ladder()` with hooks that record what the pipeline did at each step."""
    log = {}
    real_link, real_sales = pipeline.link_products, pipeline.add_sales

    def link(estimates):
        log["before"] = [
            {"title": e.title, "kind": e.kind, "targets": e.targets, "base_price": e.base_price, "problem": problem(e.problem),
             "query": e.query, "base_query": e.base_query, "item_level": e.item_level, "demand": e.demand,
             "base": e.base, "used_base": e.used_base, "bases": e.bases, "bench": e.bench, "skipped": e.skipped, "fixed": e.fixed,
             "fixed_ids": e.fixed_ids, "filler_families": e.filler_families, "base_source": e.base_source, "skills": e.skills,
             "rows": [[r.role, r.side, r.text, r.tier] for r in e.rows], "trade_url": e.trade_url, "base_url": e.base_url,
             "search": e.search,
             "fractures": [{"target": f.target, "mod_id": f.mod_id, "problem": problem(f.problem), "query": f.query, "trade_url": f.trade_url,
                            "made": dataclasses.asdict(f.made) if f.made else None} for f in e.fractures]}
            for e in estimates
        ]  # fmt: skip
        log["order"] = list(estimates)
        real_link(estimates)

    def add_sales(e):
        real_sales(e)
        log.setdefault("sales", {})[id(e)] = {"estimate": sells(e.problem), "fractures": [sells(f.problem) for f in e.fractures]}

    pipeline.link_products, pipeline.add_sales = link, add_sales
    try:
        started = time.time()
        report = pipeline.ladder(chars, **MID)
        log["seconds"] = time.time() - started
    finally:
        pipeline.link_products, pipeline.add_sales = real_link, real_sales
    return report, log


def export_tree(out: Path, chars, table, db) -> tuple:
    report, log = mid_run(chars, table)
    order = log["order"]
    index = {id(e): i for i, e in enumerate(order)}
    pairs = []
    for i, big in enumerate(order):
        for j, small in enumerate(order):
            if i != j and big.title == small.title and big.problem and small.problem and not big.error and not small.error:
                c = pipeline.covers(big.problem, small.problem)
                pairs.append([i, j, None if c is None else {"need": sorted(c[0]), "prefix": c[1][0], "suffix": c[1][1], "any": c[1][2], "spare": sorted(c[2])}])
    sizes = sorted({len(e.problem.targets) for e in order if e.problem})
    doc = {
        "prices": {"table": table, "chaos_per_hour": craft.CHAOS_PER_HOUR, "league": LEAGUE},
        "options": MID,
        "before": log["before"],
        "covers": pairs,
        "smaller": [[index[id(s)] for s in e.smaller] for e in order],
        "bigger": [[index[id(s)] for s in e.bigger] for e in order],
        "sizes": sizes,
        "sales": [log["sales"].get(id(e)) for e in order],
        "after": [{"estimate": cost_of(e), "fractures": [cost_of(f) for f in e.fractures], "cheapest": e.cheapest} for e in order],
        "summary": summary(report),
        "seconds": log["seconds"],
    }  # fmt: skip
    # a few estimates as the CLI shows them: one gear craft, one cluster, the one with the most sales
    pick = [next(i for i, e in enumerate(order) if e.kind == "gear" and e.plan), next(i for i, e in enumerate(order) if e.kind == "cluster" and e.plan)]
    busiest = max(range(len(order)), key=lambda i: len(sells(order[i].problem)) if order[i].problem else 0)
    if busiest not in pick:
        pick.append(busiest)
    doc["to_dict"] = {str(i): order[i].to_dict() for i in pick}
    save(out, "tree", doc)
    print(f"  {len(order)} estimates, {sum(bool(e.smaller) for e in order)} with smaller products, {sum(len(v['estimate']) for v in log['sales'].values())} sale actions")
    return report, order


def export_trade(out: Path, report, table) -> None:
    calls = []

    answers = {}  # like the disk cache: a query asked twice is searched once

    def fake_price(league, query, rates, listings=10, max_age=3600):
        h = hashlib.sha1(json.dumps(query).encode()).hexdigest()
        if h not in answers:
            calls.append(h)
            trade.searches += 1
            n = int(h[:8], 16)
            answers[h] = None if n % 7 == 0 else (float(5 + n % 1000) / 10, 3 + n % 40)
        return answers[h]

    real = trade.price
    trade.price = fake_price
    try:
        ests = report.estimates
        used = pipeline.price_on_trade(sorted(ests, key=lambda e: -e.demand), table, LEAGUE, budget=TRADE_BUDGET)
    finally:
        trade.price = real
    # price_on_trade sorts a copy: the order of `ests` is the ladder's
    doc = {
        "budget": TRADE_BUDGET, "used": used, "calls": calls,
        "estimates": [
            {"market": e.market, "market_error": e.market_error, "base_price": e.base_price, "base_error": e.base_error, "base_source": e.base_source,
             "fractures": [{"base_price": f.base_price, "base_error": f.base_error} for f in e.fractures]}
            for e in ests
        ],
    }  # fmt: skip
    save(out, "trade", doc)
    print(f"  {used} searches, {sum(e.market is not None for e in ests)} markets priced")


def item_json(db, est, m, table, solve):
    f = pipeline.finish(db, est, m, table, solve=solve)
    return {
        "error": f.error, "have": f.have, "craft_cost": f.craft_cost, "saves": f.saves, "worth": f.worth,
        "state": craft.describe(f.problem, f.state) if f.problem and f.state else None, "plan": f.plan is not None,
    }  # fmt: skip


def export_finish(out: Path, db, table) -> None:
    items, ests, by_key = [], [], {}
    for path in sorted(FIXTURES.glob("*.txt")):
        build = pob.load(str(path))
        for pob_item in build.items:
            item = parse_item(pob_item.text)
            if item.rarity != "RARE":
                continue
            m = db.match(item)
            items.append({"build": path.name, "text": pob_item.text, "base": item.base, "match": m})
            est = pipeline.estimate_item(db, m, table, LEAGUE, solve=True)
            if est and est.targets and est.key not in by_key:
                by_key[est.key] = est
                ests.append(est)
    gear_ests = [e for e in ests if e.kind == "gear" and e.plan]
    doc = {
        "builds": sorted({i["build"] for i in items}),
        "estimates": [{"title": e.title, "targets": e.targets, "base": e.base, "craft_cost": e.craft_cost, "total": e.total, "error": e.error} for e in ests],
        "items": [],
    }  # fmt: skip
    for it in items:
        m = it["match"]
        row = {"build": it["build"], "base": it["base"], "finish": {}, "finishes": []}
        for j, e in enumerate(ests):
            if e.kind != "gear" or not e.plan:
                continue
            f = item_json(db, e, m, table, False)
            if f["error"] is None or f["craft_cost"] is not None:
                row["finish"][str(j)] = f
        for f in pipeline.finishes(db, m, ests, table, top=3):
            row["finishes"].append({"product": f.estimate.title, "targets": f.estimate.targets, "craft_cost": f.craft_cost, "saves": f.saves, "have": f.have,
                                    "plan_cost": f.plan.cost if f.plan else None, "total": f.total})  # fmt: skip
        doc["items"].append(row)
    doc["prices"] = {"table": table, "chaos_per_hour": craft.CHAOS_PER_HOUR}
    save(out, "finish", doc)
    done = sum(len(r["finish"]) for r in doc["items"])
    print(f"  {len(items)} items, {len(gear_ests)} gear estimates, {done} finishes")


def export_full(out: Path, chars) -> None:
    started = time.time()
    report = pipeline.ladder(chars)
    seconds = time.time() - started
    doc = summary(report)
    doc["seconds"] = seconds
    doc["chaos_per_hour"] = craft.CHAOS_PER_HOUR
    save(out, "ladder_full", doc)
    print(f"  {len(report.estimates)} estimates in {seconds:.0f}s")


def main() -> None:
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "rust/goldens/pipeline")
    out.mkdir(parents=True, exist_ok=True)
    db = ModDB()
    table, _ = prices.load()
    chars = list(lad.cached())
    report, _order = export_tree(out, chars, table, db)
    export_trade(out, report, table)
    export_finish(out, db, table)
    export_full(out, chars)


if __name__ == "__main__":
    main()
