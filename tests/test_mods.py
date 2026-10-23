"""Matching against real repoe data. Skipped until `ponder data fetch` has run."""

import pytest

from ponder import data
from ponder.item import parse_item
from ponder.mods import ModDB, normalize, template_ranges

needs_data = pytest.mark.skipif(not data.available(), reason="run `ponder data fetch` first")


def test_normalize_and_ranges():
    assert normalize("+(100-114) to maximum Life") == normalize("+105 to maximum Life") == "+# to maximum Life"
    assert template_ranges("Adds (5-7) to (11-13) Fire Damage") == [(5, 7), (11, 13)]


@pytest.fixture(scope="module")
def db():
    return ModDB()


@needs_data
def test_tiers_and_sources(db):
    item = parse_item(
        "Rarity: RARE\nX\nHubris Circlet\nShaper Item\nItem Level: 86\nImplicits: 0\n"
        "+135 to maximum Life\n+44% to Lightning Resistance\n{crafted}+23% to Lightning Resistance\n"
    )
    match = db.match(item)
    by_text = {m.lines[0].text: m for m in match.mods}
    life = by_text["+135 to maximum Life"]
    assert (life.kind, life.source, life.tier) == ("prefix", "natural", 1)
    assert by_text["+44% to Lightning Resistance"].tier == 2
    assert by_text["+23% to Lightning Resistance"].source == "crafted"
    assert match.unmatched == []


@needs_data
def test_hybrid_mod_is_matched_as_one(db):
    item = parse_item(
        "Rarity: RARE\nX\nHubris Circlet\nItem Level: 86\nImplicits: 0\n"
        "40% increased Energy Shield\n16% increased Stun and Block Recovery\n"
    )
    match = db.match(item)
    assert len(match.mods) == 1 and len(match.mods[0].lines) == 2


@needs_data
def test_influence_mod_needs_influence(db):
    text = "Rarity: RARE\nX\nHubris Circlet\n{}Item Level: 86\nImplicits: 0\nAdds 20 to 38 Cold Damage to Spells\n"
    shaper = db.match(parse_item(text.format("Shaper Item\n"))).mods[0]
    plain = db.match(parse_item(text.format(""))).mods[0]
    assert shaper.source == "natural"
    assert plain.source == "special"  # not natural without the influence


@needs_data
def test_unknown_line_is_reported(db):
    item = parse_item("Rarity: RARE\nX\nHubris Circlet\nImplicits: 0\nYou feel great\n")
    assert [line.text for line in db.match(item).unmatched] == ["You feel great"]
