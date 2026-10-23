"""Cluster jewel mod pools.

A cluster jewel's enchant ("Added Small Passive Skills grant: 12% increased
Fire Damage") adds a tag such as `affliction_fire_damage`, and that tag sets
which notables and other mods can roll, and how often. The number of added
passives is fixed on the base, so crafting only changes the mods.

A craft wants notables (by name) and/or tiered small-passive mods, written
"Group@tier": "HaveIncreasedEffect@1" is 35% increased Effect, and
"GrantAttributes@2" is +3 to All Attributes or better. "Passive point" jewels
(a 12-passive large with Effect and three "also grant" mods, no notables) are
crafts of those.
"""

from __future__ import annotations

from collections import defaultdict
from dataclasses import dataclass
from functools import cache

from . import actions, data
from .craft import Mod, Plan, Problem, Target, solve

SIZES = {
    "small": "Metadata/Items/Jewels/JewelPassiveTreeExpansionSmall",
    "medium": "Metadata/Items/Jewels/JewelPassiveTreeExpansionMedium",
    "large": "Metadata/Items/Jewels/JewelPassiveTreeExpansionLarge",
}
# Most notables a cluster of each size can hold (community knowledge, not in the data).
NOTABLE_CAP = {"small": 1, "medium": 2, "large": 3}
# Passive counts with the fewest small passives, which buyers want.
DEFAULT_PASSIVES = {"small": 2, "medium": 4, "large": 8}
_ENCHANT_PREFIX = "Added Small Passive Skills grant: "
_NOTABLE_PREFIX = "1 Added Passive Skill is "
GROUP_PREFIX = "AfflictionJewelSmallPassives"


@dataclass
class Enchant:
    id: str
    name: str
    tag: str
    text: list[str]

    @property
    def short(self) -> str:
        """The enchant as poe.ninja names it, e.g. "12% increased Cold Damage"."""
        return ", ".join(t.removeprefix(_ENCHANT_PREFIX) for t in self.text)


def enchants(size: str) -> list[Enchant]:
    jewel = data.load("cluster_jewels")[SIZES[size]]
    return [Enchant(p["id"], p["name"], p["tag"], p["stat_text"]) for p in jewel["passive_skills"]]


def find_enchant(size: str, name: str) -> Enchant:
    options = enchants(size)
    wanted = name.lower()
    exact = [e for e in options if e.name.lower() == wanted or e.short.lower() == wanted] or [e for e in options if e.tag == wanted]
    hits = exact or [e for e in options if wanted in e.name.lower() or any(wanted in t.lower() for t in e.text)]
    if len(hits) != 1:
        names = ", ".join(e.name for e in (hits or options))
        raise ValueError(f"enchant {name!r} matches {len(hits)} of: {names}")
    return hits[0]


def is_notable(mod: dict) -> bool:
    text = mod.get("text") or ""
    return text.startswith(_NOTABLE_PREFIX) and "Jewel Socket" not in text


def notable_name(mod: dict) -> str:
    return mod["text"][len(_NOTABLE_PREFIX):]


def pool(size: str, enchant: Enchant, item_level: int = 84) -> list[Mod]:
    """Every mod that can roll on this cluster, with its weight."""
    tags = set(data.load("base_items")[SIZES[size]]["tags"]) | {enchant.tag}
    out = []
    for mod_id, mod in data.load("mods").items():
        if mod["domain"] != "affliction_jewel" or mod["generation_type"] not in ("prefix", "suffix"):
            continue
        if mod["required_level"] > item_level:
            continue
        weight = next((w["weight"] for w in mod["spawn_weights"] if w["tag"] in tags), 0)
        if weight <= 0:
            continue
        label = notable_name(mod) if is_notable(mod) else mod["text"].replace("\n", " / ")
        out.append(
            Mod(mod_id, mod["generation_type"], weight, tuple(mod["groups"]), is_notable(mod), label, tuple(mod["implicit_tags"]))
        )
    return out


def tiers(mods: list[Mod]) -> dict[str, int]:
    """Tier of each non-notable mod in a pool: 1 is its group's highest-level roll."""
    levels = data.load("mods")
    by_group: dict[str, list[Mod]] = defaultdict(list)
    for m in mods:
        if not m.notable and m.groups:
            by_group[m.groups[0]].append(m)
    out = {}
    for group in by_group.values():
        ranks = sorted({levels[m.id]["required_level"] for m in group}, reverse=True)
        out.update({m.id: ranks.index(levels[m.id]["required_level"]) + 1 for m in group})
    return out


@cache
def mod_tiers(size: str, tag: str) -> dict[str, int]:
    return tiers(pool(size, Enchant(tag, tag, tag, [])))  # the pool depends only on the tag


def feature(mod_id: str, tier: int) -> str:
    """A tiered small-passive mod as a craft names it, e.g. "HaveIncreasedEffect@1"."""
    return f"{data.load('mods')[mod_id]['groups'][0].removeprefix(GROUP_PREFIX)}@{tier}"


def is_feature(name: str) -> bool:
    return "@" in name


def _parse(name: str) -> tuple[str, int]:
    group, tier = name.rsplit("@", 1)
    return GROUP_PREFIX + group, int(tier)


def feature_label(name: str, mods: list[Mod] | None = None) -> str:
    """ "HaveIncreasedEffect@1" -> "35% increased Effect"; tiers past 1 read "(T2+)"."""
    if not is_feature(name):
        return name
    group, tier = _parse(name)
    texts = data.load("mods")
    hits = [m for m in mods or [] if m.groups and m.groups[0] == group]
    ranked = tiers(hits) if hits else {}
    mod_id = next((m for m, t in ranked.items() if t == tier), None) or next((m for m, mod in texts.items() if group in mod["groups"]), None)
    text = (texts[mod_id]["text"] or group) if mod_id else group
    text = text.replace("Added Small Passive Skills also grant: ", "").replace("Added Small Passive Skills have ", "").replace("\n", " / ")
    return text + (f" (T{tier}+)" if tier > 1 else "")


def resolve(mods: list[Mod], names: list[str]) -> list[Target]:
    """Targets for notable names and "Group@tier" small-passive mods."""
    ranked = tiers(mods)
    out = []
    for name in names:
        if not is_feature(name):
            m = resolve_notables(mods, [name])[0]
            out.append(Target(m.label, m.side, frozenset({m.id}), frozenset(m.groups), True))
            continue
        group, tier = _parse(name)
        good = [m for m in mods if m.groups and m.groups[0] == group and ranked.get(m.id, 99) <= tier]
        if not good:
            raise ValueError(f"{name!r} can't roll here")
        out.append(Target(feature_label(name, mods), good[0].side, frozenset(m.id for m in good), frozenset(g for m in good for g in m.groups)))
    return out


def resolve_fixed(mods: list[Mod], names: list[str]) -> list[Mod]:
    """Fractured mods: notables by name, small-passive mods at exactly their tier."""
    ranked = tiers(mods)
    out = []
    for name in names:
        if not is_feature(name):
            out += resolve_notables(mods, [name])
            continue
        group, tier = _parse(name)
        hit = next((m for m in mods if m.groups and m.groups[0] == group and ranked.get(m.id) == tier), None)
        if hit is None:
            raise ValueError(f"{name!r} can't roll here")
        out.append(hit)
    return out


def resolve_notables(mods: list[Mod], names: list[str]) -> list[Mod]:
    by_name = {m.label.lower(): m for m in mods if m.notable}
    out = []
    for name in names:
        hit = by_name.get(name.strip().lower())
        if hit is None:
            raise ValueError(f"notable {name!r} can't roll here; options: {', '.join(sorted(by_name))}")
        out.append(hit)
    return out


def cost(
    size: str,
    enchant: str,
    notables: list[str],
    prices: dict[str, float],
    need: int | None = None,
    item_level: int = 84,
    fractured: list[str] = (),
) -> tuple[Problem, Plan]:
    """Cheapest plan for `need` of `notables`, over basic currency and Harvest reforges.

    `fractured` notables are already on the base and survive every reroll.
    """
    p = problem(size, enchant, notables, need, item_level, fractured)
    return p, solve(p, prices)


def problem(
    size: str,
    enchant: str,
    notables: list[str],
    need: int | None = None,
    item_level: int = 84,
    fractured: list[str] = (),
) -> Problem:
    """The crafting problem `cost` solves, for solving elsewhere (e.g. in a process pool)."""
    mods = pool(size, find_enchant(size, enchant), item_level)
    targets = resolve(mods, notables)
    fixed = resolve_fixed(mods, list(fractured))
    need = need or len(targets)
    if sum(t.notable for t in targets) > NOTABLE_CAP[size]:
        raise ValueError(f"a {size} cluster holds at most {NOTABLE_CAP[size]} notables")
    harvest, profiles = actions.harvest(mods)
    return Problem(
        pool=mods,
        targets=targets,
        need=need,
        actions=actions.BASIC + harvest,
        kind="jewel",
        notable_cap=NOTABLE_CAP[size],
        fixed=fixed,
        profiles=profiles,
    )
