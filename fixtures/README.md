Fixed test data, kept separate from the replaceable runtime data in `.cache/`.

- `build_a.xml` and `build_b.xml` are hand-written PoB parser inputs using real mod templates.
- `real/` contains real build exports.
- `prices.json.gz` is a fixed price table.
- `solver/` contains complete crafting problems checked against the simulator.
- `crafting-guides.json` records sourced crafting goals, reference methods, coverage and a runtime budget.
- `guides/` contains the 46 sheet recipes and 20 source-linked AMA TOML recipes. The [AMA reference guide](../docs/crafting-ama-recipes.md) maps the 19 curated references to those fixtures, including the Ghastly Eye alternative. Representative base/level choices and supplied starting items are explicit; supplied fractures are priced at zero for marginal crafting-cost comparisons. Five jewel/cluster guides validate against the raw snapshot and remain inactive until their affix domains are implemented; the Horror ring awaits Glyphic. The separate AMA recipe test compares exact recipe costs and real-item samples without requiring the best-plan search to finish.
- `expected/` contains parser, matcher and network protocol expectations. JSON and URLs are compared by their contents, independent of formatting.
- `repoe/` is a compressed, pinned game-data snapshot; its manifest records the source version and retained fields.

Routine tests require no downloaded data, live prices or network access. Missing fixtures fail tests instead of silently skipping coverage.
