# goldens

Frozen reference answers: solver plans, matched items, problems, recombinator trees,
pipeline runs and trade queries, all recorded from ponder's original implementation on
fixed inputs. The tests compare against them (`*_matches_python*` in the test names). They
are not regenerated: when a change is meant to alter an answer, update the test's
expectation and say why in the commit.

Most of the comparisons need the repoe data (`ponder data fetch`). The ladder comparisons
also need the cached ladder in `.cache/ladder`. Without those, the tests skip.
