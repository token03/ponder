"""Characters from the poe.ninja ladder, with their items as exact mod ids.

poe.ninja's builds API serves a search (protobuf: dimensions with counts, and
the top 100 characters for a filter) and one JSON document per character,
whose items list every explicit mod by id. That makes ladder items exact:
no text matching, just ids into the mod database.

`sample()` takes the top characters of every main skill with enough players,
so many classes, skills and weapon types are covered, and caches each
character under .cache/ladder/<snapshot>/.
"""

from __future__ import annotations

import json
import re
import struct
import time
import urllib.parse
import urllib.request
from collections.abc import Callable, Iterator
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path

API = "https://poe.ninja/poe1/api"
USER_AGENT = "ponder/0.1 (crafting research; caches everything)"
CACHE = Path(".cache/ladder")
LITE_VERSION = 2  # bump when `lite` keeps more fields; older .lite copies are rebuilt


@dataclass(frozen=True)
class Snapshot:
    league: str  # display name, e.g. "Allflame"
    overview: str  # url name, e.g. "allflame"
    version: str


@dataclass(frozen=True)
class Row:
    account: str
    name: str


def snapshot(league: str | None = None) -> Snapshot:
    """The current experience-ladder snapshot of `league` (default: the first build league)."""
    index = _get_json(f"{API}/data/index-state")
    for s in index["snapshotVersions"]:
        if s["type"] == "exp" and (league is None or league.lower() in (s["name"].lower(), s["url"])):
            return Snapshot(s["name"], s["url"], s["version"])
    raise ValueError(f"no ladder snapshot for {league!r}")


def search(snap: Snapshot, **filters: str) -> tuple[list[Row], dict[str, list[tuple[str, int]]]]:
    """The top 100 characters for `filters` (e.g. skills="Lightning Arrow", class="Deadeye"),
    and every dimension's values with character counts (e.g. "skills" -> [(skill, count)])."""
    query = urllib.parse.urlencode({"overview": snap.overview, "type": "exp", **filters})
    body = _get(f"{API}/builds/{snap.version}/search?{query}")
    inner = _fields(_fields(body)[0][2])
    dict_hash = {}
    for f, _, v in inner:
        if f == 6:
            sub = _fields(v)
            dict_hash[sub[0][2].decode()] = sub[1][2].decode()  # field 2: the dictionary's hash
    dims = {}
    for f, _, v in inner:
        if f == 2:
            sub = _fields(v)
            name, dname = sub[0][2].decode(), sub[1][2].decode()
            words = dictionary(dict_hash[dname]) if dname in dict_hash else []
            counts = []
            for g, _, x in sub:
                if g == 3:
                    e = {a: b for a, _, b in _fields(x)}
                    i = e.get(1, 0)
                    counts.append((words[i] if i < len(words) else str(i), e.get(2, 0)))
            dims[name] = sorted(counts, key=lambda c: -c[1])
    columns = {}
    for f, _, v in inner:
        if f == 12:
            sub = _fields(v)
            if sub[0][2] in (b"name", b"account"):
                columns[sub[0][2].decode()] = [x.decode() for g, _, x in sub if g == 7]
    rows = [Row(a, n) for a, n in zip(columns.get("account", []), columns.get("name", []))]
    return rows, dims


def dictionary(hash_: str) -> list[str]:
    path = CACHE / "dictionaries" / hash_
    if not path.is_file():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(_get(f"{API}/builds/dictionary/{hash_}"))
    return _ndic(path.read_bytes())


def character(snap: Snapshot, row: Row) -> dict | None:
    """One character's JSON (items with mod ids, PoB export...), cached per snapshot."""
    path = CACHE / snap.version / _safe(row.account) / f"{_safe(row.name)}.json"
    if path.is_file():
        return json.loads(path.read_text())
    query = urllib.parse.urlencode({"account": row.account, "name": row.name, "overview": snap.overview, "type": "exp"})
    try:
        data = _get_json(f"{API}/builds/{snap.version}/character?{query}")
    except Exception:
        return None
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data))
    return data


def sample(
    snap: Snapshot,
    per_skill: int = 15,
    min_players: int = 40,
    per_class: int = 20,
    workers: int = 4,
    progress: Callable[[str], None] | None = None,
) -> Iterator[dict]:
    """Top characters of every main skill with at least `min_players`, plus of every ascendancy.

    Ladder order (level and experience) puts invested characters first, which
    is what crafted items are sold to. Duplicates across filters are fetched once.
    """
    say = progress or (lambda _msg: None)
    _, dims = search(snap)
    filters = [("skills", s) for s, n in dims.get("skills", []) if n >= min_players]
    filters += [("class", c) for c, n in dims.get("class", []) if n > 0]
    rows: dict[Row, None] = {}
    for i, (key, value) in enumerate(filters, 1):
        say(f"ladder search {i}/{len(filters)}: {value}")
        found, _ = search(snap, **{key: value})
        for r in found[: per_skill if key == "skills" else per_class]:
            rows.setdefault(r, None)
    say(f"fetching {len(rows)} characters")
    with ThreadPoolExecutor(workers) as pool:
        for i, data in enumerate(pool.map(lambda r: character(snap, r), rows), 1):
            if i % 50 == 0:
                say(f"characters {i}/{len(rows)}")
            if data:
                yield data


def cached(snap: Snapshot | None = None) -> Iterator[dict]:
    """Characters already downloaded (for `snap`, else the newest snapshot on disk), as `lite` dicts; no network.

    Full documents are big (PoB export, passive tree); the first read writes a
    small `.lite` copy next to each, which later reads use.
    """
    if snap is None:
        dirs = sorted(p for p in CACHE.iterdir() if p.is_dir() and p.name != "dictionaries") if CACHE.is_dir() else []
        if not dirs:
            return
        root = dirs[-1]
    else:
        root = CACHE / snap.version
    # A snapshot unpacked from a zip of `.lite` files has no full documents: read those alone.
    stems = sorted({p.with_suffix("") for p in root.glob("*/*.json")} | {p.with_suffix("") for p in root.glob("*/*.lite")})
    for stem in stems:
        path, small = stem.with_suffix(".json"), stem.with_suffix(".lite")
        try:
            if small.is_file():
                data = json.loads(small.read_text())
                if data.get("v") == LITE_VERSION:
                    yield data
                    continue
            if not path.is_file():
                continue  # an outdated lite copy with nothing to rebuild it from
            data = lite(json.loads(path.read_text()))
        except ValueError:
            continue
        small.write_text(json.dumps(data))
        yield data


def lite(data: dict) -> dict:
    """What crafting analysis needs from a character: who, main skill, and rare items by mod id."""
    from .demand import main_skill

    return {
        "v": LITE_VERSION,
        "account": data.get("account"),
        "name": data.get("name"),
        "class": data.get("class"),
        "skill": main_skill(data),
        "rares": [
            {k: item.get(k) for k in ("baseType", "ilvl", "influences", "mods", "inventoryId", "enchantMods") if k in item}
            for item in rare_items(data)
        ],
    }


def rare_items(data: dict) -> Iterator[dict]:
    """A character's rare equipment and jewels (poe.ninja `itemData`, or a `lite` dict's)."""
    if "rares" in data:
        yield from data["rares"]
        return
    for entry in data.get("items", []) + data.get("jewels", []):
        item = entry.get("itemData", {})
        if item.get("frameType") == 2 and item.get("identified", True):
            yield item


# -- transport and formats --------------------------------------------------------------


def _get(url: str, retries: int = 3) -> bytes:
    for attempt in range(retries):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(req, timeout=60) as resp:
                return resp.read()
        except Exception:
            if attempt == retries - 1:
                raise
            time.sleep(2 * (attempt + 1))
    raise AssertionError


def _get_json(url: str):
    return json.loads(_get(url))


def _safe(name: str) -> str:
    return re.sub(r"[^\w.-]", "_", name)


def _varint(b: bytes, i: int) -> tuple[int, int]:
    r = s = 0
    while True:
        x = b[i]
        i += 1
        r |= (x & 0x7F) << s
        s += 7
        if x < 0x80:
            return r, i


def _fields(b: bytes) -> list[tuple[int, int, object]]:
    """Protobuf wire format, schema-less: (field number, wire type, value)."""
    i, out = 0, []
    while i < len(b):
        key, i = _varint(b, i)
        f, wt = key >> 3, key & 7
        if wt == 0:
            v, i = _varint(b, i)
        elif wt == 2:
            n, i = _varint(b, i)
            v, i = b[i : i + n], i + n
        elif wt == 1:
            v, i = b[i : i + 8], i + 8
        elif wt == 5:
            v, i = b[i : i + 4], i + 4
        else:
            raise ValueError(f"wire type {wt}")
        out.append((f, wt, v))
    return out


def _ndic(b: bytes) -> list[str]:
    """poe.ninja's dictionary format ("NDIC"): a header, a block index, varint string lengths, the strings.

    Header (little-endian u32s): magic, version, 0, count, then an 8-byte hash,
    block size, block count, and the byte length of the varint length table;
    then (offset, first index) per block.
    """
    if b[:4] != b"NDIC":
        raise ValueError("not an NDIC dictionary")
    count = struct.unpack_from("<I", b, 12)[0]
    blocks, length_bytes = struct.unpack_from("<II", b, 28)
    i = 36 + 8 * blocks
    end = i + length_bytes
    lengths = []
    while i < end:
        n, i = _varint(b, i)
        lengths.append(n)
    if len(lengths) != count:
        raise ValueError(f"NDIC: {len(lengths)} lengths for {count} strings")
    out = []
    for n in lengths:
        out.append(b[i : i + n].decode("utf-8", "replace"))
        i += n
    return out
