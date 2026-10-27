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

## 2026-10-27: where solve time goes

Timings below are the solver alone ("best plan"), not the guide harness, which also loads game data and simulates the guide's own recipe on 500 real items.

The int/chaos boots (5 requirements, 22 classes, 29 operations) explore 101,755 states and 3.95 million outcomes; the best policy visits 7,213. Raising the action-cost ceiling used to rebuild the graph from scratch on each of five attempts (16 s of 22 s exploring). The graph now persists across attempts and only gains the actions a higher ceiling admits: 10.3 s, same cost. Values take 3.4 s, pruning 0.9 s.

Lazy expansion against a relaxed lens (LAO*) re-solved the whole graph each round: 107 rounds and 15 s for the relaxed bound on the boots, 43 s in all, while the bound still let 94% of states be expanded. Exploring everything is the default again; the relaxed bound is only the fallback when a craft exceeds the state limit (the dex-resist ring, the BV gloves, the minion wand need it).

Recombination is allowed from any item, not only right after a purchase: the purchase-only restriction cost 30-40% on the double-dot quiver, the redeemer ring and the physical weapon, confirmed on real items.

Of 44 active final-item solves, 22 take under 2 s (most under 0.5 s) and 13 take 2-28 s. The state count is mostly the goal itself: each requirement brings its serving class and its blocking lower tiers, plus the bench crafts and metamods the operations force.

Solving stage by stage along the guide's own stages (each stage's real finished items, at its cost, are all the next stage can buy) was tried and dropped. It is faster on some (archmage wand 13.8 s to 2.5 s, boots 10.8 s to 5.9 s) but slower and dearer on others (Hunter belt 6.1 s to 26.1 s and +15%, fire-dot quiver +38%, physical-dot wand +29%), and two guides lost their plans. Each stage still tracks every class the operations can force, so stage graphs stay at 10-25 thousand states.

The previous planner's millisecond cases were single stages of two or three requirements (131-276 states), with junk lumped into six counters and at most eight targets.
