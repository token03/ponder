import struct
from collections import Counter

import pytest

from ponder import data, demand, ladder


def ndic(words):
    raw = [w.encode() for w in words]
    lengths = b"".join(_varint(len(r)) for r in raw)
    header = b"NDIC" + struct.pack("<III", 2, 0, len(words)) + b"\0" * 8 + struct.pack("<III", 16, 1, len(lengths))
    return header + struct.pack("<II", 0, 0) + lengths + b"".join(raw)


def _varint(n):
    out = b""
    while True:
        b, n = n & 0x7F, n >> 7
        out += bytes([b | (0x80 if n else 0)])
        if not n:
            return out


def test_ndic_round_trip_with_long_strings():
    words = ["Ascendant", "", "x" * 300, "Ünïcode"]
    assert ladder._ndic(ndic(words)) == words


def test_protobuf_fields():
    msg = bytes([0x08, 0x96, 0x01, 0x12, 0x03]) + b"abc"  # field 1 = 150, field 2 = "abc"
    assert ladder._fields(msg) == [(1, 0, 150), (2, 2, b"abc")]


def test_lift_compares_ladder_tiers_with_roll_odds():
    s = demand.FamilyStat("Gloves (ES)", "EnergyShield", "prefix", "+# to maximum Energy Shield", 50, Counter({1: 6, 2: 3, 5: 1}))
    s.natural = {1: 0.1, 2: 0.2, 3: 0.3, 4: 0.2, 5: 0.2}
    assert s.share_at_or_above(1) == pytest.approx(0.6)
    assert s.lift(1) == pytest.approx(6.0)
    assert s.lift(2) == pytest.approx(0.9 / 0.3)


@pytest.mark.skipif(not data.available(), reason="run `ponder data fetch` first")
def test_match_by_mod_ids():
    from ponder.mods import ModDB

    db = ModDB()
    m = db.match_ids("Heavy Belt", {"explicit": [{"id": "IncreasedLife6"}, {"id": "ChaosResist6"}, {"id": "Nope"}], "crafted": [{"id": "JunMaster2AllDamage3"}]})
    assert m.item_class == "Belt"
    got = {(x.mod_id, x.kind, x.source) for x in m.mods}
    assert got == {("IncreasedLife6", "prefix", "natural"), ("ChaosResist6", "suffix", "natural"), ("JunMaster2AllDamage3", "prefix", "crafted")}
    assert demand.core_families(m) == ("IncreasedLife", demand.RES)


@pytest.mark.skipif(not data.available(), reason="run `ponder data fetch` first")
def test_price_model_recovers_a_t1_premium():
    import random

    from ponder import pricing
    from ponder.mods import ModDB

    db = ModDB()
    from ponder import gear

    # Life tiers as they roll on Vaal Gauntlets, best first.
    base = db.find_base("Vaal Gauntlets")
    tags = db.item_tags(base, set())
    life_ids = [m.id for m in gear.pool(db, base, tags, 86) if db.mods[m.id]["type"] == "IncreasedLife"]
    lifes = [db.mods[i] for i in sorted(life_ids, key=lambda i: db.tier(i, tags))]
    rng = random.Random(1)
    listings = []
    for i in range(60):
        tier = rng.choice([0, 0, 1, 2, 3])
        mod = lifes[tier]
        low = mod["stats"][0]["min"]
        price = 100 * (3.0 if tier == 0 else 1.5 if tier == 1 else 1.0) * rng.uniform(0.9, 1.1)
        listings.append({
            "item": {"baseType": "Vaal Gauntlets", "ilvl": 86, "explicitMods": [f"+{low} to maximum Life", "+40% to Fire Resistance"]},
            "listing": {"price": {"amount": price, "currency": "chaos"}},
        })  # fmt: skip
    model = pricing.fit(db, "Gloves (Armour)", ("base_maximum_life", pricing.RES), listings, {"chaos": 1.0})
    life = model.tiers[0]
    assert life.t1 == pytest.approx(3.0, rel=0.15) and life.t2 == pytest.approx(1.5, rel=0.15)
