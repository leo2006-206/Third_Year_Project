Decisions the user made or agreed (ADR-lite): one line each, numbered, never deleted. updated 2026-11-07
RULE: a new user decision -> add a row in the same step (next number). A decision that changes an old one -> new row with 'replaces Dnn', and mark the old row 'REPLACED by Dmm' (don't delete it).
Columns: id | date | decision | why | source (detail). Only decisions with a source; my own recommendations the user has NOT agreed to do not go here.

## Scope + method
| id | date | decision | why | source |
|---|---|---|---|---|
| D01 | 10-29 | Dana / OBM core used UNMODIFIED; Rust app layer at "normal default" level (nginx-like: no keep-alive, buffering, no cache, rr). Don't over-optimise the baseline | features + policies are what is measured | user.md |
| D02 | 10-30 | GPU-native renderer is NOT the project direction (parked) | focus = cost prediction, LB policies, caching | user.md, findings 'GPU-native offload feasibility' |
| D03 | ~10-28 | Place policies between worst and best (oracle) case | supervisor's request | user.md, findings 'Offline best case' |
| D04 | 11-04 | Every computational feature independently switchable (env flag); later stage = find the best combination by ablation (MOST IMPORTANT rule) | clean per-feature attribution | todo 'COMPUTATIONAL FEATURES', lb_features 0.3 |
| D05 | 11-04 | Experiment plan in 3 parts: (1) LB policies no feature, (2) mct + ONE feature, (3) later policy x feature combos | separate LB effect from feature effect | run_table.md |

## Workload + cluster
| id | date | decision | why | source |
|---|---|---|---|---|
| D10 | 10-29..31 | Target load rho 0.8 for all cases (generator capacity from replay runs, window-mix fit); planned 0.7 was really ~0.95 | realistic queueing without collapse | experiments 'Load level decision', history 10-31 |
| D11 | 10-31 | Set 2 = seed 2037 (user pick from a 2000-2047 search); shared ramp kept | representative window, stable shape | history 10-31, handoff |
| D12 | 11-02 | 2nd seed (2042) over a 3rd repeat round of set 2 | one-sequence limitation > run noise | findings 'RUN-TO-RUN NOISE' |
| D13 | 11-05 | Seeds 2043, 2044, 2045 pre-declared (next three after 2042) BEFORE any run; 1 round each | no picking seeds by result | experiments 'SEEDS 2043-2045 SET' |
| D14 | 11-02 / 11-05 | rr on c1 dropped from repeats / seed batches (times out, ~19 min); later rr c1 run once for 2043-45 in batch_features1 | cost vs value (rr c1 always worst) | run_table Part 1, batch_run 'Timing' |
| D15 | 10-30 | Power profile change mid-batch recorded, runs NOT marked _e ("just record it") | data judged usable | experiments 'POWER PROFILE CHANGE' |

## LB policies
| id | date | decision | why | source |
|---|---|---|---|---|
| D20 | 10-30 | MCT tests = pure MCT: fallback OFF by default (OBM_MCT_FALLBACK=1 to enable) | measure MCT alone | code.md 'MCT fallback switch' |
| D21 | 10-30 | CONST_WORK_S = 5.59 s, work only (no asset), flat request-weighted mean | = expected next request | history 10-30 |
| D22 | 10-30 | Cost model frozen for set 2 and later (no retrain) | keep MCT identical across sets, no leakage | code.md 'Cost model status' |
| D23 | 10-31 | Throttled + ACO built instead of min-min / max-min | user's pick | todo (min-min line) |
| D24 | 11-01 | ACO NOT retuned (beta -> inf = MCT); report as-is ('not good in this workload') | pheromone has no signal | findings 'ACO parameters', lb_features aco |
| D25 | 11-02 | Lookup table keyed by (site config, path), NO machine (user correction) | machine handled by MCT factor | code.md 'Lookup-table work predictor' |
| D26 | 11-02 | Recommendation wording: MCT with model > MCT without model >= throttled2 > esce; lookup optional | repeats data | findings 'FINAL LATENCY SUMMARY' |
| D27 | 11-05 | Final LB claim = user-agreed wording in findings 'FINAL LB RESULT' (c1 mct best 10/10; c2 tie; not a strict 5-way order) | 5-seed check | findings 'FINAL LB RESULT', 'RANK CLAIM CHECK' |

## Features
| id | date | decision | why | source |
|---|---|---|---|---|
| D30 | 11-04 | Segment cache policy SIMPLE (LRU / LFU) or paper-A cost bins; GDSF / WGDSF NOT wanted | paper gains small / fragile | code.md 'USER DECISION 11-04' |
| D31 | 11-04 | Bin cost = measured work / machine const / config const, independent of LB policy, no cost model (later: asset_ms + work_ms / config) | cache must not depend on the LB | code.md 'BIN COST SIMPLE', 'BIN COST DATA' |
| D32 | 11-04 | Asset cache lives on the offload SITE, not the main server | asset round trips are per site | todo 'COMPUTATIONAL FEATURES' 2 |
| D33 | 11-04 | Partial segment caching IGNORED | needs a new Dana API | todo item 5, lb_features |
| D34 | 11-05 | Features do NOT share cache files (own CacheTable / subdir each) | independence (D04) | code.md '[11-05] base_cache.rs' |
| D35 | 11-06 | LRU and bins are the SAME feature (can't run together; both set = server exits) | one segment cache | code.md '[11-06] Bin cache wired' |
| D36 | 11-06 | Cache returns facts, main server prints the log (policies print inside only because their state is too big) | testable cache code | code.md '[11-06] COMMITTED 4a07eb3' |
| D37 | 11-06 | Sizes: segment cache 100 MiB (sweep 50/100/200 later), asset cache 50 MB per site ('hold everything' unrealistic; 80 too big) | strong eviction so policies differ | findings 'CACHE SIZE CHOICE', 'ASSET CACHE POLICY' |
| D38 | 11-06 | Single segment cache LFU real run agreed (single only; LFU bins not worth coding) | sim +4.3 pts hit | todo 'SEGMENT LFU', findings 'LRU vs LFU SIM' |
| D39 | 11-06 | Coalescing: leader failure -> followers 502 (no fallback, no panic) | simple, rare | code.md 'COALESCING WIRED' |
| D40 | 11-06 | Prefetch rule '2 continue' (seg k-1 + k of same show+combo within window -> k+1), no per-user tracking; designed to PAIR with the segment cache, not to win alone | user design | prediction.md 'User expectation', 'USER DESIGN' |
| D41 | 11-06 | Prefetched segment added to the cache AFTER the 30 s hold (user) and only if read (my add, agreed) | admission filter | prediction.md 'Decision 11-06' |
| D42 | 11-07 | Review bug #4 (hold reads invisible to cache policy) recorded, NOT fixed | sized <= 0.15 s, conservative | prediction.md 'Review bug #4 sized', handoff 22 |

## Code style + workflow
| id | date | decision | why | source |
|---|---|---|---|---|
| D50 | 10-29 | Commit only when asked; user pushes / pulls; never touch rs without asking; never stage local_host.txt / set_ssh.sh | user controls the repo + machines | user.md |
| D51 | 11-06 | HONEST vs DISHONEST code: keep I/O at a thin edge; tag doc comments [Honest] / [Dishonest I/O] | testable code | user.md |
| D52 | 10-30 | Policy dispatch stays a `match` in main_server (run_policy / WithBalancer reverted as too complex) | simplicity | code.md 'Policy Ticket + MCT' |
| D53 | 11-06 | main_server split: client flow in main_server.rs, segment logic lib_segment.rs, startup state lib_main_state.rs | file size / readability | code.md 'SPLIT', 'SPLIT 2' |
| D54 | 11-05 | New env var -> env.md row in the same step | reproducibility | env.md |
| D55 | 11-03 / 11-07 | Sessions per deliverable; long session -> suggest new session + handoff, KB audit first | context limits | user.md, INDEX.md |
| D56 | 11-07 | KB moved to repo root kb/ (temp/kb = symlink); lb_features.md cards + decisions.md added | findable, grouped facts | history 11-07 |
| D57 | 11-07 | Feature labels carry their setting: S -> Slru (/ Slfu), B -> B6 (/ B8); existing Part 2 files renamed (contents unchanged); all feature PNGs deleted, user regenerates during slide analysis | one label per config, no ambiguity once Slfu / B8 exist | lb_features 0.3, history 11-07 |

## Reporting + plots
| id | date | decision | why | source |
|---|---|---|---|---|
| D60 | 11-01..05 | Plot style rules (family colours, mean/p50/p90/p99, cut rr bar, badges, whole-number win counts) | consistent slides | plot.md 'Style rules' |
| D61 | 11-05 | Per-seed plots live with their data (stat/seed<S>/); no latency_detail for seeds | tidy | plot.md |
| D62 | 11-07 | Every plot tool also prints its numbers on stdout | AI can analyse without the PNG | plot.md 'CLI OUTPUT RULE' |
