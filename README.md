# ponder

Find Path of Exile 3.30 market holes: what ladder characters wear, the cheapest
way to craft it, and what the trade site has.

- `ponder` is a command line that prints JSON, for scripts and LLMs.
- `ponder-app` is a desktop app (egui) for browsing the same results.

Both sit on one Rust library.

## Setup

Needs Rust 1.99 or newer.

```sh
cargo build --release
./target/release/ponder data fetch        # ~60 MB of repoe-fork game data into .cache/repoe
./target/release/ponder prices fetch      # current league prices from poe.ninja into .cache/ninja
./target/release/ponder ladder fetch      # top characters of every popular skill and class, cached
cargo run --release -p ponder-app         # the desktop app
cargo test                                # the tests (some need the data and ladder caches)
```

`cargo install --path crates/ponder-cli` and `cargo install --path crates/ponder-app` put
`ponder` and `ponder-app` on your PATH. Run them from the project directory, or pass
`ponder-app` that directory: every cache lives under `.cache/` in the working directory.

Game data comes from [repoe-fork](https://github.com/repoe-fork/repoe), which exports it
from the game files each patch. Set `PONDER_DATA` to keep it somewhere else.

On Linux the app needs the usual desktop libraries: OpenGL, X11 or Wayland, and
`libxkbcommon-x11`.

### On Windows

Work from a checkout on the Windows side (say `C:\dev\ponder`), not one inside WSL: builds
through `\\wsl$` are slow and the app would run under WSLg.

1. Install the [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
   with "Desktop development with C++" (the MSVC linker and Windows SDK).
2. Install Rust with [rustup-init.exe](https://rustup.rs) and keep the default
   `x86_64-pc-windows-msvc` toolchain.
3. In PowerShell, in the checkout, run the same commands as above
   (`.\target\release\ponder.exe data fetch`, ..., `cargo run --release -p ponder-app`).

`target\release\ponder-app.exe` opens without a console window; a crash is written to
`ponder-crash.log` in the project directory. Pin it to the taskbar or make a shortcut whose
"Start in" is the project directory (or pass the directory as its argument). `.cache\` and
`.env` can be copied over from another checkout as they are.

To build the Windows binaries from WSL instead:
`rustup target add x86_64-pc-windows-gnu`, `sudo apt install gcc-mingw-w64-x86-64`, then
`cargo build --release --target x86_64-pc-windows-gnu -p ponder-app -p ponder-cli`.

## The desktop app

`ponder-app` runs the ladder pipeline in the background (about 5 s from scratch) and lays
the results out in tiles that you can drag, split, close and reopen from the View menu.
The layout, filters and run settings are remembered.

- **Crafts**: every costed product, ranked market holes first and then by how many
  characters wear it. Click any column to sort. ↑/↓ and PgUp/PgDn move through the table,
  and Ctrl+R runs the pipeline again.
- **Overview**: run totals, plus a plot of cheapest cost against demand. Click a point to
  select that craft.
- **Item**: the selected craft's mods (target, filler, bench), its costs and the expected
  currency. Buttons price it on trade, find half-done items for sale, cost a recombinator
  tree, or copy its JSON.
- **Recipe**: the plan as numbered steps. Each outcome links to the step it leads to.
- **Fractured bases**, **Products & starts**, **Recombinator**: the alternatives to
  crafting from scratch.
- **Finish an item**: paste an item copied in game (Ctrl+C) anywhere in the window to see
  what it can become and what finishing it costs.

The left panel holds the run settings (how many archetypes, niche combos and clusters to
cost, the chaos-per-hour charge, recombinators, trade) and the filters (text, gear or
clusters, base group, minimum demand, maximum cost, has-a-fracture, and more).

## The command line

Every command prints one JSON document on stdout. Progress goes to stderr (`-q` silences
it), errors come out as `{"error": ...}` with exit code 1, and `--pretty` indents the output.

```sh
ponder ladder fetch                          # top characters of every popular skill and class (poe.ninja), cached
ponder ladder archetypes --group Bow         # mod combinations many characters share = crafting candidates
ponder ladder tiers --group "Gloves (ES)"    # how often each mod is worn, and how much its tier matters
ponder ladder prices --group Bow --top 3     # fit what each tier is worth from trade listings
ponder ladder crafts                         # cost what the ladder wears (gear + clusters), rank market holes
ponder ladder crafts --trade --recomb        # ... priced on trade, with recombinator trees
ponder ladder crafts --starts 10 --top 20    # ... and half-done items for sale for the best 10
ponder ladder finish item.txt                # what an item you have can become, and the recipe from it
ponder run https://pobb.in/XXXX build.txt    # cost every rare in some builds (--fractures, --trade, --recomb)
ponder item item.txt                         # read an item and match its mods to mod ids and tiers
ponder prices show                           # the poe.ninja prices costs use
ponder data version
```

A source for `run` can be a pobb.in or pastebin link, a file holding a PoB code or XML, a
directory, or a `.txt` file of links (one per line, `#` comments). For example,
`builds/3.29.txt` is a list of build links. Mobalytics sits behind a Cloudflare bot check,
so copy the PoB code from that page instead.

### What the ladder commands do

`ladder fetch` reads poe.ninja's builds data: the top 10 characters for every main skill
with at least 40 players, and the top 20 of every ascendancy (about 2,500 characters).
Their items come with exact mod ids, so nothing is text-matched. Only what crafting reads
is cached, under `.cache/ladder/<snapshot>`: each character's class, main skill, and rare
items and jewels by mod id.

- **archetypes**: the mod combinations shared by the most characters, per base group.
  - Mods count by main stat, so a hybrid "%phys / accuracy" merges with plain "%phys"
    (flat and % ES stay apart), and resistances are one token.
  - A combination worn mostly as part of a bigger one (70% of its characters) is folded
    into it.
  - Each archetype lists the bases it's worn on and the skills that wear it.
- **tiers**: for each base group and mod, the share of ladder items at T1 (and T2 or
  better), and the *lift*: that share divided by the chance a roll lands there. A big lift
  means players pay for the tier (T1 life, T1 ES). A lift near 1× means any tier will do.
- **prices**: for the top archetypes, up to 50 trade listings each, with every core mod at
  T4 or better. A least-squares fit of log price on which core mods are T1 or T2, the
  number of resistances and the open affixes gives the multipliers: what T1 is worth per
  mod, in this market.
- **crafts**: turns the ladder into crafting candidates and costs them.
  - **Finished items.** Gear archetypes become finished items, the way their wearers'
    items are:
    - the archetype's mods, plus any other mod at least half of its items carry
      (`demand::COMPLETE_SHARE`), each at the tier most wearers reach;
    - then each side is filled to the number of useful mods its wearers usually have
      there, from *filler*: any mod at least 10% of them wear (`demand::FILLER_SHARE`;
      resistances always count), at the tier 80% of them have or beat;
    - one open slot is finished at the bench when the bench offers a filler mod for that
      side.
  - **Which archetypes.** The most worn overall (`--gear`), the top few of every base group
    (`--per-group`), and the most *niche* 3+ mod combos (`--niche`): mods worn together far
    more often than chance, which marks one build's item.
  - **Cluster jewels** are costed by size, enchant, usual passive count and notable
    combination. Passive-point jewels are costed by mod combination.
  - **Market holes.** With `--trade`, each craft gets a selling price and a listing count,
    and `hole = margin × wearers / (listed + 5)` ranks the holes: wanted, thinly supplied,
    profitable.

**The product tree.** A product is *under* another of the same base group when every mod
it wants is one the other makes at the same tier or better, with fewer wanted mods and no
more filler. For example, "life + block" sits under "life + block + recover on block".
Products are solved smallest first, and a bigger craft gets "sell as <smaller product>"
actions: an item that bricks late can be sold for 70% of that product's cheapest cost
(`SALE_SHARE`), and you start over. The recipe says when to cash out. If a sale would pay
more than making that item again, the solver plans without sales.

**Finishing a real item.** A plan already knows the expected cost to finish from every
state it reaches, and any other state is solved from where it is. So `ladder finish` takes
an item you dropped or half-crafted, lists every ladder product of its base group it can
become, and shows what finishing costs, what the product is worth and what that saves.
Crafted mods are ignored (the bench removes them), and fractured ones fix the craft.
**Half-done items for sale** work the same way: `--starts N` searches trade for rares of a
craft's bases that carry any of its wanted mods, finishes each listing from its own mods,
and keeps the cheapest totals of price plus finishing.

### Targets, filler and bases

A craft doesn't copy an item mod for mod.

- **Resistances are filler.** The item needs as many resistances as it had, any of them,
  at T3 or better, because buyers want enough res and rarely a particular one. That is what
  makes resistance-guaranteeing tools (Harvest reforges, essences, fossils) and side-only
  finishing (eldritch currency) worth using. `run --exact` turns this off.
- **Everything else stays a target** at its tier or better. Hybrid mods count as their main
  stat.
- **Base groups.** Each craft covers the group of bases that roll exactly the same mods:
  Warlock, Sage and Sorcerer Gloves are one "Gloves (ES)" craft. Crafts and trade searches
  use the group's best base, since the same mods sell for more on it.

## How the solver works (`crates/ponder-core`)

**States.** An item is reduced to a small state:
- its rarity;
- whether each target is missing, on the item, or *blocked* (a worse tier or an exclusive
  mod of the same group is there);
- how many other mods sit on each side.

**Policy iteration.** Every currency maps a state to a distribution over states. Policy
iteration picks, for every state, the action with the lowest expected cost **to finish**,
and solves those expectations exactly. That is full lookahead, not a greedy rule: it will
regal into a rare, exalt, or annul whenever that is cheaper over the whole remaining craft.
`simulate` replays a recipe on real mods, and the tests check that the two agree.

**Actions** are built from the repoe data and costed in poe.ninja prices:
- transmute, alteration, augmentation, regal, exalted, annulment, alchemy, chaos, and
  scour + alchemy / transmute;
- Harvest "reforge with a <tag> modifier";
- Shrieking and Deafening essences;
- fossil rerolls of 1-4 fossils in a resonator, with the best combo per subset of wanted
  mods kept;
- metamods ("Prefixes/Suffixes Cannot Be Changed" with a scour or a reroll);
- eldritch exalts and annulments on armour pieces without influence;
- bench crafts as the last step;
- self-made fractured bases: reroll until the mod lands with few enough mods, then use a
  Fracturing Orb.

**Time is charged.** Clicking time is charged at `PONDER_CHAOS_PER_HOUR` (default 1500;
set it in the environment or `.env`, and 0 turns it off), so plans don't call for ten
thousand alterations on a long shot.

**Plan cache.** Solved plans are cached in `.cache/plans`, keyed by the problem and by
the prices its actions use (to 3 significant figures).

**Recombinators** (`crates/ponder/src/recomb.rs`): clean donors, then recombinations,
cheapest first over every split of the wanted mods.
- The output takes either input's base, 50/50.
- Per side, how many mods the output keeps depends only on that side's pool size
  (`recomb::SIDE_COUNTS`; edit it when a patch changes the odds).
- Kept mods are drawn after the mods that can't roll on the output base are removed. That
  is NNN padding.

### Assumptions not in the game data

The constants below are kept so they're easy to correct. Everything that decides *what* to
craft is an input that you can swap for your own:

| What | Default | Where |
|---|---|---|
| Which mods a product wants | the `*_SHARE` constants and `MAX_KEYS` | `demand` |
| What a brick sells for | 70% (`SALE_SHARE`), at least `MIN_SALE` | `pipeline` |
| Resistances as filler | T3 or better | `gear::FILLER_FAMILIES`, `FILLER_TIER` |
| Magic items: 1 or 2 mods | equal chance | `MAGIC_TWO_MOD_CHANCE` |
| Rare gear: 4 / 5 / 6 mods | 9:3:1 | `RARE_MOD_COUNTS_GEAR` |
| Harvest costs for tags without a lifeforce source | priced at the dearest lifeforce | `actions::HARVEST` |
| Clicking time per action | — | `ponder_core::SECONDS` |
| Recombinator odds by pool size | — | `recomb::SIDE_COUNTS` |

## Prices and the trade site

`ponder prices fetch` caches currency, essence, fossil, resonator and cluster base prices
under `.cache/ninja`.

The trade API wants your pathofexile.com session cookie. Put `POESESSID=...` in a `.env`
in the project directory (it is git-ignored), or export it. It is a login token, so keep it
out of anything you share; ponder never prints it. A 403 with a valid cookie means the site
is blocking the network, and the printed search links still work in a browser.

The API allows only a few dozen searches per five minutes, so:
- every call waits as long as the API's own rate-limit headers say it must;
- answers are cached in `.cache/trade` for an hour (`PONDER_TRADE_TTL` seconds);
- `--trade-budget N` (default 40) caps the searches one run sends: finished items first,
  in priority order, then fractured bases.

## Layout

```
crates/
  ponder-core/  the solver: lumped item states, currency outcomes, policy iteration, recipes (no I/O)
  ponder-game/  repoe game data, item text and PoB parsing, matching mod lines to mod ids
  ponder-net/   poe.ninja ladder and prices, the trade site, share links, .env; every call cached
  ponder/       the library: actions, gear and cluster problems, demand, recombinators, the pipeline
  ponder-cli/   `ponder`: JSON command line
  ponder-app/   `ponder-app`: egui desktop app
goldens/        frozen reference answers the tests compare against (see goldens/README.md)
fixtures/       PoB exports and builds used by the tests
builds/         lists of build links
docs/           design notes
```

The library has no UI code. `ponder::pipeline::ladder` returns a `LadderReport`, and
`pipeline::run` returns a `Report`. Their `to_dict()` is the JSON the CLI prints.
