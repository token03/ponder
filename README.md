# ponder

Mine Path of Exile builds for profitable crafting candidates.

The plan, in stages:

1. **Mine** (working): read Path of Building exports, match every rare's mod
   lines to exact mod ids, tiers and sources, and rank mod combinations that
   many builds share.
2. **Cost** (working for cluster jewels): find the cheapest crafting policy
   for a target and its expected cost. Next: essences, fossils, chaos and
   Harvest reforges for ordinary gear.
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
ponder mine builds/3.29.txt              # mod combos shared across builds
ponder cluster cost medium "area damage" "Vast Power" "Assert Dominance"
```

A `.txt` file of links (one per line, `#` comments) works anywhere a source
does; `builds/3.29.txt` is the current list.

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

## Crafting costs

`ponder cluster cost SIZE ENCHANT NOTABLE...` (add `--need N` for "any N of
these") prints the expected cost, the expected currency used, and the recipe:

```
medium cluster, Area Damage, ilvl 84: 2 of Vast Power, Assert Dominance
expected cost ~127.6c (prices: placeholder defaults), not counting the base
expected currency: 462.4 alt, 140.8 aug, 49.2 regal, 48.2 scour
recipe (most common situations first):
   31.3%  magic: 1 other prefix + 1 other suffix                       -> alt
   18.4%  magic: 1 other suffix                                        -> aug
    5.1%  magic: Vast Power + 1 other suffix                           -> regal
    2.6%  rare: Vast Power + 2 other suffix                            -> scour
```

How it works (`craft.py`): an item is reduced to which target mods it has
plus how many other mods sit on each side (notables counted apart, since
they share a cap). From each such state the next currency is chosen by policy
iteration, solving the expected costs exactly, so the printed recipe is the
cheapest one over alt / aug / regal / exalt / scour. `craft.simulate`
replays a recipe on real mods and the tests check the two agree.
`ponder mine` adds this cost to every cluster jewel combo it finds.
`ponder cluster enchants SIZE` and `ponder cluster notables SIZE ENCHANT`
list what can roll and how often.

Assumptions not in the game data, kept as constants in the code:

- magic items roll 1 or 2 mods with equal chance (`craft.MAGIC_TWO_MOD_CHANCE`)
- jewels hold 2 prefixes and 2 suffixes (`craft.JEWEL_AFFIX_LIMIT`)
- clusters hold at most 1 / 2 / 3 notables by size (`cluster.NOTABLE_CAP`)

Prices default to rough placeholders. `ponder prices fetch --league NAME`
pulls real ones from poe.ninja into `prices.json` (poe.ninja must be
reachable), or edit `prices.json` by hand.

## Layout

```
src/ponder/
  pob.py    decode PoB codes, fetch share links, list items and slots
  item.py   parse PoB item text (header, influences, markers, variants)
  data.py   download and load repoe-fork JSON
  mods.py   match lines to mod ids, tiers and sources
  mine.py   rank shared mod combinations
  craft.py  crafting states, policy iteration, simulation
  cluster.py  cluster jewel enchants, mod pools, costs
  prices.py currency prices
  cli.py    the `ponder` command
builds/     lists of build links per league
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
