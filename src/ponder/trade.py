"""Path of Exile trade site: build searches, and run them when allowed.

The site's API (the one Awakened PoE Trade uses) refuses some networks and
wants a logged-in session: put `POESESSID=...` in a git-ignored `.env` in the
directory you run ponder from (or export it). `search_url` always works in a
browser.

The API allows only a few dozen searches per five minutes. Every response
says how much of each limit is used (X-Rate-Limit-* headers), so calls wait
just long enough to stay under them, and answers are cached on disk for
`CACHE_TTL` seconds so reruns don't search again.
"""

from __future__ import annotations

import hashlib
import json
import os
import statistics
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable
from pathlib import Path

from . import data

SITE = "https://www.pathofexile.com"
USER_AGENT = "ponder/0.1 (crafting cost research)"
ENCHANT_STAT = "enchant.stat_3948993189"  # Added Small Passive Skills grant: #
PASSIVES_STAT = "enchant.stat_3086156145"  # Adds # Passive Skills
CACHE = Path(".cache/trade")
CACHE_TTL = int(os.environ.get("PONDER_TRADE_TTL", 3600))

# Trade categories for item classes; groups of several bases search by category.
CATEGORIES = {
    "Ring": "accessory.ring", "Amulet": "accessory.amulet", "Belt": "accessory.belt",
    "Gloves": "armour.gloves", "Boots": "armour.boots", "Helmet": "armour.helmet",
    "Body Armour": "armour.chest", "Shield": "armour.shield", "Quiver": "armour.quiver",
    "Jewel": "jewel.base", "AbyssJewel": "jewel.abyss",
    "Wand": "weapon.wand", "Sceptre": "weapon.sceptre", "Dagger": "weapon.dagger",
    "Rune Dagger": "weapon.runedagger", "Claw": "weapon.claw", "Bow": "weapon.bow",
    "Staff": "weapon.staff", "Warstaff": "weapon.warstaff",
    "One Hand Sword": "weapon.onesword", "Thrusting One Hand Sword": "weapon.onesword",
    "Two Hand Sword": "weapon.twosword", "One Hand Axe": "weapon.oneaxe", "Two Hand Axe": "weapon.twoaxe",
    "One Hand Mace": "weapon.onemace", "Two Hand Mace": "weapon.twomace",
}  # fmt: skip

on_wait: Callable[[float], None] | None = None  # told how long a rate-limit wait will be
searches = 0  # searches sent to the site this run (cached answers don't count)


def stat_id(mod_id: str, kind: str = "explicit") -> str | None:
    """Trade stat id for a mod's (first) stat, from repoe's stat translations."""
    stats = [s["id"] for s in data.load("mods")[mod_id]["stats"]]
    for entry in _translations().get(stats[0], []):
        for trade in entry.get("trade_stats") or []:
            if trade["type"] == kind:
                return trade["id"]
    return None


def stat_min(mod_ids) -> float | None:
    """Lowest roll of the first stat across `mod_ids`: the value a search must ask for."""
    mods = data.load("mods")
    lows = [mods[m]["stats"][0]["min"] for m in mod_ids if mods[m].get("stats")]
    return min(lows) if lows else None


def item_query(
    *,
    base: str | None = None,
    category: str | None = None,
    stats: list[tuple[str, float | None]] = (),
    item_level: int | None = None,
    fractured_only: bool = False,
    extra: list[dict] = (),
) -> dict:
    """A search for non-unique items with all of `stats` (trade stat id, minimum value)."""
    filters = [{"id": sid, "value": {"min": low} if low is not None else {}, "disabled": False} for sid, low in stats]
    filters += list(extra)
    type_filters: dict = {"rarity": {"option": "nonunique"}}
    if category:
        type_filters["category"] = {"option": category}
    query: dict = {
        "status": {"option": "available"},
        "stats": [{"type": "and", "filters": filters}],
        "filters": {"type_filters": {"filters": type_filters}},
    }
    if base:
        query["type"] = base
    misc: dict = {}
    if item_level:
        misc["ilvl"] = {"min": item_level}
    if fractured_only:
        misc["fractured_item"] = {"option": "true"}
    if misc:
        query["filters"]["misc_filters"] = {"filters": misc}
    return {"query": query, "sort": {"price": "asc"}}


def cluster_query(
    size: str,
    enchant_option: str | None,
    passives: int | None,
    notable_ids: list[str],
    item_level: int | None = None,
    fractured_ids: list[str] = (),
) -> dict:
    extra = []
    if enchant_option:
        extra.append({"id": ENCHANT_STAT, "value": {"option": enchant_option}, "disabled": False})
    if passives:
        extra.append({"id": PASSIVES_STAT, "value": {"min": passives, "max": passives}, "disabled": False})
    stats = [(stat_id(m) or m, None) for m in notable_ids] + [(stat_id(m, "fractured") or m, None) for m in fractured_ids]
    return item_query(
        base=f"{size.title()} Cluster Jewel", stats=stats, item_level=item_level, fractured_only=bool(fractured_ids), extra=extra
    )


def search_url(league: str, query: dict) -> str:
    q = urllib.parse.quote(json.dumps({"query": query["query"]}, separators=(",", ":")))
    return f"{SITE}/trade/search/{urllib.parse.quote(league)}?q={q}"


def price(league: str, query: dict, rates: dict[str, float], listings: int = 10, max_age: int = CACHE_TTL) -> tuple[float, int] | None:
    """Median asking price (chaos) of the cheapest `listings`, and the total found.

    Answers younger than `max_age` seconds come from the disk cache.
    """
    key = hashlib.sha1(json.dumps([league, query, listings], sort_keys=True).encode()).hexdigest()
    path = CACHE / f"{key}.json"
    if path.is_file():
        saved = json.loads(path.read_text())
        if time.time() - saved["time"] < max_age:
            return _priced(saved["asks"], saved["total"], rates)
    global searches
    searches += 1
    found = _call(f"{SITE}/api/trade/search/{urllib.parse.quote(league)}", query, "search")
    ids = found.get("result", [])[:listings]
    asks: list[tuple[float, str]] = []
    if ids:
        fetched = _call(f"{SITE}/api/trade/fetch/{','.join(ids)}?query={found['id']}", None, "fetch")
        for r in fetched.get("result", []):
            p = (r.get("listing") or {}).get("price") or {}
            if p.get("amount") and p.get("currency"):
                asks.append((p["amount"], p["currency"]))
    CACHE.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"time": time.time(), "asks": asks, "total": found.get("total", len(ids))}))
    return _priced(asks, found.get("total", len(ids)), rates)


def _priced(asks, total: int, rates: dict[str, float]) -> tuple[float, int] | None:
    """Asks are cached in their own currency, so a divine-rate change reprices them."""
    chaos = [amount * rates[_currency_id(cur)] for amount, cur in asks if _currency_id(cur) in rates]
    return (statistics.median(chaos), total) if chaos else None


def _currency_id(trade_currency: str) -> str:
    return {"exalted": "exalted", "divine": "divine", "chaos": "chaos", "alch": "alch", "alt": "alt"}.get(trade_currency, trade_currency)


def cookie() -> str | None:
    """POESESSID from the environment, else from `.env` in the working directory."""
    if os.environ.get("POESESSID"):
        return os.environ["POESESSID"]
    path = Path(".env")
    if path.is_file():
        for line in path.read_text().splitlines():
            key, _, value = line.strip().removeprefix("export ").partition("=")
            if key.strip() == "POESESSID" and value.strip():
                return value.strip().strip("'\"")
    return None


# -- rate limits ------------------------------------------------------------------

# endpoint ("search"/"fetch") -> (when seen, [(max hits, period s, hits now, restricted s)])
_limits: dict[str, tuple[float, list[tuple[int, int, int, int]]]] = {}


def _call(url: str, body: dict | None, endpoint: str, retries: int = 2) -> dict:
    _wait_for(endpoint)
    headers = {"User-Agent": USER_AGENT, "Content-Type": "application/json"}
    if c := cookie():
        headers["Cookie"] = f"POESESSID={c}"
    req = urllib.request.Request(url, data=json.dumps(body).encode() if body else None, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            _remember_limits(endpoint, resp.headers)
            return json.load(resp)
    except urllib.error.HTTPError as e:
        _remember_limits(endpoint, e.headers)
        if e.code == 429 and retries:
            _sleep(float(e.headers.get("Retry-After") or 60))
            return _call(url, body, endpoint, retries - 1)
        raise


def _remember_limits(endpoint: str, headers) -> None:
    """Parse e.g. `X-Rate-Limit-Ip: 8:10:60,15:60:300` and `X-Rate-Limit-Ip-State: 1:10:0,1:60:0`."""
    if not headers or not headers.get("X-Rate-Limit-Rules"):
        return
    rules = []
    for rule in headers["X-Rate-Limit-Rules"].split(","):
        limits = headers.get(f"X-Rate-Limit-{rule}", "")
        states = headers.get(f"X-Rate-Limit-{rule}-State", "")
        for lim, st in zip(limits.split(","), states.split(",")):
            try:
                max_hits, period, _ = (int(x) for x in lim.split(":"))
                hits, _, restricted = (int(x) for x in st.split(":"))
            except ValueError:
                continue
            rules.append((max_hits, period, hits, restricted))
    _limits[endpoint] = (time.time(), rules)


def _wait_for(endpoint: str) -> None:
    """Sleep until one more call fits under every limit the last response reported."""
    if endpoint not in _limits:
        return
    seen, rules = _limits[endpoint]
    elapsed = time.time() - seen
    wait = 0.0
    for max_hits, period, hits, restricted in rules:
        if restricted:
            wait = max(wait, restricted - elapsed)
        elif hits >= max_hits - 1:  # keep one call spare: other tools may share the IP
            wait = max(wait, period - elapsed)
    if wait > 0:
        _sleep(wait)


def _sleep(seconds: float) -> None:
    if on_wait:
        on_wait(seconds)
    time.sleep(seconds)


_cache: dict = {}


def _translations() -> dict[str, list[dict]]:
    if "t" not in _cache:
        index: dict[str, list[dict]] = {}
        for entry in data.load("stat_translations"):
            for sid in entry["ids"]:
                index.setdefault(sid, []).append(entry)
        _cache["t"] = index
    return _cache["t"]
