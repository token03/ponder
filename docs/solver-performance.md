# Solver performance on overhaul

Measured in release mode with four Rayon workers on 2026-10-26. Runs use an 8 GB virtual-memory limit, a 120-second wall limit, and at most 200,000 explored states per solve.

`ponder_solve::Solution` retains the states reachable under its selected policy. `explored`, `actions`, and `outcomes` report the full search work, including discarded alternatives. The state limit still applies during expansion, before compaction.

`ponder_plan::session::Session` reuses solved requests within one immutable game, assumptions, and market context. Its key includes the item kind, complete target, ordered methods, and the names, exact price bits, and raw item samples of every start and recombination partner. Changing the available methods or an item detail causes a separate solve. Plans retain their own lens and operations; abstract states are never reused with a different lens. Expansion caches are cleared before retaining a plan.

The existing state limit also bounds session retention: cached policy states plus raw source samples must fit that limit. The session evicts the least recently used plans when necessary. Callers can keep returned plans alive independently. This bounds cache growth without another tuning parameter; the virtual-memory guard remains the byte-level limit.

The session is an API in the overhaul planner. The existing CLI and app still use the older planner. The guide harness calls the solver directly and reports both explored and retained states.

The batch benchmark constructs 1,000 requests for the axe guide's flat-physical donor, with base prices from 5.000c to 5.999c. All share one target and method set, but all 1,000 source prices differ, so the cold run performs 1,000 solves. It then repeats those same requests in the same session:

- Cold solves: 23.378 seconds.
- Warm lookups: 0.003 seconds.
- Peak process RSS, including all retained plans: 67.3 MiB.
- Explored states across cold solves: 656,000.
- Retained policy states across all plans: 23,000.

This measures reuse of known requests and retention of many solved donor plans. It does not establish throughput for 1,000 different final-item goals, disk-cache performance, or the cost of simulating 1,000 policies. Fixture prices are unchanged.

Compile once before measuring, so compiler memory does not dominate the reported peak:

```sh
cargo test --release -p ponder-plan --test session --no-run
(ulimit -v 8000000; RAYON_NUM_THREADS=4 timeout 120 \
  /usr/bin/time -v cargo test --release -p ponder-plan --test session \
  thousand_axe_donors -- --ignored --nocapture)
```

The full physical axe still explores 40,233 states and 23,507,449 outcomes. It takes 16.525 seconds and about 630 MiB peak RSS, retaining 11,451 policy states afterward. Its optimum remains 2,278.6c; 500 real-item runs average 2,171.7c ± 66.3c with zero lost runs. Compaction reduces retained memory, while the one-time expansion peak remains.

The fractured regalia retains 309 of 24,585 explored states. It solves in 1.209 seconds at 79.5c; real-item runs average 78.7c ± 1.5c with zero lost runs.

An action-lazy experiment used immediate action cost as an optimistic bound and expanded the actions selected by the partial graph. It preserved the checked costs and reduced the axe intermediate from 23,541 to roughly 13,700 explored states, and from 3.4 million to roughly 695,000 outcomes. Repeated graph work outweighed those savings. Policy reuse and Bellman updates reduced that overhead but still took 1.659 seconds for the fractured axe, against approximately 0.2 seconds eagerly. The full axe was stopped after more than 80 seconds in its final solve. The experiment was removed; no search-mode switch or tuning parameters were added.

Cold final-item solving remains the bottleneck. The minion wand's known 200,000-state failure and the final regalia's timeout are not resolved by retaining compact policies or memoizing completed requests.
