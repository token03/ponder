"""Command line entry point: `ponder data|prices|items|mine|cluster|craft`."""

from __future__ import annotations

import argparse
import sys
import urllib.error
from pathlib import Path

from . import cluster, craft, data, gear, pob, prices, trade
from .item import parse_item
from .mine import mine
from .mods import ItemMatch, ModDB


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="ponder")
    sub = parser.add_subparsers(dest="command", required=True)

    data_cmd = sub.add_parser("data", help="manage downloaded game data")
    data_cmd.add_argument("action", choices=["fetch", "version"])

    prices_cmd = sub.add_parser("prices", help="poe.ninja prices used for costs")
    prices_cmd.add_argument("action", choices=["fetch", "show"])
    prices_cmd.add_argument("--league", help="poe.ninja league id (default: current challenge league)")

    items_cmd = sub.add_parser("items", help="show the rare items in builds, matched to mod ids")
    items_cmd.add_argument("sources", nargs="+", help="pobb.in/pastebin URL, file, PoB code, or .txt list of links")
    items_cmd.add_argument("--all", action="store_true", help="include non-rare items")

    mine_cmd = sub.add_parser("mine", help="rank mod combinations shared across builds")
    mine_cmd.add_argument("sources", nargs="+", help="pobb.in/pastebin URL, file, PoB code, or .txt list of links")
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
    cl_cost = cl_sub.add_parser("cost", help="expected cost, recipe and margin for notables")
    cl_cost.add_argument("size", choices=list(cluster.SIZES))
    cl_cost.add_argument("enchant")
    cl_cost.add_argument("notables", nargs="+")
    cl_cost.add_argument("--need", type=int, help="how many of the notables are required (default all)")
    cl_cost.add_argument("--ilvl", type=int, default=84)
    cl_cost.add_argument("--passives", type=int, help="added passives on the base (default fewest)")
    cl_cost.add_argument("--fractured", nargs="*", default=[], help="notables already fractured on the base")
    cl_cost.add_argument("--trade", action="store_true", help="look up selling prices on the trade site")

    args = parser.parse_args(argv)
    if args.command == "data":
        if args.action == "fetch":
            print(f"fetched repoe data for {data.fetch()} into {data.data_dir()}")
        else:
            print(data.version() or "no data; run `ponder data fetch`")
        return 0
    if args.command == "prices":
        if args.action == "fetch":
            league, n = prices.fetch(args.league)
            print(f"fetched {n} prices for {league}")
        table, source = prices.load()
        print(f"prices from {source}:")
        for pid in sorted(prices.SNAPSHOT):
            print(f"  {table.get(pid, 0):>9.3f}c  {pid}")
        return 0
    if args.command == "cluster":
        return _cluster(args)
    if args.command == "craft":
        return _craft(args)

    db = ModDB()
    matched: list[tuple[str, ItemMatch]] = []
    args.sources = _expand(args.sources)
    for source in args.sources:
        build = pob.load(source)
        if args.command == "items":
            print(f"== {build.ascendancy or build.class_name} lvl {build.level}: {build.main_skill}  [{source[:60]}]")
        for pob_item in build.items:
            item = parse_item(pob_item.text)
            if item.rarity != "RARE" and not getattr(args, "all", False):
                continue
            match = db.match(item)
            matched.append((source, match))
            if args.command == "items":
                _print_match(pob_item.slot, pob_item.sets, match)

    if args.command == "mine":
        table, source = prices.load()
        print(f"{len(matched)} rare items from {len(args.sources)} builds; costs use {source}")
        for c in mine(matched, min_builds=args.min_builds)[: args.top]:
            cost = _cluster_cost(c.item_class, c.families, table)
            print(f"{c.builds:>3} builds {c.count:>3} items  {c.item_class:<14} {' + '.join(c.families)}{cost}")
    return 0


def _expand(sources: list[str]) -> list[str]:
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


def _cluster_cost(key: str, notables, table) -> str:
    if " cluster: " not in key:
        return ""
    size, tag = key.split(" cluster: ")
    try:
        _, plan = cluster.cost(size, tag, list(notables), table)
    except ValueError as e:
        return f"  (no cost: {e})"
    base = prices.cluster_base(size, cluster.find_enchant(size, tag).short, cluster.DEFAULT_PASSIVES[size], 84)
    base_text = f" + {base[0]:,.0f}c base" if base else ""
    return f"  ~{plan.cost:,.0f}c craft{base_text}"


def _cluster(args) -> int:
    if args.cluster_command == "enchants":
        for e in cluster.enchants(args.size):
            print(f"{e.tag:<48} {' / '.join(e.text)}")
        return 0
    enchant = cluster.find_enchant(args.size, args.enchant)
    if args.cluster_command == "notables":
        mods = cluster.pool(args.size, enchant, args.ilvl)
        for side in ("prefix", "suffix"):
            side_total = sum(m.weight for m in mods if m.side == side)
            print(f"{side}es ({side_total} total weight):")
            for m in sorted((m for m in mods if m.side == side and m.notable), key=lambda m: -m.weight):
                print(f"  {m.weight:>6} {m.weight / side_total:>6.1%}  {m.label}")
        return 0

    table, source = prices.load()
    problem, plan = cluster.cost(args.size, enchant.tag, args.notables, table, args.need, args.ilvl, args.fractured)
    passives = args.passives or cluster.DEFAULT_PASSIVES[args.size]
    print(f"{args.size} cluster, {enchant.short}, {passives} passives, ilvl {args.ilvl}+: {problem.need} of {', '.join(args.notables)}")
    if args.fractured:
        print(f"starting from a base with fractured {', '.join(args.fractured)}")
    _print_plan(problem, plan, source)

    base = prices.cluster_base(args.size, enchant.short, passives, args.ilvl)
    total = plan.cost + (base[0] if base else 0)
    if base and not args.fractured:
        print(f"base: ~{base[0]:,.1f}c on poe.ninja ({base[1]} listed); total ~{total:,.1f}c")
    elif not args.fractured:
        print("base: no poe.ninja price (run `ponder prices fetch`)")

    league = _league()
    wanted = [m.id for m in problem.pool if m.label.lower() in {n.lower() for n in args.notables}]
    query = trade.cluster_query(args.size, prices.enchant_option(enchant.short), passives, wanted, args.ilvl)
    print(f"trade search: {trade.search_url(league, query)}")
    if args.trade:
        try:
            found = trade.price(league, query, table)
        except urllib.error.HTTPError as e:
            print(f"trade lookup failed ({e.code}); set POESESSID or use the link above")
            return 0
        if found:
            ask, listed = found
            print(f"market: median of cheapest asks ~{ask:,.1f}c ({listed} listed); margin ~{ask - total:,.1f}c")
        else:
            print("market: no listings found")
    return 0


def _craft(args) -> int:
    db = ModDB()
    table, source = prices.load()
    build = pob.load(args.source)
    seen = set()
    for pob_item in build.items:
        item = parse_item(pob_item.text)
        if item.rarity != "RARE" or (args.slot and pob_item.slot != args.slot):
            continue
        match = db.match(item)
        if match.base_id is None or item.base.endswith("Cluster Jewel"):
            continue  # clusters: use `ponder cluster cost`
        gp = gear.build(db, match, args.settle, args.ilvl)
        key = (item.base, tuple(t.name for t in gp.problem.targets))
        if key in seen or not gp.problem.targets:
            continue
        seen.add(key)
        print(f"\n[{pob_item.slot or 'unequipped'}] {item.base}, ilvl {gp.item_level}")
        for t in gp.problem.targets:
            print(f"   target {t.side:<6} {t.name}")
        for m in gp.problem.fixed:
            print(f"   fractured     {m.label}")
        for s in gp.skipped:
            print(f"   not crafted   {s}")
        if len(gp.problem.targets) > args.max_targets:
            print(f"   skipped: {len(gp.problem.targets)} targets (raise --max-targets, or --settle to loosen)")
            continue
        plan = craft.solve(gp.problem, table)
        _print_plan(gp.problem, plan, source, rows=6)
    return 0


def _print_plan(problem: craft.Problem, plan: craft.Plan, source: str, rows: int = 12) -> None:
    print(f"expected cost ~{plan.cost:,.1f}c (prices: {source}; base not included)")
    used = ", ".join(f"{n:,.1f} {a}" for a, n in sorted(plan.usage.items(), key=lambda x: -x[1]) if n >= 0.05)
    print(f"expected currency: {used}")
    print("recipe (most common situations first):")
    for state, action, share in craft.recipe(problem, plan)[:rows]:
        print(f"  {share:>6.1%}  {state:<62} -> {action}")


def _league() -> str:
    try:
        return prices.leagues()[0]
    except (urllib.error.URLError, OSError, IndexError):
        return "Standard"


def _print_match(slot: str | None, sets: list[str], m: ItemMatch) -> None:
    item = m.item
    extra = ", ".join(sorted(item.influences | item.flags))
    print(f"\n[{slot or 'unequipped'}] {item.name} / {item.base}  ilvl {item.item_level or '?'}" + (f"  ({extra})" if extra else ""))
    if sets:
        print(f"   sets: {', '.join(sets)}")
    if m.base_id is None:
        print("   ! base type not found in data")
    for mod in m.prefixes + m.suffixes:
        tier = f"T{mod.tier}" if mod.tier else "  "
        flags = "".join(["F" if mod.fractured else "", " +" if mod.merged else "", f" ~{mod.alternatives}" if mod.alternatives else ""])
        text = " / ".join(line.text for line in mod.lines)
        print(f"   {mod.kind[0].upper()} {tier:<3} {mod.source:<8} {mod.mod_id:<32} {text}  {flags}".rstrip())
    for line in m.unmatched:
        print(f"   ?              {'(no mod fits)':<32} {line.text}")
    for line in m.notes:
        print(f"   # {line.text}")


if __name__ == "__main__":
    sys.exit(main())
