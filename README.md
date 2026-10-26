# ponder

Find profitable Path of Exile crafts. What ponder is and how it is built: [SPEC.md](SPEC.md).

The code is mid-rebuild toward that spec. Where the code and the spec disagree, the spec is the
target.

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

Runtime caches live under `.cache/`. Game exports come from
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
ponder ladder finish item.txt # finishes for a pasted item
ponder item item.txt          # price-check one item
ponder run https://pobb.in/XXXX
ponder prices show
```

## Checks

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
