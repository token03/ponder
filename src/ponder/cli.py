"""Command line entry point: `ponder [run] SOURCES` and the `data|prices|items|mine|craft|cluster|tui` tools.

The work happens in `pipeline` and the other library modules; this file only
parses arguments and renders results with rich.
"""

from __future__ import annotations

import argparse
import json
import sys

from rich.console import Console
from rich.markup import escape
from rich.table import Table

from . import cluster, data, gear, pipeline, pob, prices, render, trade
from .item import parse_item
from .mine import mine
from .mods import ModDB

COMMANDS = {"run", "data", "prices", "items", "mine", "craft", "cluster", "tui", "ladder"}
SOURCES_HELP = "pobb.in/pastebin URL, file, PoB code, or .txt list of links"

# Piped output (files, scripts, LLMs) never wraps, so long trade links stay whole.
console = Console() if sys.stdout.isatty() else Console(width=100_000)


def out(*renderables, **kw) -> None:
    """console.print, minus the padding rich adds out to a very wide piped console."""
    if console.is_terminal:
        console.print(*renderables, **kw)
        return
    with console.capture() as cap:
        console.print(*renderables, **kw)
    sys.stdout.write("\n".join(line.rstrip() for line in cap.get().splitlines()) + "\n")


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    if argv and argv[0] not in COMMANDS and argv[0] not in ("-h", "--help"):
        argv.insert(0, "run")  # `ponder build.txt` is `ponder run build.txt`
    args = parser().parse_args(argv)
    render.DIVINE = prices.load()[0].get("divine", 0.0)
    try:
        return COMMAND_FUNCS[args.command](args)
    except FileNotFoundError as e:
        out(str(e), style="red", markup=False)
        return 1


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="ponder", description="Cost crafting the rares in PoB builds. `ponder SOURCES` runs the whole pipeline.")
    sub = p.add_subparsers(dest="command", required=True)

    run = sub.add_parser("run", help="the whole pipeline: cost every rare, mine shared combos, price on trade")
    run.add_argument("sources", nargs="+", help=SOURCES_HELP)
    run.add_argument("--trade", action="store_true", help="price finished items and fractured bases on the trade site (POESESSID in .env)")
    run.add_argument("--trade-budget", type=int, default=40, help="most trade searches to send (cached answers are free; default 40)")
    run.add_argument("--fractures", action="store_true", help="also cost starting from a base with each target already fractured")
    run.add_argument("--recomb", action="store_true", help="also cost recombinator trees of clean donors (NNN padding included)")
    run.add_argument("--exact", action="store_true", help="every mod as on the PoB item (resistances aren't treated as interchangeable filler)")
    run.add_argument("--settle", type=int, default=0, help="accept tiers this many steps worse")
    run.add_argument("--max-targets", type=int, default=4, help="skip items with more target mods than this")
    run.add_argument("--min-builds", type=int, default=2)
    run.add_argument("--top", type=int, default=40, help="rows to show")
    run.add_argument("--detail", type=int, default=3, help="show the full recipe for this many top rows")
    run.add_argument("--workers", type=int, help="processes solving in parallel (default: all CPUs)")
    run.add_argument("--json", action="store_true", help="print the report as JSON")

    tui = sub.add_parser("tui", help="browse a run interactively (needs the `tui` extra)")
    tui.add_argument("sources", nargs="*", help=SOURCES_HELP)
    tui.add_argument("--trade", action="store_true")
    tui.add_argument("--ladder", action="store_true", help="browse ladder crafts (`ponder ladder fetch` first) instead of builds")

    lad = sub.add_parser("ladder", help="what ladder characters wear (poe.ninja): tiers that matter, archetypes")
    lad_sub = lad.add_subparsers(dest="ladder_command", required=True)
    lad_fetch = lad_sub.add_parser("fetch", help="download the top characters of every popular skill and class")
    lad_fetch.add_argument("--league", help="league name (default: the current one)")
    lad_fetch.add_argument("--per-skill", type=int, default=10)
    lad_fetch.add_argument("--per-class", type=int, default=20)
    lad_fetch.add_argument("--min-players", type=int, default=40, help="skills with fewer players are skipped")
    lad_tiers = lad_sub.add_parser("tiers", help="per base group: how often each mod is worn, and how much its tier matters")
    lad_tiers.add_argument("--group", help='only this base group, e.g. "Gloves (ES)" (substring)')
    lad_tiers.add_argument("--min-characters", type=int, default=15)
    lad_arch = lad_sub.add_parser("archetypes", help="mod combinations many characters share (crafting candidates)")
    lad_arch.add_argument("--group", help="only base groups containing this text")
    lad_arch.add_argument("--min-characters", type=int, default=10)
    lad_arch.add_argument("--top", type=int, default=40)
    lad_price = lad_sub.add_parser("prices", help="fit what each core mod's tier is worth from trade listings (POESESSID in .env)")
    lad_price.add_argument("--group", help="only base groups containing this text")
    lad_price.add_argument("--min-characters", type=int, default=10)
    lad_price.add_argument("--top", type=int, default=5, help="archetypes to price (one search + 5 fetches each)")
    lad_price.add_argument("--listings", type=int, default=50)

    lad_crafts = lad_sub.add_parser("crafts", help="cost what the ladder wears (gear archetypes + cluster jewels); rank market holes")
    lad_crafts.add_argument("--group", help="only base groups / cluster enchants containing this text")
    lad_crafts.add_argument("--min-characters", type=int, default=5, help="rarer combinations are skipped (lower finds more niche ones)")
    lad_crafts.add_argument("--gear", type=int, default=80, help="gear archetypes to cost, most worn first")
    lad_crafts.add_argument("--per-group", type=int, default=3, help="also cost the top few of every base group (niche groups)")
    lad_crafts.add_argument("--niche", type=int, default=40, help="also cost the most niche 3+ mod combos (worn together far above chance)")
    lad_crafts.add_argument("--clusters", type=int, default=80, help="cluster notable combos to cost")
    lad_crafts.add_argument("--trade", action="store_true", help="price finished items on trade: market, listings, hole score (POESESSID in .env)")
    lad_crafts.add_argument("--trade-budget", type=int, default=40)
    lad_crafts.add_argument("--recomb", action="store_true", help="also cost recombinator trees (slow the first time; plans are cached)")
    lad_crafts.add_argument("--top", type=int, default=50)
    lad_crafts.add_argument("--detail", type=int, default=0, help="show the full recipe for this many top rows")
    lad_crafts.add_argument("--workers", type=int)
    lad_crafts.add_argument("--json", action="store_true")

    data_cmd = sub.add_parser("data", help="manage downloaded game data")
    data_cmd.add_argument("action", choices=["fetch", "version"])

    prices_cmd = sub.add_parser("prices", help="poe.ninja prices used for costs")
    prices_cmd.add_argument("action", choices=["fetch", "show"])
    prices_cmd.add_argument("--league", help="poe.ninja league id (default: current challenge league)")

    items_cmd = sub.add_parser("items", help="show the rare items in builds, matched to mod ids")
    items_cmd.add_argument("sources", nargs="+", help=SOURCES_HELP)
    items_cmd.add_argument("--all", action="store_true", help="include non-rare items")

    mine_cmd = sub.add_parser("mine", help="rank mod combinations shared across builds")
    mine_cmd.add_argument("sources", nargs="+", help=SOURCES_HELP)
    mine_cmd.add_argument("--min-builds", type=int, default=2)
    mine_cmd.add_argument("--top", type=int, default=30)

    craft_cmd = sub.add_parser("craft", help="cheapest recipe for rares in a build")
    craft_cmd.add_argument("source", help="pobb.in/pastebin URL, file, or PoB code")
    craft_cmd.add_argument("--slot", help='only items in this slot, e.g. "Helmet"')
    craft_cmd.add_argument("--settle", type=int, default=0, help="accept tiers this many steps worse")
    craft_cmd.add_argument("--ilvl", type=int, help=f"item level when PoB has none (default {gear.DEFAULT_ITEM_LEVEL})")
    craft_cmd.add_argument("--max-targets", type=int, default=4, help="skip items with more target mods than this")

    cl = sub.add_parser("cluster", help="cluster jewel enchants, notables and crafting costs")
    cl_sub = cl.add_subparsers(dest="cluster_command", required=True)
    cl_ench = cl_sub.add_parser("enchants", help="list enchants for a size")
    cl_ench.add_argument("size", choices=list(cluster.SIZES))
    cl_not = cl_sub.add_parser("notables", help="notables that can roll for an enchant")
    cl_not.add_argument("size", choices=list(cluster.SIZES))
    cl_not.add_argument("enchant", help='enchant name or tag, e.g. "cold damage"')
    cl_not.add_argument("--ilvl", type=int, default=84)
    cl_cost = cl_sub.add_parser("cost", help="expected cost, recipe and margin for notables and Group@tier small-passive mods")
    cl_cost.add_argument("size", choices=list(cluster.SIZES))
    cl_cost.add_argument("enchant")
    cl_cost.add_argument("notables", nargs="+")
    cl_cost.add_argument("--need", type=int, help="how many of the notables are required (default all)")
    cl_cost.add_argument("--ilvl", type=int, default=84)
    cl_cost.add_argument("--passives", type=int, help="added passives on the base (default fewest)")
    cl_cost.add_argument("--fractured", nargs="*", default=[], help="notables (or Group@tier mods) already fractured on the base")
    cl_cost.add_argument("--trade", action="store_true", help="look up selling prices on the trade site")
    return p


def _links() -> bool:
    return console.is_terminal


def _rule() -> None:
    console.rule() if console.is_terminal else out("-" * 80)


def _print_estimates(rows, title: str) -> None:
    links = _links()
    out()
    out(render.estimates_table(rows, links, title=title))
    if not links:
        for i, e in enumerate(rows, 1):
            if e.trade_url:
                out(f"  #{i} trade: {e.trade_url}", markup=False, highlight=False)


def _run(args) -> int:
    options = dict(
        settle=args.settle,
        max_targets=args.max_targets,
        min_builds=args.min_builds,
        fractures=args.fractures,
        exact=args.exact,
        trade_lookup=args.trade,
        trade_budget=args.trade_budget,
        recombs=args.recomb,
        workers=args.workers,
    )
    if args.json:
        print(json.dumps(pipeline.run(args.sources, **options).to_dict(), indent=1))
        return 0
    with console.status("starting") as status:
        trade.on_wait = lambda secs: status.update(f"waiting {secs:.0f}s for the trade site's rate limit")
        report = pipeline.run(args.sources, progress=lambda msg: status.update(escape(msg[:100])), **options)
    links = _links()
    out(render.report_summary(report))
    ranked = report.ranked()
    clusters = [e for e in ranked if e.kind == "cluster"]
    gear_rows = [e for e in ranked if e.kind == "gear"]
    if clusters:
        _print_estimates(clusters[: args.top], "Cluster jewels")
    if gear_rows:
        _print_estimates(gear_rows[: args.top], "Gear")
    if report.shared:
        out()
        out(render.shared_table(report.shared[: args.top], links))
    for e in [e for e in clusters + gear_rows if e.plan][: args.detail]:
        _rule()
        out(render.estimate_detail(e, report.prices_source, links))
    hints = []
    if not args.trade:
        hints.append("--trade to price items on the trade site")
    if not args.fractures:
        hints.append("--fractures to cost starting from fractured bases")
    if hints:
        out("\n[dim]add " + " · ".join(hints) + "[/dim]")
    return 0


def _tui(args) -> int:
    try:
        from .tui import run_tui
    except ImportError:
        out("the TUI needs textual: `uv sync --extra tui` (or `pip install 'ponder[tui]'`)", style="red", markup=False)
        return 1
    return run_tui(args.sources, trade=args.trade, ladder=args.ladder)


def _ladder(args) -> int:
    from . import demand, ladder

    if args.ladder_command == "fetch":
        snap = ladder.snapshot(args.league)
        with console.status("ladder") as status:
            n = sum(1 for _ in ladder.sample(snap, args.per_skill, args.min_players, args.per_class, progress=status.update))
        out(f"{n} characters from {snap.league} ({snap.version}) cached in {ladder.CACHE}", markup=False)
        return 0
    if args.ladder_command == "crafts":
        return _ladder_crafts(args)
    db = ModDB()
    with console.status("reading cached characters"):
        chars = list(ladder.cached())
        worn = demand.wear(db, chars)
    if not worn:
        out("no ladder data yet: run `ponder ladder fetch`", style="red", markup=False)
        return 1
    out(f"{len(chars)} characters, {len(worn)} rare items", style="dim", markup=False)
    if args.ladder_command == "tiers":
        stats = [s for s in demand.family_stats(db, worn, args.min_characters) if not args.group or args.group.lower() in s.group.lower()]
        out(render.tier_table(stats))
    elif args.ladder_command == "prices":
        return _ladder_prices(args, db, worn)
    else:
        found = [a for a in demand.archetypes(worn, args.min_characters) if not args.group or args.group.lower() in a.group.lower()]
        out(render.archetype_table(found[: args.top], db))
    return 0


def _ladder_crafts(args) -> int:
    from . import ladder

    if not any(ladder.cached()):
        out("no ladder data yet: run `ponder ladder fetch`", style="red", markup=False)
        return 1
    with console.status("ladder") as status:
        trade.on_wait = lambda secs: status.update(f"waiting {secs:.0f}s for the trade site's rate limit")
        r = pipeline.ladder(
            gear_top=args.gear,
            per_group=args.per_group,
            niche=args.niche,
            cluster_top=args.clusters,
            min_characters=args.min_characters,
            group=args.group,
            trade_lookup=args.trade,
            trade_budget=args.trade_budget,
            recombs=args.recomb,
            workers=args.workers,
            progress=status.update,
        )
    if args.json:
        print(json.dumps(r.to_dict(), indent=1, default=str))
        return 0
    out(render.ladder_summary(r))
    ranked = r.ranked()
    out(render.ladder_table(ranked[: args.top], _links(), title="Ladder crafts: market holes first, then most worn"))
    for e in [e for e in ranked if e.plan][: args.detail]:
        out(render.estimate_detail(e, r.prices_source, _links()))
    return 0


def _ladder_prices(args, db, worn) -> int:
    from . import demand, pricing

    found = [a for a in demand.archetypes(worn, args.min_characters) if not args.group or args.group.lower() in a.group.lower()]
    table, _ = prices.load()
    league = pipeline.current_league()
    for a in [a for a in found if len(a.families) >= 2][: args.top]:
        query = pricing.archetype_query(db, a.examples[0], a.families)
        if not query:
            continue
        with console.status(f"trade: {a.group}") as status:
            trade.on_wait = lambda secs: status.update(f"waiting {secs:.0f}s for the trade site's rate limit")
            try:
                listings = trade.listings(league, query, args.listings)
            except Exception as e:  # 403 here, or rate limits; keep going with the next one
                out(f"{a.group}: trade failed ({e})", style="red", markup=False)
                continue
        model = pricing.fit(db, a.group, a.families, listings, table)
        out(render.price_model(a, model, db, trade.search_url(league, query), _links()))
    return 0


def _data(args) -> int:
    if args.action == "fetch":
        with console.status("downloading repoe data"):
            version = data.fetch()
        out(f"fetched repoe data for {version} into {data.data_dir()}", markup=False)
    else:
        out(data.version() or "no data; run `ponder data fetch`", markup=False)
    return 0


def _prices(args) -> int:
    if args.action == "fetch":
        with console.status("fetching poe.ninja prices"):
            league, n = prices.fetch(args.league)
        out(f"fetched {n} prices for {league}", markup=False)
    table, source = prices.load()
    t = Table(title=f"prices from {source}", title_justify="left", box=None)
    t.add_column("chaos", justify="right")
    t.add_column("id")
    for pid in sorted(prices.SNAPSHOT):
        t.add_row(f"{table.get(pid, 0):.3f}", pid)
    out(t)
    return 0


def _items(args) -> int:
    db = ModDB()
    for source in pipeline.expand_sources(args.sources):
        build = pob.load(source)
        title = f"{escape(pipeline.build_name(build))}  [dim]{escape(source[:60])}[/dim]"
        console.rule(title) if console.is_terminal else out(f"== {title}")
        for pob_item in build.items:
            item = parse_item(pob_item.text)
            if item.rarity != "RARE" and not args.all:
                continue
            out(render.item_match(pob_item.slot, pob_item.sets, db.match(item)))
    return 0


def _mine(args) -> int:
    db = ModDB()
    sources = pipeline.expand_sources(args.sources)
    matched = []
    with console.status("loading builds") as status:
        for source in sources:
            status.update(f"loading {source[:70]}")
            for pob_item in pob.load(source).items:
                item = parse_item(pob_item.text)
                if item.rarity == "RARE":
                    matched.append((source, db.match(item)))
        table, source_note = prices.load()
        league = pipeline.current_league()
        shared = []
        for c in mine(matched, min_builds=args.min_builds)[: args.top]:
            est = None
            if " cluster: " in c.item_class:
                size, tag = c.item_class.split(" cluster: ")
                est = pipeline.estimate_cluster(size, tag, list(c.families), table, league, solve=False)
            shared.append(pipeline.Shared(c, est))
        pipeline.solve_all([x.estimate for x in shared if x.estimate], table, progress=status.update)
    out(f"{len(matched)} rare items from {len(sources)} builds; costs use {source_note}", markup=False)
    out(render.shared_table(shared, _links()))
    return 0


def _craft(args) -> int:
    db = ModDB()
    table, source = prices.load()
    build = pob.load(args.source)
    league = pipeline.current_league()
    seen, estimates = set(), []
    for pob_item in build.items:
        item = parse_item(pob_item.text)
        if item.rarity != "RARE" or (args.slot and pob_item.slot != args.slot):
            continue
        match = db.match(item)
        if match.base_id is None or item.base.endswith("Cluster Jewel"):
            continue  # clusters: use `ponder cluster cost`
        est = pipeline.estimate_gear(db, match, table, league, settle=args.settle, item_level=args.ilvl, max_targets=args.max_targets, solve=False)
        if est.key in seen:
            continue
        seen.add(est.key)
        if est.targets:
            est.slot = pob_item.slot or "unequipped"
            estimates.append(est)
    with console.status("solving") as status:
        pipeline.solve_all(estimates, table, progress=status.update)
    for est in estimates:
        _rule()
        out(render.estimate_detail(est, source, _links()))
    return 0


def _cluster(args) -> int:
    if args.cluster_command == "enchants":
        t = Table(box=None, header_style="bold")
        t.add_column("tag")
        t.add_column("enchant")
        for e in cluster.enchants(args.size):
            t.add_row(e.tag, " / ".join(e.text))
        out(t)
        return 0
    enchant = cluster.find_enchant(args.size, args.enchant)
    if args.cluster_command == "notables":
        mods = cluster.pool(args.size, enchant, args.ilvl)
        for side in ("prefix", "suffix"):
            side_total = sum(m.weight for m in mods if m.side == side)
            t = Table(title=f"{side}es ({side_total} total weight)", title_justify="left", box=None)
            t.add_column("weight", justify="right")
            t.add_column("chance", justify="right")
            t.add_column("notable")
            for m in sorted((m for m in mods if m.side == side and m.notable), key=lambda m: -m.weight):
                t.add_row(str(m.weight), f"{m.weight / side_total:.1%}", m.label)
            out(t)
        tiers = cluster.tiers(mods)
        t = Table(title="small-passive mods (name them as shown, e.g. HaveIncreasedEffect@1)", title_justify="left", box=None)
        t.add_column("craft name")
        t.add_column("side")
        t.add_column("weight", justify="right")
        t.add_column("mod")
        for m in sorted((m for m in mods if not m.notable and m.id in tiers), key=lambda m: (m.side, m.groups[0], tiers[m.id])):
            t.add_row(cluster.feature(m.id, tiers[m.id]), m.side, str(m.weight), cluster.feature_label(cluster.feature(m.id, tiers[m.id]), mods))
        out(t)
        return 0

    table, source = prices.load()
    league = pipeline.current_league()
    with console.status("solving"):
        est = pipeline.estimate_cluster(
            args.size, enchant.name, args.notables, table, league,
            passives=args.passives, item_level=args.ilvl, need=args.need, fractured=args.fractured,
        )  # fmt: skip
    if args.trade and est.query:
        with console.status("asking the trade site"):
            pipeline.lookup_market(est, table, league)
    out(render.estimate_detail(est, source, _links()))
    return 0 if est.error is None else 1


COMMAND_FUNCS = {
    "run": _run,
    "tui": _tui,
    "ladder": _ladder,
    "data": _data,
    "prices": _prices,
    "items": _items,
    "mine": _mine,
    "craft": _craft,
    "cluster": _cluster,
}


if __name__ == "__main__":
    sys.exit(main())
