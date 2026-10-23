"""Find mod combinations that recur across many builds' rare items.

A candidate is an item class plus a set of mod families (e.g. Helmet with
IncreasedLife + FireResistance + ColdResistance). Tiers are ignored here; the
crafting solver decides later what tier is worth settling for.
"""

from __future__ import annotations

from collections import Counter, defaultdict
from dataclasses import dataclass
from itertools import combinations

from .mods import ItemMatch

NOTABLE_PREFIX = "1 Added Passive Skill is "


@dataclass
class Candidate:
    item_class: str
    families: tuple[str, ...]
    count: int  # rare items containing all of these families
    builds: int  # distinct builds those items came from


def mine(
    matches: list[tuple[str, ItemMatch]],
    min_size: int = 2,
    max_size: int = 4,
    min_builds: int = 2,
    closed: bool = True,
) -> list[Candidate]:
    """Count family subsets per item class.

    `matches` pairs each matched rare with an id for the build it came from.
    Rares have at most six explicit affixes, so enumerating subsets is cheap.
    With `closed`, a combo is dropped when a larger combo containing it is
    shared by exactly the same builds, since it adds nothing new.
    """
    counts: Counter = Counter()
    builds: dict = defaultdict(set)
    for build_id, match in matches:
        if match.item_class is None or match.item.rarity != "RARE":
            continue
        item_class, families = target_key(match)
        for size in range(min_size, min(max_size, len(families)) + 1):
            for combo in combinations(families, size):
                key = (item_class, combo)
                counts[key] += 1
                builds[key].add(build_id)

    keys = [key for key in counts if len(builds[key]) >= min_builds]
    if closed:
        keys = [key for key in keys if not _absorbed(key, keys, builds)]
    out = [Candidate(item_class=cls, families=combo, count=counts[(cls, combo)], builds=len(builds[(cls, combo)])) for cls, combo in keys]
    # Prefer combos many builds share, then bigger combos (more specific targets).
    out.sort(key=lambda c: (-c.builds, -len(c.families), -c.count, c.item_class, c.families))
    return out


def target_key(match: ItemMatch) -> tuple[str, list[str]]:
    """What a rare is "made of" for mining purposes.

    Cluster jewels are keyed by size and enchant, and only their notables
    count, since the other mods barely matter to buyers.
    """
    enchant = match.item.header.get("Cluster Jewel Skill")
    if enchant and match.item.base.endswith("Cluster Jewel"):
        size = match.item.base.split()[0].lower()
        notables = sorted(m.lines[0].text.removeprefix(NOTABLE_PREFIX) for m in match.mods if is_notable_line(m))
        return f"{size} cluster: {enchant}", notables
    return match.item_class, sorted({m.family for m in match.mods if not m.source.endswith("crafted")})


def is_notable_line(mod) -> bool:
    text = mod.lines[0].text
    return text.startswith(NOTABLE_PREFIX) and "Jewel Socket" not in text


def _absorbed(key, keys, builds) -> bool:
    cls, combo = key
    return any(
        other_cls == cls and len(other) > len(combo) and set(combo) <= set(other) and builds[(cls, other)] == builds[key]
        for other_cls, other in keys
    )
