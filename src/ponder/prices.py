"""Currency prices in chaos.

Defaults are rough placeholders so the numbers have a scale; run
`ponder prices fetch --league NAME` (needs poe.ninja reachable) or write
`prices.json` by hand for real values.
"""

from __future__ import annotations

import json
import urllib.parse
import urllib.request
from pathlib import Path

PLACEHOLDER = {
    "Orb of Transmutation": 0.05,
    "Orb of Augmentation": 0.05,
    "Orb of Alteration": 0.15,
    "Orb of Scouring": 0.5,
    "Regal Orb": 0.5,
    "Exalted Orb": 12.0,
    "Orb of Annulment": 2.0,
    "Divine Orb": 150.0,
}
DEFAULT_FILE = Path("prices.json")
NINJA = "https://poe.ninja/api/data/currencyoverview?league={league}&type=Currency"


def load(path: Path | None = None) -> tuple[dict[str, float], str]:
    """Prices plus where they came from."""
    path = path or DEFAULT_FILE
    if path.is_file():
        return {**PLACEHOLDER, **json.loads(path.read_text())}, str(path)
    return dict(PLACEHOLDER), "placeholder defaults"


def fetch(league: str, path: Path | None = None) -> dict[str, float]:
    url = NINJA.format(league=urllib.parse.quote(league))
    req = urllib.request.Request(url, headers={"User-Agent": "ponder"})
    with urllib.request.urlopen(req, timeout=30) as resp:
        lines = json.load(resp)["lines"]
    prices = {line["currencyTypeName"]: line["chaosEquivalent"] for line in lines}
    prices["Chaos Orb"] = 1.0
    (path or DEFAULT_FILE).write_text(json.dumps(prices, indent=1, sort_keys=True) + "\n")
    return prices
