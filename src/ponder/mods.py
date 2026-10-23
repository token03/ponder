"""Match item text lines back to exact mod ids, tiers and sources.

Every repoe mod carries a display template such as `+(100-114) to maximum
Life`. Replacing numbers and ranges with `#` gives a key we can look item lines
up by; the ranges then pick out the tier. Hybrid mods span several lines and
are matched as a unit before single-line mods.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from functools import cached_property

from . import data
from .item import Item, ModLine

_TOKEN = re.compile(r"\((-?\d+(?:\.\d+)?)-(-?\d+(?:\.\d+)?)\)|(\d+(?:\.\d+)?)")

# repoe spawn tags use the old internal influence names.
INFLUENCE_TAGS = {
    "shaper": "shaper",
    "elder": "elder",
    "crusader": "crusader",
    "redeemer": "eyrie",
    "hunter": "basilisk",
    "warlord": "adjudicator",
}

# Lower sorts first when several mods fit a line.
_SOURCE_RANK = {"natural": 0, "essence": 1, "special": 2, "crafted": 0, "legacy-crafted": 1}


@dataclass
class ModMatch:
    mod_id: str
    kind: str  # "prefix" or "suffix"
    family: str  # repoe mod `type`, shared by every tier of a mod
    source: str  # natural | essence | special | crafted | legacy-crafted
    tier: int | None  # 1 = best natural tier on this base
    lines: list[ModLine]
    alternatives: int = 0  # other mods that fit the same lines equally well

    @property
    def fractured(self) -> bool:
        return any(line.fractured for line in self.lines)


@dataclass
class ItemMatch:
    item: Item
    base_id: str | None
    item_class: str | None
    tags: set[str] = field(default_factory=set)
    mods: list[ModMatch] = field(default_factory=list)
    unmatched: list[ModLine] = field(default_factory=list)

    @property
    def prefixes(self) -> list[ModMatch]:
        return [m for m in self.mods if m.kind == "prefix"]

    @property
    def suffixes(self) -> list[ModMatch]:
        return [m for m in self.mods if m.kind == "suffix"]


def normalize(text: str) -> str:
    return _TOKEN.sub("#", text)


def template_ranges(text: str) -> list[tuple[float, float]]:
    out = []
    for m in _TOKEN.finditer(text):
        if m.group(3) is not None:
            v = float(m.group(3))
            out.append((v, v))
        else:
            lo, hi = float(m.group(1)), float(m.group(2))
            out.append((min(lo, hi), max(lo, hi)))
    return out


def line_values(line: ModLine) -> list[float]:
    """Numbers on an item line; ranges are resolved with PoB's roll position."""
    roll = line.roll if line.roll is not None else 0.5
    values = []
    for lo, hi in template_ranges(line.text):
        values.append(lo + (hi - lo) * roll)
    return values


def fits(values: list[float], ranges: list[tuple[float, float]]) -> bool:
    return len(values) == len(ranges) and all(lo - 0.5 <= v <= hi + 0.5 for v, (lo, hi) in zip(values, ranges))


class ModDB:
    def __init__(self, mods: dict | None = None, bases: dict | None = None, bench: dict | None = None):
        self.mods = mods if mods is not None else data.load("mods")
        self.bases = bases if bases is not None else data.load("base_items")
        self._tiers: dict = {}
        self._bench = bench if bench is not None else _load_bench()

    @cached_property
    def _affixes(self) -> dict[str, dict]:
        return {
            mod_id: mod
            for mod_id, mod in self.mods.items()
            if mod["domain"] in ("item", "crafted")
            and mod["generation_type"] in ("prefix", "suffix")
            and mod.get("text")
        }

    @cached_property
    def _index(self) -> dict[tuple[str, ...], list[str]]:
        """Normalized template lines -> mod ids."""
        index: dict[tuple[str, ...], list[str]] = {}
        for mod_id, mod in self._affixes.items():
            key = tuple(normalize(line) for line in mod["text"].splitlines())
            index.setdefault(key, []).append(mod_id)
        return index

    @cached_property
    def _hybrids_by_line(self) -> dict[str, list[tuple[str, ...]]]:
        out: dict[str, list[tuple[str, ...]]] = {}
        for key in self._index:
            if len(key) > 1:
                for line in key:
                    out.setdefault(line, []).append(key)
        return out

    @cached_property
    def _bases_by_name(self) -> dict[str, str]:
        out: dict[str, str] = {}
        for base_id, base in self.bases.items():
            if base.get("domain") != "item" or not base.get("name"):
                continue
            if base["name"] not in out or base.get("release_state") == "released":
                out[base["name"]] = base_id
        return out

    def find_base(self, name: str) -> str | None:
        if name in self._bases_by_name:
            return self._bases_by_name[name]
        # Magic and normal items carry affixes around the base name.
        hits = [n for n in self._bases_by_name if n in name]
        return self._bases_by_name[max(hits, key=len)] if hits else None

    def item_tags(self, base_id: str, influences: set[str]) -> set[str]:
        tags = set(self.bases[base_id].get("tags", []))
        for influence in influences:
            suffix = INFLUENCE_TAGS[influence]
            tags |= {f"{tag}_{suffix}" for tag in list(tags)}
        return tags

    @staticmethod
    def spawn_weight(mod: dict, tags: set[str]) -> int:
        for entry in mod["spawn_weights"]:
            if entry["tag"] in tags:
                return entry["weight"]
        return 0

    def source(self, mod: dict, tags: set[str]) -> str:
        if mod["domain"] == "crafted":
            return "crafted"
        if self.spawn_weight(mod, tags) > 0:
            return "natural"
        return "essence" if mod.get("is_essence_only") else "special"

    def tier(self, mod_id: str, tags: set[str]) -> int | None:
        mod = self.mods[mod_id]
        key = (mod["type"], mod["generation_type"], frozenset(tags))
        if key not in self._tiers:
            levels = sorted(
                {
                    m["required_level"]
                    for m in self._affixes.values()
                    if m["domain"] == "item"
                    and m["type"] == mod["type"]
                    and m["generation_type"] == mod["generation_type"]
                    and self.spawn_weight(m, tags) > 0
                },
                reverse=True,
            )
            self._tiers[key] = levels
        levels = self._tiers[key]
        if self.spawn_weight(mod, tags) <= 0 or mod["required_level"] not in levels:
            return None
        return levels.index(mod["required_level"]) + 1

    def match(self, item: Item) -> ItemMatch:
        base_id = self.find_base(item.base)
        if base_id is None:
            return ItemMatch(item=item, base_id=None, item_class=None, unmatched=list(item.explicits))
        tags = self.item_tags(base_id, item.influences)
        result = ItemMatch(item=item, base_id=base_id, item_class=self.bases[base_id]["item_class"], tags=tags)
        pob_ids = {mod_id for _, mod_id in item.pob_affixes}

        remaining = list(item.explicits)
        # Hybrids first, so their lines aren't claimed one by one.
        for line in list(remaining):
            if line not in remaining:
                continue
            for key in self._hybrids_by_line.get(normalize(line.text), []):
                claimed = self._claim(key, remaining)
                if claimed is None:
                    continue
                found = self._best(key, claimed, item, result.item_class, tags, pob_ids)
                if found:
                    result.mods.append(found)
                    for c in claimed:
                        remaining.remove(c)
                    break

        for line in list(remaining):
            found = self._best((normalize(line.text),), [line], item, result.item_class, tags, pob_ids)
            if found:
                result.mods.append(found)
                remaining.remove(line)
        result.unmatched = remaining
        return result

    @staticmethod
    def _claim(key: tuple[str, ...], lines: list[ModLine]) -> list[ModLine] | None:
        claimed = []
        for norm in key:
            line = next((l for l in lines if l not in claimed and normalize(l.text) == norm), None)
            if line is None:
                return None
            claimed.append(line)
        return claimed

    def _best(self, key, lines, item, item_class, tags, pob_ids) -> ModMatch | None:
        crafted = any(line.crafted for line in lines)
        values = [line_values(line) for line in lines]
        scored = []
        for mod_id in self._index.get(key, []):
            mod = self._affixes[mod_id]
            if (mod["domain"] == "crafted") != crafted:
                continue
            ranges = [template_ranges(t) for t in mod["text"].splitlines()]
            if not all(fits(v, r) for v, r in zip(values, ranges)):
                continue
            src = self.source(mod, tags)
            if src == "crafted" and mod_id not in self._bench.get(item_class, ()):
                src = "legacy-crafted"  # an old master craft, not on today's bench
            too_high = item.item_level is not None and mod["required_level"] > item.item_level
            scored.append(((mod_id not in pob_ids, too_high, _SOURCE_RANK[src], -mod["required_level"]), mod_id, src))
        if not scored:
            return None
        scored.sort()
        best_rank, mod_id, src = scored[0]
        mod = self._affixes[mod_id]
        ties = sum(1 for rank, _, _ in scored[1:] if rank[:3] == best_rank[:3])
        return ModMatch(
            mod_id=mod_id,
            kind=mod["generation_type"],
            family=mod["type"],
            source=src,
            tier=self.tier(mod_id, tags) if src == "natural" else None,
            lines=lines,
            alternatives=ties,
        )


def _load_bench() -> dict[str, set[str]]:
    """Item class -> mod ids the crafting bench can currently add."""
    try:
        options = data.load("crafting_bench_options")
    except FileNotFoundError:
        return {}
    bench: dict[str, set[str]] = {}
    for option in options:
        mod_id = (option.get("actions") or {}).get("add_explicit_mod")
        if mod_id:
            for item_class in option.get("item_classes", []):
                bench.setdefault(item_class, set()).add(mod_id)
    return bench
