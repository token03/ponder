"""What invested characters actually wear: mod and tier demand from ladder rares.

Two views, both per base group (bases that roll the same mods, see `gear.group`):

- `family_stats`: for each mod family, how many characters wear it, which
  tiers, and the *tier lift*: the ladder's share at a tier or better divided
  by the chance a roll of that family lands there. A lift far above 1 means
  people paid for the top tiers (e.g. T1 ES on ES bases); near 1 means tier
  barely matters to them.
- `archetypes`: mod-family combinations that many characters share, with
  resistances folded into one "resistances" token since they're filler.
  These are the crafting candidates: "Gloves (ES): flat ES + %ES + res".
"""

from __future__ import annotations

import re
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from itertools import combinations

from . import gear
from .mods import ItemMatch, ModDB

RES = "resistances"
NATURAL_LEVEL = 86


@dataclass
class Worn:
    character: str  # account/name
    skill: str
    match: ItemMatch
    group: str


@dataclass
class FamilyStat:
    group: str
    family: str
    side: str
    text: str  # e.g. "+# to maximum Energy Shield"
    characters: int
    tiers: Counter = field(default_factory=Counter)  # tier -> items
    natural: dict[int, float] = field(default_factory=dict)  # tier -> chance a roll of the family lands on it

    def share_at_or_above(self, tier: int) -> float:
        total = sum(self.tiers.values())
        return sum(n for t, n in self.tiers.items() if t <= tier) / total if total else 0.0

    def natural_at_or_above(self, tier: int) -> float:
        return sum(p for t, p in self.natural.items() if t <= tier)

    def lift(self, tier: int = 1) -> float:
        nat = self.natural_at_or_above(tier)
        return self.share_at_or_above(tier) / nat if nat else 0.0


@dataclass
class Archetype:
    group: str
    families: tuple[str, ...]
    characters: int
    items: int
    skills: Counter = field(default_factory=Counter)
    examples: list[ItemMatch] = field(default_factory=list)


def wear(db: ModDB, characters, skill_of=None) -> list[Worn]:
    """Every rare gear item and jewel on `characters` (poe.ninja JSON), matched by mod id."""
    from .ladder import rare_items

    out = []
    for data in characters:
        who = f"{data.get('account')}/{data.get('name')}"
        skill = skill_of(data) if skill_of else data.get("skill") or main_skill(data)
        for it in rare_items(data):
            influences = {k for k, v in (it.get("influences") or {}).items() if v}
            m = db.match_ids(it.get("baseType", ""), it.get("mods") or {}, it.get("ilvl"), influences)
            if m.base_id is None or not m.mods or m.item.base.endswith("Cluster Jewel"):
                continue
            out.append(Worn(who, skill, m, gear.group(db, m).label))
    return out


def main_skill(data: dict) -> str:
    """The character's main skill: the first active gem of the first skill group with DPS numbers."""
    groups = data.get("skills") or []
    for group in sorted(groups, key=lambda g: not g.get("dps")):
        for gem in group.get("allGems") or []:
            name = gem.get("name") or ""
            if name and not name.endswith("Support"):
                return name
    return "?"


def family_stats(db: ModDB, worn: list[Worn], min_characters: int = 10) -> list[FamilyStat]:
    stats: dict[tuple[str, str], FamilyStat] = {}
    who: dict[tuple[str, str], set[str]] = defaultdict(set)
    example: dict[str, ItemMatch] = {}
    for w in worn:
        example.setdefault(w.group, w.match)
        for m in w.match.mods:
            if m.source != "natural" or not m.tier:
                continue
            key = (w.group, m.family)
            if key not in stats:
                stats[key] = FamilyStat(w.group, m.family, m.kind, _template(db, m.mod_id), 0)
            stats[key].tiers[m.tier] += 1
            who[key].add(w.character)
    out = []
    for key, s in stats.items():
        s.characters = len(who[key])
        if s.characters < min_characters:
            continue
        s.natural = natural_tiers(db, example[s.group], s.family)
        out.append(s)
    return sorted(out, key=lambda s: (s.group, -s.characters))


_natural: dict[tuple, dict[str, dict[int, float]]] = {}


def natural_tiers(db: ModDB, match: ItemMatch, family: str) -> dict[int, float]:
    """Chance that a roll of `family` lands on each tier, on this item's mod pool at ilvl 86."""
    key = (match.item_class, frozenset(match.tags))
    if key not in _natural:  # one pool scan per base group, every family at once
        weights: dict[str, Counter] = defaultdict(Counter)
        for m in gear.pool(db, match.base_id, match.tags, NATURAL_LEVEL):
            if t := db.tier(m.id, match.tags):
                weights[db.mods[m.id]["type"]][t] += m.weight
        _natural[key] = {f: {t: w / sum(c.values()) for t, w in c.items()} for f, c in weights.items()}
    return _natural[key].get(family, {})


def archetypes(worn: list[Worn], min_characters: int = 8, max_size: int = 4, min_size: int = 2, implied: float = 0.7) -> list[Archetype]:
    """Mod combinations shared by many characters, per base group.

    Mods are keyed by main stat (`stat_key`), resistances folded into one
    token. A combination is dropped when one bigger combination is worn by at
    least `implied` of its characters: it is mostly that one.
    """
    who: dict[tuple, set[str]] = defaultdict(set)
    items: Counter = Counter()
    skills: dict[tuple, Counter] = defaultdict(Counter)
    examples: dict[tuple, list] = defaultdict(list)
    for w in worn:
        fams = stat_keys(w.match)
        for size in range(min_size, min(max_size, len(fams)) + 1):
            for combo in combinations(fams, size):
                key = (w.group, combo)
                who[key].add(w.character)
                items[key] += 1
                skills[key][w.skill] += 1
                if len(examples[key]) < 5:
                    examples[key].append(w.match)
    keys = [k for k, v in who.items() if len(v) >= min_characters]
    by_group: dict[str, list] = defaultdict(list)
    for k in keys:
        by_group[k[0]].append(k)
    out = []
    for k in keys:
        n = len(who[k])
        if any(len(o[1]) > len(k[1]) and set(k[1]) <= set(o[1]) and len(who[o]) >= implied * n for o in by_group[k[0]]):
            continue
        out.append(Archetype(k[0], k[1], len(who[k]), items[k], skills[k], examples[k]))
    return sorted(out, key=lambda a: (-a.characters, -len(a.families)))


_stat_key: dict[str, str] = {}


def stat_key(mod_id: str) -> str:
    """What a mod is "for": its first stat. A hybrid "%phys / accuracy" keys as %phys, and
    "%ES / stun recovery" as %ES; flat ES and %ES stay apart (different stats)."""
    if mod_id not in _stat_key:
        from . import data

        stats = data.load("mods")[mod_id].get("stats") or [{"id": mod_id}]
        _stat_key[mod_id] = stats[0]["id"]
    return _stat_key[mod_id]


def stat_keys(match: ItemMatch) -> tuple[str, ...]:
    """The item's natural and essence mods by main stat, resistances folded into one token."""
    keys = set()
    for m in match.mods:
        if m.source in ("crafted", "legacy-crafted"):
            continue
        keys.add(RES if m.family in gear.FILLER_FAMILIES else stat_key(m.mod_id))
    return tuple(sorted(keys))


def core_families(match: ItemMatch) -> tuple[str, ...]:
    """The item's natural and essence mod families, resistances folded into one token."""
    fams = set()
    for m in match.mods:
        if m.source in ("crafted", "legacy-crafted"):
            continue
        fams.add(RES if m.family in gear.FILLER_FAMILIES else m.family)
    return tuple(sorted(fams))


def _template(db: ModDB, mod_id: str) -> str:
    """A mod's text with numbers as #, every line (hybrids show both halves)."""
    text = " / ".join(db.mods[mod_id].get("text", mod_id).splitlines())
    return re.sub(r"\(?-?\d+(\.\d+)?(-\d+(\.\d+)?)?\)?", "#", text)
