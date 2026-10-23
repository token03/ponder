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


def _notable_id(name):
    from ponder import cluster

    return next(i for i, m in data.load("mods").items() if cluster.is_notable(m) and cluster.notable_name(m) == name and m["domain"] == "affliction_jewel")


def _cluster_char(who, notables, fractured=()):
    enchant = "Added Small Passive Skills grant: Sword Attacks deal 12% increased Damage with Hits and Ailments\nAdded Small Passive Skills grant: Axe Attacks deal 12% increased Damage with Hits and Ailments"
    mods = {
        "enchant": [{"id": "JewelExpansionPassiveNodes", "stats": {"local_jewel_expansion_passive_node_count": 8}}],
        "explicit": [{"id": _notable_id(n)} for n in notables if n not in fractured],
        "fractured": [{"id": _notable_id(n)} for n in fractured],
    }
    item = {"baseType": "Large Cluster Jewel", "ilvl": 84, "mods": mods, "enchantMods": ["Adds 8 Passive Skills", enchant]}
    return {"account": who, "name": who, "skill": "Cyclone", "rares": [item]}


@pytest.mark.skipif(not data.available(), reason="run `ponder data fetch` first")
def test_cluster_demand_reads_enchant_in_any_line_order_and_merges_subsets():
    chars = [_cluster_char(f"a{i}", ["Feed the Fury", "Martial Prowess", "Fuel the Fight"]) for i in range(4)]
    chars += [_cluster_char(f"b{i}", ["Feed the Fury", "Martial Prowess", "Bloodscent"], fractured=["Bloodscent"]) for i in range(3)]
    worn = demand.wear_clusters(chars)
    assert len(worn) == 7 and worn[0].tag == "affliction_axe_and_sword_damage" and worn[0].passives == 8
    assert worn[-1].fractured == ("Bloodscent",)
    found = {a.notables: a for a in demand.cluster_archetypes(worn, min_characters=3)}
    # The shared pair is kept (7 wear it, no single triple covers 80%); both triples are kept.
    assert found[("Feed the Fury", "Martial Prowess")].characters == 7
    assert ("Feed the Fury", "Fuel the Fight", "Martial Prowess") in found
    assert all(len(k) >= 2 for k in found)  # large clusters: combos of 2+ notables


def test_archetypes_drop_subsets_a_bigger_combination_covers():
    from types import SimpleNamespace

    def worn(who, keys):
        m = SimpleNamespace(mods=[], item=SimpleNamespace(base="Vaal Gauntlets"))
        return demand.Worn(who, "Cyclone", m, "Gloves"), keys

    items = [worn(f"a{i}", ("life", "ms", demand.RES)) for i in range(9)] + [worn(f"b{i}", ("es", "life")) for i in range(5)]
    real = demand.stat_keys
    demand.stat_keys = lambda m: next(k for w, k in items if w.match is m)
    try:
        found = {a.families: a.characters for a in demand.archetypes([w for w, _ in items], min_characters=3)}
    finally:
        demand.stat_keys = real
    assert found[("life", "ms", demand.RES)] == 9
    assert ("life", "ms") not in found and ("ms", demand.RES) not in found  # all 9 wear the triple
    assert found[("es", "life")] == 5


def test_plan_key_is_stable_and_price_sensitive():
    from ponder import craft, pipeline

    def problem():
        mods = [craft.Mod("A", "prefix", 100, ("ga",), False, "A"), craft.Mod("B", "suffix", 100, ("gb",), False, "B")]
        return craft.Problem(mods, [craft.Target("A", "prefix", frozenset({"A"}), frozenset({"ga", "x", "y"}))], 1, [])

    from ponder import actions

    p1, p2 = problem(), problem()
    p1.actions = p2.actions = list(actions.BASIC)
    table = {"alt": 0.1, "aug": 0.05, "transmute": 0.02, "regal": 0.3, "scour": 0.4, "alch": 0.2, "chaos": 1.0, "exalted": 10.0, "annul": 2.0}
    assert pipeline.plan_key(p1, table) == pipeline.plan_key(p2, table)
    assert pipeline.plan_key(p1, table) != pipeline.plan_key(p1, {**table, "alt": 0.2})
    assert pipeline.plan_key(p1, table) == pipeline.plan_key(p1, {**table, "alt": 0.10001})  # 3 significant figures


def test_passive_point_clusters_become_tiered_crafts():
    from ponder import cluster

    w = lambda who, mods, tag="affliction_minion_damage": demand.WornCluster(who, "Raise Spectre", "large", "Minions deal 10% increased Damage", tag, 12, 84, (), (), tuple(sorted(mods)))
    worn = [w(f"a{i}", ["HaveIncreasedEffect@1", "GrantMinionAttackAndCastSpeed@1"]) for i in range(4)]
    worn += [w(f"b{i}", ["HaveIncreasedEffect@1", "GrantMinionAttackAndCastSpeed@2", "GrantLife@1"]) for i in range(2)]
    found = {a.notables: a for a in demand.passive_archetypes(worn, min_characters=3)}
    # 6 wear Effect + minion speed; most at T1, so the craft asks for T1.
    pair = found[("GrantMinionAttackAndCastSpeed@1", "HaveIncreasedEffect@1")]
    assert pair.characters == 6 and pair.passive and pair.usual_passives == 12
    p = cluster.problem("large", "minion damage", list(pair.notables))
    effect = next(t for t in p.targets if "Effect" in t.name)
    assert effect.name == "35% increased Effect" and len(effect.good) == 1  # T1 only
    assert len(cluster.resolve(p.pool, ["HaveIncreasedEffect@2"])[0].good) == 2  # T2+: 25% or 35%
    fixed = cluster.resolve_fixed(p.pool, ["HaveIncreasedEffect@1"])
    assert fixed[0].label.endswith("35% increased Effect")


@pytest.mark.skipif(not data.available(), reason="run `ponder data fetch` first")
def test_archetypes_become_finished_items():
    """Belts worn as life + armour + 3 suffixes cost as that, not as the mods everyone shares."""
    from ponder import gear
    from ponder.mods import ModDB

    db = ModDB()
    suffixes = [["ChaosResist6", "FireResist5", "Strength5"], ["ColdResist5", "FireResist5", "LightningResist5"], ["Strength5", "ColdResist5", "ChaosResist6"]]
    worn = []
    for i in range(12):
        ids = ["IncreasedLife6", "IncreasedPhysicalDamageReductionRating5", *suffixes[i % 3]]
        m = db.match_ids("Heavy Belt", {"explicit": [{"id": x} for x in ids]})
        worn.append(demand.Worn(f"c{i}", "Cyclone", m, gear.group(db, m).label))
    a = next(a for a in demand.archetypes(worn, min_characters=5) if set(a.families) >= {demand.stat_key("IncreasedLife6")})
    match = demand.archetype_match(db, a)
    assert match.finish["sides"] == (2 - sum(m.kind == "prefix" for m in match.mods), 3 - sum(m.kind == "suffix" for m in match.mods))
    gp = gear.build(db, match, item_level=84)
    p = gp.problem
    assert len(p.targets) + p.filler_need + len(gp.bench) == 5  # a full item
    assert gp.bench and p.reserve  # one open suffix is a bench resistance
    assert any(db.mods[x]["type"] == "Strength" for t in p.targets for x in t.good)  # on 2 of 3: wanted
    assert any(db.mods[x]["type"] == "ColdResistance" for x in p.filler)


def test_cache_keeps_only_the_lite_character(monkeypatch, tmp_path):
    """Fetched characters are stored as their lite record only; full documents left on disk become lite."""
    monkeypatch.setattr(ladder, "CACHE", tmp_path)
    full = {"account": "a", "name": "n", "class": "Witch", "skills": [], "passives": list(range(1000)),
            "items": [{"itemData": {"frameType": 2, "baseType": "Bone Ring", "mods": {"explicit": [{"id": "IncreasedLife7"}]}}}]}
    monkeypatch.setattr(ladder, "_get_json", lambda url: full)
    snap = ladder.Snapshot("L", "l", "v1")
    got = ladder.character(snap, ladder.Row("a", "n"))
    assert got["rares"][0]["baseType"] == "Bone Ring" and "passives" not in got
    assert [p.suffix for p in (tmp_path / "v1").rglob("*.*")] == [".lite"]
    old = tmp_path / "v1" / "b" / "m.json"
    old.parent.mkdir()
    old.write_text(__import__("json").dumps({**full, "name": "m"}))
    assert sorted(c["name"] for c in ladder.cached(snap)) == ["m", "n"]
    assert not old.exists() and old.with_suffix(".lite").is_file()
