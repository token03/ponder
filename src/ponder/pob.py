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


@dataclass
class PobItem:
    id: str
    text: str
    slot: str | None = None


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
        ascendancy=attrs.get("ascendClassName", ""),
        level=int(attrs.get("level", 0) or 0),
        main_skill=_main_skill(root, attrs.get("mainSocketGroup")),
    )

    items_el = root.find("Items")
    if items_el is None:
        return build
    slots = _active_slots(items_el)
    for item_el in items_el.findall("Item"):
        item_id = item_el.get("id", "")
        text = "\n".join(line.strip() for line in "".join(item_el.itertext()).strip().splitlines())
        build.items.append(PobItem(id=item_id, text=text, slot=slots.get(item_id)))
    return build


def _active_slots(items_el: ET.Element) -> dict[str, str]:
    """Map item id -> slot name for the active item set."""
    active = items_el.get("activeItemSet")
    slot_els = items_el.findall("Slot")  # older exports keep slots at top level
    for item_set in items_el.findall("ItemSet"):
        if item_set.get("id") == active or not slot_els:
            slot_els = item_set.findall("Slot")
            if item_set.get("id") == active:
                break
    return {
        s.get("itemId"): s.get("name")
        for s in slot_els
        if s.get("itemId") not in (None, "0") and "Swap" not in s.get("name", "")
    }


def _main_skill(root: ET.Element, main_group: str | None) -> str | None:
    skills = root.find("Skills")
    if skills is None:
        return None
    groups = skills.findall(".//Skill")
    try:
        group = groups[int(main_group) - 1]
    except (TypeError, ValueError, IndexError):
        return None
    gems = group.findall("Gem")
    active = [g for g in gems if "Support" not in (g.get("skillId") or "")]
    gem = (active or gems or [None])[0]
    return gem.get("nameSpec") if gem is not None else None
