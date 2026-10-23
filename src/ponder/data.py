"""Download and load game data exported by repoe-fork.

repoe-fork publishes its JSON exports to the repoe-fork.github.io repository;
we read them through raw.githubusercontent.com and cache them locally.
"""

from __future__ import annotations

import json
import os
import urllib.request
from functools import cache
from pathlib import Path

SOURCE = "https://raw.githubusercontent.com/repoe-fork/repoe-fork.github.io/master"
FILES = ["mods", "base_items", "item_classes", "tags", "essences", "fossils", "crafting_bench_options"]


def data_dir() -> Path:
    return Path(os.environ.get("PONDER_DATA", Path.cwd() / ".cache" / "repoe"))


def fetch(dest: Path | None = None) -> str:
    """Download the data files and return the game version they were exported from."""
    dest = dest or data_dir()
    dest.mkdir(parents=True, exist_ok=True)
    version = _get(f"{SOURCE}/exported-version.txt").decode().strip()
    for name in FILES:
        (dest / f"{name}.json").write_bytes(_get(f"{SOURCE}/data/{name}.json"))
    (dest / "version.txt").write_text(version + "\n")
    return version


def available(dest: Path | None = None) -> bool:
    dest = dest or data_dir()
    return all((dest / f"{name}.json").is_file() for name in ("mods", "base_items"))


@cache
def load(name: str, dest: Path | None = None) -> dict | list:
    path = (dest or data_dir()) / f"{name}.json"
    if not path.is_file():
        raise FileNotFoundError(f"{path} missing; run `ponder data fetch` first")
    with path.open(encoding="utf-8") as f:
        return json.load(f)


def version(dest: Path | None = None) -> str | None:
    path = (dest or data_dir()) / "version.txt"
    return path.read_text().strip() if path.is_file() else None


def _get(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "ponder"})
    with urllib.request.urlopen(req, timeout=120) as resp:
        return resp.read()
