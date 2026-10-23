"""Rich renderables for pipeline results, shared by the CLI and the TUI.

Links are terminal hyperlinks when `links` is true; otherwise the raw URL is
printed, so piped output (and anything reading it, like an LLM) keeps them.
Colours come from a `Palette`: plain ANSI names by default, so the terminal's
own colour scheme applies; the TUI passes one built from its current theme.
"""

from __future__ import annotations

from dataclasses import dataclass

from rich.console import Group, RenderableType
from rich.table import Table
from rich.text import Text

from .mods import ItemMatch
from .pipeline import Estimate, Report, Shared


@dataclass(frozen=True)
class Palette:
    accent: str = "magenta"  # cluster jewels
    link: str = "cyan"
    good: str = "green"
    bad: str = "red"
    warn: str = "yellow"  # mods the plan doesn't make
    action: str = "yellow"  # what to do in a recipe
    share: str = "cyan"


ANSI = Palette()


def chaos(x: float | None, digits: int = 0) -> str:
    return "-" if x is None else f"{x:,.{digits}f}c"


def link(label: str, url: str | None, links: bool = True, pal: Palette = ANSI) -> Text:
    if not url:
        return Text("")
    return Text(label, style=f"link {url} underline {pal.link}") if links else Text(url)


def margin_text(x: float | None, pal: Palette = ANSI) -> Text:
    if x is None:
        return Text("-", style="dim")
    return Text(chaos(x), style=f"bold {pal.good if x > 0 else pal.bad}")


def estimates_table(estimates: list[Estimate], links: bool = True, title: str | None = None, pal: Palette = ANSI) -> Table:
    t = Table(title=title, title_justify="left", header_style="bold", expand=False, pad_edge=False)
    t.add_column("#", justify="right", style="dim")
    t.add_column("item")
    t.add_column("targets", overflow="fold")
    t.add_column("builds", justify="right")
    t.add_column("craft", justify="right")
    t.add_column("total", justify="right")
    t.add_column("market", justify="right")
    t.add_column("margin", justify="right")
    if links:
        t.add_column("trade")
    for i, e in enumerate(estimates, 1):
        style = pal.accent if e.kind == "cluster" else ""
        total = Text(chaos(e.total)) if e.error is None else Text(e.error, style="dim italic")
        t.add_row(
            str(i),
            Text(e.title, style=style),
            Text("\n".join(e.targets)),
            str(len(e.builds)),
            chaos(e.craft_cost),
            total,
            chaos(e.market[0]) if e.market else Text(e.market_error or "-", style="dim"),
            margin_text(e.margin, pal),
            *([link("search", e.trade_url, pal=pal)] if links else []),
        )
    return t


def shared_table(shared: list[Shared], links: bool = True, pal: Palette = ANSI) -> Table:
    t = Table(title="Mod combinations shared across builds", title_justify="left", header_style="bold")
    t.add_column("builds", justify="right")
    t.add_column("items", justify="right")
    t.add_column("class")
    t.add_column("mods", overflow="fold")
    t.add_column("craft", justify="right")
    if links:
        t.add_column("trade")
    for s in shared:
        c, e = s.candidate, s.estimate
        t.add_row(
            str(c.builds),
            str(c.count),
            Text(c.item_class),
            Text(" + ".join(c.families)),
            (chaos(e.total) if e.error is None else Text(e.error, style="dim")) if e else "",
            *([link("search", e.trade_url, pal=pal) if e else ""] if links else []),
        )
    return t


def estimate_detail(e: Estimate, prices_source: str = "", links: bool = True, rows: int = 12, pal: Palette = ANSI) -> RenderableType:
    parts: list[RenderableType] = []
    head = Text(e.title, style=f"bold {pal.accent}" if e.kind == "cluster" else "bold")
    if e.slot:
        head.append(f"  [{e.slot}]", style="dim")
    head.append(f"  ilvl {e.item_level}+", style="dim")
    parts.append(head)

    info = Table.grid(padding=(0, 1))
    info.add_column(style="dim", justify="right")
    info.add_column()
    for t in e.targets:
        info.add_row("target", Text(t))
    for m in e.fixed:
        info.add_row("fractured", Text(m))
    for label, c in e.bench:
        info.add_row("bench last", Text(f"{label}  ({chaos(c, 1)})"))
    for s in e.skipped:
        info.add_row("not crafted", Text(s, style=pal.warn))
    if e.builds:
        info.add_row("used by", f"{len(e.builds)} build(s)")
    parts.append(info)

    if e.error:
        parts.append(Text(f"no plan: {e.error}", style=pal.bad))
        return Group(*parts)

    costs = Table.grid(padding=(0, 1))
    costs.add_column(style="dim", justify="right")
    costs.add_column(justify="right")
    costs.add_column(style="dim")
    costs.add_row("craft", chaos(e.craft_cost, 1), Text(prices_source))
    if e.bench:
        costs.add_row("bench", chaos(e.bench_cost, 1), "")
    if e.kind == "cluster" and not e.fixed:
        costs.add_row("base", chaos(e.base_price[0], 1) if e.base_price else "?", f"{e.base_price[1]} listed on poe.ninja" if e.base_price else "no poe.ninja price (run `ponder prices fetch`)")
    costs.add_row(Text("total", style="bold"), Text(chaos(e.total, 1), style="bold"), "")
    if e.market:
        costs.add_row("market", chaos(e.market[0], 1), f"median of cheapest asks, {e.market[1]} listed")
        costs.add_row(Text("margin", style="bold"), margin_text(e.margin, pal), "")
    elif e.market_error:
        costs.add_row("market", "-", e.market_error)
    parts.append(costs)

    used = ", ".join(f"{n:,.1f} {a}" for a, n in e.usage())
    parts.append(Text.assemble(("expected currency: ", "dim"), used))

    recipe = Table(title="recipe (most common situations first)", title_justify="left", title_style="bold", box=None, pad_edge=False)
    recipe.add_column("share", justify="right", style=pal.share)
    recipe.add_column("situation")
    recipe.add_column("do", style=f"bold {pal.action}")
    for state, action, share in e.recipe(rows):
        recipe.add_row(f"{share:.1%}", Text(state), Text(action))
    parts.append(recipe)

    if e.trade_url:
        parts.append(Text.assemble(("trade: ", "dim"), link("open search on pathofexile.com", e.trade_url, links, pal)))
    return Group(*parts)


def report_summary(r: Report) -> Text:
    rares = sum(b.rares for b in r.builds)
    out = Text()
    out.append(f"{len(r.builds)} build(s), {rares} rares, {len(r.estimates)} distinct crafts", style="bold")
    out.append(f"  ·  league {r.league}  ·  prices: {r.prices_source}", style="dim")
    for b in r.builds:
        out.append(f"\n  {b.name}", style="")
        out.append(f"  {b.source[:70]}", style="dim")
    for err in r.errors:
        out.append(f"\n  failed: {err}", style="red")
    return out


def item_match(slot: str | None, sets: list[str], m: ItemMatch) -> RenderableType:
    """One rare, mod by mod (`ponder items`)."""
    item = m.item
    extra = ", ".join(sorted(item.influences | item.flags))
    head = Text.assemble((f"[{slot or 'unequipped'}] ", "dim"), (f"{item.name} / {item.base}", "bold"), f"  ilvl {item.item_level or '?'}")
    if extra:
        head.append(f"  ({extra})", style="yellow")
    t = Table.grid(padding=(0, 1))
    for _ in range(6):
        t.add_column()
    if sets:
        t.add_row("", "", "", Text("sets", style="dim"), Text(", ".join(sets)), "")
    if m.base_id is None:
        t.add_row(Text("!", style="red"), "", "", "", Text("base type not found in data", style="red"), "")
    source_style = {"natural": "green", "crafted": "blue", "essence": "magenta", "special": "yellow"}
    for mod in m.prefixes + m.suffixes:
        flags = "".join(["F" if mod.fractured else "", " +" if mod.merged else "", f" ~{mod.alternatives}" if mod.alternatives else ""])
        t.add_row(
            mod.kind[0].upper(),
            f"T{mod.tier}" if mod.tier else "",
            Text(mod.source, style=source_style.get(mod.source, "dim")),
            Text(mod.mod_id, style="dim"),
            Text(" / ".join(line.text for line in mod.lines)),
            Text(flags, style="bold"),
        )
    for line in m.unmatched:
        t.add_row(Text("?", style="red"), "", "", Text("(no mod fits)", style="dim"), Text(line.text), "")
    for line in m.notes:
        t.add_row(Text("#", style="dim"), "", "", "", Text(line.text, style="dim italic"), "")
    return Group(Text(""), head, t)
