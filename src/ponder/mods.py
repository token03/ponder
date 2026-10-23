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

# Base domains we match items for; jewels and flasks keep their mods in their own domain.
BASE_DOMAINS = {"item", "misc", "abyss_jewel", "affliction_jewel", "flask"}
# Mod domains that apply to ordinary (`item` domain) bases, and the source they imply.
ITEM_EXTRA_DOMAINS = {"crafted": "crafted", "unveiled": "veiled", "delve": "delve"}

# Lower sorts first when several mods fit a line.
_SOURCE_RANK = {
    "natural": 0,
    "crafted": 0,
    "essence": 1,
    "veiled": 1,
    "delve": 1,
    "legacy-crafted": 1,
    "special": 2,
}


@dataclass
class ModMatch:
    mod_id: str
    kind: str  # "prefix" or "suffix"
    family: str  # repoe mod `type`, shared by every tier of a mod
    source: str  # natural | essence | veiled | delve | special | crafted | legacy-crafted
    tier: int | None  # 1 = best natural tier on this base
    lines: list[ModLine]
    alternatives: int = 0  # other mods that fit the same lines equally well
    merged: bool = False  # shares a summed stat line with another mod

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
    unmatched: list[ModLine] = field(default_factory=list)  # look like mods, but no mod fits
    notes: list[ModLine] = field(default_factory=list)  # text no mod has, e.g. author notes

    @property
    def prefixes(self) -> list[ModMatch]:
        return [m for m in self.mods if m.kind == "prefix"]

    @property
    def suffixes(self) -> list[ModMatch]:
        return [m for m in self.mods if m.kind == "suffix"]


def normalize(text: str) -> str:
    return _TOKEN.sub("#", text).lower()


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


@dataclass
class _Context:
    item: Item
    base_domain: str
    item_class: str
    tags: set[str]
    pob_ids: set[str]


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
            if (mod["domain"] in BASE_DOMAINS or mod["domain"] in ITEM_EXTRA_DOMAINS)
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
            if base.get("domain") not in BASE_DOMAINS or not base.get("name"):
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
        if mod["domain"] in ITEM_EXTRA_DOMAINS:
            return ITEM_EXTRA_DOMAINS[mod["domain"]]
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
                    if m["domain"] == mod["domain"]
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
        ctx = _Context(item, self.bases[base_id]["domain"], result.item_class, tags, {i for _, i in item.pob_affixes})

        remaining = list(item.explicits)
        # Hybrids first, so their lines aren't claimed one by one.
        for line in list(remaining):
            if line not in remaining:
                continue
            for key in self._hybrids_by_line.get(normalize(line.text), []):
                claimed = self._claim(key, remaining)
                if claimed is None:
                    continue
                found = self._best(key, claimed, ctx)
                if found:
                    result.mods.append(found)
                    for c in claimed:
                        remaining.remove(c)
                    break

        for line in list(remaining):
            found = self._best((normalize(line.text),), [line], ctx)
            if found:
                result.mods.append(found)
                remaining.remove(line)

        for line in list(remaining):
            if self._split(line, result, ctx, remaining):
                remaining.remove(line)
        result.notes = [line for line in remaining if not self._known(line)]
        result.unmatched = [line for line in remaining if self._known(line)]
        return result

    def _known(self, line: ModLine) -> bool:
        norm = normalize(line.text)
        return (norm,) in self._index or norm in self._hybrids_by_line

    def _split(self, line: ModLine, result: ItemMatch, ctx: _Context, remaining: list[ModLine]) -> bool:
        """Explain a line the game summed from a hybrid mod plus a single mod.

        E.g. `140% increased Energy Shield` = hybrid `(39-42)% ES / Stun
        Recovery` + `(92-100)% ES`. The hybrid's other lines must be on the item;
        if they were matched to a single mod already, the hybrid takes them over.
        """
        norm = normalize(line.text)
        value = line_values(line)
        for key in self._hybrids_by_line.get(norm, []):
            others = list(key)
            others.remove(norm)
            taken = {id(l): m for m in result.mods for l in m.lines if len(m.lines) == 1}
            pool = [l for l in ctx.item.explicits if l is not line and (id(l) in taken or l in remaining)]
            claimed = self._claim(tuple(others), pool)
            if claimed is None:
                continue
            for hybrid_id in self._candidates(key, ctx, crafted=line.crafted):
                hybrid = self._affixes[hybrid_id]
                h_lines = hybrid["text"].splitlines()
                h_other = [t for t in h_lines if normalize(t) != norm]
                if not all(fits(line_values(l), template_ranges(t)) for l, t in zip(claimed, h_other)):
                    continue
                h_range = template_ranges(next(t for t in h_lines if normalize(t) == norm))
                for single_id in self._candidates((norm,), ctx, crafted=line.crafted):
                    s_range = template_ranges(self._affixes[single_id]["text"])
                    summed = [(a[0] + b[0], a[1] + b[1]) for a, b in zip(h_range, s_range)]
                    if not fits(value, summed):
                        continue
                    for l in claimed:
                        if id(l) in taken and taken[id(l)] in result.mods:
                            result.mods.remove(taken[id(l)])
                    ordered = [line if normalize(t) == norm else claimed[h_other.index(t)] for t in h_lines]
                    result.mods.append(self._make(hybrid_id, ordered, ctx, merged=True))
                    result.mods.append(self._make(single_id, [line], ctx, merged=True))
                    return True
        return False

    @staticmethod
    def _claim(key: tuple[str, ...], lines: list[ModLine]) -> list[ModLine] | None:
        claimed = []
        for norm in key:
            line = next((l for l in lines if l not in claimed and normalize(l.text) == norm), None)
            if line is None:
                return None
            claimed.append(line)
        return claimed

    def _candidates(self, key: tuple[str, ...], ctx: _Context, crafted: bool) -> list[str]:
        """Mods with this template that could be on the item, best first."""
        out = []
        for mod_id in self._index.get(key, []):
            mod = self._affixes[mod_id]
            if (mod["domain"] == "crafted") != crafted:
                continue
            if mod["domain"] != ctx.base_domain and not (
                mod["domain"] in ITEM_EXTRA_DOMAINS and ctx.base_domain == "item"
            ):
                continue
            out.append(mod_id)
        return sorted(out, key=lambda mod_id: self._rank(mod_id, ctx))

    def _source(self, mod_id: str, ctx: _Context) -> str:
        src = self.source(self._affixes[mod_id], ctx.tags)
        if src == "crafted" and mod_id not in self._bench.get(ctx.item_class, ()):
            return "legacy-crafted"  # an old master craft, not on today's bench
        return src

    def _rank(self, mod_id: str, ctx: _Context) -> tuple:
        mod = self._affixes[mod_id]
        too_high = ctx.item.item_level is not None and mod["required_level"] > ctx.item.item_level
        return (mod_id not in ctx.pob_ids, too_high, _SOURCE_RANK[self._source(mod_id, ctx)], -mod["required_level"])

    def _best(self, key, lines, ctx: _Context) -> ModMatch | None:
        values = [line_values(line) for line in lines]
        fitting = [
            mod_id
            for mod_id in self._candidates(key, ctx, crafted=any(line.crafted for line in lines))
            if all(fits(v, template_ranges(t)) for v, t in zip(values, self._affixes[mod_id]["text"].splitlines()))
        ]
        if not fitting:
            return None
        best = self._rank(fitting[0], ctx)[:3]
        ties = sum(1 for mod_id in fitting[1:] if self._rank(mod_id, ctx)[:3] == best)
        return self._make(fitting[0], lines, ctx, alternatives=ties)

    def _make(self, mod_id, lines, ctx: _Context, alternatives: int = 0, merged: bool = False) -> ModMatch:
        mod = self._affixes[mod_id]
        src = self._source(mod_id, ctx)
        return ModMatch(
            mod_id=mod_id,
            kind=mod["generation_type"],
            family=mod["type"],
            source=src,
            tier=self.tier(mod_id, ctx.tags) if src == "natural" else None,
            lines=lines,
            alternatives=alternatives,
            merged=merged,
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
