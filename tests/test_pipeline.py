import json

import pytest

from ponder import cli, data, pipeline
from ponder.craft import Plan

needs_data = pytest.mark.skipif(
    not (data.available() and (data.data_dir() / "cluster_jewels.json").is_file()),
    reason="run `ponder data fetch` first",
)


def estimate(title, cost, market=None):
    e = pipeline.Estimate("cluster", title, ["x"], plan=Plan(cost, {}, {}, {}, 0))
    e.market = (market, 5) if market is not None else None
    return e


def test_ranked_puts_margins_first_then_dearest():
    report = pipeline.Report([], [estimate("cheap", 5), estimate("profit", 50, market=500), estimate("dear", 900)], [], "", "")
    assert [e.title for e in report.ranked()] == ["profit", "dear", "cheap"]
    assert report.ranked()[0].margin == 450


def test_bare_sources_run_the_pipeline(monkeypatch, capsys):
    seen = {}

    def fake_run(sources, **kw):
        seen.update(sources=sources, **kw)
        return pipeline.Report([], [estimate("a", 10)], [], "snapshot", "Standard")

    monkeypatch.setattr(pipeline, "run", fake_run)
    assert cli.main(["builds.txt", "--json", "--trade"]) == 0
    assert seen["sources"] == ["builds.txt"] and seen["trade_lookup"] is True
    out = json.loads(capsys.readouterr().out)
    assert out["estimates"][0]["total"] == 10


@needs_data
def test_cluster_estimate_has_cost_link_and_json(monkeypatch):
    est = pipeline.estimate_cluster("medium", "area damage", ["Vast Power", "Assert Dominance"], {"alt": 0.074, "aug": 0.017, "regal": 0.093, "scour": 0.35, "transmute": 0.004, "chaos": 1.0}, "Allflame")
    assert est.error is None and 10 < est.craft_cost < 500
    assert "Medium%20Cluster%20Jewel" in est.trade_url and "available" in est.trade_url
    json.dumps(est.to_dict())


@needs_data
def test_fractures_and_best_base():
    from ponder import pob
    from ponder.item import parse_item
    from ponder.mods import ModDB

    db = ModDB()
    build = pob.load("tests/fixtures/real/occultist_winter_orb.txt")
    gloves = next(parse_item(i.text) for i in build.items if parse_item(i.text).base == "Warlock Gloves" and parse_item(i.text).rarity == "RARE")
    table = {"alt": 0.074, "aug": 0.017, "regal": 0.093, "scour": 0.35, "transmute": 0.004, "chaos": 1.0, "exalted": 5, "annul": 11, "alch": 0.03}
    est = pipeline.estimate_gear(db, db.match(gloves), table, "Allflame", solve=False)
    assert est.title == "Gloves (ES)" and est.base == "Warlock Gloves" and est.bases > 5
    pipeline.add_fractures(est, "Allflame")
    pipeline.solve_all([est, *est.fractures], table, workers=1)
    assert len(est.fractures) == len(est.targets)
    # Starting with one target already on the item can only make the rest cheaper.
    assert all(f.craft_cost <= est.craft_cost + 1e-6 for f in est.fractures)
    assert all("fractured.stat_" in f.trade_url and "fractured_item" in f.trade_url for f in est.fractures)


@needs_data
def test_finish_a_real_item_and_find_half_done_ones(monkeypatch, tmp_path):
    """A ring with the wanted life already on finishes for less than a fresh one; a listed one is
    priced as its price plus finishing it from its own mods."""
    from ponder import prices, trade
    from ponder.item import parse_item
    from ponder.mods import ModDB

    monkeypatch.setattr(pipeline, "PLAN_CACHE", tmp_path)
    db = ModDB()
    table, _ = prices.load()
    product = db.match_ids("Bone Ring", {"explicit": [{"id": "IncreasedLife7"}, {"id": "IncreasedEnergyShield10"}]}, item_level=84)
    est = pipeline.estimate_gear(db, product, table, exact=True)
    assert est.plan, est.error
    have = parse_item("Rarity: RARE\nGale Loop\nBone Ring\nItem Level: 84\nImplicits: 0\n+105 to maximum Life\n+30% to Fire Resistance")
    f = pipeline.finish(db, est, db.match(have), table)
    assert f.error is None and f.have == [t for t in est.targets if "Life" in t]
    assert f.craft_cost <= est.craft_cost + table["scour"] + 1e-6  # never worse than scouring it
    assert "Life" in " ".join(f.steps()[0].have)  # the recipe starts from the item
    both = parse_item("Rarity: RARE\nX\nBone Ring\nItem Level: 84\nImplicits: 0\n+105 to maximum Life\n+40 to maximum Energy Shield")
    assert pipeline.finish(db, est, db.match(both), table).craft_cost == pytest.approx(est.bench_cost)  # already done
    corrupted = parse_item("Rarity: RARE\nX\nBone Ring\nImplicits: 0\n+105 to maximum Life\nCorrupted")
    assert pipeline.finish(db, est, db.match(corrupted), table).error
    listing = {"item": {"rarity": "Rare", "baseType": "Bone Ring", "ilvl": 84, "explicitMods": ["+105 to maximum Life"]},
               "listing": {"price": {"amount": 10, "currency": "chaos"}, "account": {"name": "seller"}}}
    monkeypatch.setattr(trade, "listings", lambda league, query, count: [listing])
    starts = pipeline.find_starts(db, est, table, "Standard")
    assert starts and starts[0].price == 10 and starts[0].total == pytest.approx(10 + starts[0].craft_cost)
    assert est.starts == starts and est.starts_url
