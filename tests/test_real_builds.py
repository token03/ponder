"""Match rates on real PoB exports. Skipped until `ponder data fetch` has run."""

from pathlib import Path

import pytest

from ponder import data, pob
from ponder.item import parse_item
from ponder.mods import ModDB

REAL = Path(__file__).parent / "fixtures" / "real"


@pytest.mark.skipif(not data.available(), reason="run `ponder data fetch` first")
@pytest.mark.parametrize("name", ["occultist_winter_orb.txt", "marauder_rf_leveling.txt"])
def test_rare_mod_lines_match(name):
    db = ModDB()
    build = pob.load(str(REAL / name))
    matched = missed = 0
    for pob_item in build.items:
        item = parse_item(pob_item.text)
        if item.rarity != "RARE":
            continue
        m = db.match(item)
        assert m.base_id is not None, item.base
        matched += len({id(line) for mod in m.mods for line in mod.lines})
        missed += len(m.unmatched)  # author notes don't count against us
    assert matched / (matched + missed) >= 0.95


def test_item_sets_are_recorded():
    build = pob.load(str(REAL / "occultist_winter_orb.txt"))
    assert (build.ascendancy, build.main_skill) == ("Occultist", "Winter Orb")
    sets = {s for item in build.items for s in item.sets}
    assert {"Early game", "Mid game", "Endgame Optional"} <= sets
