"""Command line entry point: `ponder data fetch`, `ponder items`, `ponder mine`."""

from __future__ import annotations

import argparse
import sys

from . import data, pob
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

    args = parser.parse_args(argv)
    if args.command == "data":
        if args.action == "fetch":
            print(f"fetched repoe data for {data.fetch()} into {data.data_dir()}")
        else:
            print(data.version() or "no data; run `ponder data fetch`")
        return 0

    db = ModDB()
    matched: list[tuple[str, ItemMatch]] = []
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
        print(f"{len(matched)} rare items from {len(args.sources)} builds")
        for c in mine(matched, min_builds=args.min_builds)[: args.top]:
            print(f"{c.builds:>3} builds {c.count:>3} items  {c.item_class:<14} {' + '.join(c.families)}")
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
