"""Command line entry point: `ponder data|prices|items|mine|cluster`."""

from __future__ import annotations

import argparse
import sys

from pathlib import Path

from . import cluster, craft, data, pob, prices
from .item import parse_item
from .mine import mine
from .mods import ItemMatch, ModDB


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="ponder")
    sub = parser.add_subparsers(dest="command", required=True)

    data_cmd = sub.add_parser("data", help="manage downloaded game data")
    data_cmd.add_argument("action", choices=["fetch", "version"])

    items_cmd = sub.add_parser("items", help="show the rare items in builds, matched to mod ids")
    items_cmd.add_argument("sources", nargs="+", help="pobb.in/pastebin URL, file, or PoB code")
    items_cmd.add_argument("--all", action="store_true", help="include non-rare items")

    mine_cmd = sub.add_parser("mine", help="rank mod combinations shared across builds")
    mine_cmd.add_argument("sources", nargs="+", help="pobb.in/pastebin URL, file, or PoB code")
    mine_cmd.add_argument("--min-builds", type=int, default=2)
    mine_cmd.add_argument("--top", type=int, default=30)
    mine_cmd.add_argument("--prices", type=Path, help="price file (default prices.json)")

    prices_cmd = sub.add_parser("prices", help="currency prices used for costs")
    prices_cmd.add_argument("action", choices=["fetch", "show"])
    prices_cmd.add_argument("--league", help="league name on poe.ninja, for fetch")
    prices_cmd.add_argument("--prices", type=Path, help="price file (default prices.json)")

    cl = sub.add_parser("cluster", help="cluster jewel enchants, notables and crafting costs")
    cl_sub = cl.add_subparsers(dest="cluster_command", required=True)
    cl_ench = cl_sub.add_parser("enchants", help="list enchants for a size")
    cl_ench.add_argument("size", choices=list(cluster.SIZES))
    cl_not = cl_sub.add_parser("notables", help="notables that can roll for an enchant")
    cl_not.add_argument("size", choices=list(cluster.SIZES))
    cl_not.add_argument("enchant", help='enchant name or tag, e.g. "cold damage"')
    cl_not.add_argument("--ilvl", type=int, default=84)
    cl_cost = cl_sub.add_parser("cost", help="expected cost and recipe to hit notables")
    cl_cost.add_argument("size", choices=list(cluster.SIZES))
    cl_cost.add_argument("enchant")
    cl_cost.add_argument("notables", nargs="+")
    cl_cost.add_argument("--need", type=int, help="how many of the notables are required (default all)")
    cl_cost.add_argument("--ilvl", type=int, default=84)
    cl_cost.add_argument("--prices", type=Path, help="price file (default prices.json)")

    args = parser.parse_args(argv)
    if args.command == "data":
        if args.action == "fetch":
            print(f"fetched repoe data for {data.fetch()} into {data.data_dir()}")
        else:
            print(data.version() or "no data; run `ponder data fetch`")
        return 0
    if args.command == "prices":
        if args.action == "fetch":
            if not args.league:
                parser.error("prices fetch needs --league")
            print(f"fetched {len(prices.fetch(args.league, args.prices))} prices")
        table, source = prices.load(args.prices)
        print(f"prices from {source}:")
        for name in sorted(prices.PLACEHOLDER):
            print(f"  {table.get(name, 0):>8.2f}c  {name}")
        return 0
    if args.command == "cluster":
        return _cluster(args)

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
        table, source = prices.load(args.prices)
        print(f"{len(matched)} rare items from {len(args.sources)} builds; craft costs use {source}")
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
    size, enchant = key.split(" cluster: ")
    try:
        _, plan = cluster.cost(size, enchant, list(notables), table)
    except ValueError as e:
        return f"  (no cost: {e})"
    return f"  ~{plan.cost:,.0f}c to craft"


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

    table, source = prices.load(args.prices)
    problem, plan = cluster.cost(args.size, enchant.tag, args.notables, table, args.need, args.ilvl)
    need = problem.need
    print(f"{args.size} cluster, {enchant.name}, ilvl {args.ilvl}: {need} of {', '.join(args.notables)}")
    print(f"expected cost ~{plan.cost:,.1f}c (prices: {source}), not counting the base")
    used = ", ".join(f"{n:,.1f} {a}" for a, n in plan.usage.items() if n >= 0.05)
    print(f"expected currency: {used}  (scour = scour + transmute)")
    print("recipe (most common situations first):")
    for state, action, share in craft.recipe(problem, plan):
        print(f"  {share:>6.1%}  {state:<60} -> {action}")
    return 0


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
