from ponder.item import parse_item

HELMET = """\
Rarity: RARE
Doom Crown
Hubris Circlet
Shaper Item
Item Level: 86
Quality: 20
Implicits: 0
+121 to maximum Life
{fractured}84% increased Energy Shield
{crafted}+23% to Lightning Resistance
Corrupted
"""

BOOTS = """\
Rarity: RARE
Grim Stride
Two-Toned Boots
Selected Variant: 2
Implicits: 2
{variant:1}+(8-12)% to Fire and Cold Resistances
{variant:2}+(8-12)% to Cold and Lightning Resistances
{range:0.5}+(42-45)% to Lightning Resistance
"""


def test_header_and_markers():
    item = parse_item(HELMET)
    assert (item.rarity, item.name, item.base) == ("RARE", "Doom Crown", "Hubris Circlet")
    assert (item.item_level, item.quality) == (86, 20)
    assert item.influences == {"shaper"}
    assert item.flags == {"corrupted"}
    assert [m.text for m in item.explicits] == [
        "+121 to maximum Life",
        "84% increased Energy Shield",
        "+23% to Lightning Resistance",
    ]
    assert item.explicits[1].fractured and item.explicits[2].crafted


def test_variants_split_after_implicit_count():
    item = parse_item(BOOTS)
    assert [m.text for m in item.implicits] == ["+(8-12)% to Cold and Lightning Resistances"]
    assert len(item.explicits) == 1
    assert item.explicits[0].roll == 0.5


def test_magic_item_has_single_name_line():
    item = parse_item("Rarity: MAGIC\nRunner's Two-Toned Boots\nImplicits: 0\n25% increased Movement Speed\n")
    assert item.base == "Runner's Two-Toned Boots"
    assert [m.text for m in item.explicits] == ["25% increased Movement Speed"]


def test_pob_crafter_affix_ids():
    item = parse_item("Rarity: RARE\nX\nOnyx Amulet\nPrefix: {range:0.5}IncreasedLife7\nSuffix: None\nImplicits: 0\n")
    assert item.pob_affixes == [("prefix", "IncreasedLife7")]


COPIED = """\
Item Class: Rings
Rarity: Rare
Gale Loop
Bone Ring
--------
Requirements:
Level: 75
--------
Item Level: 84
--------
+15 to maximum Energy Shield (implicit)
--------
{ Prefix Modifier "Virile" (Tier: 1) — Life }
+105(100-114) to maximum Life
+40 to maximum Energy Shield (fractured)
+42% to Fire Resistance
+12 to Dexterity (crafted)
--------
Shaper Item
--------
Corrupted
"""


def test_item_copied_in_game():
    item = parse_item(COPIED)
    assert (item.rarity, item.name, item.base, item.item_level) == ("RARE", "Gale Loop", "Bone Ring", 84)
    assert [m.text for m in item.implicits] == ["+15 to maximum Energy Shield"]
    assert [m.text for m in item.explicits] == ["+105 to maximum Life", "+40 to maximum Energy Shield", "+42% to Fire Resistance", "+12 to Dexterity"]
    assert item.explicits[1].fractured and item.explicits[3].crafted
    assert item.influences == {"shaper"} and {"corrupted", "fractured"} <= item.flags
