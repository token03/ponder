# ponder

Mine Path of Exile builds for profitable crafting candidates.

The plan, in stages:

1. **Mine** (working): read Path of Building exports, match every rare's mod
   lines to exact mod ids, tiers and sources, and rank mod combinations that
   many builds share.
2. **Cost** (working): the cheapest crafting policy for a target and its
   expected cost, for cluster jewels and ordinary rares.
3. **Price and margin** (started): poe.ninja for currency and cluster bases,
   trade-site lookups for selling prices.
4. **Explain**: an LLM layer that turns plans and numbers into plain advice.

## Setup

Needs [uv](https://docs.astral.sh/uv/).

```sh
uv sync                      # makes .venv, installs ponder + pytest
uv run ponder data fetch     # ~60 MB of repoe-fork game data into .cache/repoe
uv run pytest                # run the tests
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

```sh
ponder prices fetch                      # current league prices from poe.ninja
ponder cluster cost large "cold damage" "Prismatic Heart" "Widespread Destruction" "Blast-Freeze"
ponder cluster cost large "cold damage" ... --fractured "Prismatic Heart"   # start from a fractured base
ponder craft https://pobb.in/XXXX --slot Helmet --settle 1                 # any rare in a build
```

Each prints the expected cost, the expected currency used, and the recipe:
the action to take in each situation you'll actually meet. `cluster cost`
also prints the poe.ninja base price, a trade-site search link, and with
`--trade` the market price and margin.

### How the solver works (`craft.py`)

An item is reduced to a small state: its rarity, whether each target mod is
missing / on the item / *blocked* (a worse tier or an exclusive mod of the
same group is there), and how many other mods sit on each side. Every
currency maps a state to a distribution over states. Policy iteration then
picks, for every state, the action with the lowest expected cost **to
finish**, solving those expectations exactly. That is full lookahead, not a
greedy rule: it will regal into a rare, or exalt, or annul, whenever that is
cheaper over the whole remaining craft. `craft.simulate` replays a recipe on
real mods with exact group exclusion; the tests check the two agree.

Actions, built from repoe data and costed in poe.ninja prices:

- transmute, alteration, augmentation, regal, exalted, annulment
- alchemy, chaos, scour + alchemy, scour + transmute
- Harvest "reforge with a <tag> modifier" for every tag the pool has
- essences (Shrieking and Deafening) for the item class, with a scour on rares
- single fossils with a Primitive Chaotic Resonator (gear only)
- metamods: "Prefixes/Suffixes Cannot Be Changed" then scour, which keeps one
  side and clears the other (gear only; the metamod needs a free slot on the
  side it takes)

Crafted mods on a PoB item that the bench still offers become the last step:
the plan must finish with a free slot on that side, and the bench cost (plus
"Can have up to 3 Crafted Modifiers" when there are two) is added on top.

Item level filters the mod pool. When a PoB item has no item level, it is
raised from 84 to whatever its mods need (T1 life or movement speed need 86).
Fractured mods (`--fractured`, or read from a PoB item) stay through every
reroll and can't be annulled. Actions with no price are left out rather than
counted as free.

Not modelled yet: bench crafts used mid-craft (e.g. to block a mod group
before an exalt), "Cannot roll Attack/Caster Modifiers", multi-fossil combos,
Eldritch currency, influence orbs, Awakener, Recombinator, veiled mods, and
Harvest's newer crafts. `ponder craft` lists the mods on an item it doesn't
try to make (veiled, legacy-crafted and special mods).

### Assumptions not in the game data

Kept as constants so they're easy to correct:

- magic items roll 1 or 2 mods with equal chance (`craft.MAGIC_TWO_MOD_CHANCE`)
- rare mod counts: gear 4/5/6 at 9:3:1 (a 1,000-chaos sample), jewels 3/4 at
  3:1 (`craft.RARE_MOD_COUNTS`)
- jewels hold 2 prefixes and 2 suffixes; clusters hold 1 / 2 / 3 notables
- Harvest reforge costs: fire, cold and lightning are sourced; the other tags
  are guesses priced at the dearest lifeforce (`actions.HARVEST`)

### Prices and the trade site

`ponder prices fetch` caches currency, essence, fossil, resonator and cluster
base prices under `.cache/ninja`; without it a built-in snapshot is used.
The trade API (the one Awakened PoE Trade uses) refuses some networks; set
`POESESSID` to your pathofexile.com session cookie if `--trade` gets a 403.
The printed search link always works in a browser.

## Layout

```
src/ponder/
  pob.py    decode PoB codes, fetch share links, list items and slots
  item.py   parse PoB item text (header, influences, markers, variants)
  data.py   download and load repoe-fork JSON
  mods.py   match lines to mod ids, tiers and sources
  mine.py   rank shared mod combinations
  craft.py  crafting states, policy iteration, simulation
  actions.py currencies, Harvest, essences, fossils, bench and metamods
  cluster.py cluster jewel enchants, mod pools, costs
  gear.py   targets from a PoB rare (tiers, fractures, essence and bench mods)
  prices.py poe.ninja prices
  trade.py  trade-site queries
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
