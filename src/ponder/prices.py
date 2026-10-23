"""Prices in chaos from poe.ninja, keyed by poe.ninja's ids (e.g. "alt", "regal").

`ponder prices fetch` saves the current league's currency, essence, fossil,
resonator and cluster jewel prices under .cache/ninja. Without a fetch, a
small snapshot (Allflame, October 2026) keeps the numbers on the right scale.
"""

from __future__ import annotations

import json
import time
import urllib.parse
import urllib.request
from pathlib import Path

API = "https://poe.ninja/poe1/api/economy"
USER_AGENT = "ponder/0.1 (crafting cost research)"
CACHE = Path(".cache/ninja")
EXCHANGE_TYPES = ["Currency", "Essence", "Fossil", "Resonator"]

SNAPSHOT = {
    "chaos": 1.0, "divine": 332.3, "transmute": 0.004, "aug": 0.017, "alt": 0.074, "regal": 0.093,
    "alch": 0.032, "scour": 0.35, "exalted": 5.09, "annul": 11.33, "fracturing-orb": 240.7,
    "wild-lifeforce": 0.017, "vivid-lifeforce": 0.175, "primal-lifeforce": 0.047,
    "primitive-chaotic-resonator": 0.56,
}  # fmt: skip


def leagues() -> list[str]:
    return [entry["id"] for entry in _get(f"{API}/leagues")]


def fetch(league: str | None = None) -> tuple[str, int]:
    league = league or leagues()[0]
    CACHE.mkdir(parents=True, exist_ok=True)
    table = {"chaos": 1.0}
    for kind in EXCHANGE_TYPES:
        body = _get(f"{API}/exchange/current/overview?league={urllib.parse.quote(league)}&type={kind}")
        table.update({line["id"]: line["primaryValue"] for line in body["lines"]})
        time.sleep(1)  # be gentle; the data only refreshes every ~15 minutes anyway
    clusters = _get(f"{API}/stash/current/item/overview?league={urllib.parse.quote(league)}&type=ClusterJewel")
    (CACHE / "prices.json").write_text(json.dumps({"league": league, "fetched": time.time(), "prices": table}, indent=1))
    (CACHE / "clusters.json").write_text(json.dumps(clusters["lines"]))
    return league, len(table)


def load() -> tuple[dict[str, float], str]:
    """Prices plus a note on where they came from."""
    path = CACHE / "prices.json"
    if path.is_file():
        saved = json.loads(path.read_text())
        age = (time.time() - saved["fetched"]) / 3600
        return {**SNAPSHOT, **saved["prices"]}, f"poe.ninja {saved['league']}, {age:.0f}h old"
    return dict(SNAPSHOT), "built-in Allflame snapshot (run `ponder prices fetch`)"


def cluster_lines() -> list[dict]:
    path = CACHE / "clusters.json"
    return json.loads(path.read_text()) if path.is_file() else []


def cluster_base(size: str, enchant_text: str, passives: int, item_level: int) -> tuple[float, int] | None:
    """Cheapest-listed price of a base with this enchant and passive count, at or above `item_level`.

    poe.ninja buckets bases by item level (1, 50, 68, 75, 84); a higher bucket
    still works for lower-level notables, so take the cheapest qualifying one
    with real volume. Returns (chaos, listings).
    """
    hits = [
        line
        for line in cluster_lines()
        if line["baseType"].lower().startswith(size)
        and line["name"] == enchant_text
        and line.get("variant") == f"{passives} passives"
        and line.get("levelRequired", 0) >= item_level
        and line.get("count", 0) >= 3
    ]
    if not hits:
        return None
    best = min(hits, key=lambda line: line["chaosValue"])
    return best["chaosValue"], best["count"]


def enchant_option(enchant_text: str) -> str | None:
    """The trade-site option id for a cluster enchant, as poe.ninja lists it."""
    for line in cluster_lines():
        if line["name"] == enchant_text:
            for info in line.get("tradeInfo", []):
                if info["mod"] == "enchant.stat_3948993189":
                    return info["option"]
    return None


def _get(url: str):
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.load(resp)
