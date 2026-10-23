"""Textual front end: `ponder tui SOURCES` (or `ponder-tui`).

Runs the same `pipeline.run` the CLI does (fracture options included) in a
worker thread, then lets you browse crafts and shared combos, read recipes,
price things on the trade site, and open or copy their search links. All the
numbers come from the library; this file is layout and key bindings.

Keys are vim-flavoured; press `?` in the app for the list.
"""

from __future__ import annotations

import json
import os
import sys
import urllib.error
import webbrowser
from pathlib import Path

from rich.console import Group
from rich.markup import escape
from rich.table import Table
from rich.text import Text
from textual import work
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, VerticalScroll
from textual.screen import ModalScreen
from textual.widgets import DataTable, Footer, Input, Static, TabbedContent, TabPane

from . import pipeline, prices, render, trade
from .mods import ModDB

CONFIG = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config") / "ponder" / "tui.json"
DEFAULT_THEME = "ansi-dark"  # the terminal's own colours
TAB_OF = {"crafts": "crafts-tab", "shared": "shared-tab"}
FILTERS = ["all", "cluster", "gear"]
SORTS = {
    "margin": pipeline.priority,
    "craft": lambda e: (e.total is None, -(e.total or 0)),
    "cheapest": lambda e: (e.total is None, e.total or 0),
    "fracture saving": lambda e: (e.best_fracture is None, -((e.total or 0) - (e.best_fracture.total if e.best_fracture else 0))),
    "builds": lambda e: (-len(e.builds), e.total is None, -(e.total or 0)),
    "hole": lambda e: (e.hole is None, -(e.hole or 0), -e.demand),
    "demand": lambda e: (-e.demand, e.total is None, -(e.total or 0)),
    "recomb saving": lambda e: (e.recomb_total is None, (e.recomb_total or 0) - (e.total or 0)),
}
CRAFT_COLUMNS = [("item", 30), ("craft", 9), ("fractured", 11), ("recomb", 10), ("market", 8), ("margin", 9), ("builds", 6), ("mods", 64)]
KEYS = [
    ("j / k", "move down / up"),
    ("g / G", "first / last row"),
    ("ctrl+d / ctrl+u", "half a page down / up"),
    ("h / l", "previous / next tab"),
    ("J / K", "scroll the detail pane"),
    ("/", "search items and mods (esc clears)"),
    ("f", "filter: all · clusters · gear"),
    ("s", "sort: margin · craft · cheapest · fracture saving · builds (ladder: hole · demand · recomb saving)"),
    ("R", "cost a recombinator tree for the selected item (clean donors, NNN pads)"),
    ("t", "price the selected item on trade (item, fractured bases)"),
    ("F", "price the 2 most promising fractured bases of every item shown"),
    ("S", "search trade for half-done items of the selected craft, finished from their own mods"),
    ("o / y", "open / copy the trade search"),
    ("b", "load other builds"),
    ("ctrl+p", "command palette (themes live here)"),
    ("q", "quit"),
]


class VimTable(DataTable):
    """A DataTable with vim motions."""

    BINDINGS = [
        Binding("j", "cursor_down", show=False),
        Binding("k", "cursor_up", show=False),
        Binding("g", "scroll_top", show=False),
        Binding("G", "scroll_bottom", show=False),
        Binding("ctrl+d", "page_down", show=False),
        Binding("ctrl+u", "page_up", show=False),
    ]


class PonderApp(App):
    TITLE = "ponder"
    CSS = """
    * { scrollbar-size: 1 1; }
    DataTable { scrollbar-size-horizontal: 0; }  /* columns are sized to fit; long mod lists are cut */
    #sources { height: 1; border: none; padding: 0 1; background: $boost; }
    #main { height: 1fr; }
    #tables { width: 3fr; }
    #detail-box { width: 2fr; border-left: vkey $foreground 20%; padding: 0 1; }
    #search { height: 1; border: none; padding: 0 1; display: none; }
    #search.visible { display: block; }
    #status { height: 1; padding: 0 1; color: $text-muted; background: $boost; }
    DataTable { height: 1fr; }
    """
    BINDINGS = [
        Binding("t", "trade", "price"),
        Binding("F", "fracture_prices", "fracture bases"),
        Binding("S", "starts", "half-done"),
        Binding("R", "recomb", "recomb"),
        Binding("o", "open_link", "open"),
        Binding("y", "copy_link", "copy link", show=False),
        Binding("f", "cycle_filter", "filter"),
        Binding("s", "cycle_sort", "sort"),
        Binding("slash", "search", "search"),
        Binding("h", "tab(-1)", show=False),
        Binding("l", "tab(1)", show=False),
        Binding("J", "detail(4)", show=False),
        Binding("K", "detail(-4)", show=False),
        Binding("b", "new_sources", "builds", show=False),
        Binding("question_mark", "help", "keys"),
        Binding("q", "quit", "quit"),
    ]

    def __init__(self, sources: list[str], trade_on_load: bool = False, ladder: bool = False):
        super().__init__()
        self.sources = sources
        self.trade_on_load = trade_on_load
        self.ladder = ladder
        self.report: pipeline.Report | None = None
        self.table: dict[str, float] = {}
        self.by_key: dict[str, pipeline.Estimate] = {}
        self.filter = "all"
        self.sort = "margin"
        self.search = ""
        self.current: pipeline.Estimate | None = None
        self.pal = render.ANSI

    def compose(self) -> ComposeResult:
        yield Input(placeholder="builds: pobb.in / pastebin links, PoB files or a .txt list · enter to run", id="sources")
        with Horizontal(id="main"):
            with TabbedContent(id="tables"):
                with TabPane("Crafts", id="crafts-tab"):
                    yield VimTable(id="crafts", cursor_type="row", zebra_stripes=True)
                with TabPane("Shared across builds", id="shared-tab"):
                    yield VimTable(id="shared", cursor_type="row", zebra_stripes=True)
            with VerticalScroll(id="detail-box"):
                yield Static("Pick a row to see its recipe.", id="detail")
        yield Input(placeholder="search · enter to keep, esc to clear", id="search")
        yield Static("", id="status")
        yield Footer()

    def on_mount(self) -> None:
        self.theme_changed_signal.subscribe(self, self.on_theme_changed)
        saved = _load_config().get("theme", DEFAULT_THEME)
        self.theme = saved if saved in self.available_themes else DEFAULT_THEME
        self.pal = self.palette()
        self.table = prices.load()[0]
        render.DIVINE = self.table.get("divine", 0.0)
        trade.on_wait = lambda secs: self.call_from_thread(self.set_status, f"waiting {secs:.0f}s for the trade site's rate limit")
        crafts = self.query_one("#crafts", DataTable)
        for label, width in CRAFT_COLUMNS:
            crafts.add_column(label, key=label, width=width)
        shared = self.query_one("#shared", DataTable)
        for label in ["builds", "items", "class", "mods", "craft"]:
            shared.add_column(label, key=label)
        box = self.query_one("#sources", Input)
        if self.ladder:
            crafts.columns["builds"].label = Text("chars")
            self.sort = "demand"
            box.value = "ladder: what poe.ninja characters wear"
            crafts.focus()
            self.analyze_ladder()
        elif self.sources:
            box.value = " ".join(self.sources)
            crafts.focus()  # so the keys work straight away
            self.analyze(self.sources)
        else:
            box.focus()

    # -- running the pipeline ----------------------------------------------

    def on_input_submitted(self, event: Input.Submitted) -> None:
        if event.input.id == "search":
            self.query_one("#crafts", DataTable).focus()
            return
        sources = event.value.split()
        if sources:
            self.analyze(sources)
            self.query_one("#crafts", DataTable).focus()

    def on_input_changed(self, event: Input.Changed) -> None:
        if event.input.id == "search":
            self.search = event.value.strip().lower()
            self.fill_crafts(quiet=True)

    @work(thread=True, exclusive=True)
    def analyze(self, sources: list[str]) -> None:
        self.call_from_thread(self.set_status, "starting…")
        try:
            report = pipeline.run(sources, fractures=True, progress=lambda m: self.call_from_thread(self.set_status, escape(m)))
        except Exception as e:  # show it rather than crash the app
            self.call_from_thread(self.set_status, f"[red]failed: {escape(str(e))}[/red]")
            return
        self.call_from_thread(self.show_report, report)
        if self.trade_on_load:
            self.call_from_thread(self.trade_shown)

    @work(thread=True, exclusive=True)
    def analyze_ladder(self) -> None:
        try:
            r = pipeline.ladder(progress=lambda m: self.call_from_thread(self.set_status, escape(m)))
        except Exception as e:  # no cache yet, a bad file...
            self.call_from_thread(self.set_status, f"[red]failed: {escape(str(e))}[/red]")
            return
        report = pipeline.Report([], r.estimates, [], r.prices_source, r.league, divine=r.divine)
        self.call_from_thread(self.show_report, report)
        self.call_from_thread(self.set_status, escape(render.ladder_summary(r).plain) + " · R recomb, t trade, s sort, ? keys")

    def set_status(self, message: str) -> None:
        self.query_one("#status", Static).update(message)

    def show_report(self, report: pipeline.Report) -> None:
        self.report = report
        self.sub_title = f"{len(report.builds)} build(s) · {report.league}"
        failed = f" · [red]{len(report.errors)} source(s) failed: {escape(report.errors[0])}[/red]" if report.errors else ""
        self.set_status(
            f"{len(report.builds)} build(s) · {len(report.estimates)} crafts · {len(report.shared)} shared combos · "
            f"{escape(report.prices_source)} · 1d = {report.divine:,.0f}c · t prices on trade, ? for keys{failed}"
        )
        self.fill_crafts()
        shared = self.query_one("#shared", DataTable)
        shared.clear()
        for i, s in enumerate(report.shared):
            c, e = s.candidate, s.estimate
            craft = render.total_text(e, self.pal) if e else ""
            shared.add_row(str(c.builds), str(c.count), Text(c.item_class), Text(" + ".join(c.families)), craft, key=f"shared:{i}")
            if e:
                self.by_key[f"shared:{i}"] = e

    def rows(self) -> list[pipeline.Estimate]:
        if not self.report:
            return []
        rows = [e for e in self.report.estimates if self.filter == "all" or e.kind == self.filter]
        if self.search:
            rows = [e for e in rows if self.search in (e.title + " " + e.base + " " + " ".join(r.text for r in e.rows)).lower()]
        return sorted(rows, key=SORTS[self.sort])

    def fill_crafts(self, quiet: bool = False) -> None:
        if not self.report:
            return
        crafts = self.query_one("#crafts", DataTable)
        cursor = crafts.cursor_row if quiet else 0  # a re-colour keeps your place
        crafts.clear()
        self.by_key = {k: v for k, v in self.by_key.items() if not k.startswith("craft:")}
        rows = self.rows()
        for i, e in enumerate(rows):
            key = f"craft:{i}"
            self.by_key[key] = e
            crafts.add_row(*self.craft_cells(e), key=key)
        if rows:
            crafts.move_cursor(row=min(cursor, len(rows) - 1))
        if not quiet:
            what = {"all": "all crafts", "cluster": "cluster jewels", "gear": "gear"}[self.filter]
            self.notify(f"{len(rows)} · {what} · sorted by {self.sort}", timeout=2)

    def craft_cells(self, e: pipeline.Estimate) -> list:
        return [
            render.item_title(e, self.pal),
            render.total_text(e, self.pal),
            render.fracture_text(e, self.pal),
            render.recomb_text(e, self.pal),
            render.market_text(e),
            render.margin_text(e.margin, self.pal),
            Text(str(e.demand), style=self.pal.share) if self.ladder else Text(str(len(e.builds)), style=self.pal.good if len(e.builds) > 1 else "dim"),
            _cut(render.mods_inline(e, self.pal, width=24, sep="  "), 64),
        ]

    # -- detail pane ---------------------------------------------------------

    def on_data_table_row_highlighted(self, event: DataTable.RowHighlighted) -> None:
        # Both tables post highlights when filled; only the visible one picks the detail.
        if TAB_OF.get(event.data_table.id) != self.query_one(TabbedContent).active:
            return
        self.current = self.by_key.get(event.row_key.value) if event.row_key else None
        self.show_detail()

    def on_tabbed_content_tab_activated(self, event: TabbedContent.TabActivated) -> None:
        table = self.query_one("#crafts" if event.pane.id == "crafts-tab" else "#shared", DataTable)
        self.current = None
        if table.row_count:
            row_key = table.coordinate_to_cell_key(table.cursor_coordinate).row_key
            self.current = self.by_key.get(row_key.value)
        self.show_detail()

    def show_detail(self) -> None:
        detail = self.query_one("#detail", Static)
        if self.current is None:
            detail.update(Text("No estimate for this row (only cluster combos are costed here).", style="dim"))
            return
        detail.update(render.estimate_detail(self.current, self.report.prices_source if self.report else "", links=True, pal=self.pal))

    # -- actions -------------------------------------------------------------

    def action_trade(self) -> None:
        if self.current:
            self.lookup([self.current], budget=10_000)

    def trade_shown(self) -> None:
        """`--trade`: price what's shown on load, within the search budget."""
        if self.report:
            self.lookup(self.rows(), budget=40)

    def action_recomb(self) -> None:
        if self.current and self.current.kind == "gear":
            self.cost_recomb([self.current])

    @work(thread=True, exclusive=True, group="recomb")
    def cost_recomb(self, estimates: list[pipeline.Estimate]) -> None:
        pipeline.add_recombs(estimates, self.table, progress=lambda m: self.call_from_thread(self.set_status, escape(m)))
        self.call_from_thread(self.after_lookup, estimates, 0)
        self.call_from_thread(self.set_status, "recombinator tree costed" if estimates[0].recomb else "no recombinator plan for this item")

    def action_starts(self) -> None:
        if self.current and self.current.kind == "gear" and self.current.plan:
            self.find_starts(self.current)

    @work(thread=True, exclusive=True, group="trade")
    def find_starts(self, est: pipeline.Estimate) -> None:
        league = self.report.league if self.report else pipeline.current_league()
        self.call_from_thread(self.set_status, "searching trade for half-done items…")
        before = trade.searches
        try:
            self._db = getattr(self, "_db", None) or ModDB()
            pipeline.find_starts(self._db, est, self.table, league)
        except (urllib.error.URLError, OSError) as e:
            self.call_from_thread(self.notify, f"trade failed: {e}", severity="warning")
        self.call_from_thread(self.after_lookup, [est], trade.searches - before)

    def action_fracture_prices(self) -> None:
        if self.report:
            self.price_fractures(self.rows())

    @work(thread=True, exclusive=True, group="trade")
    def price_fractures(self, estimates: list[pipeline.Estimate]) -> None:
        league = self.report.league
        used = pipeline.price_fractures(
            estimates, self.table, league, per_item=2, budget=40, progress=lambda m: self.call_from_thread(self.set_status, escape(m))
        )
        self.call_from_thread(self.after_lookup, estimates, used)

    @work(thread=True, exclusive=True, group="trade")
    def lookup(self, estimates: list[pipeline.Estimate], budget: int) -> None:
        league = self.report.league if self.report else pipeline.current_league()
        self.call_from_thread(self.set_status, "asking the trade site…")
        used = pipeline.price_on_trade(estimates, self.table, league, budget=budget, progress=lambda m: self.call_from_thread(self.set_status, escape(m)))
        self.call_from_thread(self.after_lookup, estimates, used)

    def after_lookup(self, estimates: list[pipeline.Estimate], used: int) -> None:
        crafts = self.query_one("#crafts", DataTable)
        for key, est in self.by_key.items():
            if key.startswith("craft:") and est in estimates:
                for (col, _), value in zip(CRAFT_COLUMNS, self.craft_cells(est)):
                    crafts.update_cell(key, col, value)
        errors = {e.market_error for e in estimates if e.market_error and e.market_error.startswith("trade")}
        if errors:
            self.notify(next(iter(errors)), severity="warning")
        self.set_status(f"trade: {used} new search(es), the rest from cache (kept {trade.CACHE_TTL // 60} min)")
        self.show_detail()

    def action_open_link(self) -> None:
        url = self.current.trade_url if self.current else None
        if not url:
            self.notify("no trade search for this row", severity="warning")
            return
        try:
            self.open_url(url)
        except Exception:
            webbrowser.open(url)
        self.notify("opened the trade search (y copies it instead)", timeout=3)

    def action_copy_link(self) -> None:
        if self.current and self.current.trade_url:
            self.copy_to_clipboard(self.current.trade_url)
            self.notify("trade link copied", timeout=2)

    def action_cycle_filter(self) -> None:
        self.filter = FILTERS[(FILTERS.index(self.filter) + 1) % len(FILTERS)]
        self.fill_crafts()

    def action_cycle_sort(self) -> None:
        names = list(SORTS)
        self.sort = names[(names.index(self.sort) + 1) % len(names)]
        self.fill_crafts()

    def action_search(self) -> None:
        box = self.query_one("#search", Input)
        box.add_class("visible")
        box.focus()

    def on_key(self, event) -> None:
        box = self.query_one("#search", Input)
        if event.key == "escape" and box.has_class("visible"):
            box.value = ""
            box.remove_class("visible")
            self.query_one("#crafts", DataTable).focus()

    def action_tab(self, step: int) -> None:
        tabs = self.query_one(TabbedContent)
        order = ["crafts-tab", "shared-tab"]
        tabs.active = order[(order.index(tabs.active) + step) % len(order)]
        self.query_one("#crafts" if tabs.active == "crafts-tab" else "#shared", DataTable).focus()

    def action_detail(self, lines: int) -> None:
        self.query_one("#detail-box", VerticalScroll).scroll_relative(y=lines, animate=False)

    def action_new_sources(self) -> None:
        self.query_one("#sources", Input).focus()

    def action_help(self) -> None:
        self.push_screen(HelpScreen(self.pal))

    # -- themes ----------------------------------------------------------------

    def on_theme_changed(self, theme) -> None:
        """Re-colour the rich content (tables, recipe) and remember the choice."""
        self.pal = self.palette()
        self.fill_crafts(quiet=True)
        if self.current:
            self.show_detail()
        _save_config({**_load_config(), "theme": theme.name})

    def palette(self) -> render.Palette:
        """The theme's colours as rich styles; ANSI themes keep terminal colour names."""
        if self.current_theme.ansi:
            return render.ANSI
        v = self.get_css_variables()

        def pick(name: str, fallback: str) -> str:
            value = (v.get(name) or "").split(" ")[0]
            return value if value.startswith("#") else fallback

        return render.Palette(
            accent=pick("secondary", "magenta"),
            link=pick("primary", "cyan"),
            good=pick("success", "green"),
            bad=pick("error", "red"),
            warn=pick("warning", "yellow"),
            action=pick("warning", "yellow"),
            share=pick("primary", "cyan"),
            magic="#8888FF",
            rare="#E8D86A",
            fractured="#A29162",
            bench="#9ECBFF",
            essence=pick("accent", "magenta"),
        )


class HelpScreen(ModalScreen):
    DEFAULT_CSS = """
    HelpScreen { align: center middle; }
    #help { width: auto; height: auto; padding: 1 2; border: round $primary; background: $surface; }
    """

    def __init__(self, pal: render.Palette):
        super().__init__()
        self.pal = pal

    def compose(self) -> ComposeResult:
        keys = Table.grid(padding=(0, 2))
        keys.add_column(style="bold")
        keys.add_column()
        for key, what in KEYS:
            keys.add_row(key, what)
        p = self.pal
        legend = Text.assemble(
            "\ncolours  ",
            ("fractured", p.fractured), " · ", ("bench", p.bench), " · ", ("essence", p.essence), " · ",
            ("T1", f"bold {p.rare}"), " · ", ("magic", p.magic), " · ", ("rare", p.rare), " · ",
            ("on the item", p.good), " · ", ("blocked", p.bad),
            "\nprices   1 divine and up in d (bold), below in c",
        )  # fmt: skip
        yield Static(Group(keys, legend), id="help")

    def on_key(self, event) -> None:
        self.dismiss()


def _cut(text: Text, width: int) -> Text:
    text.truncate(width, overflow="ellipsis")
    return text


def _load_config() -> dict:
    try:
        return json.loads(CONFIG.read_text())
    except (OSError, ValueError):
        return {}


def _save_config(config: dict) -> None:
    try:
        CONFIG.parent.mkdir(parents=True, exist_ok=True)
        CONFIG.write_text(json.dumps(config))
    except OSError:
        pass


def run_tui(sources: list[str], trade: bool = False, ladder: bool = False) -> int:
    PonderApp(sources, trade, ladder).run()
    return 0


def main() -> int:
    """`ponder-tui [--trade] SOURCES...`"""
    args = sys.argv[1:]
    return run_tui([a for a in args if a not in ("--trade", "--ladder")], "--trade" in args, "--ladder" in args)


if __name__ == "__main__":
    sys.exit(main())
