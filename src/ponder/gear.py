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
        if m["domain"] in ("item", "misc", "abyss_jewel") and m["generation_type"] in ("prefix", "suffix")
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
    domain = db.bases[match.base_id].get("domain", "item")  # jewels and abyss jewels have their own
    ranked = []
    for base_id, base in db.bases.items():
        if base.get("item_class") != match.item_class or base.get("domain") != domain or base.get("release_state") != "released":
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
    filler_sides = None
    families = sorted({m.family for m, _ in filler_mods})
    if match.finish and not exact:
        filler, filler_sides, families = _finish(db, match, mods, rows)
        if not bench and (craft := _bench_filler(db, item_class, match.finish["families"], filler_sides)):
            side, mod_id, text, cost = craft
            bench.append((text, cost))
            bench_ids.append(mod_id)
            reserve[side] = reserve.get(side, 0) + 1
            filler_sides = (filler_sides[0] - (side == "prefix"), filler_sides[1] - (side == "suffix"))
            i = next(i for i, r in enumerate(rows) if r.role == "filler" and r.side == side)
            rows[i] = Row("bench", side, text, "bench")
        filler_mods = [None] * sum(filler_sides)
    elif filler_mods:
        worst = max(FILLER_TIER, max(m.tier for m, _ in filler_mods) + settle)
        filler = frozenset(x.id for x in mods if db.mods[x.id]["type"] in FILLER_FAMILIES and (db.tier(x.id, match.tags) or 99) <= worst)
        for m, text in filler_mods:
            rows.append(Row("filler", m.kind, text, f"any res T{worst}+"))
    harvest, h_profiles = actions.harvest(mods)
    fossil, f_profiles = _fossils(mods, targets, filler, len(filler_mods), item_class)
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
        filler_sides=filler_sides,
    )
    return GearProblem(problem, ilvl, skipped, bench, rows, bench_ids, families)


def _finish(db: ModDB, match: ItemMatch, mods: list[Mod], rows: list[Row]) -> tuple[frozenset[str], tuple[int, int], list[str]]:
    """Filler from `match.finish`: any of its families at its tier or better, so many per side."""
    spec = match.finish
    families = spec["families"]
    filler = frozenset(x.id for x in mods if (f := db.mods[x.id]["type"]) in families and (db.tier(x.id, match.tags) or 99) <= families[f])
    need = spec["sides"]
    for side, n in zip(("prefix", "suffix"), need):
        on_side = [f for f in [*spec["order"], *sorted(FILLER_FAMILIES)] if any(x.side == side and db.mods[x.id]["type"] == f for x in mods if x.id in filler)]
        if not on_side:
            continue
        names = list(dict.fromkeys(_family_name(db, f, mods) for f in on_side))
        shown = ", ".join(names[:4]) + (f" +{len(names) - 4} more" if len(names) > 4 else "")
        for _ in range(n):
            rows.append(Row("filler", side, f"any of: {shown}", "filler"))
    sides = tuple(n if any(x.side == side for x in mods if x.id in filler) else 0 for side, n in zip(("prefix", "suffix"), need))
    return filler, sides, list(spec["order"][:2])


def _bench_filler(db: ModDB, item_class: str, families: dict[str, int], need: tuple[int, int]):
    """The bench craft that fills one open filler slot, suffix side first: (side, mod id, text, cost)."""
    for side, n in zip(("suffix", "prefix"), (need[1], need[0])):
        if n <= 0:
            continue
        best = None
        for option in data.load("crafting_bench_options"):
            mod_id = (option.get("actions") or {}).get("add_explicit_mod")
            mod = db.mods.get(mod_id or "")
            if not mod or mod["type"] not in families or mod["generation_type"] != side or item_class not in option["item_classes"]:
                continue
            if best is None or mod["required_level"] > db.mods[best]["required_level"]:
                best = mod_id
        if best and (cost := actions.bench_cost(item_class, best)):
            return side, best, db.mods[best]["text"].splitlines()[0], cost
    return None


def _family_name(db: ModDB, family: str, mods: list[Mod]) -> str:
    if family in FILLER_FAMILIES:
        return "res"
    m = next(x for x in mods if db.mods[x.id]["type"] == family)
    return _short(re.sub(r"\(?-?\d+(\.\d+)?(-\d+(\.\d+)?)?\)?", "#", m.label), 28)


def problem_on(
    db: ModDB,
    base_id: str,
    targets: list[Target],
    item_level: int = DEFAULT_ITEM_LEVEL,
    influences: frozenset[str] = frozenset(),
    junk_cap: tuple[int, int] | None = None,
) -> Problem:
    """A craft of `targets` on a plain base, with every action `build` offers (no bench, no filler)."""
    base = db.bases[base_id]
    item_class = base["item_class"]
    tags = db.item_tags(base_id, set(influences))
    mods = pool(db, base_id, tags, item_level)
    harvest, h_profiles = actions.harvest(mods)
    fossil, f_profiles = _fossils(mods, targets, frozenset(), 0, item_class)
    essence, forced = actions.essences(item_class, item_level)
    extra = {m: _mod(m, db.mods[m], 0) for m in forced.values()}
    return Problem(
        pool=mods,
        targets=targets,
        need=len(targets),
        actions=actions.BASIC + harvest + fossil + essence + actions.metamods(item_class, _lock_rerolls(harvest, h_profiles, targets, frozenset()))
        + actions.eldritch(item_class, set(influences)),
        kind="jewel" if item_class in JEWEL_CLASSES else "gear",
        profiles={**h_profiles, **f_profiles},
        extra_mods=extra,
        junk_cap=junk_cap,
    )


FOSSIL_COMBOS = True  # 1-4 fossil resonators, pre-scored (actions.fossil_combos); False: single fossils only


@cache
def _prices() -> dict[str, float]:
    from . import prices

    return prices.load()[0]


def _fossils(mods: list[Mod], targets: list[Target], filler: frozenset[str], filler_need: int, item_class: str):
    if not FOSSIL_COMBOS:
        return actions.fossils(mods)
    wants = [t.good for t in targets] + [filler] * filler_need
    per_side = 1.6 if item_class in JEWEL_CLASSES else 2.3  # half the mods a rare rolls
    return actions.fossil_combos(mods, wants, _prices(), mods_per_side=per_side)


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
