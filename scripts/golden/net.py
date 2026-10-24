"""Export network-layer goldens for the Rust port (ponder-net): query text, cache keys, lite characters.

    uv run python scripts/golden/net.py [OUT_DIR]   # default rust/goldens/net

Writes search_url.json, cache_keys.json, item_query.json, cluster_query.json and lite.json:
lists of {"input": ..., "output": ...} where every output is *text* (a URL, a sha1 hex, a
`json.dumps`), so the Rust tests can compare byte for byte. cluster_query.json also carries the
tiny repoe tables the queries were built against ({"repoe": {...}, "cases": [...]}).
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path
from urllib.parse import quote

from ponder import data, ladder, trade

COMPACT = (",", ":")

# -- item queries (the keyword arguments, as JSON) ---------------------------------------------

ITEM_QUERIES: list[dict] = [
    {},
    {"base": "Warlock Gloves", "stats": [["explicit.stat_1", 40]]},
    {"category": "accessory.ring", "stats": [["fractured.stat_9", 5]], "item_level": 84, "fractured_only": True},
    {"base": "Maelström Staff", "stats": [["explicit.stat_2", 0.5], ["explicit.stat_3", None]], "rarity": "rare"},
    {"base": 'Quote " and \\ slash', "category": "weapon.bow", "stats": [["explicit.stat_4", 12.0]], "corrupted": False},
    {"base": "Vaal Regalia", "stats": [["explicit.stat_5", 1e-05], ["explicit.stat_6", 123456789012345678]], "corrupted": True, "item_level": 86},
    {"category": "armour.chest", "stats": [["a", 1], ["b", 2]], "count": [2, [["c", 3], ["d", None], ["e", 4.5]]]},
    {"category": "armour.chest", "stats": [], "count": [0, [["c", 3]]]},
    {"category": "armour.boots", "stats": [], "count": [1, []]},
    {"base": "Hubris Circlet", "stats": [["x", 1]], "extra": [{"id": "enchant.stat_1", "value": {"option": "3"}, "disabled": False}], "rarity": "unique"},
    {"base": "Large Cluster Jewel", "item_level": 0, "stats": [["s", 0]], "fractured_only": False},
    {"base": "", "category": "", "stats": [["s", None]], "rarity": "nonunique"},
    {"base": "Sapphire Ring", "stats": [["s", 1]] * 3, "item_level": 100, "corrupted": False, "fractured_only": True, "count": [3, [["t", 1], ["u", 2]]]},
]


def make_item_query(kw: dict) -> dict:
    kw = dict(kw)
    kw["stats"] = [tuple(s) for s in kw.get("stats", [])]
    if "count" in kw:
        kw["count"] = (kw["count"][0], [tuple(s) for s in kw["count"][1]])
    return trade.item_query(**kw)


LEAGUES = ["Allflame", "Settlers of Kalguur", "Standard", "Hardcore Settlers", "Résistance/Test", "Hall of Grandmasters 100%"]

# -- cluster queries ---------------------------------------------------------------------------

REPOE = {
    "mods": {
        "AfflictionNotableA": {"stats": [{"id": "stat_a", "min": 1, "max": 1}]},
        "AfflictionNotableB": {"stats": [{"id": "stat_b", "min": 1, "max": 1}]},
        "AfflictionNotableNoTrade": {"stats": [{"id": "stat_c", "min": 1, "max": 1}]},
        "SmallPassive1": {"stats": [{"id": "stat_d", "min": 8, "max": 10}]},
        "SmallPassive2": {"stats": [{"id": "stat_d", "min": 0.5, "max": 1}]},
        "Orphan": {"stats": [{"id": "stat_e", "min": 3, "max": 4}]},
    },
    "stat_translations": [
        {"ids": ["stat_a"], "trade_stats": [{"type": "explicit", "id": "explicit.stat_111"}, {"type": "fractured", "id": "fractured.stat_111"}]},
        {"ids": ["stat_b", "stat_other"], "trade_stats": [{"type": "explicit", "id": "explicit.stat_222"}]},
        {"ids": ["stat_c"], "trade_stats": None},
        {"ids": ["stat_d"], "trade_stats": [{"type": "explicit", "id": "explicit.stat_444"}]},
    ],
}

CLUSTER_QUERIES: list[dict] = [
    {"size": "large", "enchant_option": "3", "passives": 8, "notables": ["AfflictionNotableA", "AfflictionNotableB"], "item_level": 84, "fractured_ids": []},
    {"size": "medium", "enchant_option": None, "passives": None, "notables": ["AfflictionNotableA"], "item_level": None, "fractured_ids": ["AfflictionNotableA"]},
    {"size": "small", "enchant_option": "12", "passives": 3, "notables": [["SmallPassive1", 8], ["SmallPassive2", 0.5]], "item_level": 50, "fractured_ids": []},
    {"size": "large", "enchant_option": "7", "passives": 12, "notables": ["AfflictionNotableNoTrade", "Orphan", ["Orphan", 2.0]], "item_level": 75, "fractured_ids": ["Orphan", "AfflictionNotableB"]},
    {"size": "LARGE", "enchant_option": "", "passives": 0, "notables": [], "item_level": 0, "fractured_ids": []},
]


def make_cluster_query(c: dict) -> dict:
    notables = [tuple(n) if isinstance(n, list) else n for n in c["notables"]]
    return trade.cluster_query(c["size"], c["enchant_option"], c["passives"], notables, c["item_level"], c["fractured_ids"])


# -- lite characters ---------------------------------------------------------------------------

def item(frame=2, **kw) -> dict:
    return {"itemData": {"frameType": frame, **kw}}


LITE_DOCS: list[dict] = [
    {
        "account": "Ann#1234", "name": "Zed", "class": "Deadeye", "level": 100, "passives": list(range(50)),
        "skills": [
            {"dps": [], "allGems": [{"name": "Herald of Ash"}]},
            {"dps": [{"name": "Lightning Arrow", "dps": 1234.5}], "allGems": [{"name": "Mirage Archer Support"}, {"name": "Lightning Arrow"}]},
        ],
        "items": [
            item(baseType="Bone Ring", ilvl=84, influences=["shaper"], inventoryId="Ring", mods={"explicit": [{"id": "IncreasedLife7", "tier": 1, "magnitudes": [{"min": 1.5, "max": 2e-05}]}], "implicit": [{"id": "Imp"}]}, enchantMods=["Adds 8 Passive Skills"], extra="dropped", name="Gale Loop"),
            item(frame=3, baseType="Unique Thing", mods={"explicit": []}),
            item(baseType="Unidentified", identified=False),
            item(baseType="Café Belt", ilvl=None),
            item(baseType="Plain Rare"),
        ],
        "jewels": [item(baseType="Large Cluster Jewel", ilvl=84, mods={"enchant": [{"id": "JewelExpansionPassiveNodes", "stats": {"n": 8}}], "explicit": [{"id": "N1"}, {"id": "N2"}]}, enchantMods=["Adds 8 Passive Skills", "Added Small Passive Skills grant: X\nAdded Small Passive Skills grant: Y"])],
    },
    {"account": None, "name": "Solo", "class": None, "skills": [], "items": [], "jewels": []},
    {"account": "B", "name": "NoSkill", "class": "Witch", "skills": [{"allGems": [{"name": ""}, {"name": "Foo Support"}]}], "items": [item(baseType="X", influences={"shaper": True}, ilvl=0, inventoryId="Weapon")]},
    {"account": "C", "name": "AlreadyLite", "class": "Ranger", "rares": [{"baseType": "Y", "mods": {"explicit": [{"id": "Z"}]}}, {"junk": 1}], "skills": [{"dps": True, "allGems": [{"name": "Tornado Shot"}]}]},
]


def py_json(v, **kw) -> str:
    return json.dumps(v, **kw)


def main() -> None:
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "rust/goldens/net")
    out.mkdir(parents=True, exist_ok=True)
    tables = REPOE
    data.load = lambda name, dest=None: tables[name]  # the queries read repoe through data.load
    trade._cache.clear()

    queries = [make_item_query(kw) for kw in ITEM_QUERIES]
    cluster = [make_cluster_query(c) for c in CLUSTER_QUERIES]

    urls = []
    for i, q in enumerate(queries + cluster):
        league = LEAGUES[i % len(LEAGUES)]
        urls.append({"input": {"league": league, "query": q}, "output": trade.search_url(league, q)})
        assert trade.search_url(league, q) == f"{trade.SITE}/trade/search/{quote(league)}?q={quote(json.dumps({'query': q['query']}, separators=COMPACT))}"
    write(out / "search_url.json", urls)

    keys = []
    for i, q in enumerate(queries + cluster):
        league, n = LEAGUES[i % len(LEAGUES)], [10, 25, 50][i % 3]
        keys.append({"input": {"kind": "price", "league": league, "query": q, "n": n}, "output": hashlib.sha1(json.dumps([league, q, n], sort_keys=True).encode()).hexdigest()})
        keys.append({"input": {"kind": "listings", "league": league, "query": q, "n": n}, "output": hashlib.sha1(json.dumps(["listings", league, q, n], sort_keys=True).encode()).hexdigest()})
    write(out / "cache_keys.json", keys)

    write(out / "item_query.json", [{"input": kw, "output": py_json(q, separators=COMPACT), "output_default": py_json(q)} for kw, q in zip(ITEM_QUERIES, queries)])
    write(out / "cluster_query.json", {"repoe": REPOE, "cases": [{"input": c, "output": py_json(q, separators=COMPACT)} for c, q in zip(CLUSTER_QUERIES, cluster)]})
    write(out / "lite.json", [{"input": d, "output": py_json(ladder.lite(d))} for d in LITE_DOCS])
    print(f"wrote {out}: {len(urls)} urls, {len(keys)} keys, {len(queries)} item queries, {len(cluster)} cluster queries, {len(LITE_DOCS)} characters")


def write(path: Path, doc) -> None:
    path.write_text(json.dumps(doc, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
