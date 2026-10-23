"""Rich renderables for pipeline results, shared by the CLI and the TUI.

Colour means something here, and only a few things: what a mod line is
(target, fractured, bench, essence, not crafted), how good a tier is, item
rarity the way the game shows it (magic blue, rare yellow), what's already on
the item vs what blocks a target, and whether a margin is a profit or a loss.
Everything else stays plain.

Prices at or above one divine are shown in divines (`2.4d`), below in chaos
(`85c`); set `DIVINE` to the chaos price of a divine.

Links are terminal hyperlinks when `links` is true; otherwise the raw URL is
printed, so piped output (and anything reading it, like an LLM) keeps them.
"""

from __future__ import annotations

from dataclasses import dataclass

from rich.console import Group, RenderableType
from rich.table import Table
from rich.text import Text

from . import craft, pipeline
from .mods import ItemMatch
from .pipeline import Estimate, Report, Shared

DIVINE = 0.0  # chaos per divine; 0 shows everything in chaos


@dataclass(frozen=True)
class Palette:
    accent: str = "magenta"  # cluster jewels
    link: str = "cyan"
    good: str = "green"  # profit, a target already on the item, a cheaper route
    bad: str = "red"  # loss, a blocked target
    warn: str = "yellow"  # mods the plan doesn't make
    action: str = "yellow"  # what to do in a recipe
    share: str = "cyan"
    magic: str = "bright_blue"
    rare: str = "bright_yellow"
    fractured: str = "yellow"  # the game shows fractured mods in a muted gold
    bench: str = "bright_cyan"  # crafted mods are light blue in game
    essence: str = "magenta"
    best_tier: str = "bold"


ANSI = Palette()


def money(x: float | None) -> str:
    if x is None:
        return "-"
    if DIVINE and abs(x) >= DIVINE:
        d = x / DIVINE
        return f"{d:,.0f}d" if abs(d) >= 100 else f"{d:,.1f}d"
    return f"{x:,.1f}c" if abs(x) < 10 else f"{x:,.0f}c"


def money_text(x: float | None, style: str = "") -> Text:
    """Divine amounts in bold, so the expensive end of a column stands out."""
    if x is None:
        return Text("-", style="dim")
    big = DIVINE and abs(x) >= DIVINE
    return Text(money(x), style=f"{style} bold".strip() if big else style)


def link(label: str, url: str | None, links: bool = True, pal: Palette = ANSI) -> Text:
    if not url:
        return Text("")
    return Text(label, style=f"link {url} underline {pal.link}") if links else Text(url)


def margin_text(x: float | None, pal: Palette = ANSI) -> Text:
    if x is None:
        return Text("-", style="dim")
    return Text(("+" if x > 0 else "") + money(x), style=f"bold {pal.good if x > 0 else pal.bad}")


def market_text(e: Estimate) -> Text:
    if e.market:
        return money_text(e.market[0])
    return Text("?" if e.market_error else "-", style="dim")


def total_text(e: Estimate, pal: Palette = ANSI) -> Text:
    if e.error:
        return Text("no plan", style="dim italic")
    t = money_text(e.total)
    if e.needs_base_price:
        t.append(" +base", style=pal.fractured)
    return t


def fracture_text(e: Estimate, pal: Palette = ANSI) -> Text:
    """The best fracture route against crafting from scratch, for tables."""
    f = e.best_fracture
    if f and e.total is not None and f.total < e.total:
        return Text(f"{money(f.total)} ↓", style=pal.good)
    if e.fractures and not f:
        best = min((x.craft_cost for x in e.fractures if x.plan), default=None)
        return Text(f"{money(best)}+b", style="dim") if best is not None else Text("")
    return Text("")


def fracture_worth_text(e: Estimate, pal: Palette = ANSI) -> Text:
    """ "≤ 140d: Effect" — buy a base with that mod fractured for less than this and it pays."""
    w = e.fracture_worth
    if w is None or w[0] <= 0:
        return Text("")
    worth, f = w
    return Text.assemble((f"≤ {money(worth)}", pal.good if worth >= 0.25 * (e.total or 1) else ""), (f" {f.target}", pal.fractured))


def recomb_text(e: Estimate, pal: Palette = ANSI) -> Text:
    """The recombinator route against crafting in one go, for tables."""
    r = e.recomb_total
    if r is None:
        return Text("")
    if e.total is None or r < e.total:
        return Text(f"{money(r)} ↓", style=pal.good)
    return Text(money(r), style="dim")


def estimates_table(estimates: list[Estimate], links: bool = True, title: str | None = None, pal: Palette = ANSI) -> Table:
    t = Table(title=title, title_justify="left", title_style="bold", header_style="bold", expand=False, pad_edge=False, box=None)
    t.add_column("#", justify="right", style="dim")
    t.add_column("item")
    t.add_column("mods", overflow="fold")
    t.add_column("builds", justify="right")
    t.add_column("craft", justify="right")
    t.add_column("fractured", justify="right")
    recombs = any(e.recomb for e in estimates)
    if recombs:
        t.add_column("recomb", justify="right")
    t.add_column("market", justify="right")
    t.add_column("margin", justify="right")
    if links:
        t.add_column("trade")
    for i, e in enumerate(estimates, 1):
        t.add_row(
            str(i),
            item_title(e, pal),
            mods_inline(e, pal),
            str(len(e.builds)),
            total_text(e, pal),
            fracture_text(e, pal),
            *([recomb_text(e, pal)] if recombs else []),
            market_text(e),
            margin_text(e.margin, pal),
            *([link("search", e.trade_url, pal=pal)] if links else []),
        )
    return t


def ladder_table(estimates: list[Estimate], links: bool = True, title: str | None = None, pal: Palette = ANSI) -> Table:
    """Ladder crafting candidates: demand (characters wearing it), cost, market, and the hole score."""
    t = Table(title=title, title_justify="left", title_style="bold", header_style="bold", pad_edge=False, box=None)
    t.add_column("#", justify="right", style="dim")
    t.add_column("item")
    t.add_column("mods", overflow="fold")
    t.add_column("chars", justify="right", style=pal.share)
    t.add_column("craft", justify="right")
    recombs = any(e.recomb for e in estimates)
    if recombs:
        t.add_column("recomb", justify="right")
    t.add_column("market", justify="right")
    t.add_column("listed", justify="right", style="dim")
    t.add_column("margin", justify="right")
    t.add_column("hole", justify="right")
    fractures = any(e.fracture_worth for e in estimates)
    if fractures:
        t.add_column("fractured base worth", overflow="fold", max_width=30)
    t.add_column("worn by", style="dim", max_width=28, overflow="ellipsis", no_wrap=True)
    if links:
        t.add_column("trade")
    for i, e in enumerate(estimates, 1):
        hole = Text("-", style="dim") if e.hole is None else Text(f"{e.hole:,.0f}", style=f"bold {pal.good}" if e.hole > 0 else pal.bad)
        t.add_row(
            str(i),
            item_title(e, pal),
            mods_inline(e, pal),
            str(e.demand),
            total_text(e, pal),
            *([recomb_text(e, pal)] if recombs else []),
            market_text(e),
            str(e.market[1]) if e.market else "",
            margin_text(e.margin, pal),
            hole,
            *([fracture_worth_text(e, pal)] if fractures else []),
            ", ".join(e.skills),
            *([link("search", e.trade_url, pal=pal)] if links else []),
        )
    return t


def ladder_summary(r) -> Text:
    """One line on where the ladder candidates came from (`pipeline.LadderReport`)."""
    return Text(
        f"{r.characters:,} characters · {r.items:,} rare items → {r.archetypes:,} gear archetypes · "
        f"{r.clusters:,} cluster jewels → {r.cluster_archetypes:,} cluster combos · {len(r.estimates)} costed · prices: {r.prices_source}",
        style="dim",
    )


def item_title(e: Estimate, pal: Palette = ANSI) -> Text:
    return Text(e.title, style=pal.accent if e.kind == "cluster" else "")


def mods_inline(e: Estimate, pal: Palette = ANSI, width: int = 0, sep: str = "\n") -> Text:
    """Targets prefixes first, with a dim P/S and the tier; filler as one "N× any res" entry."""
    out = Text()
    for r in sorted(e.rows, key=lambda r: (r.side != "prefix", r.role != "fractured")):
        if r.role not in ("target", "fractured", "essence"):
            continue
        if out:
            out.append(sep, style="dim")
        out.append(f"{r.side[0].upper()} ", style="dim")
        text = r.text if not width or len(r.text) <= width else r.text[: width - 1] + "…"
        out.append(text, style=_role_style(r.role, pal))
        if r.tier and r.role == "target":
            out.append(f" {r.tier}", style=_tier_style(r.tier, pal))
    for side in ("prefix", "suffix"):
        filler = [r for r in e.rows if r.role == "filler" and r.side == side]
        if filler:
            if out:
                out.append(sep, style="dim")
            out.append(f"{side[0].upper()} ", style="dim")
            out.append(f"{len(filler)}× {filler[0].tier}", style=pal.share)
    return out


def shared_table(shared: list[Shared], links: bool = True, pal: Palette = ANSI) -> Table:
    t = Table(title="Mod combinations shared across builds", title_justify="left", title_style="bold", header_style="bold", box=None)
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
            total_text(e, pal) if e else "",
            *([link("search", e.trade_url, pal=pal) if e else ""] if links else []),
        )
    return t


def estimate_detail(e: Estimate, prices_source: str = "", links: bool = True, pal: Palette = ANSI) -> RenderableType:
    parts: list[RenderableType] = [_header(e, pal), Text(""), _mod_lines(e, pal)]
    if e.error:
        parts += [Text(""), Text(f"no plan: {e.error}", style=pal.bad)]
        return Group(*parts)
    parts += [Text(""), _costs(e, prices_source, links, pal)]
    if e.fractures:
        parts += [Text(""), _fractures(e, links, pal)]
    if e.recomb is not None and e.recomb.cost is not None:
        parts += [Text(""), recomb_steps(e, pal)]
    if e.smaller or e.bigger:
        parts += [Text(""), product_tree(e, pal)]
    if e.starts or e.starts_url:
        parts += [Text(""), starts_table(e, links, pal)]
    pick = suggested_fracture(e)
    if pick and pick.plan:
        how = "making" if pick.self_made else "buying"
        title = Text.assemble(f"recipe · {how} a base with ", (f"fractured {pick.target}", pal.fractured))
        made = pick.made if pick.self_made else None
        parts += [Text(""), steps_text(pick.problem, pick.plan, title, pal, made), Text(""), _currency(pick.plan)]
    else:
        parts += [Text(""), steps_text(e.problem, e.plan, Text("recipe · step by step"), pal)]
        parts += [Text(""), _currency(e.plan)]
    return Group(*parts)


def _header(e: Estimate, pal: Palette) -> Text:
    head = Text()
    head.append(e.title, style=f"bold {pal.accent}" if e.kind == "cluster" else "bold")
    if e.kind == "gear" and e.bases > 1:
        head.append(f"  on {e.base}")
        if e.used_base:
            head.append(f" (build has {e.used_base})", style="dim")
    head.append(f"  ilvl {e.item_level}+", style="dim")
    if len(e.builds) > 1:
        head.append(f"  · {len(e.builds)} builds", style=pal.good)
    return head


def _role_style(role: str, pal: Palette) -> str:
    return {"fractured": pal.fractured, "bench": pal.bench, "essence": pal.essence, "skipped": "dim strike"}.get(role, "")


def _tier_style(tier: str, pal: Palette) -> str:
    if tier == "T1":
        return f"{pal.best_tier} {pal.rare}"
    if tier.startswith("T") and tier[1:].rstrip("+").isdigit():
        return "" if int(tier[1:].rstrip("+")) <= 3 else "dim"
    return "dim"


def _mod_lines(e: Estimate, pal: Palette) -> Table:
    t = Table.grid(padding=(0, 1))
    t.add_column(justify="right", style="dim")  # prefix / suffix
    t.add_column()  # mod text
    t.add_column(justify="right")  # tier or role
    order = {"fractured": 0, "target": 1, "filler": 2, "essence": 3, "bench": 4, "skipped": 5}
    for side in ("prefix", "suffix"):
        rows = sorted((r for r in e.rows if r.side == side), key=lambda r: order.get(r.role, 9))
        for i, r in enumerate(rows):
            label = Text(side if i == 0 else "")
            if r.role == "target":
                tag = Text(r.tier, style=_tier_style(r.tier, pal))
            elif r.role == "filler":
                tag = Text(r.tier, style=pal.share)
            elif r.role == "skipped":
                tag = Text(f"not crafted ({r.tier})", style=pal.warn)
            else:
                tag = Text(r.role, style=_role_style(r.role, pal))
            t.add_row(label, Text(r.text, style=_role_style(r.role, pal)), tag)
    return t


def _costs(e: Estimate, prices_source: str, links: bool, pal: Palette) -> Table:
    t = Table.grid(padding=(0, 1))
    t.add_column(style="dim", justify="right")
    t.add_column(justify="right")
    t.add_column(style="dim")
    t.add_row("craft", money_text(e.craft_cost), Text(f"expected, {prices_source}"))
    if e.bench:
        t.add_row("bench", money_text(e.bench_cost), Text(", ".join(label for label, _ in e.bench)))
    if e.fixed:
        note = Text("fractured base: ", style="dim")
        note.append(" + ".join(e.fixed), style=pal.fractured)
        if e.base_price:
            note.append(f"  ({e.base_price[1]} listed)", style="dim")
        elif e.base_error:
            note.append(f"  ({e.base_error})", style=pal.warn)
        else:
            note.append("  (not priced yet: needs a trade search)", style=pal.warn)
        if e.base_url:
            note.append("  ")
            note.append_text(link("search", e.base_url, links, pal))
        t.add_row("base", money_text(e.base_price[0]) if e.base_price else Text("?", style=pal.warn), note)
    elif e.kind == "cluster":
        note = f"{e.base_price[1]} listed on poe.ninja" if e.base_price else "no poe.ninja price (run `ponder prices fetch`)"
        t.add_row("base", money_text(e.base_price[0]) if e.base_price else Text("?", style="dim"), Text(note))
    total = money_text(e.total, "bold")
    if e.needs_base_price:
        total.append(" + base", style=pal.fractured)
    t.add_row(Text("total", style="bold"), total, "")
    if e.market:
        t.add_row("market", money_text(e.market[0]), Text.assemble(f"median of cheapest asks, {e.market[1]} listed  ", link("search", e.trade_url, links, pal)))
        uses_fracture = e.best_fracture is not None and e.cheapest != e.total
        t.add_row(Text("margin", style="bold"), margin_text(e.margin, pal), Text("buying the best fractured base" if uses_fracture else ""))
    else:
        t.add_row("market", Text("-", style="dim"), Text.assemble(e.market_error or "not priced yet  ", link("search", e.trade_url, links, pal)))
    return t


def recomb_steps(e: Estimate, pal: Palette = ANSI) -> Table:
    """The recombinator tree as numbered steps: clean donors, then each recombination."""
    from . import recomb

    r = e.recomb
    cheaper = e.total is None or r.cost + e.bench_cost < e.total
    title = Text.assemble("or recombine clean donors  ", (f"{money(r.cost)} expected", f"bold {pal.good}" if cheaper else "bold"))
    title.append(f"  · odds table: {recomb.TABLE_VERSION}", style="dim")
    t = Table(title=title, title_justify="left", box=None, pad_edge=False, show_header=False)
    t.add_column(justify="right", style="dim")
    t.add_column()
    t.add_column(justify="right")
    for i, (do, what, cost) in enumerate(r.steps(), 1):
        style = pal.action if do == "recombine" else ""
        t.add_row(f"{i}.", Text.assemble((do, style), "  ", what), money_text(cost, "dim" if do == "craft" else ""))
    return t


def _fractures(e: Estimate, links: bool, pal: Palette) -> Table:
    t = Table(title="or start from a fractured base", title_justify="left", title_style="bold", box=None, pad_edge=False, header_style="dim")
    t.add_column("fractured mod")
    t.add_column("craft rest", justify="right")
    t.add_column("base", justify="right")
    t.add_column("total", justify="right")
    t.add_column("vs scratch", justify="right")
    t.add_column("")
    pick = suggested_fracture(e)
    for f in sorted(e.fractures, key=lambda f: (f.total is None, f.total or 0, f.craft_cost or 0)):
        if f.error:
            t.add_row(Text(f.target, style=pal.fractured), Text("no plan", style="dim"), "", "", "", "")
            continue
        saving = e.total - f.total if f.total is not None and e.total is not None else None
        if saving is None:
            vs = Text("-", style="dim")
        else:
            vs = Text(("−" if saving > 0 else "+") + money(abs(saving)), style=pal.good if saving > 0 else pal.bad)
        if f.self_made:
            base = Text.assemble(money_text(f.made.cost), (" make", pal.action))
        elif f.base_price:
            base = money_text(f.base_price[0])
        else:
            base = Text(f.base_error or "?", style="dim")
        name = Text(f.target, style=pal.fractured)
        if f is pick:
            name.append("  ◀ suggested", style=f"bold {pal.good}")
        t.add_row(name, money_text(f.craft_cost), base, money_text(f.total), vs, link("bases", f.trade_url, links, pal))
    return t


def starts_table(e: Estimate, links: bool = True, pal: Palette = ANSI) -> RenderableType:
    """Half-done items for sale (`pipeline.find_starts`): price, what's on them, the cost to finish."""
    t = Table(title="or buy a half-done one", title_justify="left", title_style="bold", box=None, pad_edge=False, header_style="dim")
    t.add_column("price", justify="right")
    t.add_column("has")
    t.add_column("finish", justify="right")
    t.add_column("total", justify="right")
    t.add_column("vs cheapest", justify="right")
    t.add_column("seller", style="dim")
    for f in e.starts:
        saves = f.saves
        vs = Text("-", style="dim") if saves is None else Text(("−" if saves > 0 else "+") + money(abs(saves)), style=pal.good if saves > 0 else pal.bad)
        seller = (((f.listing or {}).get("listing") or {}).get("account") or {}).get("name", "")
        t.add_row(money_text(f.price), situation(f.problem, f.state, pal), money_text(f.craft_cost), money_text(f.total), vs, seller)
    if not e.starts:
        t.add_row(Text("none listed beat crafting it", style="dim"), "", "", "", "", "")
    return Group(t, link("  listings", e.starts_url, links, pal)) if e.starts_url else t


def finishes_view(item: str, found, pal: Palette = ANSI) -> RenderableType:
    """What one real item can become (`pipeline.finishes`), and the recipe for the best."""
    t = Table(title=f"finishing {item}", title_justify="left", title_style="bold", box=None, pad_edge=False, header_style="dim")
    t.add_column("product")
    t.add_column("has", style=pal.good)
    t.add_column("finish", justify="right")
    t.add_column("worth", justify="right")
    t.add_column("saves", justify="right")
    t.add_column("wear it", justify="right", style="dim")
    short = pipeline._short_name
    for f in found:
        e = f.estimate
        name = " + ".join(short(x) for x in e.targets)
        if e.problem and e.problem.filler_need:
            name += f" + {e.problem.filler_need} filler"
        cost = money_text(f.craft_cost)
        if f.plan is not None and not f.plan.policy:  # nothing left but bench crafts
            cost = Text("ready", style=f"bold {pal.good}") + (Text(f" +{money(f.craft_cost)} bench", style="dim") if e.bench else Text(""))
        t.add_row(name, ", ".join(short(x) for x in f.have) or "-", cost, money_text(f.worth), money_text(f.saves), str(e.demand))
    parts: list[RenderableType] = [t]
    best = next((f for f in found if f.plan and f.plan.policy), None)
    if best:
        filler = best.estimate.problem.filler_need if best.estimate.problem else 0
        title = Text.assemble("recipe · into ", (" + ".join(short(x) for x in best.estimate.targets) + (f" + {filler} filler" if filler else ""), pal.good))
        parts += [Text(""), steps_text(best.problem, best.plan, title, pal), Text(""), _currency(best.plan)]
    elif not found:
        parts.append(Text("no ladder product of this base fits it", style="dim"))
    return Group(*parts)


def suggested_fracture(e: Estimate):
    """The fracture to start from: the cheapest priced route if it beats scratch, else
    (before any base is priced) the one that leaves the cheapest craft."""
    if e.best_fracture:
        return e.best_fracture if e.total is None or e.best_fracture.total < e.total else None
    solved = [f for f in e.fractures if f.plan]
    best = min(solved, key=lambda f: f.plan.cost, default=None)
    return best if best and e.total and best.plan.cost < 0.5 * e.total else None


def product_tree(e: Estimate, pal: Palette = ANSI, rows: int = 5) -> Text:
    """Where this product sits among the others of its base group: what a brick sells as, what it leads to."""
    out = Text("product tree", style="bold")

    def line(x: Estimate, arrow: str) -> None:
        out.append(f"\n  {arrow} ", style="dim")
        out.append(" + ".join(x.targets) or "-")
        if x.problem and x.problem.filler_need:
            out.append(f" + {x.problem.filler_need} filler", style=pal.share)
        out.append(f"  {money(x.cheapest)}" if x.cheapest is not None else "  -", style="dim")
        out.append(f"  {x.demand} wear it", style="dim")

    for x in sorted(e.smaller, key=lambda x: -(x.cheapest or 0))[:rows]:
        line(x, "⊂ sells as")
    for x in sorted(e.bigger, key=lambda x: -x.demand)[:rows]:
        line(x, "⊃ leads to")
    return out


def steps_text(problem: craft.Problem, plan: craft.Plan, title: Text, pal: Palette = ANSI, made: craft.SelfFracture | None = None) -> Text:
    """The plan as numbered steps: what to spam, and where each outcome sends you.

    1. magic, nothing yet: spam alt ~38× (aug if 1 other suffix)
         a. 50% +Life → 3
         b. 50% +Mana → 4
    """
    out = Text()
    out.append_text(title.copy())
    out.stylize("bold")
    if made:
        t = problem.fixed[-1].label
        out.append("\n 0. ", style="bold")
        out.append("make the base: ")
        out.append(f"spam {made.action}", style=f"bold {pal.action}")
        out.append(f" until {t} lands with ≤{made.max_mods} mods, then ")
        out.append("Fracturing Orb", style=f"bold {pal.action}")
        out.append(f"\n      a. {made.fracture_chance:.0%} it fractures {t} → 1", style="")
        out.append(f"\n      b. {1 - made.fracture_chance:.0%} another mod fractures: new base, back to 0", style=pal.bad)
        out.append(f"\n      (~{made.rerolls:,.0f} rerolls and {made.orbs:.1f} orbs in all, {money(made.cost)})", style="dim")
    for st in craft.steps(problem, plan):
        out.append(f"\n{st.number:2d}. ", style="bold")
        out.append(("normal", "magic", "rare")[st.rarity], style={1: pal.magic, 2: pal.rare}.get(st.rarity, ""))
        if st.have:
            out.append(" with ")
            out.append(" + ".join(st.have), style=pal.good)
        elif not (st.number == 1 and problem.fixed):
            out.append(", nothing yet", style="dim")
        out.append(": ")
        for i, (a, when, n) in enumerate(st.actions[:3]):
            if i:
                out.append(f"\n      · if {when}: ", style="dim")
            out.append(f"spam {a}" if n >= 3 else a, style=f"bold {pal.action}")
            if n >= 1.5:
                out.append(f" ~{n:,.0f}×" if n >= 10 else f" ~{n:.1f}×", style="dim")
        letter = 0
        rest = 0.0
        main = st.actions[0][0] if st.actions else ""
        for x in st.exits:
            if x.chance < 0.01:
                rest += x.chance
                continue
            out.append(f"\n      {'abcdefghij'[letter]}. {x.chance:4.0%} ", style="dim")
            letter += 1
            if x.kind == "done":
                if x.what != "done":
                    out.append(f"{x.what}  ", style=pal.good)
                out.append("→ done", style=f"bold {pal.good}")
                continue
            if x.kind == "rare":
                out.append(f"{x.what}: start over", style="dim")
                continue
            if x.kind == "sell":
                out.append(x.what, style=f"bold {pal.good}")
                out.append(f"  → back to {x.to}", style="bold")
                continue
            if x.via != main:
                out.append(f"{x.via}: ", style=pal.action)
            out.append(x.what, style={"win": pal.good, "back": pal.bad}.get(x.kind, ""))
            out.append(f"  → back to {x.to}" if x.kind == "back" else f"  → {x.to}", style="bold")
            if letter >= 6:
                break
        if rest >= 0.005:
            out.append(f"\n      {'abcdefghij'[letter]}. {rest:4.0%} other outcomes", style="dim")
    return out


def situation(problem: craft.Problem, state: craft.State, pal: Palette = ANSI) -> Text:
    rarity, parts = craft.describe_parts(problem, state)
    out = Text()
    out.append(rarity, style={"magic": pal.magic, "rare": pal.rare}.get(rarity, ""))
    out.append(": ")
    if not parts:
        out.append("nothing useful", style="dim")
    for i, (text, kind) in enumerate(parts):
        if i:
            out.append(" + ", style="dim")
        out.append(text, style={"good": pal.good, "blocked": pal.bad, "other": "dim", "filler": pal.share}[kind])
    return out


def _currency(plan: craft.Plan) -> Text:
    out = Text("expected currency  ", style="dim")
    usage = [(a, n) for a, n in sorted(plan.usage.items(), key=lambda x: -x[1]) if n >= 0.05]
    for i, (a, n) in enumerate(usage):
        if i:
            out.append(" · ", style="dim")
        out.append(f"{n:,.0f}" if n >= 10 else f"{n:,.1f}", style="bold")
        out.append(f" {a}")
    if plan.hours:
        out.append(f"  ·  ~{_hours(plan.hours)} clicking", style="dim")
        out.append(f" (charged {plan.cost - plan.currency:,.0f}c at {craft.CHAOS_PER_HOUR:,.0f}c/h)", style="dim")
    return out


def _hours(h: float) -> str:
    return f"{h * 60:.0f} min" if h < 1 else f"{h:,.1f} h"


def report_summary(r: Report) -> Text:
    rares = sum(b.rares for b in r.builds)
    out = Text()
    out.append(f"{len(r.builds)} build(s), {rares} rares, {len(r.estimates)} distinct crafts", style="bold")
    out.append(f"  ·  league {r.league}  ·  prices: {r.prices_source}", style="dim")
    if r.divine:
        out.append(f"  ·  1d = {r.divine:,.0f}c", style="dim")
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
    source_style = {"natural": "green", "crafted": "bright_cyan", "essence": "magenta", "special": "yellow"}
    for mod in m.prefixes + m.suffixes:
        flags = "".join(["F" if mod.fractured else "", " +" if mod.merged else "", f" ~{mod.alternatives}" if mod.alternatives else ""])
        t.add_row(
            mod.kind[0].upper(),
            Text(f"T{mod.tier}" if mod.tier else "", style="bold" if mod.tier == 1 else ""),
            Text(mod.source, style=source_style.get(mod.source, "dim")),
            Text(mod.mod_id, style="dim"),
            Text(" / ".join(line.text for line in mod.lines), style="yellow" if mod.fractured else ""),
            Text(flags, style="bold"),
        )
    for line in m.unmatched:
        t.add_row(Text("?", style="red"), "", "", Text("(no mod fits)", style="dim"), Text(line.text), "")
    for line in m.notes:
        t.add_row(Text("#", style="dim"), "", "", "", Text(line.text, style="dim italic"), "")
    return Group(Text(""), head, t)


def tier_table(stats) -> Table:
    """`demand.FamilyStat`s: how often each mod is worn per base group, and its tier lift."""
    t = Table(title="Mods worn by ladder characters · lift = share at that tier or better / chance a roll lands there",
              title_justify="left", title_style="bold", header_style="bold", box=None)  # fmt: skip
    t.add_column("group")
    t.add_column("mod")
    t.add_column("side", style="dim")
    t.add_column("chars", justify="right")
    t.add_column("T1", justify="right")
    t.add_column("T1 lift", justify="right")
    t.add_column("≤T2", justify="right")
    t.add_column("≤T2 lift", justify="right")
    last = None
    for s in stats:
        group = Text(s.group if s.group != last else "", style="bold")
        last = s.group
        t.add_row(
            group, Text(s.text), s.side[0].upper(), str(s.characters),
            f"{s.share_at_or_above(1):.0%}", _lift(s.lift(1)), f"{s.share_at_or_above(2):.0%}", _lift(s.lift(2)),
        )  # fmt: skip
    return t


def _lift(x: float) -> Text:
    """Above ~3x, players pay for the tier; near 1x, any tier will do."""
    style = "bold green" if x >= 3 else "green" if x >= 1.8 else "dim"
    return Text(f"{x:.1f}×", style=style)


def archetype_table(found, db) -> Table:
    t = Table(title="Archetypes: mod combinations ladder characters share", title_justify="left", title_style="bold", header_style="bold", box=None)
    t.add_column("chars", justify="right")
    t.add_column("group")
    t.add_column("mods")
    t.add_column("worn on", style="dim")
    t.add_column("top skills", style="dim")
    from .demand import RES, _template, stat_key

    for a in found:
        names = []
        for key in a.families:
            if key == RES:
                names.append("resistances")
                continue
            mods = [m for ex in a.examples for m in ex.mods if m.source not in ("crafted", "legacy-crafted") and stat_key(m.mod_id) == key]
            plain = [m for m in mods if "/" not in _template(db, m.mod_id)]  # name it by the plain mod when there is one
            names.append(_template(db, (plain or mods)[0].mod_id) if mods else key)
        skills = ", ".join(f"{s}" for s, _ in a.skills.most_common(3))
        total = sum(a.bases.values()) or 1
        worn_on = ", ".join(f"{b} {n / total:.0%}" for b, n in a.bases.most_common(2))
        t.add_row(str(a.characters), Text(a.group, style="bold"), Text(" + ".join(names)), Text(worn_on), Text(skills))
    return t


def price_model(arch, model, db, url: str, links: bool = True) -> RenderableType:
    """What each core mod's tier is worth for one archetype (`pricing.PriceModel`)."""
    from .demand import _template, stat_key

    head = Text.assemble((arch.group, "bold"), f"  {arch.characters} characters  ", link("search", url, links))
    if model is None:
        return Group(head, Text("  too few listings to fit", style="dim"))
    t = Table(box=None, header_style="dim", pad_edge=False)
    t.add_column("mod")
    t.add_column("T1 ×", justify="right")
    t.add_column("T2 ×", justify="right")
    t.add_column("listings T1/T2", justify="right", style="dim")
    for tp in model.tiers:
        mod = next((m for ex in arch.examples for m in ex.mods if stat_key(m.mod_id) == tp.key), None)
        name = _template(db, mod.mod_id) if mod else tp.key
        t.add_row(Text(name), _mult(tp.t1), _mult(tp.t2), f"{tp.n_t1}/{tp.n_t2}")
    extra = Text(f"  {model.listings} listings · base ~{money(model.base_price)} · per resistance ×{model.per_res:.2f}"
                 f" · per open affix ×{model.per_open_affix:.2f} · fit r² {model.r2:.2f}", style="dim")  # fmt: skip
    return Group(head, t, extra, Text(""))


def _mult(x: float | None) -> Text:
    if x is None:
        return Text("-", style="dim")
    return Text(f"{x:.2f}", style="bold green" if x >= 2 else "green" if x >= 1.3 else "dim")
