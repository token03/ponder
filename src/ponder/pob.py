"""Decode Path of Building export codes and pull out the items.

A PoB code is URL-safe base64 of zlib-compressed XML. Share sites (pobb.in,
pastebin) serve the same code at a `/raw` URL.
"""

from __future__ import annotations

import base64
import re
import urllib.request
import xml.etree.ElementTree as ET
import zlib
from dataclasses import dataclass, field
from pathlib import Path

_RAW_URLS = [
    (re.compile(r"^https?://pobb\.in/([\w-]+)/?$"), "https://pobb.in/{}/raw"),
    (re.compile(r"^https?://pastebin\.com/(?:raw/)?(\w+)/?$"), "https://pastebin.com/raw/{}"),
]


_COLOR = re.compile(r"\^(x[0-9a-fA-F]{6}|\d)")


@dataclass
class PobItem:
    id: str
    text: str
    slot: str | None = None  # slot in the active item set, else the first set using it
    sets: list[str] = field(default_factory=list)  # titles of item sets using it, e.g. "Early game"


@dataclass
class Build:
    class_name: str
    ascendancy: str
    level: int
    main_skill: str | None
    items: list[PobItem] = field(default_factory=list)


def decode(code: str) -> str:
    """Turn a PoB export code into its XML string."""
    code = "".join(code.split())
    code = code.replace("-", "+").replace("_", "/")
    code += "=" * (-len(code) % 4)
    return zlib.decompress(base64.b64decode(code)).decode("utf-8")


def encode(xml: str) -> str:
    """Inverse of `decode`, producing a code PoB can import."""
    raw = base64.b64encode(zlib.compress(xml.encode("utf-8")))
    return raw.decode("ascii").replace("+", "-").replace("/", "_")


def read_code(source: str) -> str:
    """Get a PoB code from a share URL, a file path, or the code itself."""
    for pattern, raw in _RAW_URLS:
        if m := pattern.match(source.strip()):
            req = urllib.request.Request(raw.format(m.group(1)), headers={"User-Agent": "ponder"})
            with urllib.request.urlopen(req, timeout=30) as resp:
                return resp.read().decode("utf-8").strip()
    try:
        path = Path(source)
        if path.is_file():
            return path.read_text().strip()
    except OSError:  # a long code isn't a valid path
        pass
    return source.strip()


def load(source: str) -> Build:
    """Load a build from a share URL, a file holding a code or XML, or a code."""
    text = read_code(source)
    xml = text if text.lstrip().startswith("<") else decode(text)
    return parse(xml)


def parse(xml: str) -> Build:
    root = ET.fromstring(xml)
    build_el = root.find("Build")
    attrs = build_el.attrib if build_el is not None else {}
    build = Build(
        class_name=attrs.get("className", ""),
        ascendancy=attrs.get("ascendClassName", "").replace("None", ""),
        level=int(attrs.get("level", 0) or 0),
        main_skill=_main_skill(root, attrs.get("mainSocketGroup")),
    )

    items_el = root.find("Items")
    if items_el is None:
        return build
    usage = _item_usage(items_el)
    for item_el in items_el.findall("Item"):
        item_id = item_el.get("id", "")
        text = "\n".join(line.strip() for line in "".join(item_el.itertext()).strip().splitlines())
        uses = usage.get(item_id, [])
        slot = next((slot for _, slot, active in uses if active), uses[0][1] if uses else None)
        sets = list(dict.fromkeys(title for title, _, _ in uses if title))
        build.items.append(PobItem(id=item_id, text=text, slot=slot, sets=sets))
    return build


def _item_usage(items_el: ET.Element) -> dict[str, list[tuple[str, str, bool]]]:
    """Item id -> (item set title, slot, is active set) for every set equipping it."""
    active = items_el.get("activeItemSet")
    sets = [(s.get("title") or f"Set {s.get('id')}", s.get("id") == active, s) for s in items_el.findall("ItemSet")]
    if not sets:  # older exports keep slots at top level
        sets = [("", True, items_el)]
    usage: dict[str, list[tuple[str, str, bool]]] = {}
    for title, is_active, el in sets:
        for slot in el.findall("Slot"):
            item_id, name = slot.get("itemId"), slot.get("name", "")
            if item_id in (None, "0") or "Swap" in name:
                continue
            usage.setdefault(item_id, []).append((_COLOR.sub("", title).strip(), name, is_active))
    return usage


def _main_skill(root: ET.Element, main_group: str | None) -> str | None:
    skills = root.find("Skills")
    if skills is None:
        return None
    active = skills.get("activeSkillSet")
    skill_sets = skills.findall("SkillSet")
    container = next((s for s in skill_sets if s.get("id") == active), skill_sets[0] if skill_sets else skills)
    groups = container.findall("Skill")
    try:
        group = groups[int(main_group) - 1]
    except (TypeError, ValueError, IndexError):
        return None
    gems = group.findall("Gem")
    active = [g for g in gems if "Support" not in (g.get("skillId") or "")]
    gem = (active or gems or [None])[0]
    return gem.get("nameSpec") if gem is not None else None
