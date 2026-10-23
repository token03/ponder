"""Turn a matched rare item into a crafting problem.

Each natural mod on the item becomes a target that any tier at least as good
satisfies (`settle` loosens that by N tiers). Fractured mods are kept fixed.
Essence-only mods become targets only an essence can hit. Crafted mods the
bench still offers become a last bench step: the plan keeps a slot open on
their side and pays the bench cost. Veiled and other special mods are left
for the user.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from . import actions
from .craft import Mod, Plan, Problem, Target, solve
from .mods import ItemMatch, ModDB

JEWEL_CLASSES = {"Jewel", "AbyssJewel"}
DEFAULT_ITEM_LEVEL = 84


@dataclass
class GearProblem:
    problem: Problem
    item_level: int
    skipped: list[str] = field(default_factory=list)  # mods we don't try to craft
    bench: list[tuple[str, tuple[tuple[str, float], ...]]] = field(default_factory=list)  # (label, cost) crafted last

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


def build(db: ModDB, match: ItemMatch, settle: int = 0, item_level: int | None = None) -> GearProblem:
    if match.base_id is None:
        raise ValueError(f"unknown base {match.item.base!r}")
    ilvl = item_level or match.item.item_level or DEFAULT_ITEM_LEVEL
    if not item_level:  # PoB items are often theorycraft: raise the level to what their mods need
        ilvl = max([ilvl] + [db.mods[m.mod_id]["required_level"] for m in match.mods if m.source == "natural"])
    mods = pool(db, match.base_id, match.tags, ilvl)
    item_class = match.item_class
    targets, fixed, skipped, extra, bench = [], [], [], {}, []
    reserve: dict[str, int] = {}
    has_multimod = False
    for m in match.mods:
        text = " / ".join(line.text for line in m.lines)
        if m.fractured:
            fixed.append(_mod(m.mod_id, db.mods[m.mod_id], 0))
        elif m.source == "natural" and m.tier:
            good = [
                x for x in mods
                if db.mods[x.id]["type"] == m.family and x.side == m.kind and (db.tier(x.id, match.tags) or 99) <= m.tier + settle
            ]  # fmt: skip
            if not good:
                skipped.append(f"needs ilvl {db.mods[m.mod_id]['required_level']}: {text}")
                continue
            worst = m.tier + settle
            groups = frozenset(g for x in good for g in x.groups)
            targets.append(Target(f"{_short(text)} (T{worst}+)" if worst > 1 else f"{_short(text)} (T1)", m.kind, frozenset(x.id for x in good), groups))
        elif m.source == "essence":
            extra[m.mod_id] = _mod(m.mod_id, db.mods[m.mod_id], 0)
            targets.append(Target(f"{_short(text)} (essence)", m.kind, frozenset({m.mod_id}), frozenset(db.mods[m.mod_id]["groups"])))
        elif m.source == "crafted" and (cost := actions.bench_cost(item_class, m.mod_id)):
            bench.append((text, cost))
            reserve[m.kind] = reserve.get(m.kind, 0) + 1
            has_multimod = has_multimod or m.mod_id == actions.MULTIMOD
        else:
            skipped.append(f"{m.source}: {text}")
    if len(bench) > 1 and not has_multimod and (cost := actions.bench_cost(item_class, actions.MULTIMOD)):
        bench.insert(0, ("Can have up to 3 Crafted Modifiers", cost))
        reserve["suffix"] = reserve.get("suffix", 0) + 1
    harvest, h_profiles = actions.harvest(mods)
    fossil, f_profiles = actions.fossils(mods)
    essence, forced = actions.essences(item_class, ilvl)
    for mod_id in forced.values():
        extra.setdefault(mod_id, _mod(mod_id, db.mods[mod_id], 0))
    problem = Problem(
        pool=mods,
        targets=targets,
        need=len(targets),
        actions=actions.BASIC + harvest + fossil + essence + actions.metamods(item_class),
        kind="jewel" if item_class in JEWEL_CLASSES else "gear",
        fixed=fixed,
        profiles={**h_profiles, **f_profiles},
        extra_mods=extra,
        reserve=reserve,
    )
    return GearProblem(problem, ilvl, skipped, bench)


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
