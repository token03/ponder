"""What tiers are worth: a price model fitted on trade listings.

For an archetype (a base group plus the mods that define it), search trade
for items that have every core mod at a low floor, so listings span many
tiers. Each listing's mods are matched to ids and tiers, then

    log(price) ~ intercept + sum_k (T1_k, T2_k dummies) + resistances + open affixes

is fitted by least squares. exp(coefficient) is the price multiplier for
having mod k at T1 (or T2) instead of a lower tier, holding the rest fixed.
That's how "T1 ES doubles the price, T1 resistances don't matter" comes out
of data instead of a fixed 1 / 0.9 / 0.6 curve.

Listings are cheapest-first and asking prices include fixers, so treat the
multipliers as directions and sizes, not exact values; more listings help.
"""

from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np

from . import gear, trade
from .demand import RES, stat_key
from .item import Item, ModLine
from .mods import ItemMatch, ModDB

FLOOR_TIER = 4  # search floor: listings at this tier or better on every core mod


@dataclass
class TierPrice:
    key: str  # main stat
    text: str
    t1: float | None  # price multiplier for T1 vs below T2
    t2: float | None
    n_t1: int = 0  # listings with it at T1
    n_t2: int = 0


@dataclass
class PriceModel:
    group: str
    listings: int
    base_price: float  # fitted price with every core mod below T2, no res, no open affix
    tiers: list[TierPrice] = field(default_factory=list)
    per_res: float | None = None  # multiplier per resistance mod
    per_open_affix: float | None = None
    r2: float = 0.0


def archetype_query(db: ModDB, example: ItemMatch, keys: tuple[str, ...], floor: int = FLOOR_TIER) -> dict | None:
    """A trade search for the archetype's core mods at `floor` tier or better, on its base group."""
    grp = gear.group(db, example)
    where = {"category": trade.CATEGORIES.get(grp.item_class)} if grp.item_class in ("Ring", "Amulet", "Belt") and len(grp.bases) > 1 else {"base": grp.best}
    stats = []
    for key in keys:
        if key == RES:
            continue
        mods = [m.id for m in gear.pool(db, example.base_id, example.tags, 86) if stat_key(m.id) == key and (db.tier(m.id, example.tags) or 99) <= floor]
        if mods and (sid := trade.stat_id(mods[0])):
            stats.append((sid, trade.stat_min(mods)))
    return trade.item_query(**where, stats=stats) if stats else None


def listing_match(db: ModDB, listing: dict) -> ItemMatch | None:
    """A trade listing's item, matched to mod ids and tiers through its mod text."""
    it = listing.get("item") or {}
    lines = [ModLine(t) for t in it.get("explicitMods") or []]
    lines += [ModLine(t, {"fractured": ""}) for t in it.get("fracturedMods") or []]
    lines += [ModLine(t, {"crafted": ""}) for t in it.get("craftedMods") or []]
    influences = {k for k, v in (it.get("influences") or {}).items() if v}
    item = Item("RARE", it.get("name", ""), it.get("baseType", ""), it.get("ilvl"), influences=influences, explicits=lines)
    m = db.match(item)
    return m if m.base_id else None


def fit(db: ModDB, group: str, keys: tuple[str, ...], listings: list[dict], rates: dict[str, float]) -> PriceModel | None:
    core = [k for k in keys if k != RES]
    rows, prices = [], []
    counts = {k: [0, 0] for k in core}
    for listing in listings:
        price = trade.chaos_price(listing, rates)
        m = listing_match(db, listing)
        if not price or m is None:
            continue
        best: dict[str, int] = {}
        res = open_affixes = 0
        for mod in m.mods:
            if mod.source in ("crafted", "legacy-crafted"):
                continue
            if mod.family in gear.FILLER_FAMILIES:
                res += 1
            k = stat_key(mod.mod_id)
            if k in counts and mod.tier:
                best[k] = min(best.get(k, 99), mod.tier)
        open_affixes = max(0, 6 - sum(1 for mod in m.mods if mod.source != "crafted"))
        row = [1.0]
        for k in core:
            t = best.get(k, 99)
            row += [float(t == 1), float(t == 2)]
            counts[k][0] += t == 1
            counts[k][1] += t == 2
        row += [float(res), float(open_affixes)]
        rows.append(row)
        prices.append(np.log(price))
    if len(rows) < 2 * len(core) + 4:
        return None
    x, y = np.array(rows), np.array(prices)
    coef, *_ = np.linalg.lstsq(x, y, rcond=None)
    fitted = x @ coef
    r2 = 1 - float(np.sum((y - fitted) ** 2) / max(np.sum((y - y.mean()) ** 2), 1e-12))
    model = PriceModel(group, len(rows), float(np.exp(coef[0])), r2=r2)
    for i, k in enumerate(core):
        t1, t2 = coef[1 + 2 * i], coef[2 + 2 * i]
        n1, n2 = counts[k]
        model.tiers.append(TierPrice(k, k, float(np.exp(t1)) if n1 else None, float(np.exp(t2)) if n2 else None, n1, n2))
    model.per_res = float(np.exp(coef[-2]))
    model.per_open_affix = float(np.exp(coef[-1]))
    return model
