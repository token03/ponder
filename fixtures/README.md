Fixed test data, kept separate from the replaceable runtime data in `.cache/`.

- `build_a.xml` and `build_b.xml` are hand-written PoB parser inputs using real mod templates.
- `real/` contains real build exports.
- `prices.json.gz` is a fixed price table.
- `solver/` contains complete crafting problems checked against the simulator.
- `crafting-guides.json` records sourced crafting goals, reference methods, coverage and a runtime budget.
- [Supplemental AMA recipes](../docs/crafting-ama-recipes.md) collect 19 core-crafting references from the 3.26–3.29 AMAs, filtered against the 46 sheet guides. They are human test references, not additional executable or validated fixtures.
- `expected/` contains parser, matcher and network protocol expectations. JSON and URLs are compared by their contents, independent of formatting.
- `repoe/` is a compressed, pinned game-data snapshot; its manifest records the source version and retained fields.

Routine tests require no downloaded data, live prices or network access. Missing fixtures fail tests instead of silently skipping coverage.
