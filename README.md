# ponder

A stochastic model of Path of Exile crafting: item costs, recipes and profitable crafts. What ponder is and how it is built: [SPEC.md](docs/SPEC.md).

The code is mid-rebuild toward that spec. Where the code and the spec disagree, the spec is the
target.

Current measurements, correctness findings and remaining planner work are
summarized in [the research report](docs/RESEARCH.md).

## Layout

The application and guide harness use `ponder-plan`, the mechanics in
`ponder-rules`, and the shared `ponder-solve` engine. `ponder-game` handles
downloaded data, item parsing and matching. The legacy crafting model and its
separate route planners have been removed.

## Setup

Requires Rust 1.99 or newer.

```sh
cargo build
./target/debug/ponder data fetch
./target/debug/ponder prices fetch
./target/debug/ponder ladder fetch
cargo run -p ponder-app
cargo test --workspace
```

Downloaded game exports live under `data/repoe/` and ladder snapshots under
`data/ladder/`. These inputs survive clearing `.cache/`, which holds derived
plans, price responses, trade responses and temporary benchmark runs.
Existing downloads in `.cache/repoe/` and `.cache/ladder/` can be moved into the
corresponding `data/` directories without downloading them again.
Game exports come from
[repoe-fork](https://github.com/repoe-fork/repoe); `PONDER_DATA` selects another game-data
directory. Tests use the pinned snapshot in `fixtures/repoe` and never download it.
Trade uses `POESESSID` from the environment or an ignored `.env`.

Linux needs OpenGL, X11 or Wayland, and `libxkbcommon-x11`. On Windows install the Visual
Studio Build Tools (Desktop development with C++) and Rust's MSVC toolchain, and build from a
Windows-side checkout.

## Use

Every CLI command prints one JSON document; progress goes to stderr (`-q` hides it, `--pretty`
indents).

```sh
ponder ladder crafts          # rank craftable ladder items
ponder item item.txt          # price-check one item
ponder run https://pobb.in/XXXX
ponder prices show
```

## Checks

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Linux developer tools provide bounded benchmarks and policy inspection:

```sh
cargo build --release -p ponder-tools
cargo build --release -p ponder-plan --example ladder
target/release/ponder-tools bench --guide cheap-ele-bow --repeat 3 --out .cache/benchmarks/bow-baseline
target/release/ponder-tools inspect --guide ama-327-fire-dot-spell-crit-sceptre
target/release/ponder-tools run --seconds 30 -- cargo test --release -p ponder-solve
```

`ponder-tools` defaults are 120 seconds per child, 8 GB address space per process, four workers,
and 200,000 solver states per attempt. Reports use Markdown, TSV and plain logs.
See [the developer tools reference](docs/RESEARCH.md#reusable-developer-tools) for
selection, comparisons, interpretation and limits.
