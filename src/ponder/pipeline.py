"""The whole pipeline as a library: builds in, priced crafting estimates out.

`run(sources)` loads each build, matches its rares to mod ids, works out the
cheapest craft for every gear rare and cluster jewel, merges the same item
across builds, ranks mod combinations the builds share, and (with `trade`)
looks up selling prices for cluster jewels. Nothing here prints; the CLI and
TUI render the `Report`, and `Report.to_dict()` gives plain JSON.
"""

from __future__ import annotations

import multiprocessing
import os
import sys
import time
import urllib.error
from collections.abc import Callable, Iterator
from concurrent.futures import ProcessPoolExecutor, as_completed
from contextlib import contextmanager
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np

from . import cluster, craft, gear, pob, prices, trade
from .item import parse_item
from .mine import Candidate, mine, target_key
from .mods import ItemMatch, ModDB

Progress = Callable[[str], None]


@dataclass
class Estimate:
    """Expected cost of crafting one item, and what it sells for when known."""

    kind: str  # "cluster" or "gear"
    title: str  # e.g. "medium cluster, 10% increased Area Damage" or "Warlock Gloves"
    targets: list[str]
    key: tuple = ()  # identifies the same craft across builds
    slot: str | None = None
    builds: list[str] = field(default_factory=list)  # sources whose rares need this craft
    item_level: int = 84
    problem: craft.Problem | None = None
    plan: craft.Plan | None = None
    bench: list[tuple[str, float]] = field(default_factory=list)  # (label, chaos) crafted last
    skipped: list[str] = field(default_factory=list)  # mods the plan doesn't make
    fixed: list[str] = field(default_factory=list)  # fractured mods on the base
    base_price: tuple[float, int] | None = None  # (chaos, listings) from poe.ninja
    query: dict | None = None  # trade-site query, when we can build one
    trade_url: str | None = None
    market: tuple[float, int] | None = None  # (median of cheapest asks, total listed)
    market_error: str | None = None
    error: str | None = None  # why there is no plan

    @property
    def craft_cost(self) -> float | None:
        return self.plan.cost if self.plan else None

    @property
    def bench_cost(self) -> float:
        return sum(c for _, c in self.bench)

    @property
    def total(self) -> float | None:
        if self.plan is None:
            return None
        return self.plan.cost + self.bench_cost + (self.base_price[0] if self.base_price else 0.0)

    @property
    def margin(self) -> float | None:
        return self.market[0] - self.total if self.market and self.total is not None else None

    def recipe(self, rows: int = 12) -> list[tuple[str, str, float]]:
        return craft.recipe(self.problem, self.plan)[:rows] if self.plan else []

    def usage(self) -> list[tuple[str, float]]:
        if not self.plan:
            return []
        return [(a, n) for a, n in sorted(self.plan.usage.items(), key=lambda x: -x[1]) if n >= 0.05]

    def to_dict(self) -> dict:
        return {
            "kind": self.kind,
            "title": self.title,
            "targets": self.targets,
            "slot": self.slot,
            "builds": self.builds,
            "item_level": self.item_level,
            "craft_cost": self.craft_cost,
            "bench": [{"mod": label, "chaos": c} for label, c in self.bench],
            "base_price": self.base_price[0] if self.base_price else None,
            "total": self.total,
            "market": self.market[0] if self.market else None,
            "listed": self.market[1] if self.market else None,
            "margin": self.margin,
            "currency": dict(self.usage()),
            "recipe": [{"state": s, "action": a, "share": share} for s, a, share in self.recipe()],
            "fractured": self.fixed,
            "not_crafted": self.skipped,
            "trade_url": self.trade_url,
            "error": self.error or self.market_error,
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

    def ranked(self) -> list[Estimate]:
        """Best margin first, then the dearest crafts (where tooling saves the most)."""
        return sorted(self.estimates, key=lambda e: (e.margin is None, -(e.margin or 0), -(e.total or 0)))

    def to_dict(self) -> dict:
        return {
            "league": self.league,
            "prices": self.prices_source,
            "builds": [vars(b) for b in self.builds],
            "estimates": [e.to_dict() for e in self.ranked()],
            "shared": [s.to_dict() for s in self.shared],
            "errors": self.errors,
        }


def run(
    sources: list[str],
    *,
    settle: int = 0,
    max_targets: int = 4,
    min_builds: int = 2,
    top: int = 30,
    trade_lookup: bool = False,
    workers: int | None = None,
    progress: Progress | None = None,
) -> Report:
    """`workers` processes solve in parallel (default: every CPU; 1 solves inline)."""
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
            key = item_key(match, settle)
            if key is None:
                continue
            if key not in by_key:
                by_key[key] = estimate_item(db, match, table, league, settle=settle, max_targets=max_targets, solve=False)
                by_key[key].key = key
                by_key[key].slot = pob_item.slot
            if source not in by_key[key].builds:
                by_key[key].builds.append(source)
        infos.append(BuildInfo(source, build_name(build), rares))

    estimates = [e for e in by_key.values() if e.targets]
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
    solve_all(estimates + [s.estimate for s in shared if s.estimate and s.estimate not in estimates], table, workers, say)

    if trade_lookup:
        for e in [e for e in estimates if e.query]:
            say(f"trade lookup: {', '.join(e.targets)}")
            lookup_market(e, table, league)
            time.sleep(2)  # the trade API rate-limits hard
    return Report(infos, estimates, shared, prices_source, league, errors)


def item_key(match: ItemMatch, settle: int = 0) -> tuple | None:
    """What makes two rares the same craft, or None when we can't craft it."""
    item = match.item
    if item.base.endswith("Cluster Jewel"):
        cls, notables = target_key(match)
        if " cluster: " not in cls or not notables:
            return None
        size, tag = cls.split(" cluster: ")
        return ("cluster", size, tag, tuple(notables), item.header.get("Cluster Jewel Node Count"))
    if match.base_id is None:
        return None
    families = tuple(sorted((m.family, m.tier) for m in match.mods if m.source == "natural"))
    return ("gear", item.base, families, settle, tuple(sorted(m.mod_id for m in match.mods if m.source != "natural")))


def estimate_item(db: ModDB, match: ItemMatch, table: dict[str, float], league: str, settle: int = 0, max_targets: int = 4, solve: bool = True) -> Estimate:
    item = match.item
    if item.base.endswith("Cluster Jewel"):
        cls, notables = target_key(match)
        size, tag = cls.split(" cluster: ")
        count = item.header.get("Cluster Jewel Node Count")
        return estimate_cluster(size, tag, notables, table, league, passives=int(count) if count else None, solve=solve)
    return estimate_gear(db, match, table, settle=settle, max_targets=max_targets, solve=solve)


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
        f"{size} cluster, {ench.short}, {passives} passives",
        list(notables),
        key=("cluster", size, ench.tag, tuple(sorted(notables)), str(passives)),
        item_level=item_level,
        fixed=list(fractured),
    )
    try:
        est.problem = cluster.problem(size, ench.tag, list(notables), need, item_level, fractured)
    except ValueError as e:
        est.error = str(e)
        return est
    if need and need < len(notables):
        est.title += f" ({need} of {len(notables)})"
    if not fractured:
        est.base_price = prices.cluster_base(size, ench.short, passives, item_level)
    wanted = [m.id for m in cluster.resolve_notables(est.problem.pool, list(notables))]
    est.query = trade.cluster_query(size, prices.enchant_option(ench.short), passives, wanted, item_level)
    est.trade_url = trade.search_url(league, est.query)
    if solve:
        solve_one(est, table)
    return est


def estimate_gear(
    db: ModDB, match: ItemMatch, table: dict[str, float], *, settle: int = 0, item_level: int | None = None, max_targets: int = 4, solve: bool = True
) -> Estimate:
    gp = gear.build(db, match, settle, item_level)
    est = Estimate(
        "gear",
        match.item.base,
        [t.name for t in gp.problem.targets],
        item_level=gp.item_level,
        problem=gp.problem,
        bench=[(label, gear.action_cost_of(cost, table)) for label, cost in gp.bench],
        skipped=list(gp.skipped),
        fixed=[m.label for m in gp.problem.fixed],
    )
    if len(gp.problem.targets) > max_targets:
        est.error = f"{len(gp.problem.targets)} target mods (raise max_targets, or settle to loosen)"
        return est
    if solve:
        solve_one(est, table)
    return est


def solve_one(est: Estimate, table: dict[str, float]) -> None:
    try:
        _attach(est, craft.solve(est.problem, table))
    except (ValueError, ArithmeticError, np.linalg.LinAlgError) as e:
        est.error = str(e)


def solve_all(estimates: list[Estimate], table: dict[str, float], workers: int | None = None, progress: Progress | None = None) -> None:
    """Solve every estimate that has a problem but no plan yet, in parallel processes.

    Each solve is independent CPU work in Python and numpy, so processes (not
    threads) scale with cores. Biggest problems go first so no core idles at the end.
    """
    say = progress or (lambda _msg: None)
    todo = [e for e in estimates if e.problem is not None and e.plan is None and e.error is None]
    todo.sort(key=lambda e: -len(e.problem.targets))
    workers = workers or os.cpu_count() or 1
    if workers == 1 or len(todo) < 4:
        for i, e in enumerate(todo, 1):
            say(f"solving {i}/{len(todo)}: {e.title}")
            solve_one(e, table)
        return
    with _worker_env(), ProcessPoolExecutor(min(workers, len(todo)), mp_context=multiprocessing.get_context("spawn")) as pool:
        futures = {pool.submit(craft.solve, e.problem, table): e for e in todo}
        for i, done in enumerate(as_completed(futures), 1):
            e = futures[done]
            say(f"solved {i}/{len(todo)}: {e.title}")
            try:
                _attach(e, done.result())
            except (ValueError, ArithmeticError, np.linalg.LinAlgError) as err:
                e.error = str(err)


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


def _attach(est: Estimate, plan: craft.Plan) -> None:
    # A worker solved a copy; drop the same unpriced actions here so recipe and simulate agree.
    est.problem.actions = [a for a in est.problem.actions if a.name not in plan.unpriced]
    est.plan = plan


def lookup_market(est: Estimate, table: dict[str, float], league: str) -> None:
    """Fill `est.market` from the trade site; failures land in `est.market_error`."""
    if not est.query:
        est.market_error = "no trade query for this item"
        return
    try:
        found = trade.price(league, est.query, table)
    except urllib.error.HTTPError as e:
        hint = "check your POESESSID (`ponder login`)" if e.code in (401, 403) else "try again later"
        est.market_error = f"trade lookup failed ({e.code}); {hint}, or use the link"
        return
    except (urllib.error.URLError, OSError) as e:
        est.market_error = f"trade lookup failed ({e})"
        return
    est.market = found
    est.market_error = None if found else "no listings found"


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
