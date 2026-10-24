Pinned RePoE test data. All records and their file order are retained; files are projected to the fields Ponder reads and gzip compressed. `manifest.json` records the game version, original source hashes, record counts and retained fields.

Tests use this directory explicitly. They never download data or read the mutable runtime database in `.cache/repoe`. Changes to game data should update these snapshots deliberately, alongside any affected expectations.

Regenerate from a downloaded database with `uv run scripts/pin-test-data.py .cache/repoe`. The manifest defines which fields to keep; update it when Ponder starts reading another field.
