from ponder.item import Item
from ponder.mine import mine
from ponder.mods import ItemMatch, ModMatch


def rare(item_class, *families, crafted=()):
    mods = [ModMatch(f, "prefix", f, "natural", 1, []) for f in families]
    mods += [ModMatch(f, "suffix", f, "crafted", None, []) for f in crafted]
    return ItemMatch(item=Item("RARE", "x", "y"), base_id="b", item_class=item_class, mods=mods)


def test_shared_combos_ranked_by_builds():
    matches = [
        ("a", rare("Helmet", "Life", "FireRes", "ColdRes")),
        ("b", rare("Helmet", "Life", "FireRes", "ChaosRes")),
        ("c", rare("Helmet", "Life", "FireRes", "ColdRes", crafted=["ChaosRes"])),
        ("c", rare("Amulet", "Life", "Crit")),
    ]
    out = mine(matches)
    top = out[0]
    assert (top.item_class, top.families, top.builds) == ("Helmet", ("FireRes", "Life"), 3)
    assert ("Helmet", ("ColdRes", "FireRes", "Life")) in {(c.item_class, c.families) for c in out}
    # Crafted mods don't count, and single-build combos are dropped.
    assert all("ChaosRes" not in c.families or c.builds >= 2 for c in out)
    assert not any(c.item_class == "Amulet" for c in out)


def test_closed_drops_redundant_subsets():
    matches = [("a", rare("Ring", "A", "B", "C")), ("b", rare("Ring", "A", "B", "C"))]
    assert [c.families for c in mine(matches)] == [("A", "B", "C")]
    assert len(mine(matches, closed=False)) == 4
