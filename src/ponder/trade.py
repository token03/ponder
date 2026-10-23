"""Path of Exile trade site: build searches, and run them when allowed.

The site's API (the one Awakened PoE Trade uses) refuses some networks and
may want a logged-in session; set POESESSID to your pathofexile.com session
cookie if searches come back 403. `search_url` always works in a browser.
"""

from __future__ import annotations

import json
import os
import statistics
import time
import urllib.parse
import urllib.request

from . import data

SITE = "https://www.pathofexile.com"
USER_AGENT = "ponder/0.1 (crafting cost research)"
ENCHANT_STAT = "enchant.stat_3948993189"  # Added Small Passive Skills grant: #
PASSIVES_STAT = "enchant.stat_3086156145"  # Adds # Passive Skills


def stat_id(mod_id: str, kind: str = "explicit") -> str | None:
    """Trade stat id for a mod's (first) stat, from repoe's stat translations."""
    stats = [s["id"] for s in data.load("mods")[mod_id]["stats"]]
    for entry in _translations().get(stats[0], []):
        for trade in entry.get("trade_stats") or []:
            if trade["type"] == kind:
                return trade["id"]
    return None


def cluster_query(size: str, enchant_option: str | None, passives: int | None, notable_ids: list[str], item_level: int | None = None) -> dict:
    filters = [{"id": stat_id(m) or m, "disabled": False} for m in notable_ids]
    if enchant_option:
        filters.append({"id": ENCHANT_STAT, "value": {"option": enchant_option}, "disabled": False})
    if passives:
        filters.append({"id": PASSIVES_STAT, "value": {"min": passives, "max": passives}, "disabled": False})
    query: dict = {
        "status": {"option": "available"},
        "type": f"{size.title()} Cluster Jewel",
        "stats": [{"type": "and", "filters": filters}],
        "filters": {"type_filters": {"filters": {"rarity": {"option": "nonunique"}}}},
    }
    if item_level:
        query["filters"]["misc_filters"] = {"filters": {"ilvl": {"min": item_level}}}
    return {"query": query, "sort": {"price": "asc"}}


def search_url(league: str, query: dict) -> str:
    q = urllib.parse.quote(json.dumps({"query": query["query"]}, separators=(",", ":")))
    return f"{SITE}/trade/search/{urllib.parse.quote(league)}?q={q}"


def price(league: str, query: dict, rates: dict[str, float], listings: int = 10) -> tuple[float, int] | None:
    """Median asking price (chaos) of the cheapest `listings`, and the total found."""
    found = _call(f"{SITE}/api/trade/search/{urllib.parse.quote(league)}", query)
    ids = found.get("result", [])[:listings]
    if not ids:
        return None
    time.sleep(1.5)  # the trade API rate-limits hard; one search + one fetch per check
    fetched = _call(f"{SITE}/api/trade/fetch/{','.join(ids)}?query={found['id']}")
    asks = []
    for r in fetched.get("result", []):
        p = (r.get("listing") or {}).get("price") or {}
        rate = rates.get(_currency_id(p.get("currency", "")))
        if p.get("amount") and rate:
            asks.append(p["amount"] * rate)
    return (statistics.median(asks), found.get("total", len(ids))) if asks else None


def _currency_id(trade_currency: str) -> str:
    return {"exalted": "exalted", "divine": "divine", "chaos": "chaos"}.get(trade_currency, trade_currency)


def _call(url: str, body: dict | None = None) -> dict:
    headers = {"User-Agent": USER_AGENT, "Content-Type": "application/json"}
    if os.environ.get("POESESSID"):
        headers["Cookie"] = f"POESESSID={os.environ['POESESSID']}"
    req = urllib.request.Request(url, data=json.dumps(body).encode() if body else None, headers=headers)
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.load(resp)


_cache: dict = {}


def _translations() -> dict[str, list[dict]]:
    if "t" not in _cache:
        index: dict[str, list[dict]] = {}
        for entry in data.load("stat_translations"):
            for sid in entry["ids"]:
                index.setdefault(sid, []).append(entry)
        _cache["t"] = index
    return _cache["t"]
