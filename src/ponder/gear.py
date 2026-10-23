"""Turn a matched rare item into a crafting problem.

Each natural mod on the item becomes a target that any tier at least as good
satisfies (`settle` loosens that by N tiers). Fractured mods are kept fixed.
Essence-only mods become targets only an essence can hit. Crafted mods the
bench still offers become a last bench step: the plan keeps a slot open on
their side and pays the bench cost. Veiled and other special mods are left
for the user.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from functools import cache

from . import actions, data
from .craft import Mod, Plan, Problem, Target, solve
from .mods import ItemMatch, ModDB

JEWEL_CLASSES = {"Jewel", "AbyssJewel"}
DEFAULT_ITEM_LEVEL = 84


DEFENCES = {"str": "Armour", "dex": "Evasion", "int": "ES"}
# Resistances are filler: buyers want enough of them, rarely a particular one.
FILLER_FAMILIES = frozenset({"FireResistance", "ColdResistance", "LightningResistance", "AllResistances", "ChaosResistance"})
FILLER_TIER = 3  # filler at this tier or better counts (or the item's own tier, if worse)
_DEFENCE_TAG = re.compile(r"^((?:str|dex|int)(?:_(?:str|dex|int))*)_(armour|shield)$")


@dataclass(frozen=True)
class Row:
    """One mod line of a craft, for display."""

    role: str  # target | essence | fractured | bench | skipped
    side: str  # prefix | suffix
    text: str
    tier: str = ""  # e.g. "T2+"


@dataclass
class BaseGroup:
    """Bases that roll exactly the same mods with the same weights: one craft for all of them."""

    item_class: str
    signature: frozenset[str]  # the base's tags that some mod's spawn weight uses
    label: str  # e.g. "Gloves (ES)", "Ring", "Cobalt Jewel"
    bases: list[str]  # names, best first (see `group`)

    @property
    def best(self) -> str:
        """The base to craft on and sell: the same mods roll on it, and it sells for more."""
        return self.bases[0]


@dataclass
class GearProblem:
    problem: Problem
    item_level: int
    skipped: list[str] = field(default_factory=list)  # mods we don't try to craft
    bench: list[tuple[str, tuple[tuple[str, float], ...]]] = field(default_factory=list)  # (label, cost) crafted last
    rows: list[Row] = field(default_factory=list)
    bench_ids: list[str] = field(default_factory=list)
    filler_families: list[str] = field(default_factory=list)  # families of the item's filler mods

    def bench_cost(self, prices: dict[str, float]) -> float:
        return sum(action_cost_of(cost, prices) for _, cost in self.bench)


def pool(db: ModDB, base_id: str, tags: set[str], item_level: int) -> list[Mod]:
    domain = db.bases[base_id]["domain"]
    out = []
    for mod_id, mod in db.mods.items():
        if mod["domain"] != domain or mod["generation_type"] not in ("prefix", "suffix") or not mod.get("text"):
            continue
        if mod["required_level"] > item_level:
            continue
        weight = db.spawn_weight(mod, tags)
        if weight > 0:
            out.append(_mod(mod_id, mod, weight))
    return out


@cache
def spawn_tags() -> frozenset[str]:
    """Every tag that some item mod's spawn weight mentions."""
    return frozenset(
        w["tag"]
        for m in data.load("mods").values()
        if m["domain"] == "item" and m["generation_type"] in ("prefix", "suffix")
        for w in m["spawn_weights"]
        if w["weight"] > 0
    )


_groups: dict[tuple, BaseGroup] = {}


def group(db: ModDB, match: ItemMatch) -> BaseGroup:
    """The bases sharing this item's mod pool: same class, same spawn-relevant tags (influences included)."""
    used = spawn_tags()
    influences = match.item.influences
    signature = frozenset(match.tags & used)
    key = (id(db), match.item_class, signature, frozenset(influences))
    if key not in _groups:
        _groups[key] = _group(db, match, signature, influences)
    return _groups[key]


def _group(db: ModDB, match: ItemMatch, signature: frozenset[str], influences) -> BaseGroup:
    used = spawn_tags()
    ranked = []
    for base_id, base in db.bases.items():
        if base.get("item_class") != match.item_class or base.get("domain") != "item" or base.get("release_state") != "released":
            continue
        if frozenset(db.item_tags(base_id, influences) & used) == signature:
            tags = set(base.get("tags", []))
            # Best first: real drops over experimental bases, end-game bases, then highest drop level.
            ranked.append(("experimental_base" not in tags, "top_tier_base_item_type" in tags, base.get("drop_level", 0), base["name"]))
    names = list(dict.fromkeys(n for *_, n in sorted(ranked, reverse=True))) or [match.item.base]
    if len(names) == 1:
        label = names[0]
    else:
        defence = next((_defence(t) for t in signature if _DEFENCE_TAG.match(t)), None)
        label = f"{match.item_class} ({defence})" if defence else match.item_class
    if influences:
        label += " · " + "/".join(sorted(i.title() for i in influences))
    return BaseGroup(match.item_class, signature, label, names)


def _defence(tag: str) -> str:
    parts = _DEFENCE_TAG.match(tag).group(1).split("_")
    return "/".join(DEFENCES[p] for p in parts)


def build(db: ModDB, match: ItemMatch, settle: int = 0, item_level: int | None = None, exact: bool = False) -> GearProblem:
    """The craft for a PoB rare. Its resistances become filler ("any N resistances at
    T3+") unless `exact`, which asks for every mod as it is."""
    if match.base_id is None:
        raise ValueError(f"unknown base {match.item.base!r}")
    ilvl = item_level or match.item.item_level or DEFAULT_ITEM_LEVEL
    if not item_level:  # PoB items are often theorycraft: raise the level to what their mods need
        ilvl = max([ilvl] + [db.mods[m.mod_id]["required_level"] for m in match.mods if m.source == "natural"])
    mods = pool(db, match.base_id, match.tags, ilvl)
    item_class = match.item_class
    targets, fixed, skipped, extra, bench, rows, bench_ids = [], [], [], {}, [], [], []
    filler_mods = []
    reserve: dict[str, int] = {}
    has_multimod = False
    for m in match.mods:
        text = " / ".join(line.text for line in m.lines)
        if not exact and not m.fractured and m.source == "natural" and m.tier and m.family in FILLER_FAMILIES:
            filler_mods.append((m, text))
        elif m.fractured:
            fixed.append(_mod(m.mod_id, db.mods[m.mod_id], 0))
            rows.append(Row("fractured", m.kind, text, f"T{m.tier}" if m.tier else ""))
        elif m.source == "natural" and m.tier:
            good = [
                x for x in mods
                if db.mods[x.id]["type"] == m.family and x.side == m.kind and (db.tier(x.id, match.tags) or 99) <= m.tier + settle
            ]  # fmt: skip
            if not good:
                skipped.append(f"needs ilvl {db.mods[m.mod_id]['required_level']}: {text}")
                rows.append(Row("skipped", m.kind, text, f"ilvl {db.mods[m.mod_id]['required_level']}"))
                continue
            worst = m.tier + settle
            rows.append(Row("target", m.kind, text, f"T{worst}+" if worst > 1 else "T1"))
            groups = frozenset(g for x in good for g in x.groups)
            targets.append(Target(f"{_short(text)} (T{worst}+)" if worst > 1 else f"{_short(text)} (T1)", m.kind, frozenset(x.id for x in good), groups))
        elif m.source == "essence":
            extra[m.mod_id] = _mod(m.mod_id, db.mods[m.mod_id], 0)
            targets.append(Target(f"{_short(text)} (essence)", m.kind, frozenset({m.mod_id}), frozenset(db.mods[m.mod_id]["groups"])))
            rows.append(Row("essence", m.kind, text, "essence"))
        elif m.source == "crafted" and (cost := actions.bench_cost(item_class, m.mod_id)):
            bench.append((text, cost))
            bench_ids.append(m.mod_id)
            rows.append(Row("bench", m.kind, text, "bench"))
            reserve[m.kind] = reserve.get(m.kind, 0) + 1
            has_multimod = has_multimod or m.mod_id == actions.MULTIMOD
        else:
            skipped.append(f"{m.source}: {text}")
            rows.append(Row("skipped", m.kind, text, m.source))
    if len(bench) > 1 and not has_multimod and (cost := actions.bench_cost(item_class, actions.MULTIMOD)):
        bench.insert(0, ("Can have up to 3 Crafted Modifiers", cost))
        bench_ids.insert(0, actions.MULTIMOD)
        rows.append(Row("bench", "suffix", "Can have up to 3 Crafted Modifiers", "bench"))
        reserve["suffix"] = reserve.get("suffix", 0) + 1
    filler, worst = frozenset(), 0
    if filler_mods:
        worst = max(FILLER_TIER, max(m.tier for m, _ in filler_mods) + settle)
        filler = frozenset(x.id for x in mods if db.mods[x.id]["type"] in FILLER_FAMILIES and (db.tier(x.id, match.tags) or 99) <= worst)
        for m, text in filler_mods:
            rows.append(Row("filler", m.kind, text, f"any res T{worst}+"))
    harvest, h_profiles = actions.harvest(mods)
    fossil, f_profiles = actions.fossils(mods)
    essence, forced = actions.essences(item_class, ilvl)
    for mod_id in forced.values():
        extra.setdefault(mod_id, _mod(mod_id, db.mods[mod_id], 0))
    problem = Problem(
        pool=mods,
        targets=targets,
        need=len(targets),
        actions=actions.BASIC + harvest + fossil + essence + actions.metamods(item_class, _lock_rerolls(harvest, h_profiles, targets, filler))
        + actions.eldritch(item_class, match.item.influences),
        kind="jewel" if item_class in JEWEL_CLASSES else "gear",
        fixed=fixed,
        profiles={**h_profiles, **f_profiles},
        extra_mods=extra,
        reserve=reserve,
        filler=filler,
        filler_need=len(filler_mods),
    )
    families = sorted({m.family for m, _ in filler_mods})
    return GearProblem(problem, ilvl, skipped, bench, rows, bench_ids, families)


def _lock_rerolls(harvest: list, profiles: dict, targets: list[Target], filler: frozenset[str]) -> list:
    """Harvest reforges worth pairing with a side lock: their tag can hit a target or filler mod."""
    wanted = {m for t in targets for m in t.good} | set(filler)
    return [h for h in harvest if wanted & set(profiles[h.first_profile])]


def cost(db: ModDB, match: ItemMatch, prices: dict[str, float], settle: int = 0, item_level: int | None = None) -> tuple[GearProblem, Plan]:
    gp = build(db, match, settle, item_level)
    return gp, solve(gp.problem, prices)


def action_cost_of(cost: tuple[tuple[str, float], ...], prices: dict[str, float]) -> float:
    return sum(prices.get(pid, 0.0) * qty for pid, qty in cost)


def _mod(mod_id: str, mod: dict, weight: int) -> Mod:
    return Mod(
        mod_id,
        mod["generation_type"],
        weight,
        tuple(mod["groups"]),
        False,
        mod["text"].splitlines()[0] if mod.get("text") else mod_id,
        tuple(mod.get("implicit_tags", [])),
    )


def _short(text: str, width: int = 34) -> str:
    return text if len(text) <= width else text[: width - 1] + "…"
