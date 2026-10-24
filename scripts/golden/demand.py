"""Export demand.family_stats on the cached ladder for the Rust port.

    uv run python scripts/golden/demand.py [OUT_DIR]   # default rust/goldens/demand

- family_stats.json.gz: every FamilyStat (min_characters=10) with its tier counts, natural tier chances,
  and lift at tiers 1-3, in order.
"""

from __future__ import annotations

import gzip
import json
import sys
from pathlib import Path

from ponder import demand, ladder
from ponder.mods import ModDB


def main() -> None:
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "rust/goldens/demand")
    out.mkdir(parents=True, exist_ok=True)
    db = ModDB()
    worn = demand.wear(db, list(ladder.cached()))
    rows = [
        {"group": s.group, "family": s.family, "side": s.side, "text": s.text, "characters": s.characters,
         "tiers": {str(t): n for t, n in s.tiers.items()}, "natural": {str(t): p for t, p in s.natural.items()},
         "lift": {str(t): s.lift(t) for t in (1, 2, 3)}, "share": {str(t): s.share_at_or_above(t) for t in (1, 2, 3)}}
        for s in demand.family_stats(db, worn)
    ]  # fmt: skip
    with gzip.open(out / "family_stats.json.gz", "wt") as f:
        json.dump(rows, f)
    print(f"family_stats: {len(rows)} entries")


if __name__ == "__main__":
    main()
