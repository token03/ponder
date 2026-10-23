"""The crafting actions available on an item, with costs in poe.ninja price ids."""

from __future__ import annotations

import re

from . import data
from .craft import MAGIC, NORMAL, RARE, Action, Mod

BASIC = [
    Action("transmute", "transmute", (("transmute", 1),), (NORMAL,)),
    Action("alt", "alt", (("alt", 1),), (MAGIC,)),
    Action("aug", "aug", (("aug", 1),), (MAGIC,)),
    Action("regal", "regal", (("regal", 1),), (MAGIC,)),
    Action("exalt", "exalt", (("exalted", 1),), (RARE,)),
    Action("annul", "annul", (("annul", 1),), (RARE,)),
    Action("alch", "reroll_rare", (("alch", 1),), (NORMAL,)),
    Action("chaos", "reroll_rare", (("chaos", 1),), (RARE,)),
    Action("scour+alch", "reroll_rare", (("scour", 1), ("alch", 1)), (RARE,)),
    Action("scour+transmute", "reroll_magic", (("scour", 1), ("transmute", 1)), (RARE,)),
]

# Harvest "Reforge a Rare item with new random modifiers, including a <tag> modifier".
# harvest tag -> (mod implicit tag, lifeforce price id, amount).
# Fire/cold/lightning costs are from Maxroll and PoEDB; the rest are unverified
# guesses, set to the dearest colour so they don't look better than they are.
HARVEST = {
    "fire": ("fire", "wild-lifeforce", 50),
    "cold": ("cold", "vivid-lifeforce", 50),
    "lightning": ("lightning", "primal-lifeforce", 50),
    "physical": ("physical", "vivid-lifeforce", 50),
    "chaos": ("chaos", "vivid-lifeforce", 50),
    "life": ("life", "vivid-lifeforce", 75),
    "defence": ("defences", "vivid-lifeforce", 50),
    "attack": ("attack", "vivid-lifeforce", 50),
    "caster": ("caster", "vivid-lifeforce", 50),
    "speed": ("speed", "vivid-lifeforce", 50),
    "critical": ("critical", "vivid-lifeforce", 50),
}
RESONATOR = "primitive-chaotic-resonator"

# Crafting bench costs are game metadata ids; these are their poe.ninja price ids.
BENCH_CURRENCY = {
    "CurrencyUpgradeToMagic": "transmute", "CurrencyRerollMagic": "alt", "CurrencyAddModToMagic": "aug",
    "CurrencyUpgradeMagicToRare": "regal", "CurrencyUpgradeToRare": "alch", "CurrencyRerollRare": "chaos",
    "CurrencyAddModToRare": "exalted", "CurrencyUpgradeRandomly": "chance", "CurrencyModValues": "divine",
    "CurrencyCorrupt": "vaal", "CurrencyConvertToNormal": "scour", "CurrencyInstillingOrb": "instilling-orb",
    "CurrencyRerollSocketNumbers": "jewellers", "CurrencyRerollSocketLinks": "fusing",
    "CurrencyRerollSocketColours": "chrome", "CurrencyArmourQuality": "scrap",
}  # fmt: skip
LOCK_PREFIXES = "StrMasterItemGenerationCannotChangePrefixes"  # a suffix
LOCK_SUFFIXES = "DexMasterItemGenerationCannotChangeSuffixes"  # a prefix
MULTIMOD = "StrIntMasterItemGenerationCanHaveMultipleCraftedMods"  # a suffix


def harvest(pool: list[Mod]) -> tuple[list[Action], dict[str, dict[str, float]]]:
    """Harvest reforges whose tag at least one mod in the pool carries."""
    acts, profiles = [], {}
    for name, (tag, lifeforce, amount) in HARVEST.items():
        tagged = {m.id: float(m.weight) for m in pool if tag in m.tags}
        if not tagged:
            continue
        profiles[f"harvest:{name}"] = tagged
        acts.append(Action(f"harvest {name}", "reroll_rare", ((lifeforce, amount),), (RARE,), first_profile=f"harvest:{name}"))
    return acts, profiles


def essences(item_class: str, item_level: int, min_level: int = 6) -> tuple[list[Action], dict[str, str]]:
    """Shrieking and Deafening essences that work on this item class.

    On a normal item an essence makes it rare; on a rare it needs a scour first.
    Returns the actions and the forced mod id of each.
    """
    acts, forced = [], {}
    for essence in data.load("essences").values():
        mod_id = essence["mods"].get(item_class)
        restriction = essence.get("item_level_restriction")
        if not mod_id or essence["level"] < min_level or (restriction and item_level > restriction):
            continue
        if essence["type"].get("is_corruption_only"):
            continue
        pid = _slug(essence["name"])
        name = essence["name"].replace("Essence of ", "").lower()
        acts.append(Action(f"{name} essence", "reroll_rare", ((pid, 1),), (NORMAL,), forced=mod_id))
        acts.append(Action(f"scour+{name} essence", "reroll_rare", (("scour", 1), (pid, 1)), (RARE,), forced=mod_id))
        forced[f"{name} essence"] = mod_id
    return acts, forced


def fossils(pool: list[Mod]) -> tuple[list[Action], dict[str, dict[str, float]]]:
    """Single fossils (with a Primitive Resonator) that change this pool's weights."""
    acts, profiles = [], {}
    for fossil in data.load("fossils").values():
        if not fossil["name"] or fossil["added_mods"] or fossil["forced_mods"] or fossil["mirrors"] or fossil["rolls_lucky"] or fossil["changes_quality"]:
            continue
        mult = {w["tag"]: w["weight"] / 100 for w in fossil["positive_mod_weights"] + fossil["negative_mod_weights"]}
        if not mult:
            continue
        weights = {}
        for m in pool:
            w = float(m.weight)
            for tag in m.tags:
                if tag in mult:
                    w *= mult[tag]
            weights[m.id] = w
        if all(abs(weights[m.id] - m.weight) < 1e-9 for m in pool):
            continue  # changes nothing here
        name = fossil["name"].replace(" Fossil", "").lower()
        profiles[f"fossil:{name}"] = weights
        pid = _slug(fossil["name"])
        acts.append(Action(f"{name} fossil", "reroll_rare", ((pid, 1), (RESONATOR, 1)), (NORMAL, RARE), profile=f"fossil:{name}"))
    return acts, profiles


def bench_cost(item_class: str, mod_id: str) -> tuple[tuple[str, float], ...] | None:
    """What the crafting bench charges to add `mod_id`, or None if it can't."""
    for option in data.load("crafting_bench_options"):
        if (option.get("actions") or {}).get("add_explicit_mod") == mod_id and item_class in option["item_classes"]:
            return tuple((BENCH_CURRENCY.get(k.rsplit("/", 1)[-1], k), float(n)) for k, n in option["cost"].items())
    return None


def metamods(item_class: str) -> list[Action]:
    """Lock one side with a bench metamod, then scour away the other side."""
    acts = []
    for mod_id, kind in ((LOCK_PREFIXES, "keep_prefixes"), (LOCK_SUFFIXES, "keep_suffixes")):
        cost = bench_cost(item_class, mod_id)
        if cost:
            name = "lock prefixes+scour" if kind == "keep_prefixes" else "lock suffixes+scour"
            acts.append(Action(name, kind, cost + (("scour", 1),), (RARE,)))
    return acts


def _slug(name: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", name.lower().replace("'", "")).strip("-")
