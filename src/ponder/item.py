"""Parse Path of Building item text into a structured item.

PoB stores each item as text close to the in-game copy format, with extra
header keys and per-line markers such as `{crafted}`, `{fractured}`,
`{range:0.5}` or `{variant:1,2}`.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field

INFLUENCES = {
    "Shaper Item": "shaper",
    "Elder Item": "elder",
    "Crusader Item": "crusader",
    "Redeemer Item": "redeemer",
    "Hunter Item": "hunter",
    "Warlord Item": "warlord",
}
_FLAGS = {
    "Corrupted": "corrupted",
    "Mirrored": "mirrored",
    "Split": "split",
    "Synthesised Item": "synthesised",
    "Fractured Item": "fractured",
    "Searing Exarch Item": "exarch",
    "Eater of Worlds Item": "eater",
}
_MARKER = re.compile(r"^\{(\w+)(?::([^}]*))?\}")
_COLOR = re.compile(r"\^(x[0-9a-fA-F]{6}|\d)")  # PoB colour codes, e.g. in author notes


@dataclass(eq=False)  # lines are compared by identity; an item can repeat a line
class ModLine:
    text: str
    markers: dict[str, str] = field(default_factory=dict)

    @property
    def crafted(self) -> bool:
        return "crafted" in self.markers

    @property
    def fractured(self) -> bool:
        return "fractured" in self.markers

    @property
    def roll(self) -> float | None:
        """PoB's position within the roll range (0..1), if it recorded one."""
        value = self.markers.get("range")
        return float(value) if value else None


@dataclass
class Item:
    rarity: str
    name: str
    base: str
    item_level: int | None = None
    quality: int = 0
    influences: set[str] = field(default_factory=set)
    flags: set[str] = field(default_factory=set)
    implicits: list[ModLine] = field(default_factory=list)
    explicits: list[ModLine] = field(default_factory=list)
    # Mod ids PoB's item crafter recorded directly ("Prefix: IncreasedLife7").
    pob_affixes: list[tuple[str, str]] = field(default_factory=list)
    header: dict[str, str] = field(default_factory=dict)


def parse_item(text: str) -> Item:
    """A PoB item, or an item copied in game (Ctrl+C, or Ctrl+Alt+C with mod tiers)."""
    if _SEPARATOR in text:
        return _parse_copied(text)
    lines = [_COLOR.sub("", line).strip() for line in text.strip().splitlines()]
    lines = [line for line in lines if line]
    if not lines or not lines[0].startswith("Rarity:"):
        raise ValueError(f"not a PoB item: {lines[:1]}")
    rarity = lines[0].split(":", 1)[1].strip().upper()
    if rarity in ("RARE", "UNIQUE", "RELIC"):
        name, base, rest = lines[1], lines[2], lines[3:]
    else:
        name = base = lines[1]
        rest = lines[2:]
    item = Item(rarity=rarity, name=name, base=base)

    implicit_count = None
    i = 0
    while i < len(rest):
        line = rest[i]
        i += 1
        if line in INFLUENCES:
            item.influences.add(INFLUENCES[line])
        elif line in _FLAGS:
            item.flags.add(_FLAGS[line])
        elif ":" in line and not line.startswith("{"):
            key, value = (part.strip() for part in line.split(":", 1))
            if key == "Implicits":
                implicit_count = int(value)
                break
            item.header[key] = value
            if key in ("Prefix", "Suffix") and value != "None":
                item.pob_affixes.append((key.lower(), _strip_markers(value)[0]))
        else:
            i -= 1  # no Implicits header; the mod lines start here
            break

    item.item_level = _int(item.header.get("Item Level"))
    item.quality = _int(item.header.get("Quality")) or 0
    variant = item.header.get("Selected Variant")

    body = rest[i:]
    trailing = [line for line in body if line in _FLAGS]
    item.flags |= {_FLAGS[line] for line in trailing}
    body = [line for line in body if line not in _FLAGS]

    # The implicit count includes lines of every variant, so split before filtering.
    n = implicit_count or 0
    item.implicits = _mod_lines(body[:n], variant)
    item.explicits = _mod_lines(body[n:], variant)
    return item


_SEPARATOR = "--------"
_COPIED_MARKER = re.compile(r"\s*\((implicit|crafted|fractured|enchant|scourge|crucible)\)$")
_COPIED_RANGE = re.compile(r"(\d+(?:\.\d+)?)\(-?\d+(?:\.\d+)?-\d+(?:\.\d+)?\)")  # advanced copy: "105(100-109)"
_COPIED_SKIP = ("Requirements:", "Sockets:", "Item Level:", "Quality", "Note:", "Level:", "Str:", "Dex:", "Int:")


def _parse_copied(text: str) -> Item:
    """The game's copy format: sections between "--------" lines.

    The first section names the item; a section of "Key: value" lines is
    properties or requirements; mod lines carry "(implicit)", "(crafted)" or
    "(fractured)"; advanced copies (Ctrl+Alt+C) add "{ Prefix Modifier ... }"
    headers and roll ranges, which are dropped.
    """
    sections = [[line.strip() for line in part.strip().splitlines() if line.strip()] for part in text.strip().split(_SEPARATOR)]
    head = [line for line in sections[0] if not line.startswith("Item Class:")]
    if not head or not head[0].startswith("Rarity:"):
        raise ValueError(f"not a copied item: {head[:1]}")
    rarity = head[0].split(":", 1)[1].strip().upper()
    names = head[1:]
    name, base = (names[0], names[-1]) if names else ("", "")
    item = Item(rarity=rarity, name=name, base=base)
    for section in sections[1:]:
        for line in section:
            if line.startswith("Item Level:"):
                item.item_level = _int(line.split(":", 1)[1].strip())
            elif line.startswith("Quality:"):
                item.quality = _int(re.sub(r"\D", "", line.split(":", 1)[1].split("(")[0])) or 0
        if all(":" in line for line in section) or section[0].startswith(_COPIED_SKIP):
            continue  # properties, requirements, sockets, item level, notes
        for line in section:
            if line in INFLUENCES:
                item.influences.add(INFLUENCES[line])
            elif line in _FLAGS:
                item.flags.add(_FLAGS[line])
            elif line.startswith("{"):
                continue  # advanced copy: { Prefix Modifier "Virile" (Tier: 1) — Life }
            else:
                found = _COPIED_MARKER.search(line)
                kind = found.group(1) if found else ""
                line = _COPIED_RANGE.sub(r"\1", _COPIED_MARKER.sub("", line)).split(" — ")[0]
                if kind == "implicit":
                    item.implicits.append(ModLine(line))
                elif kind in ("", "crafted", "fractured"):
                    item.explicits.append(ModLine(line, {kind: ""} if kind else {}))
    if any(m.fractured for m in item.explicits):
        item.flags.add("fractured")
    return item


def _mod_lines(lines: list[str], variant: str | None) -> list[ModLine]:
    out = []
    for line in lines:
        text, markers = _strip_markers(line)
        if "variant" in markers and variant and variant not in markers["variant"].split(","):
            continue
        out.append(ModLine(text=text, markers=markers))
    return out


def _strip_markers(line: str) -> tuple[str, dict[str, str]]:
    markers: dict[str, str] = {}
    while m := _MARKER.match(line):
        markers[m.group(1)] = m.group(2) or ""
        line = line[m.end():]
    return line.strip(), markers


def _int(value: str | None) -> int | None:
    try:
        return int(value) if value is not None else None
    except ValueError:
        return None
