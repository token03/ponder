Pinned RePoE test data. All records and their file order are retained; files are projected to the fields Ponder reads and gzip compressed. `manifest.json` records the game version, original source hashes, record counts and retained fields.

Tests use this directory explicitly. They never download data or read the mutable runtime database in `data/repoe`. Changes to game data should update these snapshots deliberately, alongside any affected expectations.

The manifest defines which fields to retain when updating this snapshot from a downloaded database; update it when Ponder starts reading another field.
