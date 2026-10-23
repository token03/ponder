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
