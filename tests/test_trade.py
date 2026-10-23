import json

from ponder import render, trade


class Headers(dict):
    def get(self, k, default=None):
        return super().get(k, default)


def test_rate_limit_waits_until_a_call_fits(monkeypatch):
    slept = []
    monkeypatch.setattr(trade.time, "sleep", slept.append)
    monkeypatch.setattr(trade.time, "time", lambda: 1000.0)
    trade._limits.clear()
    headers = Headers({"X-Rate-Limit-Rules": "Ip", "X-Rate-Limit-Ip": "8:10:60,15:60:300", "X-Rate-Limit-Ip-State": "7:10:0,3:60:0"})
    trade._remember_limits("search", headers)
    trade._wait_for("search")  # 7 of 8 used in the 10 s window: wait it out
    assert slept == [10]
    trade._remember_limits("search", Headers({**headers, "X-Rate-Limit-Ip-State": "1:10:0,1:60:0"}))
    trade._wait_for("search")
    assert slept == [10]  # plenty left: no wait
    trade._remember_limits("search", Headers({**headers, "X-Rate-Limit-Ip-State": "8:10:45,8:60:0"}))
    trade._wait_for("search")
    assert slept == [10, 45]  # restricted: wait the penalty


def test_prices_are_cached_in_their_own_currency(monkeypatch, tmp_path):
    monkeypatch.setattr(trade, "CACHE", tmp_path)
    calls = []

    def fake_call(url, body, endpoint, retries=2):
        calls.append(endpoint)
        if endpoint == "search":
            return {"id": "q", "result": ["a", "b", "c"], "total": 3}
        listings = [("divine", 1), ("divine", 2), ("chaos", 100)]
        return {"result": [{"listing": {"price": {"currency": c, "amount": n}}} for c, n in listings]}

    monkeypatch.setattr(trade, "_call", fake_call)
    query = trade.item_query(base="Warlock Gloves", stats=[("explicit.stat_1", 40)])
    assert trade.price("L", query, {"divine": 200, "chaos": 1}) == (200, 3)
    assert trade.price("L", query, {"divine": 300, "chaos": 1}) == (300, 3)  # cached asks, new divine rate
    assert calls == ["search", "fetch"]


def test_cookie_from_dotenv(monkeypatch, tmp_path):
    monkeypatch.delenv("POESESSID", raising=False)
    monkeypatch.chdir(tmp_path)
    assert trade.cookie() is None
    (tmp_path / ".env").write_text('# secrets\nexport POESESSID="abc123"\n')
    assert trade.cookie() == "abc123"
    monkeypatch.setenv("POESESSID", "env-wins")
    assert trade.cookie() == "env-wins"


def test_item_query_shape():
    q = trade.item_query(category="accessory.ring", stats=[("fractured.stat_9", 5)], item_level=84, fractured_only=True)["query"]
    assert q["filters"]["type_filters"]["filters"]["category"] == {"option": "accessory.ring"}
    assert q["filters"]["misc_filters"]["filters"] == {"ilvl": {"min": 84}, "fractured_item": {"option": "true"}}
    assert q["stats"][0]["filters"][0]["value"] == {"min": 5}
    json.dumps(q)


def test_money_switches_to_divines(monkeypatch):
    monkeypatch.setattr(render, "DIVINE", 200.0)
    assert render.money(150) == "150c"
    assert render.money(300) == "1.5d"
    assert render.money(-500) == "-2.5d"
    assert render.money(5) == "5.0c"


def test_listing_item_marks_fractured_and_crafted_lines():
    listing = {"item": {"rarity": "Rare", "name": "Gale Loop", "baseType": "Bone Ring", "ilvl": 84, "influences": {"shaper": True},
                        "fracturedMods": ["+40 to maximum Energy Shield"], "explicitMods": ["+105 to maximum Life"], "craftedMods": ["+12 to Dexterity"]}}
    item = trade.listing_item(listing)
    assert (item.rarity, item.base, item.item_level, item.influences) == ("RARE", "Bone Ring", 84, {"shaper"})
    assert [(m.text, m.fractured, m.crafted) for m in item.explicits] == [
        ("+40 to maximum Energy Shield", True, False), ("+105 to maximum Life", False, False), ("+12 to Dexterity", False, True)]
