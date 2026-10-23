from pathlib import Path

from ponder import pob

FIXTURES = Path(__file__).parent / "fixtures"


def test_code_roundtrip():
    xml = (FIXTURES / "build_a.xml").read_text()
    code = pob.encode(xml)
    assert "+" not in code and "/" not in code
    assert pob.decode(code) == xml


def test_load_from_code_or_file(tmp_path):
    xml = (FIXTURES / "build_a.xml").read_text()
    code_file = tmp_path / "build.txt"
    code_file.write_text(pob.encode(xml))
    for source in (str(FIXTURES / "build_a.xml"), str(code_file), pob.encode(xml)):
        build = pob.load(source)
        assert build.ascendancy == "Elementalist"


def test_parse_build():
    build = pob.parse((FIXTURES / "build_a.xml").read_text())
    assert (build.class_name, build.level, build.main_skill) == ("Witch", 94, "Fireball")
    assert {i.slot for i in build.items} == {"Helmet", "Amulet", "Boots"}
    assert build.items[0].text.splitlines()[:3] == ["Rarity: RARE", "Doom Crown", "Hubris Circlet"]
