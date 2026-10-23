"""The whole pipeline as a library: builds in, priced crafting estimates out.

`run(sources)` loads each build, matches its rares to mod ids, and turns each
rare into a craft: the target mods plus the group of bases that roll exactly
the same mods (so Warlock and Sorcerer Gloves are one "Gloves (ES)" craft).
Crafts that several builds need are merged. Every craft gets the cheapest
plan, and optionally:

- fracture options: for each target, the cost of buying a base with that mod
  already fractured and crafting the rest, against crafting from scratch;
- trade prices: what the finished item sells for, and what fractured bases
  cost, within a budget of searches (the trade API is slow to allow more).

Nothing here prints; the CLI and TUI render the `Report`, and
`Report.to_dict()` gives plain JSON.
"""

from __future__ import annotations

import dataclasses
import hashlib
import multiprocessing
import os
import pickle
import sys
import urllib.error
import zlib
from collections import Counter
from collections.abc import Callable, Iterator
from concurrent.futures import ProcessPoolExecutor, as_completed
from concurrent.futures.process import BrokenProcessPool
from contextlib import contextmanager
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np

from . import cluster, craft, data, gear, pob, prices, trade
from .gear import Row
from .item import parse_item
from .mine import Candidate, mine, target_key
from .mods import ItemMatch, ModDB

Progress = Callable[[str], None]
JEWELLERY = {"Ring", "Amulet", "Belt"}  # one implicit apart, their bases sell alike: search by category


@dataclass
class Fracture:
    """Buy a base with one target already fractured on it, then craft the rest."""

    target: str  # the target's name
    mod_id: str  # the mod fixed in the solver (any acceptable tier)
    problem: craft.Problem | None = None
    plan: craft.Plan | None = None
    query: dict | None = None  # trade search for the fractured base
    trade_url: str | None = None
    base_price: tuple[float, int] | None = None
    base_error: str | None = None
    error: str | None = None

    @property
    def craft_cost(self) -> float | None:
        return self.plan.cost if self.plan else None

    @property
    def total(self) -> float | None:
        if self.plan is None or self.base_price is None:
            return None
        return self.plan.cost + self.base_price[0]

    def to_dict(self) -> dict:
        return {
            "target": self.target,
            "craft_cost": self.craft_cost,
            "base_price": self.base_price[0] if self.base_price else None,
            "base_listed": self.base_price[1] if self.base_price else None,
            "total": self.total,
            "trade_url": self.trade_url,
            "error": self.error or self.base_error,
        }


@dataclass
class Estimate:
    """Expected cost of crafting one item, and what it sells for when known."""

    kind: str  # "cluster" or "gear"
    title: str  # the base group, e.g. "Gloves (ES)", or "medium cluster, 10% increased Area Damage, 4 passives"
    targets: list[str]
    key: tuple = ()  # identifies the same craft across builds
    base: str = ""  # the best base with this mod pool, which crafts and searches use, e.g. "Warlock Gloves"
    used_base: str = ""  # the base the build had, when different, e.g. "Sorcerer Gloves"
    bases: int = 1  # how many bases share this mod pool
    slot: str | None = None
    builds: list[str] = field(default_factory=list)  # sources whose rares need this craft
    item_level: int = 84
    problem: craft.Problem | None = None
    plan: craft.Plan | None = None
    rows: list[Row] = field(default_factory=list)  # every mod line, for display
    bench: list[tuple[str, float]] = field(default_factory=list)  # (label, chaos) crafted last
    skipped: list[str] = field(default_factory=list)  # mods the plan doesn't make
    fixed: list[str] = field(default_factory=list)  # fractured mods the base must already have
    fixed_ids: list[str] = field(default_factory=list)
    base_price: tuple[float, int] | None = None  # (chaos, listings)
    base_source: str = ""  # "poe.ninja", "trade"
    base_query: dict | None = None  # trade search for the (fractured) base, when one is needed
    base_url: str | None = None
    base_error: str | None = None
    query: dict | None = None  # trade search for the finished item
    trade_url: str | None = None
    market: tuple[float, int] | None = None  # (median of cheapest asks, total listed)
    market_error: str | None = None
    fractures: list[Fracture] = field(default_factory=list)
    filler_families: list[str] = field(default_factory=list)  # e.g. ["ColdResistance", "FireResistance"]
    search: dict = field(default_factory=dict)  # how to search its bases: cluster size/enchant/passives, or gear base/category
    error: str | None = None  # why there is no plan
    demand: int = 0  # ladder characters wearing it (ladder runs)
    recomb: object | None = None  # recomb.Step: the cheapest recombinator tree, when costed
    skills: list[str] = field(default_factory=list)  # their most common main skills

    @property
    def craft_cost(self) -> float | None:
        return self.plan.cost if self.plan else None

    @property
    def bench_cost(self) -> float:
        return sum(c for _, c in self.bench)

    @property
    def needs_base_price(self) -> bool:
        """A fractured base costs real money; without its price the total is incomplete."""
        return bool(self.fixed) and self.base_price is None

    @property
    def total(self) -> float | None:
        if self.plan is None:
            return None
        return self.plan.cost + self.bench_cost + (self.base_price[0] if self.base_price else 0.0)

    @property
    def margin(self) -> float | None:
        if not self.market or self.total is None or self.needs_base_price:
            return None
        return self.market[0] - self.cheapest

    @property
    def hole(self) -> float | None:
        """How much of a market hole this is: margin times ladder wearers per listing.

        Many wearers, few listings and a fat margin rank first; a crowded market
        (hundreds listed) shrinks even a good margin.
        """
        if self.margin is None or not self.demand:
            return None
        return self.margin * self.demand / (self.market[1] + 5)

    @property
    def best_fracture(self) -> Fracture | None:
        priced = [f for f in self.fractures if f.total is not None]
        return min(priced, key=lambda f: f.total) if priced else None

    @property
    def fracture_worth(self) -> tuple[float, Fracture] | None:
        """The most a fractured base is worth: crafting from a plain base (base included) less the cheapest finish from a fractured one."""
        done = [f for f in self.fractures if f.plan]
        if self.total is None or not done:
            return None
        f = min(done, key=lambda f: f.plan.cost)
        return self.total - f.plan.cost, f

    @property
    def recomb_total(self) -> float | None:
        """Recombining clean donors instead of crafting in one go (then the same bench crafts)."""
        if self.recomb is None or self.recomb.cost is None:
            return None
        return self.recomb.cost + self.bench_cost

    @property
    def cheapest(self) -> float | None:
        """The cheapest of crafting from scratch, the best priced fracture and recombining."""
        options = [x for x in (self.total, self.best_fracture.total if self.best_fracture else None, self.recomb_total) if x is not None]
        return min(options) if options else None

    def recipe(self, rows: int = 12) -> list[tuple[str, str, float]]:
        return craft.recipe(self.problem, self.plan)[:rows] if self.plan else []

    def recipe_states(self, rows: int = 12) -> list[tuple[craft.State, str, float]]:
        return craft.recipe_states(self.plan)[:rows] if self.plan else []

    def usage(self) -> list[tuple[str, float]]:
        if not self.plan:
            return []
        return [(a, n) for a, n in sorted(self.plan.usage.items(), key=lambda x: -x[1]) if n >= 0.05]

    def to_dict(self) -> dict:
        return {
            "kind": self.kind,
            "title": self.title,
            "base": self.base,
            "build_base": self.used_base or self.base,
            "bases_in_group": self.bases,
            "mods": [dataclasses.asdict(r) for r in self.rows],
            "targets": self.targets,
            "slot": self.slot,
            "builds": self.builds,
            "item_level": self.item_level,
            "craft_cost": self.craft_cost,
            "bench": [{"mod": label, "chaos": c} for label, c in self.bench],
            "base_price": self.base_price[0] if self.base_price else None,
            "base_source": self.base_source or None,
            "base_url": self.base_url,
            "total": self.total,
            "market": self.market[0] if self.market else None,
            "listed": self.market[1] if self.market else None,
            "margin": self.margin,
            "currency": dict(self.usage()),
            "recipe": [{"state": s, "action": a, "share": share} for s, a, share in self.recipe()],
            "fractures": [f.to_dict() for f in self.fractures],
            "not_crafted": self.skipped,
            "trade_url": self.trade_url,
            "recomb": self.recomb.to_dict() if self.recomb else None,
            "recomb_total": self.recomb_total,
            "demand": self.demand,
            "skills": self.skills,
            "hole": self.hole,
            "fracture_worth": {"chaos": w[0], "target": w[1].target} if (w := self.fracture_worth) else None,
            "error": self.error or self.market_error or self.base_error,
        }


@dataclass
class BuildInfo:
    source: str
    name: str  # e.g. "Occultist lvl 95: Winter Orb"
    rares: int


@dataclass
class Shared:
    """A mod combination several builds' rares have in common."""

    candidate: Candidate
    estimate: Estimate | None = None  # cluster combos are costed; gear combos aren't yet

    def to_dict(self) -> dict:
        c = self.candidate
        return {
            "item_class": c.item_class,
            "mods": list(c.families),
            "builds": c.builds,
            "items": c.count,
            "estimate": self.estimate.to_dict() if self.estimate else None,
        }


@dataclass
class Report:
    builds: list[BuildInfo]
    estimates: list[Estimate]
    shared: list[Shared]
    prices_source: str
    league: str
    errors: list[str] = field(default_factory=list)  # sources that failed to load
    divine: float = 0.0  # chaos per divine, for display

    def ranked(self) -> list[Estimate]:
        """Best margin first, then crafts more builds need, then the dearest."""
        return sorted(self.estimates, key=priority)

    def to_dict(self) -> dict:
        return {
            "league": self.league,
            "prices": self.prices_source,
            "chaos_per_divine": self.divine,
            "builds": [vars(b) for b in self.builds],
            "estimates": [e.to_dict() for e in self.ranked()],
            "shared": [s.to_dict() for s in self.shared],
            "errors": self.errors,
        }


def priority(e: Estimate) -> tuple:
    return (e.margin is None, -(e.margin or 0), -len(e.builds), e.total is None, -(e.total or 0))


# -- the run -------------------------------------------------------------------------


def run(
    sources: list[str],
    *,
    settle: int = 0,
    max_targets: int = 4,
    min_builds: int = 2,
    top: int = 30,
    fractures: bool = False,
    exact: bool = False,
    trade_lookup: bool = False,
    trade_budget: int = 40,
    recombs: bool = False,
    workers: int | None = None,
    progress: Progress | None = None,
) -> Report:
    """`trade_budget` caps trade searches (cached answers are free); `workers` 1 solves inline."""
    say = progress or (lambda _msg: None)
    table, prices_source = prices.load()
    league = current_league()
    db = ModDB()

    infos, matched, errors = [], [], []
    by_key: dict[tuple, Estimate] = {}
    for source in expand_sources(sources):
        say(f"loading {source[:70]}")
        try:
            build = pob.load(source)
        except Exception as e:  # a dead link shouldn't sink the rest
            errors.append(f"{source[:70]}: {e}")
            continue
        rares = 0
        for pob_item in build.items:
            item = parse_item(pob_item.text)
            if item.rarity != "RARE":
                continue
            rares += 1
            match = db.match(item)
            matched.append((source, match))
            est = estimate_item(db, match, table, league, settle=settle, max_targets=max_targets, exact=exact, solve=False)
            if est is None or not est.targets:
                continue
            est = by_key.setdefault(est.key, est)
            est.slot = est.slot or pob_item.slot
            if source not in est.builds:
                est.builds.append(source)
        infos.append(BuildInfo(source, build_name(build), rares))

    estimates = list(by_key.values())
    shared = []
    if len(infos) >= 2:
        say("mining shared combinations")
        for c in mine(matched, min_builds=min_builds)[:top]:
            est = None
            if " cluster: " in c.item_class:
                size, tag = c.item_class.split(" cluster: ")
                est = next((e for e in estimates if e.key[:3] == ("cluster", size, tag) and set(c.families) == set(e.targets)), None)
                est = est or estimate_cluster(size, tag, list(c.families), table, league, solve=False)
            shared.append(Shared(c, est))

    if fractures:
        for e in estimates:
            add_fractures(e, league)
    extra = [s.estimate for s in shared if s.estimate and s.estimate not in estimates]
    solve_all(estimates + extra + [f for e in estimates for f in e.fractures], table, workers, say)
    if recombs:
        add_recombs(estimates, table, workers, say, db)

    if trade_lookup:
        price_on_trade(sorted(estimates, key=priority), table, league, budget=trade_budget, progress=say)
    return Report(infos, estimates, shared, prices_source, league, errors, table.get("divine", 0.0))


# -- the ladder: what invested characters wear ------------------------------------------


@dataclass
class LadderReport:
    characters: int
    items: int  # rare gear and jewels read
    clusters: int  # rare cluster jewels read
    estimates: list[Estimate]
    archetypes: int  # gear combinations found, before costing
    cluster_archetypes: int
    prices_source: str
    league: str
    divine: float = 0.0

    def ranked(self) -> list[Estimate]:
        """Market holes first (margin x wearers per listing), then unpriced ones by ladder demand."""
        return sorted(self.estimates, key=lambda e: (e.hole is None, -(e.hole or 0), -e.demand))

    def to_dict(self) -> dict:
        return {
            "league": self.league,
            "prices": self.prices_source,
            "chaos_per_divine": self.divine,
            "characters": self.characters,
            "items": self.items,
            "clusters": self.clusters,
            "archetypes": self.archetypes,
            "cluster_archetypes": self.cluster_archetypes,
            "estimates": [e.to_dict() for e in self.ranked()],
        }


def ladder(
    characters: list[dict] | None = None,
    *,
    gear_top: int = 80,
    per_group: int = 3,
    niche: int = 40,
    cluster_top: int = 80,
    min_characters: int = 5,
    group: str | None = None,
    trade_lookup: bool = False,
    trade_budget: int = 40,
    recombs: bool = False,
    workers: int | None = None,
    progress: Progress | None = None,
) -> LadderReport:
    """Crafting candidates from ladder characters (default: the cached poe.ninja sample).

    Gear archetypes are costed at the tier most of their wearers reach, with
    their usual number of resistances as filler; clusters by enchant, usual
    passive count and notables. Trade lookups (in demand order) give the
    selling price and how many are listed, which `Estimate.hole` turns into a
    market-hole score.
    """
    from . import demand
    from . import ladder as lad

    say = progress or (lambda _msg: None)
    table, prices_source = prices.load()
    league = current_league()
    db = ModDB()
    if characters is None:
        say("reading cached ladder characters")
        characters = list(lad.cached())
    say("matching ladder items")
    worn = demand.wear(db, characters)
    clusters = demand.wear_clusters(characters)
    found = [a for a in demand.archetypes(worn, min_characters) if not group or group.lower() in a.group.lower()]
    found_cl = [a for a in demand.cluster_archetypes(clusters, min_characters) if not group or group.lower() in a.title.lower()]
    estimates: dict[tuple, Estimate] = {}
    # The most worn overall, plus the top few of every base group, so niche
    # groups (quivers, claws, shields...) get costed too, not just rings and boots.
    picked, per = [], Counter()
    for a in found:
        if len(picked) < gear_top or per[a.group] < per_group:
            picked.append(a)
            per[a.group] += 1
    # ...and the most niche: 3+ mods worn together far more than chance (one build's item).
    chosen = {id(a) for a in picked}
    niches: list = []
    for a in sorted(found, key=lambda a: -a.niche):
        if len(niches) >= niche:
            break
        if id(a) in chosen or len(a.families) < 3:
            continue
        # Variants of one niche item (the same mods give or take one) are costed once.
        if any(o.group == a.group and len(set(o.families) & set(a.families)) >= min(len(o.families), len(a.families)) - 1 for o in niches):
            continue
        niches.append(a)
    picked += niches
    for a in picked:
        m = demand.archetype_match(db, a)
        if m is None:
            continue
        est = estimate_gear(db, m, table, league, solve=False)
        if est.error or est.key in estimates:
            continue
        est.demand, est.skills = a.characters, [s for s, _ in a.skills.most_common(3)]
        est.used_base = ""
        estimates[est.key] = est
    n_gear = len(estimates)
    for a in found_cl:
        if len(estimates) - n_gear >= cluster_top:
            break
        est = estimate_cluster(a.size, a.enchant, list(a.notables), table, league, passives=a.usual_passives, solve=False)  # two enchants can share a tag
        if est.error or est.key in estimates:
            continue
        est.demand, est.skills = a.characters, [s for s, _ in a.skills.most_common(3)]
        add_fractures(est, league)  # fractured bases are most of the cluster market's money
        estimates[est.key] = est
    out = list(estimates.values())
    solve_all(out + [f for e in out for f in e.fractures], table, workers, say)
    if recombs:
        add_recombs(out, table, workers, say, db)
    if trade_lookup:
        price_on_trade(sorted(out, key=lambda e: -e.demand), table, league, budget=trade_budget, progress=say)
    return LadderReport(len(characters), len(worn), len(clusters), out, len(found), len(found_cl), prices_source, league, table.get("divine", 0.0))


# -- estimates -------------------------------------------------------------------------


def estimate_item(
    db: ModDB,
    match: ItemMatch,
    table: dict[str, float],
    league: str,
    settle: int = 0,
    max_targets: int = 4,
    exact: bool = False,
    solve: bool = True,
) -> Estimate | None:
    """The craft for one rare, or None when we can't craft it (unknown base, cluster without notables)."""
    item = match.item
    if item.base.endswith("Cluster Jewel"):
        cls, notables = target_key(match)
        if " cluster: " not in cls or not notables:
            return None
        size, tag = cls.split(" cluster: ")
        count = item.header.get("Cluster Jewel Node Count")
        return estimate_cluster(size, tag, notables, table, league, passives=int(count) if count else None, solve=solve)
    if match.base_id is None:
        return None
    return estimate_gear(db, match, table, league, settle=settle, max_targets=max_targets, exact=exact, solve=solve)


def estimate_cluster(
    size: str,
    enchant: str,
    notables: list[str],
    table: dict[str, float],
    league: str,
    *,
    passives: int | None = None,
    item_level: int = 84,
    need: int | None = None,
    fractured: list[str] = (),
    solve: bool = True,
) -> Estimate:
    """With `solve=False` the problem is built but left for `solve_all`."""
    passives = passives or cluster.DEFAULT_PASSIVES[size]
    try:
        ench = cluster.find_enchant(size, enchant)
    except ValueError as e:
        return Estimate("cluster", f"{size} cluster, {enchant}", list(notables), error=str(e))
    est = Estimate(
        "cluster",
        f"{size.title()} cluster · {ench.short if len(ench.short) <= 48 else ench.name} · {passives}p",
        list(notables),
        key=("cluster", size, ench.tag, tuple(sorted(notables)), str(passives), tuple(sorted(fractured))),
        base=f"{size.title()} Cluster Jewel",
        item_level=item_level,
        fixed=list(fractured),
    )
    try:
        est.problem = cluster.problem(size, ench.name, list(notables), need, item_level, fractured)
    except ValueError as e:
        est.error = str(e)
        return est
    if need and need < len(notables):
        est.title += f" ({need} of {len(notables)})"
    est.rows = [Row("target", t.side, t.name) for t in est.problem.targets] + [Row("fractured", m.side, m.label) for m in est.problem.fixed]
    est.fixed_ids = [m.id for m in est.problem.fixed]
    option = prices.enchant_option(ench.short)
    est.search = {"size": size, "enchant": option, "passives": passives}
    wanted = [(min(t.good), None if t.notable else trade.stat_min(t.good)) for t in est.problem.targets]
    est.query = trade.cluster_query(size, option, passives, wanted, item_level)
    est.trade_url = trade.search_url(league, est.query)
    if fractured:
        est.base_query = trade.cluster_query(size, option, passives, [], item_level, fractured_ids=est.fixed_ids)
        est.base_url = trade.search_url(league, est.base_query)
    elif base := prices.cluster_base(size, ench.short, passives, item_level):
        est.base_price, est.base_source = base, "poe.ninja"
    if solve:
        solve_one(est, table)
    return est


def estimate_gear(
    db: ModDB,
    match: ItemMatch,
    table: dict[str, float],
    league: str = "Standard",
    *,
    settle: int = 0,
    item_level: int | None = None,
    max_targets: int = 4,
    exact: bool = False,
    solve: bool = True,
) -> Estimate:
    """With `exact`, resistances are targets like any other mod instead of filler."""
    gp = gear.build(db, match, settle, item_level, exact=exact)
    grp = gear.group(db, match)
    p = gp.problem
    est = Estimate(
        "gear",
        grp.label,
        [t.name for t in p.targets],
        key=(
            "gear",
            grp.item_class,
            grp.signature,
            tuple(sorted((t.side, tuple(sorted(t.good))) for t in p.targets)),
            tuple(sorted(m.id for m in p.fixed)),
            tuple(sorted(gp.bench_ids)),
            gp.item_level,
            p.filler_need,
            p.filler,
        ),
        base=grp.best,
        used_base=match.item.base if match.item.base != grp.best else "",
        bases=len(grp.bases),
        item_level=gp.item_level,
        problem=p,
        rows=gp.rows,
        bench=[(label, gear.action_cost_of(cost, table)) for label, cost in gp.bench],
        skipped=list(gp.skipped),
        fixed=[m.label for m in p.fixed],
        fixed_ids=[m.id for m in p.fixed],
        filler_families=gp.filler_families,
    )
    where = {"category": trade.CATEGORIES.get(grp.item_class)} if grp.item_class in JEWELLERY and len(grp.bases) > 1 else {"base": grp.best}
    stats = [(sid, trade.stat_min(t.good)) for t in p.targets if (sid := trade.stat_id(next(iter(t.good))))]
    stats += [(sid, trade.stat_min([m])) for m in est.fixed_ids if (sid := trade.stat_id(m))]
    filler = _filler_stats(p.filler)
    if stats or filler:
        est.query = trade.item_query(**where, stats=stats, count=(p.filler_need, filler))
        est.trade_url = trade.search_url(league, est.query)
    if est.fixed_ids:
        frac = [(sid, trade.stat_min([m])) for m in est.fixed_ids if (sid := trade.stat_id(m, "fractured"))]
        est.base_query = trade.item_query(**where, stats=frac, item_level=gp.item_level, fractured_only=True)
        est.base_url = trade.search_url(league, est.base_query)
    est.search = where
    if len(p.targets) > max_targets:
        est.error = f"{len(p.targets)} target mods (raise max_targets, or settle to loosen)"
    elif solve:
        solve_one(est, table)
    return est


def _filler_stats(filler) -> list[tuple[str, float | None]]:
    """One (trade stat, minimum) per filler family, e.g. fire resistance at its T3 minimum."""
    mods = data.load("mods")
    by_family: dict[str, list[str]] = {}
    for m in filler:
        by_family.setdefault(mods[m]["type"], []).append(m)
    out = []
    for ids in by_family.values():
        if sid := trade.stat_id(ids[0]):
            out.append((sid, trade.stat_min(ids)))
    return out


def add_recombs(estimates: list[Estimate], table: dict[str, float], workers: int | None = None, progress: Progress | None = None, db: ModDB | None = None) -> None:
    """Cost a recombinator tree for every gear estimate that has 2+ wanted mods.

    All their leaf donors are solved in one parallel batch (and plan-cached).
    """
    from . import recomb

    db = db or ModDB()
    planners = []
    for e in estimates:
        if e.kind != "gear" or e.problem is None or e.error or not (bid := db.find_base(e.base)):
            continue
        planner = recomb.Planner(db, e.problem, bid, e.item_level)
        if planner.ok:
            planners.append((e, planner))
    leaves = [d for _, p in planners for d in p.leaves()]
    if progress:
        progress(f"recombinator donors: {len(leaves)} for {len(planners)} items")
    solve_all(leaves, table, workers, progress)
    for e, planner in planners:
        e.recomb = planner.best()


def add_fractures(est: Estimate, league: str) -> None:
    """The same craft starting from a base with one wanted mod already fractured.

    One option per target that can roll naturally (an essence-only mod can't
    be on a base you buy), and one per filler family the item uses (e.g. "any
    fractured fire resistance at T3+"). The solver's numbers come from `solve_all`.
    """
    if est.problem is None or est.error or est.fractures:
        return
    p = est.problem
    if len(p.targets) + p.filler_need < 2:
        return
    pool = {m.id: m for m in p.pool}
    mods = data.load("mods")
    for family in est.filler_families:
        ids = [m for m in p.filler if mods[m]["type"] == family and m in pool]
        if not ids:
            continue
        mod = max((pool[m] for m in ids), key=lambda m: m.weight)
        # A fractured filler mod counts toward filler_need through the base state.
        problem = dataclasses.replace(p, fixed=p.fixed + [dataclasses.replace(mod, weight=0)], actions=list(p.actions))
        f = Fracture(f"{mod.label} (filler)", mod.id, problem)
        frac = [(sid, trade.stat_min(ids))] if (sid := trade.stat_id(mod.id, "fractured")) else []
        frac += [(sid, trade.stat_min([m])) for m in est.fixed_ids if (sid := trade.stat_id(m, "fractured"))]
        if frac:
            f.query = trade.item_query(**est.search, stats=frac, item_level=est.item_level, fractured_only=True)
            f.trade_url = trade.search_url(league, f.query)
        est.fractures.append(f)
    for i, t in enumerate(p.targets):
        candidates = [pool[m] for m in t.good if m in pool]
        if not candidates:
            continue
        mod = max(candidates, key=lambda m: m.weight)  # the commonest acceptable tier
        problem = dataclasses.replace(
            p,
            targets=p.targets[:i] + p.targets[i + 1 :],
            need=p.need - 1,
            fixed=p.fixed + [dataclasses.replace(mod, weight=0)],
            actions=list(p.actions),
        )
        f = Fracture(t.name, mod.id, problem)
        if est.kind == "cluster":
            c = est.search
            f.query = trade.cluster_query(c["size"], c["enchant"], c["passives"], [], est.item_level, fractured_ids=[mod.id] + est.fixed_ids)
        else:
            frac = [(sid, trade.stat_min(t.good if m == mod.id else [m])) for m in [mod.id] + est.fixed_ids if (sid := trade.stat_id(m, "fractured"))]
            if frac:
                f.query = trade.item_query(**est.search, stats=frac, item_level=est.item_level, fractured_only=True)
        if f.query:
            f.trade_url = trade.search_url(league, f.query)
        est.fractures.append(f)


# -- solving ---------------------------------------------------------------------------


PLAN_CACHE = Path(".cache/plans")


def plan_key(problem: craft.Problem, table: dict[str, float]) -> str:
    """A stable hash of a problem and the prices its actions use (to 3 significant figures).

    Sets are sorted so the key is the same across processes (string hashing is
    randomised per process, so pickles of frozensets are not).
    """
    used = sorted({pid for a in problem.actions for pid, _ in a.cost})
    rounded = [(pid, float(f"{table[pid]:.3g}")) for pid in used if pid in table]
    return hashlib.sha1(repr((_canon(problem), rounded, PLAN_VERSION)).encode()).hexdigest()


PLAN_VERSION = 1  # bump when the solver's answers change, to drop cached plans


def _canon(x):
    if dataclasses.is_dataclass(x):
        return (type(x).__name__, tuple(_canon(getattr(x, f.name)) for f in dataclasses.fields(x)))
    if isinstance(x, (set, frozenset)):
        return ("set", tuple(sorted((_canon(v) for v in x), key=repr)))
    if isinstance(x, dict):
        return ("dict", tuple(sorted(((_canon(k), _canon(v)) for k, v in x.items()), key=repr)))
    if isinstance(x, (list, tuple)):
        return tuple(_canon(v) for v in x)
    return x


def cached_plan(key: str) -> craft.Plan | None:
    path = PLAN_CACHE / f"{key}.pkl.z"
    try:
        return pickle.loads(zlib.decompress(path.read_bytes()))
    except (OSError, ValueError, pickle.UnpicklingError, zlib.error, EOFError, AttributeError):
        return None


def save_plan(key: str, plan: craft.Plan) -> None:
    try:
        PLAN_CACHE.mkdir(parents=True, exist_ok=True)
        tmp = PLAN_CACHE / f"{key}.{os.getpid()}.tmp"
        tmp.write_bytes(zlib.compress(pickle.dumps(plan)))
        tmp.replace(PLAN_CACHE / f"{key}.pkl.z")
    except OSError:
        pass  # a read-only or full disk only costs the cache


def solve_one(est, table: dict[str, float]) -> None:
    """Solve an Estimate or Fracture in place (from the plan cache when it has it)."""
    key = plan_key(est.problem, table)
    if plan := cached_plan(key):
        _attach(est, plan)
        return
    try:
        plan = craft.solve(est.problem, table)
    except (ValueError, ArithmeticError, np.linalg.LinAlgError) as e:
        est.error = str(e)
        return
    save_plan(key, plan)
    _attach(est, plan)


def solve_all(items: list, table: dict[str, float], workers: int | None = None, progress: Progress | None = None) -> None:
    """Solve every Estimate/Fracture that has a problem but no plan yet, in parallel processes.

    Each solve is independent CPU work in Python and numpy, so processes (not
    threads) scale with cores. Biggest problems go first so no core idles at the end.
    """
    say = progress or (lambda _msg: None)
    todo = []
    for e in items:
        if e.problem is None or e.plan is not None or e.error is not None:
            continue
        e._plan_key = plan_key(e.problem, table)
        if plan := cached_plan(e._plan_key):
            _attach(e, plan)
        else:
            todo.append(e)
    todo.sort(key=lambda e: -len(e.problem.targets))
    workers = workers or os.cpu_count() or 1
    if workers == 1 or len(todo) < 4:
        for i, e in enumerate(todo, 1):
            say(f"solving {i}/{len(todo)}")
            solve_one(e, table)
        return
    try:
        with _worker_env(), ProcessPoolExecutor(min(workers, len(todo)), mp_context=multiprocessing.get_context("spawn")) as pool:
            futures = {pool.submit(craft.solve, e.problem, table): e for e in todo}
            for i, done in enumerate(as_completed(futures), 1):
                e = futures[done]
                say(f"solved {i}/{len(todo)}")
                try:
                    plan = done.result()
                except (ValueError, ArithmeticError, np.linalg.LinAlgError) as err:
                    e.error = str(err)
                    continue
                save_plan(e._plan_key, plan)
                _attach(e, plan)
    except (BrokenProcessPool, OSError, RuntimeError):
        # Workers can't start where the main module can't be re-imported (a REPL,
        # stdin, a notebook, or a script without `if __name__ == "__main__"`).
        solve_all(todo, table, workers=1, progress=progress)


@contextmanager
def _worker_env() -> Iterator[None]:
    """What spawned solver processes need while the pool starts and runs.

    One BLAS thread each: N processes x N BLAS threads ran 7x slower than one
    process. Spawned workers import numpy fresh, so they read these variables;
    forked ones would inherit the parent's thread pool instead.

    A real stderr: multiprocessing hands stderr's file descriptor to its helper
    process, and a TUI (textual) swaps stderr for an object without a valid one.
    """
    names = ("OPENBLAS_NUM_THREADS", "OMP_NUM_THREADS", "MKL_NUM_THREADS")
    saved = {n: os.environ.get(n) for n in names}
    os.environ.update({n: "1" for n in names})
    saved_stderr = sys.stderr
    if not _has_fd(sys.stderr) and _has_fd(sys.__stderr__):
        sys.stderr = sys.__stderr__
    try:
        yield
    finally:
        sys.stderr = saved_stderr
        for n, v in saved.items():
            if v is None:
                os.environ.pop(n, None)
            else:
                os.environ[n] = v


def _has_fd(stream) -> bool:
    try:
        return stream is not None and stream.fileno() >= 0
    except (AttributeError, OSError, ValueError):
        return False


def _attach(est, plan: craft.Plan) -> None:
    # A worker solved a copy; drop the same unpriced actions here so recipe and simulate agree.
    est.problem.actions = [a for a in est.problem.actions if a.name not in plan.unpriced]
    est.plan = plan


# -- trade -----------------------------------------------------------------------------


def price_on_trade(
    estimates: list[Estimate], table: dict[str, float], league: str, budget: int = 40, progress: Progress | None = None
) -> int:
    """Price `estimates` in order: each one's selling price, then its fractured base, then fracture bases.

    Stops after `budget` searches that weren't cached. Returns searches used.
    """
    say = progress or (lambda _msg: None)
    used = 0
    jobs = []
    for e in estimates:
        if e.query and not e.market:
            jobs.append((e, "market"))
        if e.base_query and e.base_price is None:
            jobs.append((e, "base"))
    # Fractured bases: every item's two most promising first (cheapest to finish), then the rest.
    ranked = {id(e): sorted((f for f in e.fractures if f.query and f.plan), key=lambda f: f.plan.cost) for e in estimates}
    for top in (slice(0, 2), slice(2, None)):
        for e in estimates:
            jobs += [(f, "fracture") for f in ranked[id(e)][top] if f.base_price is None]
    for i, (obj, what) in enumerate(jobs, 1):
        if used >= budget:
            say(f"trade budget of {budget} searches used; {len(jobs) - i + 1} left unpriced")
            break
        name = obj.target if what == "fracture" else obj.title
        say(f"trade {i}/{len(jobs)}: {what} · {name}")
        used += lookup(obj, what, table, league)
    return used


def price_fractures(
    estimates: list[Estimate], table: dict[str, float], league: str, per_item: int = 2, budget: int = 40, progress: Progress | None = None
) -> int:
    """Batch-price fractured bases: each item's `per_item` most promising fractures (cheapest to finish)."""
    say = progress or (lambda _msg: None)
    jobs = []
    for e in estimates:
        best = sorted((f for f in e.fractures if f.query and f.plan and f.base_price is None), key=lambda f: f.plan.cost)
        jobs += best[:per_item]
    used = 0
    for i, f in enumerate(jobs, 1):
        if used >= budget:
            say(f"trade budget of {budget} searches used; {len(jobs) - i + 1} fractured bases left unpriced")
            break
        say(f"fractured base {i}/{len(jobs)}: {f.target}")
        used += lookup(f, "fracture", table, league)
    return used


def lookup(obj, what: str, table: dict[str, float], league: str) -> int:
    """One trade search for an Estimate's market ("market") or base ("base"), or a Fracture's base.

    Returns 1 if it hit the site, 0 if the answer was cached.
    """
    query = {"market": getattr(obj, "query", None), "base": getattr(obj, "base_query", None), "fracture": obj.query}[what]
    if not query:
        return 0
    before = trade.searches
    try:
        found = trade.price(league, query, table)
        err = None if found else "no listings"
    except urllib.error.HTTPError as e:
        found, err = None, f"trade {e.code}: " + ("check POESESSID in .env" if e.code in (401, 403) else "try again later")
    except (urllib.error.URLError, OSError, ValueError) as e:
        found, err = None, f"trade failed: {e}"
    if what == "market":
        obj.market, obj.market_error = found, err
    elif what == "base":
        obj.base_price, obj.base_error = found, err
        obj.base_source = "trade" if found else obj.base_source
    else:
        obj.base_price, obj.base_error = found, err
    return trade.searches - before


def lookup_market(est: Estimate, table: dict[str, float], league: str) -> None:
    """Price one estimate: its selling price and, if it needs one, its fractured base."""
    lookup(est, "market", table, league)
    if est.base_query and est.base_price is None:
        lookup(est, "base", table, league)


# -- sources ---------------------------------------------------------------------------


def expand_sources(sources: list[str]) -> list[str]:
    """A text file listing links (one per line, # comments) stands for those links."""
    out = []
    for source in sources:
        path = Path(source)
        if len(source) < 1024 and path.is_file() and path.suffix == ".txt":
            lines = [line.split("#")[0].strip() for line in path.read_text().splitlines()]
            links = [line for line in lines if line]
            if links and all(line.startswith("http") for line in links):
                out.extend(links)
                continue
        out.append(source)
    return out


def build_name(build: pob.Build) -> str:
    return f"{build.ascendancy or build.class_name} lvl {build.level}: {build.main_skill or '?'}"


def current_league() -> str:
    try:
        return prices.leagues()[0]
    except (urllib.error.URLError, OSError, IndexError, KeyError, ValueError):
        return "Standard"
