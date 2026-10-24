"""Export item-parsing and mod-matching goldens for the Rust port.

    PYTHONHASHSEED=0 uv run python scripts/golden/items.py [OUT_DIR]   # default rust/goldens/items

Writes gzipped JSON:
  fixtures.json.gz  every PoB fixture build: its summary, and per item the parse_item fields and ModDB.match result
  ladder.json.gz    a sample of cached ladder rare items run through ModDB.match_ids exactly as demand.wear does
  texts.json.gz     hand-written item texts (PoB and in-game copy formats): parse and match
  synth.json.gz     PoB texts synthesised from real ladder items' mod templates: parse and match

Tag sets of items with two or more influences depend on Python's set order (hash seed), so a port
can only be compared on them through mods, sources and tiers.
"""

from __future__ import annotations

import gzip
import json
import os
import random
import re
import sys
from pathlib import Path

from ponder import data, ladder, pob
from ponder.item import Item, ModLine, parse_item
from ponder.mods import ItemMatch, ModDB

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests" / "fixtures"
SAMPLE = int(os.environ.get("PONDER_GOLDEN_SAMPLE", 2000))
SEED = 20261023
SYNTH = int(os.environ.get("PONDER_GOLDEN_SYNTH", 800))


def line(l: ModLine):
    return {"text": l.text, "markers": dict(sorted(l.markers.items()))}


def item_json(item: Item):
    return {
        "rarity": item.rarity,
        "name": item.name,
        "base": item.base,
        "item_level": item.item_level,
        "quality": item.quality,
        "influences": sorted(item.influences),
        "flags": sorted(item.flags),
        "implicits": [line(l) for l in item.implicits],
        "explicits": [line(l) for l in item.explicits],
        "pob_affixes": [list(p) for p in item.pob_affixes],
        "header": dict(sorted(item.header.items())),
    }


def match_json(m: ItemMatch):
    return {
        "base_id": m.base_id,
        "item_class": m.item_class,
        "tags": sorted(m.tags),
        "mods": [
            {
                "mod_id": x.mod_id,
                "kind": x.kind,
                "family": x.family,
                "source": x.source,
                "tier": x.tier,
                "alternatives": x.alternatives,
                "merged": x.merged,
                "lines": [line(l) for l in x.lines],
            }
            for x in m.mods
        ],
        "unmatched": [l.text for l in m.unmatched],
        "notes": [l.text for l in m.notes],
    }


def write(out: Path, name: str, obj) -> None:
    path = out / f"{name}.json.gz"
    with gzip.GzipFile(path, "wb", mtime=0) as f:
        f.write(json.dumps(obj, ensure_ascii=False, separators=(",", ":")).encode())
    print(f"{path}: {path.stat().st_size / 1024:.0f} KiB")


def fixtures(db: ModDB):
    paths = sorted((FIXTURES / "real").glob("*.txt")) + sorted(FIXTURES.glob("*.xml"))
    out = []
    for path in paths:
        build = pob.load(str(path))
        items = []
        for pi in build.items:
            entry = {"id": pi.id, "slot": pi.slot, "sets": pi.sets}
            try:
                item = parse_item(pi.text)
            except ValueError as e:
                entry["error"] = str(e)
                items.append(entry)
                continue
            entry["item"] = item_json(item)
            entry["match"] = match_json(db.match(item))
            items.append(entry)
        out.append(
            {
                "file": str(path.relative_to(FIXTURES)),
                "class_name": build.class_name,
                "ascendancy": build.ascendancy,
                "level": build.level,
                "main_skill": build.main_skill,
                "items": items,
            }
        )
    return out


def ladder_sample(db: ModDB):
    rares = [it for c in ladder.cached() for it in ladder.rare_items(c)]
    random.Random(SEED).shuffle(rares)
    out = []
    for it in rares[:SAMPLE]:
        influences = {k for k, v in (it.get("influences") or {}).items() if v}
        mods = it.get("mods") or {}
        m = db.match_ids(it.get("baseType", ""), mods, it.get("ilvl"), influences)
        out.append(
            {
                "base": it.get("baseType", ""),
                "ilvl": it.get("ilvl"),
                "influences": sorted(influences),
                "mods": {k: [e.get("id", "") for e in mods.get(k) or []] for k in ("explicit", "fractured", "crafted")},
                "match": match_json(m),
            }
        )
    return {"total_rares": len(rares), "items": out}


TEXTS = [
    # tests/test_item.py
    "Rarity: RARE\nDoom Crown\nHubris Circlet\nShaper Item\nItem Level: 86\nQuality: 20\nImplicits: 0\n+121 to maximum Life\n{fractured}84% increased Energy Shield\n{crafted}+23% to Lightning Resistance\nCorrupted\n",
    "Rarity: RARE\nGrim Stride\nTwo-Toned Boots\nSelected Variant: 2\nImplicits: 2\n{variant:1}+(8-12)% to Fire and Cold Resistances\n{variant:2}+(8-12)% to Cold and Lightning Resistances\n{range:0.5}+(42-45)% to Lightning Resistance\n",
    "Rarity: MAGIC\nRunner's Two-Toned Boots\nImplicits: 0\n25% increased Movement Speed\n",
    "Rarity: RARE\nX\nOnyx Amulet\nPrefix: {range:0.5}IncreasedLife7\nSuffix: None\nImplicits: 0\n",
    # tests/test_mods.py
    "Rarity: RARE\nX\nHubris Circlet\nShaper Item\nItem Level: 86\nImplicits: 0\n+135 to maximum Life\n+44% to Lightning Resistance\n{crafted}+23% to Lightning Resistance\n",
    "Rarity: RARE\nX\nHubris Circlet\nItem Level: 86\nImplicits: 0\n40% increased Energy Shield\n16% increased Stun and Block Recovery\n",
    "Rarity: RARE\nX\nHubris Circlet\nShaper Item\nItem Level: 86\nImplicits: 0\nAdds 20 to 38 Cold Damage to Spells\n",
    "Rarity: RARE\nX\nHubris Circlet\nItem Level: 86\nImplicits: 0\nAdds 20 to 38 Cold Damage to Spells\n",
    "Rarity: RARE\nX\nHubris Circlet\nImplicits: 0\nUse a fossil here\n+999 to maximum Life\n",
    "Rarity: RARE\nX\nHubris Circlet\nItem Level: 86\nImplicits: 0\n136% increased Energy Shield\n16% increased Stun and Block Recovery\n",
    # more PoB shapes: unknown base, normal item, colour codes, a split hybrid whose other line was matched alone
    "Rarity: RARE\nNobody\nNo Such Base\nImplicits: 0\n+10 to Strength\n",
    "Rarity: NORMAL\nHubris Circlet\nImplicits: 0\n",
    "Rarity: RARE\n^x7F7F7FFoo\nHubris Circlet\nItem Level: 60\nImplicits: 0\n{range:0.2}+(100-114) to maximum Life\n^7Note: nice item\n",
    "Rarity: RARE\nX\nHubris Circlet\nItem Level: 86\nImplicits: 0\n140% increased Energy Shield\n16% increased Stun and Block Recovery\n+40 to maximum Life\n",
    "Rarity: RARE\nX\nHubris Circlet\nItem Level: 86\nImplicits: 0\n16% increased Stun and Block Recovery\n140% increased Energy Shield\n",
    "Rarity: RARE\nX\nHubris Circlet\nHunter Item\nWarlord Item\nItem Level: 86\nImplicits: 0\n+30 to maximum Life\n",
    "Rarity: RARE\nX\nTitanium Spirit Shield\nItem Level: 80\nImplicits: 0\n+30 to maximum Life\n+35% to Fire Resistance\n+35% to Fire Resistance\n",
    "Rarity: MAGIC\nStalwart Hubris Circlet of the Whale\nItem Level: 80\nImplicits: 0\n+30 to maximum Life\n",
    "Rarity: RARE\nX\nStygian Vise\nItem Level: 84\nImplicits: 0\n{crafted}+(15-25) to maximum Life\n{crafted}Can have up to 3 Crafted Modifiers\n",
    # in-game copy: tests/test_item.py COPIED
    """Item Class: Rings
Rarity: Rare
Gale Loop
Bone Ring
--------
Requirements:
Level: 75
--------
Item Level: 84
--------
+15 to maximum Energy Shield (implicit)
--------
{ Prefix Modifier "Virile" (Tier: 1) — Life }
+105(100-114) to maximum Life
+40 to maximum Energy Shield (fractured)
+42% to Fire Resistance
+12 to Dexterity (crafted)
--------
Shaper Item
--------
Corrupted
""",
    # plain Ctrl+C copy of a helmet
    """Item Class: Helmets
Rarity: Rare
Doom Crown
Hubris Circlet
--------
Quality: +20% (augmented)
Energy Shield: 120 (augmented)
--------
Requirements:
Level: 69
Int: 123
--------
Sockets: B-B-B
--------
Item Level: 86
--------
+121 to maximum Life
84% increased Energy Shield
+23% to Lightning Resistance (crafted)
+12% to Fire Resistance (scourge)
--------
Note: ~price 5 chaos
""",
    # advanced copy with hybrid and influences
    """Item Class: Helmets
Rarity: Rare
Havoc Veil
Hubris Circlet
--------
Item Level: 86
--------
{ Prefix Modifier "Saintly" (Tier: 3) — Defences, Life }
+(30-45)(20-25) to maximum Life
{ Suffix Modifier "of the Whale" (Tier: 2) — Elemental, Fire, Resistance }
+38(36-41)% to Fire Resistance
{ Searing Exarch Implicit Modifier (Lesser) — Elemental }
10(8-12)% increased Fire Damage (implicit)
--------
Searing Exarch Item
--------
Fractured Item
""",
    "Item Class: Boots\nRarity: Unique\nSundance\nTwo-Toned Boots\n--------\nItem Level: 80\n--------\n+30 to maximum Life\n",
    "Item Class: Boots\nRarity: Normal\nTwo-Toned Boots\n--------\nItem Level: 80\n",
    "not an item\n--------\nat all",
    "Rarity: Rare",
]


def synth(db: ModDB):
    """PoB texts made from real ladder items: each mod's template with ranges rolled, hybrids as several lines.

    Lines are shuffled, some crafted, and now and then two single-line mods with one template are summed
    the way the game shows them, so the hybrid-first and split paths get exercised on real mods.
    """
    rng = random.Random(SEED + 1)
    rares = [it for c in ladder.cached() for it in ladder.rare_items(c) if it.get("baseType")]
    rng.shuffle(rares)
    token = re.compile(r"\((-?\d+(?:\.\d+)?)-(-?\d+(?:\.\d+)?)\)")

    def roll(template: str, bump: float = 0.0) -> str:
        def one(m):
            lo, hi = float(m.group(1)), float(m.group(2))
            v = rng.uniform(min(lo, hi), max(lo, hi))
            return str(round(v)) if "." not in m.group(1) + m.group(2) else f"{v:.1f}"

        return token.sub(one, template)

    out = []
    for it in rares:
        if len(out) >= SYNTH:
            break
        m = db.match_ids(it["baseType"], it.get("mods") or {}, it.get("ilvl"), {k for k, v in (it.get("influences") or {}).items() if v})
        if m.base_id is None or not m.mods:
            continue
        lines = []
        for x in m.mods:
            for template in db.mods[x.mod_id]["text"].splitlines():
                text = roll(template)
                if x.source in ("crafted", "legacy-crafted"):
                    text = "{crafted}" + text
                lines.append(text)
        if rng.random() < 0.3:  # sum a pair of lines with one template, as the game does
            by = {}
            for i, l in enumerate(lines):
                by.setdefault(re.sub(r"[\d.]+", "#", l), []).append(i)
            for idx in by.values():
                if len(idx) == 2:
                    a, b = (re.search(r"[\d.]+", lines[i]) for i in idx)
                    if a and b and "{" not in lines[idx[0]]:
                        total = round(float(a.group()) + float(b.group()))
                        lines[idx[0]] = lines[idx[0]][: a.start()] + str(total) + lines[idx[0]][a.end() :]
                        lines[idx[1]] = None
            lines = [l for l in lines if l]
        rng.shuffle(lines)
        infl = "".join(f"{k.title()} Item\n" for k, v in (it.get("influences") or {}).items() if v and k in ("shaper", "elder", "crusader", "hunter", "warlord", "redeemer"))
        text = f"Rarity: RARE\nSynth\n{it['baseType']}\n{infl}Item Level: {it.get('ilvl') or 80}\nImplicits: 0\n" + "\n".join(lines) + "\n"
        item = parse_item(text)
        out.append({"text": text, "item": item_json(item), "match": match_json(db.match(item))})
    return out


def texts_json(db: ModDB):
    out = []
    for t in TEXTS:
        entry = {"text": t}
        try:
            item = parse_item(t)
        except (ValueError, IndexError) as e:  # a bare "Rarity:" line has no name; Python raises IndexError
            entry["error"] = type(e).__name__
        else:
            entry["item"] = item_json(item)
            entry["match"] = match_json(db.match(item))
        out.append(entry)
    return out


def main() -> None:
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "rust" / "goldens" / "items"
    out.mkdir(parents=True, exist_ok=True)
    db = ModDB()
    print("game version", data.version())
    write(out, "fixtures", fixtures(db))
    write(out, "ladder", ladder_sample(db))
    write(out, "texts", texts_json(db))
    write(out, "synth", synth(db))


if __name__ == "__main__":
    main()
