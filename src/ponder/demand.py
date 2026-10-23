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
from .item import ModLine
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
    bases: Counter = field(default_factory=Counter)  # which bases characters wear it on
    tiers: dict[str, Counter] = field(default_factory=dict)  # main stat -> tiers worn
    resistances: Counter = field(default_factory=Counter)  # resistance mods per item
    lift: float = 0.0  # items wearing all of it / items expected if its mods were worn independently

    @property
    def niche(self) -> float:
        """Specific demand: mods worn together far more than chance (one build's item), with real volume."""
        import math

        return math.log(max(self.lift, 1.0)) * math.sqrt(self.characters)

    def usual_tier(self, key: str, share: float = 0.5) -> int | None:
        """The tier at least `share` of the items reach: what a buyer of this archetype expects."""
        c = self.tiers.get(key)
        if not c:
            return None
        total, run = sum(c.values()), 0
        for t in sorted(c):
            run += c[t]
            if run >= share * total:
                return t
        return max(c)


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
    combos_of = []
    group_items: Counter = Counter()
    key_items: Counter = Counter()
    for w in worn:  # pass 1: who wears what (cheap), so pass 2 only details the survivors
        fams = stat_keys(w.match)
        group_items[w.group] += 1
        for k in fams:
            key_items[(w.group, k)] += 1
        mine = [(w.group, c) for size in range(min_size, min(max_size, len(fams)) + 1) for c in combinations(fams, size)]
        combos_of.append(mine)
        for key in mine:
            who[key].add(w.character)
    keys = [k for k, v in who.items() if len(v) >= min_characters]
    # Support only falls as a combination grows, so the best-supported bigger
    # combination is always one mod bigger: check those alone.
    best_super: Counter = Counter()
    for k in keys:
        if len(k[1]) > min_size:
            n = len(who[k])
            for sub in combinations(k[1], len(k[1]) - 1):
                best_super[(k[0], sub)] = max(best_super[(k[0], sub)], n)
    found = {k: Archetype(k[0], k[1], len(who[k]), 0) for k in keys if best_super[k] < implied * len(who[k])}
    for w, mine in zip(worn, combos_of):
        hits = [found[k] for k in mine if k in found]
        if not hits:
            continue
        best, n_res = best_tiers(w.match)
        for a in hits:
            a.items += 1
            a.skills[w.skill] += 1
            a.bases[w.match.item.base] += 1
            for k in a.families:
                if k in best:
                    a.tiers.setdefault(k, Counter())[best[k]] += 1
            if RES in a.families:
                a.resistances[n_res] += 1
            if len(a.examples) < 5:
                a.examples.append(w.match)
    for a in found.values():
        n = group_items[a.group]
        independent = n
        for k in a.families:
            independent *= key_items[(a.group, k)] / n
        a.lift = a.items / independent if independent else 0.0
    return sorted(found.values(), key=lambda a: (-a.characters, -len(a.families)))


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


def best_tiers(match: ItemMatch) -> tuple[dict[str, int], int]:
    """Best natural tier per main stat on the item, and how many resistance mods it has."""
    best: dict[str, int] = {}
    n_res = 0
    for m in match.mods:
        if m.source in ("crafted", "legacy-crafted"):
            continue
        if m.family in gear.FILLER_FAMILIES:
            n_res += 1
            continue
        if m.tier:
            k = stat_key(m.mod_id)
            best[k] = min(best.get(k, 99), m.tier)
    return best, n_res


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


# -- cluster jewels ---------------------------------------------------------------------


@dataclass
class WornCluster:
    character: str
    skill: str
    size: str  # small / medium / large
    enchant: str  # e.g. "12% increased Fire Damage"
    tag: str  # the enchant's spawn tag, e.g. affliction_fire_damage
    passives: int
    item_level: int
    notables: tuple[str, ...]  # notable names, sorted
    fractured: tuple[str, ...] = ()  # fractured notables among them


@dataclass
class ClusterArchetype:
    size: str
    enchant: str
    tag: str
    notables: tuple[str, ...]
    characters: int
    items: int
    skills: Counter = field(default_factory=Counter)
    passives: Counter = field(default_factory=Counter)  # passive counts worn
    item_levels: Counter = field(default_factory=Counter)
    full: int = 0  # items carrying exactly these notables and no other

    @property
    def title(self) -> str:
        return f"{self.size.title()} · {self.enchant}"

    @property
    def usual_passives(self) -> int:
        return self.passives.most_common(1)[0][0]


def wear_clusters(characters, skill_of=None) -> list[WornCluster]:
    """Every rare cluster jewel on `characters` with its enchant, passive count and notables."""
    from . import cluster, data
    from .ladder import rare_items

    mods = data.load("mods")
    by_text = {size: {frozenset(e.short.split(", ")): e for e in cluster.enchants(size)} for size in cluster.SIZES}
    out = []
    for c in characters:
        who = f"{c.get('account')}/{c.get('name')}"
        skill = skill_of(c) if skill_of else c.get("skill") or main_skill(c)
        for it in rare_items(c):
            base = it.get("baseType", "")
            if not base.endswith("Cluster Jewel"):
                continue
            size = base.split()[0].lower()
            lines = [ln for text in it.get("enchantMods") or [] for ln in text.splitlines() if ln.startswith("Added Small Passive Skills grant: ")]
            ench = by_text.get(size, {}).get(frozenset(ln.removeprefix("Added Small Passive Skills grant: ") for ln in lines))  # line order varies
            if ench is None:
                continue
            got = it.get("mods") or {}
            passives = next((m["stats"].get("local_jewel_expansion_passive_node_count") for m in got.get("enchant", []) if "stats" in m), None)
            notables, fractured = [], []
            for source in ("explicit", "fractured"):
                for m in got.get(source, []):
                    mod = mods.get(m.get("id"))
                    if mod and cluster.is_notable(mod):
                        notables.append(cluster.notable_name(mod))
                        if source == "fractured":
                            fractured.append(notables[-1])
            if notables and passives:
                out.append(WornCluster(who, skill, size, ench.short, ench.tag, int(passives), it.get("ilvl") or 0, tuple(sorted(notables)), tuple(sorted(fractured))))
    return out


def cluster_archetypes(worn: list[WornCluster], min_characters: int = 5, implied: float = 0.8) -> list[ClusterArchetype]:
    """Notable combinations per (size, enchant) that several characters wear.

    Every subset of a jewel's notables counts (a buyer of "Feed the Fury +
    Martial Prowess" takes a third notable too) down to one fewer than the
    size holds (2 of 3 on a large, 1 on a small); a subset is dropped when a
    bigger combination covers `implied` of its characters.
    """
    from .cluster import NOTABLE_CAP

    who: dict[tuple, set[str]] = defaultdict(set)
    rec: dict[tuple, ClusterArchetype] = {}
    for w in worn:
        for size in range(max(1, NOTABLE_CAP[w.size] - 1), len(w.notables) + 1):
            for combo in combinations(w.notables, size):
                key = (w.size, w.tag, combo)
                a = rec.get(key)
                if a is None:
                    a = rec[key] = ClusterArchetype(w.size, w.enchant, w.tag, combo, 0, 0)
                who[key].add(w.character)
                a.items += 1
                a.full += size == len(w.notables)
                a.skills[w.skill] += 1
                a.passives[w.passives] += 1
                a.item_levels[w.item_level] += 1
    keys = [k for k, v in who.items() if len(v) >= min_characters]
    bigger: dict[tuple, list] = defaultdict(list)
    for k in keys:
        bigger[k[:2]].append(k)
    out = []
    for k in keys:
        n = len(who[k])
        if any(len(o[2]) > len(k[2]) and set(k[2]) <= set(o[2]) and len(who[o]) >= implied * n for o in bigger[k[:2]]):
            continue
        a = rec[k]
        a.characters = n
        out.append(a)
    return sorted(out, key=lambda a: (-a.characters, -len(a.notables)))


def archetype_match(db: ModDB, a: Archetype) -> ItemMatch | None:
    """An item carrying exactly the archetype's mods, each at the tier most of its wearers reach,
    with the usual number of resistances; what `pipeline.estimate_gear` costs."""
    import dataclasses
    import statistics

    ex = a.examples[0]
    pool = {(db.mods[m.id]["type"], db.tier(m.id, ex.tags)): m.id for m in gear.pool(db, ex.base_id, ex.tags, NATURAL_LEVEL)}
    mods = []
    for key in a.families:
        if key == RES:
            continue
        found = [m for e in a.examples for m in e.mods if m.source not in ("crafted", "legacy-crafted") and m.tier and stat_key(m.mod_id) == key]
        if not found:
            return None
        m = min(found, key=lambda m: ("/" in _template(db, m.mod_id), m.tier))  # the plain mod when there is one
        tier = a.usual_tier(key) or m.tier
        mod_id = pool.get((m.family, tier), m.mod_id)
        mods.append(dataclasses.replace(m, mod_id=mod_id, tier=tier, lines=[ModLine(t) for t in db.mods[mod_id].get("text", "").splitlines()] or m.lines))
    if RES in a.families:
        want = max(1, round(statistics.median(a.resistances.elements()))) if a.resistances else 1
        seen = set()
        for e in a.examples:
            for m in e.mods:
                if m.family in gear.FILLER_FAMILIES and m.family not in seen and len(seen) < want and m.source == "natural":
                    seen.add(m.family)
                    mods.append(dataclasses.replace(m, tier=max(m.tier or 1, gear.FILLER_TIER)))
    sides = Counter(m.kind for m in mods)
    if sides["prefix"] > 3 or sides["suffix"] > 3:
        return None
    return dataclasses.replace(ex, mods=mods, unmatched=[], notes=[])
