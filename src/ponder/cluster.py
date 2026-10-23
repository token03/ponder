"""Cluster jewel mod pools.

A cluster jewel's enchant ("Added Small Passive Skills grant: 12% increased
Fire Damage") adds a tag such as `affliction_fire_damage`, and that tag sets
which notables and other mods can roll, and how often. The number of added
passives is fixed on the base, so crafting only changes the notables.
"""

from __future__ import annotations

from dataclasses import dataclass

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
    exact = [e for e in options if e.name.lower() == wanted or e.tag == wanted]
    hits = exact or [e for e in options if wanted in e.name.lower() or any(wanted in t.lower() for t in e.text)]
    if len(hits) != 1:
        names = ", ".join(e.name for e in (hits or options))
        raise ValueError(f"enchant {name!r} matches {len(hits)} of: {names}")
    return hits[0]


def is_notable(mod: dict) -> bool:
    return mod["text"].startswith(_NOTABLE_PREFIX) and "Jewel Socket" not in mod["text"]


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
    mods = pool(size, find_enchant(size, enchant), item_level)
    wanted = resolve_notables(mods, notables)
    fixed = resolve_notables(mods, list(fractured))
    need = need or len(wanted)
    if need > NOTABLE_CAP[size]:
        raise ValueError(f"a {size} cluster holds at most {NOTABLE_CAP[size]} notables")
    harvest, profiles = actions.harvest(mods)
    problem = Problem(
        pool=mods,
        targets=[Target(m.label, m.side, frozenset({m.id}), frozenset(m.groups), True) for m in wanted],
        need=need,
        actions=actions.BASIC + harvest,
        kind="jewel",
        notable_cap=NOTABLE_CAP[size],
        fixed=fixed,
        profiles=profiles,
    )
    return problem, solve(problem, prices)
