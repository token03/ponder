"""Textual front end: `ponder tui SOURCES` (or `ponder-tui`).

Runs the same `pipeline.run` the CLI does, in a worker thread, then lets you
browse the crafts and shared combos, read each recipe, price clusters on the
trade site and open or copy their search links. All the numbers come from the
library; this file is only layout and key bindings.
"""

from __future__ import annotations

import sys
import time
import webbrowser

from rich.text import Text
from textual import work
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, VerticalScroll
from textual.widgets import DataTable, Footer, Header, Input, Static, TabbedContent, TabPane

from . import pipeline, prices, render

FILTERS = ["all", "cluster", "gear"]
SORTS = {
    "margin": lambda e: (e.margin is None, -(e.margin or 0), -(e.total or 0)),
    "total": lambda e: (e.total is None, -(e.total or 0)),
    "builds": lambda e: (-len(e.builds), e.total is None, -(e.total or 0)),
    "cheapest": lambda e: (e.total is None, e.total or 0),
}


class PonderApp(App):
    TITLE = "ponder"
    CSS = """
    #status { height: 1; padding: 0 1; color: $text-muted; }
    #main { height: 1fr; }
    #tables { width: 3fr; }
    #detail-box { width: 2fr; border-left: solid $primary-darken-2; padding: 0 1; }
    DataTable { height: 1fr; }
    """
    BINDINGS = [
        Binding("t", "trade", "Price on trade"),
        Binding("T", "trade_all", "Price all clusters"),
        Binding("o", "open_link", "Open trade link"),
        Binding("y", "copy_link", "Copy link"),
        Binding("f", "cycle_filter", "Filter"),
        Binding("s", "cycle_sort", "Sort"),
        Binding("n", "new_sources", "New builds"),
        Binding("q", "quit", "Quit"),
    ]

    def __init__(self, sources: list[str], trade: bool = False):
        super().__init__()
        self.sources = sources
        self.trade_on_load = trade
        self.report: pipeline.Report | None = None
        self.table: dict[str, float] = {}
        self.by_key: dict[str, pipeline.Estimate] = {}
        self.filter = "all"
        self.sort = "margin"
        self.current: pipeline.Estimate | None = None

    def compose(self) -> ComposeResult:
        yield Header()
        yield Input(placeholder="pobb.in / pastebin links, PoB files or a .txt list, separated by spaces; Enter to run", id="sources")
        yield Static("", id="status")
        with Horizontal(id="main"):
            with TabbedContent(id="tables"):
                with TabPane("Crafts", id="crafts-tab"):
                    yield DataTable(id="crafts", cursor_type="row", zebra_stripes=True)
                with TabPane("Shared across builds", id="shared-tab"):
                    yield DataTable(id="shared", cursor_type="row", zebra_stripes=True)
            with VerticalScroll(id="detail-box"):
                yield Static("Pick a row to see its recipe.", id="detail")
        yield Footer()

    def on_mount(self) -> None:
        crafts = self.query_one("#crafts", DataTable)
        for label, key in [("item", "item"), ("total", "total"), ("market", "market"), ("margin", "margin"), ("builds", "builds"), ("targets", "targets")]:
            crafts.add_column(label, key=key)
        shared = self.query_one("#shared", DataTable)
        for label in ["builds", "items", "class", "mods", "craft"]:
            shared.add_column(label, key=label)
        box = self.query_one("#sources", Input)
        if self.sources:
            box.value = " ".join(self.sources)
            self.analyze(self.sources)
        else:
            box.focus()

    # -- running the pipeline ----------------------------------------------

    def on_input_submitted(self, event: Input.Submitted) -> None:
        sources = event.value.split()
        if sources:
            self.analyze(sources)
            self.query_one("#crafts", DataTable).focus()

    @work(thread=True, exclusive=True)
    def analyze(self, sources: list[str]) -> None:
        self.call_from_thread(self.set_status, "starting…")
        try:
            report = pipeline.run(sources, trade_lookup=self.trade_on_load, progress=lambda m: self.call_from_thread(self.set_status, m))
        except Exception as e:  # show it rather than crash the app
            self.call_from_thread(self.set_status, f"[red]failed: {e}[/red]")
            return
        self.table = prices.load()[0]
        self.call_from_thread(self.show_report, report)

    def set_status(self, message: str) -> None:
        self.query_one("#status", Static).update(message)

    def show_report(self, report: pipeline.Report) -> None:
        self.report = report
        self.sub_title = f"{len(report.builds)} build(s) · {report.league} · {report.prices_source}"
        failed = f" · [red]{len(report.errors)} source(s) failed: {report.errors[0]}[/red]" if report.errors else ""
        self.set_status(f"{len(report.estimates)} crafts, {len(report.shared)} shared combos{failed}")
        self.fill_crafts()
        shared = self.query_one("#shared", DataTable)
        shared.clear()
        for i, s in enumerate(report.shared):
            c, e = s.candidate, s.estimate
            craft = (render.chaos(e.total) if e.error is None else e.error) if e else ""
            shared.add_row(str(c.builds), str(c.count), Text(c.item_class), Text(" + ".join(c.families)), craft, key=f"shared:{i}")
            if e:
                self.by_key[f"shared:{i}"] = e

    def fill_crafts(self) -> None:
        if not self.report:
            return
        crafts = self.query_one("#crafts", DataTable)
        crafts.clear()
        self.by_key = {k: v for k, v in self.by_key.items() if not k.startswith("craft:")}
        rows = [e for e in self.report.estimates if self.filter == "all" or e.kind == self.filter]
        for i, e in enumerate(sorted(rows, key=SORTS[self.sort])):
            key = f"craft:{i}"
            self.by_key[key] = e
            crafts.add_row(*self.craft_cells(e), key=key)
        self.notify(f"{len(rows)} crafts · filter {self.filter} · sort {self.sort}", timeout=2)

    @staticmethod
    def craft_cells(e: pipeline.Estimate) -> list:
        market = render.chaos(e.market[0]) if e.market else Text(e.market_error or "-", style="dim")
        total = render.chaos(e.total) if e.error is None else Text("no plan", style="dim")
        return [
            Text(e.title, style="magenta" if e.kind == "cluster" else ""),
            total,
            market,
            render.margin_text(e.margin),
            str(len(e.builds)),
            Text(", ".join(e.targets)),
        ]

    # -- detail pane ---------------------------------------------------------

    def on_data_table_row_highlighted(self, event: DataTable.RowHighlighted) -> None:
        self.current = self.by_key.get(event.row_key.value) if event.row_key else None
        self.show_detail()

    def show_detail(self) -> None:
        detail = self.query_one("#detail", Static)
        if self.current is None:
            detail.update("No estimate for this row (only cluster combos are costed).")
            return
        detail.update(render.estimate_detail(self.current, self.report.prices_source if self.report else "", links=True))

    # -- actions -------------------------------------------------------------

    def action_trade(self) -> None:
        if self.current and self.current.query:
            self.lookup([self.current])
        else:
            self.notify("only cluster jewels have a trade search yet", severity="warning")

    def action_trade_all(self) -> None:
        if self.report:
            self.lookup([e for e in self.report.estimates if e.query and not e.market])

    @work(thread=True, exclusive=True, group="trade")
    def lookup(self, estimates: list[pipeline.Estimate]) -> None:
        league = self.report.league if self.report else pipeline.current_league()
        for i, e in enumerate(estimates):
            self.call_from_thread(self.set_status, f"trade lookup {i + 1}/{len(estimates)}: {', '.join(e.targets)}")
            pipeline.lookup_market(e, self.table or prices.load()[0], league)
            self.call_from_thread(self.refresh_estimate, e)
            if i + 1 < len(estimates):
                time.sleep(2)  # the trade API rate-limits hard
        self.call_from_thread(self.set_status, f"priced {len(estimates)} item(s)")

    def refresh_estimate(self, e: pipeline.Estimate) -> None:
        crafts = self.query_one("#crafts", DataTable)
        for key, est in self.by_key.items():
            if est is e and key.startswith("craft:"):
                for col, value in zip(["item", "total", "market", "margin", "builds", "targets"], self.craft_cells(e)):
                    crafts.update_cell(key, col, value)
        if e.market_error:
            self.notify(e.market_error, severity="warning")
        if e is self.current:
            self.show_detail()

    def action_open_link(self) -> None:
        url = self.current.trade_url if self.current else None
        if not url:
            self.notify("no trade link for this row", severity="warning")
            return
        try:
            self.open_url(url)
        except Exception:
            webbrowser.open(url)
        self.notify("opened trade search (press y to copy it instead)", timeout=3)

    def action_copy_link(self) -> None:
        url = self.current.trade_url if self.current else None
        if url:
            self.copy_to_clipboard(url)
            self.notify("trade link copied", timeout=2)

    def action_cycle_filter(self) -> None:
        self.filter = FILTERS[(FILTERS.index(self.filter) + 1) % len(FILTERS)]
        self.fill_crafts()

    def action_cycle_sort(self) -> None:
        names = list(SORTS)
        self.sort = names[(names.index(self.sort) + 1) % len(names)]
        self.fill_crafts()

    def action_new_sources(self) -> None:
        self.query_one("#sources", Input).focus()


def run_tui(sources: list[str], trade: bool = False) -> int:
    PonderApp(sources, trade).run()
    return 0


def main() -> int:
    """`ponder-tui [--trade] SOURCES...`"""
    args = sys.argv[1:]
    trade = "--trade" in args
    return run_tui([a for a in args if a != "--trade"], trade)


if __name__ == "__main__":
    sys.exit(main())
