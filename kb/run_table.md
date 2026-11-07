Which experiment runs are done / planned, with a pointer to the data. updated 2026-11-07
Rule: when a run finishes, change its cell to the count + keep the folder pointer; no results here (those are in experiments.md / findings.md).
Cell = number of finished runs (files <policy>_<features>_<N>.csv). Sequences per seed: a0.5 c1 | a0.5 c2 | a1.2 c1 | a1.2 c2.
Seed folders: 2037 = set 2 -> stat/run_a<α>_c<case>/ ; 2042..2045 -> stat/seed<S>/run_a<α>_c<case>/ ; sequences in servers_benchmark/sequences[/seed<S>]/.

## Part 1 - load balancing policies, no feature (5 seeds; all policies on 2037, key policies on the other 4)
Key policies = rr esce throttled2 mct mct_const_flat. Data committed up to 1d687a3 (seeds 2043-45).

| seed | policy | a0.5 c1 | a0.5 c2 | a1.2 c1 | a1.2 c2 | data |
|---|---|---|---|---|---|---|
| 2037 | rr | 1 | 2 | 1 | 2 | stat/run_a*_c*/rr_none_* (rr c1 round 2 dropped by user 11-02: times out) |
| 2037 | iwrr331 | 2 | - | 2 | - | stat/run_a*_c1/iwrr331_none_* (c1 only) |
| 2037 | esce | 3 | 3 | 3 | 3 | stat/run_a*_c*/esce_none_* |
| 2037 | throttled | 2 | 2 | 2 | 2 | stat/run_a*_c*/throttled_none_* |
| 2037 | throttled2 | 2 | 2 | 2 | 2 | stat/run_a*_c*/throttled2_none_* |
| 2037 | mct | 2 | 2 | 2 | 2 | stat/run_a*_c*/mct_none_* |
| 2037 | mct_lookup | 2 | 2 | 2 | 2 | stat/run_a*_c*/mct_lookup_none_* |
| 2037 | mct_const | 2 | 2 | 2 | 2 | stat/run_a*_c*/mct_const_none_* |
| 2037 | mct_const_flat | 2 | 2 | 2 | 2 | stat/run_a*_c*/mct_const_flat_none_* |
| 2037 | aco | 2 | 2 | 2 | 2 | stat/run_a*_c*/aco_none_* |
| 2037 | aco_const | 2 | 2 | 2 | 2 | stat/run_a*_c*/aco_const_none_* |
| 2042 | rr esce throttled2 mct mct_const_flat | 2 | 2 | 2 | 2 | stat/seed2042/run_a*_c*/ |
| 2043 | esce throttled2 mct mct_const_flat | 1 | 1 | 1 | 1 | stat/seed2043/run_a*_c*/ |
| 2043 | rr | 1 | 1 | 1 | 1 | stat/seed2043/run_a*_c*/rr_none_1 (c1 from batch_features1 11-06) |
| 2044 | esce throttled2 mct mct_const_flat | 1 | 1 | 1 | 1 | stat/seed2044/run_a*_c*/ |
| 2044 | rr | 1 | 1 | 1 | 1 | stat/seed2044/run_a*_c*/rr_none_1 (c1 from batch_features1 11-06) |
| 2045 | esce throttled2 mct mct_const_flat | 1 | 1 | 1 | 1 | stat/seed2045/run_a*_c*/ |
| 2045 | rr | 1 | 1 | 1 | 1 | stat/seed2045/run_a*_c*/rr_none_1 (c1 from batch_features1 11-06) |

## Part 2 - mct + ONE feature (what each feature gives on its own)
Baseline = mct_none of the same sequence (Part 1). Labels: S = segment cache 100 MiB lru; B = bin cache 50+50 MiB, threshold 6 s;
Alru / Alfu = asset cache 50 MB per site, lru / lfu; C = request coalescing (OBM_COALESCE=1); P = merged prefetch (OBM_PREFETCH=1, window 30 s, hold 30 s, segment cache OFF -> prefetched result only readable during the 30 s hold; C off -> real renders are not in the in-flight table, so a prefetch can duplicate one = known design limit). Files mct_<label>_<N>.csv. 1 run per cell.
Seeds: 2037 + 2042 DONE (batch_features1 11-06 03:10-10:31, all 32 cells = 1). Summary temp/analysis_v4/features_batch1.csv, findings FEATURES BATCH 1.
(decide after batch_features1; full 3 seeds x 4 features = 48 runs ~8.8 h).

| seed | feature | a0.5 c1 | a0.5 c2 | a1.2 c1 | a1.2 c2 | data |
|---|---|---|---|---|---|---|
| 2037 | S | 1 | 1 | 1 | 1 | stat/run_a*_c*/mct_S_* |
| 2037 | B | 1 | 1 | 1 | 1 | stat/run_a*_c*/mct_B_* |
| 2037 | Alru | 1 | 1 | 1 | 1 | stat/run_a*_c*/mct_Alru_* |
| 2037 | Alfu | 1 | 1 | 1 | 1 | stat/run_a*_c*/mct_Alfu_* |
| 2042 | S | 1 | 1 | 1 | 1 | stat/seed2042/run_a*_c*/mct_S_* |
| 2042 | B | 1 | 1 | 1 | 1 | stat/seed2042/run_a*_c*/mct_B_* |
| 2042 | Alru | 1 | 1 | 1 | 1 | stat/seed2042/run_a*_c*/mct_Alru_* |
| 2042 | Alfu | 1 (1 fail: Dana restart > 5 s, not cache) | 1 | 1 | 1 | stat/seed2042/run_a*_c*/mct_Alfu_* |
| 2037 | C | 0 | 0 | 0 | 0 | planned 11-07: stat/run_a*_c*/mct_C_* |
| 2037 | P | 0 | 0 | 0 | 0 | planned 11-07: stat/run_a*_c*/mct_P_* |
| 2042 | C | 0 | 0 | 0 | 0 | planned 11-07: stat/seed2042/run_a*_c*/mct_C_* |
| 2042 | P | 0 | 0 | 0 | 0 | planned 11-07: stat/seed2042/run_a*_c*/mct_P_* |
| 2043-2045 | - | STARTED 11-07 by user (other session): part 2 mct + one feature on seeds 2043-45 (6 features S B Alru Alfu C P x 12 seqs = 72 runs ~13 h; cheaper: drop B/Alru (lost to S/Alfu on 2037+2042) -> S Alfu C P = 48 runs ~8.8 h); add one row per seed x feature as runs finish, data stat/seed<S>/run_a*_c*/mct_<label>_1.csv | | | | |

## Part 3 - best policy + best feature set (LATER, very long)
Search over policy x feature combinations (ablation: features are independent switches, see todo COMPUTATIONAL FEATURES).
DEFERRED 11-07 (user moved to part 2 seeds 2043-45 first): prefetch vs cache = mct_S vs mct_SP (OBM_SEGMENT_CACHE_MB=100 OBM_PREFETCH=1 FEATURES=SP) on the same 8 seqs as Part 2 (2037 + 2042); S cells exist above, so 8 SP runs ~1.5 h. Optional P alone (8 more), SCP.
Not designed yet; size it from Part 2 (drop features that do nothing; e.g. 3 policies x 2^k feature sets x seeds).

## Tests (correctness / smoke only - NOT results)
| what | when | data |
|---|---|---|
| segment cache LRU smoke, c1 a0.5 set 2 mct, 100 req | 11-05 | temp/cache_smoke/run_a0.5_c1/ |
| bin cache smoke, c1 a0.5 set 2 mct, 50 req | 11-06 | temp/bin_smoke/run_a0.5_c1/ |
| bin cache cross-bin test, c1 rr custom seq (x2) | 11-06 | temp/bin_xbin/run_a0.5_c1/rr_B_{1,2}* |
| coalescing smoke + race (om only, C / CS) | 11-06 | temp/coalesce_smoke/, temp/coalesce_race/ |
| prefetch correctness, c1 mct, 50 req, P / CP / SP / SCP / SCPAlfu | 11-06 | temp/prefetch_test/run_a0.5_c1/ (check.py) |
| C/P break test (killed prefetch site etc.), c1 mct, P / CP / SP / SCP | 11-07 | temp/cp_break_test/run_a0.5_c1/ |
| asset cache correctness, c2 a0.5 set 2 rr, 50 req, 20 MB lfu | 11-06 | temp/asset_smoke/run_a0.5_c2/ |
