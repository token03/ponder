"""The crafting actions available on an item, with costs in poe.ninja price ids."""

from __future__ import annotations

import re
from itertools import combinations

import numpy as np

from . import craft, data
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
# Chaotic resonators by socket count: a reroll with 1-4 fossils.
RESONATORS = {1: RESONATOR, 2: "potent-chaotic-resonator", 3: "powerful-chaotic-resonator", 4: "prime-chaotic-resonator"}
FOSSIL_COMBOS_KEPT = 10  # combo profiles handed to the solver

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


def _fossil_rows(pool: list[Mod]) -> list[tuple[str, str, np.ndarray]]:
    """(name, price id, per-mod weight multiplier) for every weight-changing fossil.

    A mod's multiplier is the product over its tags of the fossil's modifiers
    for them; fossils in one resonator multiply together.
    """
    rows = []
    for fossil in data.load("fossils").values():
        if not fossil["name"] or fossil["added_mods"] or fossil["forced_mods"] or fossil["mirrors"] or fossil["rolls_lucky"] or fossil["changes_quality"]:
            continue
        mult = {w["tag"]: w["weight"] / 100 for w in fossil["positive_mod_weights"] + fossil["negative_mod_weights"]}
        if not mult:
            continue
        row = np.array([float(np.prod([mult[t] for t in m.tags if t in mult])) for m in pool])
        if np.allclose(row, 1.0):
            continue  # changes nothing here
        rows.append((fossil["name"].replace(" Fossil", "").lower(), _slug(fossil["name"]), row))
    return rows


def fossil_combos(
    pool: list[Mod],
    wants: list[frozenset[str]],
    prices: dict[str, float],
    *,
    mods_per_side: float = 2.3,
    max_fossils: int = 4,
    keep: int = FOSSIL_COMBOS_KEPT,
) -> tuple[list[Action], dict[str, dict[str, float]]]:
    """The fossil combinations (1-4 in a chaotic resonator) worth giving the solver.

    Every combination is scored at once in numpy: for each wanted mod (a set
    of acceptable ids), the chance a reroll hits it is about
    1 - (1 - its share of its side's weight) ** mods_per_side, and a set of
    wants is hit with the product. For every subset of the wants, the
    combination with the lowest price per hit is kept (a reroll that lands
    two prefixes is worth having even when exalts finish the item), largest
    subsets first, up to `keep`. The solver then values them exactly.
    """
    rows = [(n, pid, r) for n, pid, r in _fossil_rows(pool) if pid in prices]
    wants = [w for w in wants if w]
    if not rows or not wants:
        return [], {}
    base = np.array([float(m.weight) for m in pool])
    index = {m.id: i for i, m in enumerate(pool)}
    hit = np.zeros((len(wants), len(pool)))
    for i, w in enumerate(wants):
        hit[i, [index[m] for m in w if m in index]] = 1.0
    side = np.array([[m.side == s for m in pool] for s in ("prefix", "suffix")], dtype=float)
    want_side = np.array([0 if pool[int(np.argmax(h))].side == "prefix" else 1 for h in hit])
    mult = np.array([r for _, _, r in rows])
    fossil_price = np.array([prices[pid] for _, pid, _ in rows])
    combos, scores = [], []
    subsets = [s for k in range(len(wants), 0, -1) for s in combinations(range(len(wants)), k)]
    for k in range(1, min(max_fossils, len(rows)) + 1):
        if RESONATORS[k] not in prices:
            continue
        idx = np.array(list(combinations(range(len(rows)), k)))
        weights = base * mult[idx].prod(axis=1)  # (combos, mods)
        totals = weights @ side.T  # (combos, 2)
        share = np.divide(weights @ hit.T, totals[:, want_side], out=np.zeros((len(idx), len(wants))), where=totals[:, want_side] > 0)
        p = 1 - (1 - np.clip(share, 0, 1)) ** mods_per_side  # (combos, wants)
        clicks = (2 + craft.FOSSIL_SECONDS * k) * craft.CHAOS_PER_HOUR / 3600
        cost = fossil_price[idx].sum(axis=1) + prices[RESONATORS[k]] + clicks
        combos.append(idx)
        scores.append((cost, p))
    picked: list[tuple[int, ...]] = []
    for subset in subsets:
        best, best_score = None, np.inf
        for idx, (cost, p) in zip(combos, scores):
            per_hit = cost / np.maximum(p[:, list(subset)].prod(axis=1), 1e-12)
            i = int(np.argmin(per_hit))
            if per_hit[i] < best_score:
                best, best_score = tuple(int(x) for x in idx[i]), per_hit[i]
        if best is not None and best_score < 1e11 and best not in picked:
            picked.append(best)
        if len(picked) >= keep:
            break
    acts, profiles = [], {}
    for combo in picked:
        name = "+".join(rows[i][0] for i in combo)
        weights = base * np.prod([rows[i][2] for i in combo], axis=0)
        profiles[f"fossil:{name}"] = {m.id: float(w) for m, w in zip(pool, weights)}
        cost = tuple((rows[i][1], 1) for i in combo) + ((RESONATORS[len(combo)], 1),)
        acts.append(Action(f"{name} fossil{'s' if len(combo) > 1 else ''}", "reroll_rare", cost, (NORMAL, RARE), profile=f"fossil:{name}"))
    return acts, profiles


def bench_cost(item_class: str, mod_id: str) -> tuple[tuple[str, float], ...] | None:
    """What the crafting bench charges to add `mod_id`, or None if it can't."""
    for option in data.load("crafting_bench_options"):
        if (option.get("actions") or {}).get("add_explicit_mod") == mod_id and item_class in option["item_classes"]:
            return tuple((BENCH_CURRENCY.get(k.rsplit("/", 1)[-1], k), float(n)) for k, n in option["cost"].items())
    return None


def metamods(item_class: str, rerolls: list[Action] = ()) -> list[Action]:
    """Lock one side with a bench metamod, then scour away the other side, or reroll it.

    `rerolls` are reforges (e.g. Harvest) worth trying on a locked item; a
    chaos orb is always tried. Each becomes "lock <side>+<reroll>".
    """
    acts = []
    for mod_id, side in ((LOCK_PREFIXES, "prefixes"), (LOCK_SUFFIXES, "suffixes")):
        cost = bench_cost(item_class, mod_id)
        if not cost:
            continue
        acts.append(Action(f"lock {side}+scour", f"keep_{side}", cost + (("scour", 1),), (RARE,)))
        acts.append(Action(f"lock {side}+chaos", f"lockreroll_{side}", cost + (("chaos", 1),), (RARE,)))
        for r in rerolls:
            acts.append(Action(f"lock {side}+{r.name}", f"lockreroll_{side}", cost + r.cost, (RARE,), profile=r.profile, first_profile=r.first_profile))
    return acts


ELDRITCH_CLASSES = {"Helmet", "Body Armour", "Gloves", "Boots"}


def eldritch(item_class: str, influences=()) -> list[Action]:
    """Eldritch Exalted Orb / Orb of Annulment: add or remove a mod on one side only.

    They need eldritch implicits (Exarch/Eater, from embers and ichors) whose
    dominance picks the side, and don't work on influenced items. Setting up
    the implicits is cheap next to the orbs and isn't counted.
    """
    if item_class not in ELDRITCH_CLASSES or influences:
        return []
    return [
        Action("eldritch exalt (prefix)", "exalt_prefix", (("eldritch-exalted-orb", 1),), (RARE,)),
        Action("eldritch exalt (suffix)", "exalt_suffix", (("eldritch-exalted-orb", 1),), (RARE,)),
        Action("eldritch annul (prefix)", "annul_prefix", (("eldritch-orb-of-annulment", 1),), (RARE,)),
        Action("eldritch annul (suffix)", "annul_suffix", (("eldritch-orb-of-annulment", 1),), (RARE,)),
    ]


def _slug(name: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", name.lower().replace("'", "")).strip("-")
