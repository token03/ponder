# ponder

Mine Path of Exile builds for profitable crafting candidates.

The plan, in stages:

1. **Mine** (here now): read Path of Building exports, match every rare's mod
   lines to exact mod ids, tiers and sources, and rank mod combinations that
   many builds share.
2. **Cost**: work out the cheapest way to craft each candidate and its
   expected cost (exact probability math from spawn weights; simulation only
   for full rerolls like chaos, fossils and Harvest).
3. **Price and margin**: compare expected crafting cost with what the item
   sells for.
4. **Explain**: an LLM layer that turns plans and numbers into plain advice.

## Setup

```sh
python3 -m venv .venv
.venv/bin/pip install -e '.[dev]'
.venv/bin/ponder data fetch      # ~60 MB of repoe-fork game data into .cache/repoe
```

Game data comes from [repoe-fork](https://github.com/repoe-fork/repoe), which
exports it from the game files each patch. Set `PONDER_DATA` to keep it
somewhere else.

## Use

Each source can be a pobb.in or pastebin link, a file holding a PoB code or
XML, or a PoB code pasted directly.

```sh
ponder items https://pobb.in/XXXX        # every rare, mod by mod
ponder mine build1.txt build2.txt ...    # mod combos shared across builds
```

`ponder items` output reads like:

```
[Helmet] Doom Crown / Hubris Circlet  ilvl 86  (shaper)
   P T2  natural  IncreasedLife8                   +121 to maximum Life
   P T2  natural  LocalIncreasedEnergyShieldPercent6 84% increased Energy Shield  F
   S     crafted  EinharMasterLightningResist2     +23% to Lightning Resistance
```

`P`/`S` is prefix/suffix, `T1` is the best tier that can roll on that base,
`F` marks a fractured mod, `+` marks two mods the game summed into one line
(e.g. a hybrid %ES prefix plus a plain %ES prefix) and `~N` means N other mods
fit the line equally well. Sources:

- `natural`: can roll on this base, with these influences
- `essence`: only from essences
- `veiled` / `delve`: unveiled mods and fossil-only mods
- `special`: a real mod that can't roll naturally here (wrong influence,
  incursion, cluster notables...)
- `crafted` / `legacy-crafted`: bench craft, current or no longer offered

Each item also lists the PoB item sets that use it (guide authors often name
them by stage: "Early game", "Mid game", "Endgame"). Lines no mod has, usually
the author's own notes inside an item, are printed with `#`; lines that look
like a mod but fit no roll range are printed with `?`.

`ponder mine` counts mod families (all tiers of a mod) per item class and
lists the combinations shared by the most builds. A combo is hidden when a
larger one is shared by exactly the same builds.

## Layout

```
src/ponder/
  pob.py    decode PoB codes, fetch share links, list items and slots
  item.py   parse PoB item text (header, influences, markers, variants)
  data.py   download and load repoe-fork JSON
  mods.py   match lines to mod ids, tiers and sources
  mine.py   rank shared mod combinations
  cli.py    the `ponder` command
tests/      unit tests; the mod tests run once data is fetched
tests/fixtures/real/  real PoB exports used as regression tests
```

## Build sources

pobb.in and pastebin links are fetched directly. Mobalytics sits behind a
Cloudflare bot check, so copy the PoB code from the page instead.

## Known gaps

- A summed line is only split when it is a hybrid plus a single mod; two
  single mods summed (e.g. natural plus crafted fire res) stay unmatched.
- Mining groups by exact mod family, so `+1 to Level of all Fire Skill Gems`
  and `... Lightning Skill Gems` count separately. Grouping by what a build
  actually treats as interchangeable comes later.
