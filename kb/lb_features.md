One card per LB policy and per computational feature: how it works, control vars, code, data, tests, findings, analysis tools, limits. updated 2026-11-07
USE: grep -n "^## " kb/lb_features.md -> jump to the card. Cards are SUMMARIES + POINTERS; details live in code.md / env.md / findings.md / experiments.md / run_table.md (named per card).
RULE: new policy / feature, or a new run / finding / limit for one -> update its card in the same step (status line + pointer), and the detail file as usual.
Code paths drift: refer to file + function names, not line numbers. Status says what is TRUE NOW (commit, image, runs).

# 0. Shared pieces

## 0.1 How a request flows (main server :7000, rust_server/src/main_server.rs serve_offload)
seq_bench client -> main server: (P observe -> maybe spawn prefetch) -> segment_lookup: segment cache HIT (S/B) | FOLLOW an in-flight render (C real leader / P prefetch) | LEAD -> LB policy select(_wait) -> offload site (container, offload_server.rs FIFO -> Dana :9009; site asset fetches go to main via the site's asset cache A) -> reply -> cache add (S/B) -> followers woken (C/P) -> client.
LB never sees cache hits or followers. Requests that reach the LB = requests - hits - followers + prefetch issues.

## 0.2 LB framework
- rust_obm/src/mod_load_balancer/: base_balancer.rs (LoadBalancer, LoadBalancerPolicy trait {Ticket, select, complete}, select_wait for throttled, SelectionGuard, TimesFeedback from X-Dana-Timings), mod.rs POLICIES (11 names).
- Policy chosen by main_server's 2nd arg (`main_server <cluster_csv> <policy>`, match in main()); builders for model-based ones in rust_server/src/lib/lib_main_state.rs (*_balancer).
- Sites: servers_container/cluster_endpoint_c{1,2}.csv. c1 = heterogeneous (gpu / llvmpipe / cpu per machine), c2 = 2 gpu per machine (near-homogeneous). Machines om (main + client, 30 ms netem) and rs (60 ms). cluster.md.
- Run one: `temp/run_driver.sh <case> "<alphas>" "<policies>"` (FEATURES=none default) -> stat/[seed<S>/]run_a<α>_c<case>/<policy>_none_<N>.csv + _main.log + _site_*.log; driver runs temp/lb_log_check.py. Batches: batch_run.md.
- Results tables: run_table.md Part 1 (which runs exist). Headline: findings.md 'FINAL LB RESULT' + 'FIVE-SEED RANKING'.

## 0.3 Feature framework (user RULE 11-04: every feature independently switchable)
- Code: rust_obm/src/mod_feature/ (base_cache.rs PolicyCache Lru|Lfu, segment_cache.rs, bin_cache.rs, asset_cache.rs, coalescing.rs, merged_prefetch.rs) + rust_obm/src/obm_parse.rs (path parse for P) + rust_server/src/lib/lib_segment.rs (lookup / render / cache add / prefetch_task) + lib_main_state.rs (ServerState, *_loader per feature = env parsing).
- Switches = env vars (all in env.md, defaults in servers_container/env_setting.txt = all OFF). Driver label FEATURES=<label> -> file <policy>_<label>_<N>.csv; run_driver REFUSES a feature var with FEATURES=none (mislabel guard).
- Labels (11-07): Slru / Slfu = segment cache single + its policy, B6 / B8 = bin cache + threshold s, Alru / Alfu = asset cache, C coalescing, P prefetch; combos concatenate (e.g. CSlru, SlruP, SlruCPAlfu; older test dirs use bare S / B / SP / SCP). Old Part 2 files mct_S_1 / mct_B_1 were renamed to mct_Slru_1 / mct_B6_1 on 11-07 (contents unchanged). temp/cache_log_check.py reads the label to know what must be ON.
- Example: `OBM_COALESCE=1 FEATURES=C SEQ_DIR=servers_benchmark/sequences/seed2042 RUN_ROOT=stat/seed2042 temp/run_driver.sh 1 0.5 mct`. Batch template with feature_env(): temp/batch_features1.sh.
- Main image must contain the code: rebuild = `cargo build --release --bin main_server && docker build -f servers_container/main_server/dockerfile -t obm-main-server .`. Site features (A) need the offload image (run_all_offload.py rebuilds) and rs needs git pull of the commit.
- Results: run_table.md Part 2 (mct + one feature, 1 run per cell); findings.md 'FEATURES BATCH 1', 'FEATURE DETAIL', 'WHY NO CLEAR STRONGEST FEATURE'.
- CSV markers: worker 'cache' (segment hit), 'coalesce' (C follower), 'prefetch' (follower of a prefetch); their timing cols are 0. Log tags: main [CACHE] [COALESCE] [PREFETCH] [OFFLOAD]; site [ASSET].

# 1. LB policies
Colours / order in plots: plot.md 'Style rules'. Part 1 key policies (all 5 seeds): rr esce throttled2 mct mct_const_flat. Others: seed 2037 only.

## rr — round robin
- Status: original policy; all 5 seeds (c1 on 2043-45 from batch_features1). Key policy (baseline).
- How: atomic counter % n sites. Ignores load and speed.
- Control: policy arg `rr`; no env.
- Code: rust_obm/src/mod_load_balancer/policy_robin.rs.
- Data: stat/[seed<S>/]run_a*_c*/rr_none_*.csv (2037 c1 1 run: round 2 dropped, times out).
- Findings: findings 'FINAL LB RESULT': best/near-best p50 (c1 8/10) but by far worst tail (c1 p99 385-577 s, 504s on 2042; c2 worst p99 10/10). 'c1 rr vs iwrr' (old era).
- Tools: stat/latency_plot.py (cut bar / --exclude rr), temp/seed_rank.py, temp/case_wins_plot.py.

## iwrr (iwrr331) — interleaved weighted round robin
- Status: OLD, c1 seed 2037 only (2 runs); not a key policy.
- How: weights by site config gpu 3 : llvmpipe 3 : cpu 1, interleaved.
- Control: policy arg `iwrr` (label iwrr331 in file names; 5:5:1 variant was tried and reverted, experiments 'iwrr weights').
- Code: policy_iwrr.rs.
- Data: stat/run_a*_c1/iwrr331_none_*; older stat/old_nonfifo/.
- Findings: findings 'c1 rr vs iwrr' [old gate era]; experiments 'iwrr weights'.

## esce — least connections ("Equally Spread Current Execution")
- Status: key policy, all 5 seeds.
- How: site with fewest requests in flight; ties -> lowest index (nginx least_conn). Speed-blind.
- Control: policy arg `esce`.
- Code: policy_esce.rs.
- Data: stat/[seed<S>/]run_a*_c*/esce_none_*.
- Findings: 'FINAL LB RESULT': short middle, long tail (c1 p50 below throttled2 10/10, worst p99 of informed policies 9/10, 72-200 s). Used as load check reference (busy 0.80-0.84).

## throttled / throttled2 — central queue with per-site concurrency cap
- Status: throttled (limit 1) seed 2037 only; throttled2 (limit 2) key policy, all 5 seeds. Code cdf7ddc.
- How: Throttled<const LIMIT>: free-token deque (each site LIMIT tokens) + FIFO waiter queue; request waits centrally (select_wait, X-OBM-Wait / CSV wait_ms) until a token is free; returned token to the back (ties rotate; set 1/2 limit-1 runs used old lowest-index rule). Late binding.
- Control: policy arg `throttled` / `throttled2`.
- Code: policy_throttled.rs (ThrottledLB = <1>, Throttled2LB = <2>).
- Data: stat/[seed<S>/]run_a*_c*/throttled2_none_*, stat/run_a*_c*/throttled_none_*.
- Findings: 'FINAL LB RESULT': longer middle, shorter tail vs esce (p99 lower 9/10); best non-MCT. limit 2 beats limit 1 by 2-4 s ('REPEATS' in findings CURRENT). 'WHY MCT LOSES SOME c2 SEQS' (late binding wins some bursts).
- Checks: temp/lb_log_check.py (FIFO order, max in flight per site <= LIMIT parsed from the type line).
- Limits: prefetches queue like real requests (no priority).

## mct — minimum completion time with cost model (MAIN POLICY)
- Status: key policy, all 5 seeds; base for every feature run (Part 2). Code 2d8ff06 + generic 55167bc.
- How: pick argmin over sites of makespan (sum of in-flight predicted costs) + network_f + machine_f x predict(path, site). Per-site EWMA (alpha 0.1): machine_f = t_work / pred (clamp 0.25..4), network_f = (net + asset) s. Priors om f 1.0 / n 1.5 s, rs 0.75 / 3.0 s. Predictor = stat_predict cost model (NNLS per stage x backend), frozen since 10-30.
- Control: policy arg `mct`; OBM_COST_COEF / OBM_COST_FEATURES (default /app/stat_predict/{coef,features}.csv in image); OBM_MCT_FALLBACK (1 = const work if model fails to load / unknown path; default off = pure MCT, unknown path panics that request).
- Code: policy_mct.rs (MCT<F>, MctLB), rust_obm/src/cost_predict.rs (CostPredictor), lib_main_state.rs mct_balancer / load_cost_model; model training stat_predict/ (features.py, model.py, evaluate.py, export.py; README).
- Data: stat/[seed<S>/]run_a*_c*/mct_none_*; log lines 'mct select ... pick= est=' / 'mct done ... f= n=' in _main.log.
- Findings: 'FINAL LB RESULT' / 'FIVE-SEED RANKING': c1 best mean + p99 10/10, beats no-model flat 10/10; c2 tie. Prediction error median 8% / p90 21% (CURRENT). 'Can anything beat MCT?' (clairvoyant -3..-4 s; SJF better mean, worse p99). 'MCT worker LEARNING rate' (L irrelevant). Offline bracket (CURRENT 'Offline best case'). 'WHY MCT LOSES SOME c2 SEQS'.
- Checks / tools: temp/lb_log_check.py (pick == argmin of logged est, fallback count); stat/latency_plot.py, latency_detail_plot.py, temp/seed_rank.py, temp/case_wins_plot.py, temp/aco_sim3.py (event sim), temp/offline_opt.py, temp/model_vs_real.py.
- Limits: no dead-site detection (re-picks a just-restarted site: looks idle -> 502, cp_break_test); early commit (no late binding).

## mct_lookup — mct with lookup-table predictor
- Status: seed 2037 only (2 runs). af6a449.
- How: predict = median work from stat_predict/lookup.csv keyed (site config, path), om-equivalent; miss -> cost model (logs 'mct_lookup miss').
- Control: policy arg `mct_lookup`; OBM_COST_LOOKUP (default /app/stat_predict/lookup.csv) + cost model vars for fill.
- Code: rust_obm/src/cost_lookup.rs, stat_predict/lookup_table.py, lib_main_state.rs mct_lookup_balancer.
- Data: stat/run_a*_c*/mct_lookup_none_*.
- Findings: 'REPEATS': slight edge over mct only c1 a0.5 (-1.0 s), else tie -> optional, not a key policy.

## mct_const / mct_const_flat — mct without a cost model (ablation)
- Status: mct_const seed 2037 only; mct_const_flat key policy, all 5 seeds.
- How: predict = constant CONST_WORK_S (5.59 s) for every path; per-site f/n EWMA still learns speed. _flat = no om/rs priors (all sites start f 1.0, n 1.0 s).
- Control: policy arg `mct_const` / `mct_const_flat`.
- Code: lib_main_state.rs mct_const_balancer / mct_const_flat_balancer (MCT::with_prior), CONST_WORK_S there.
- Data: stat/[seed<S>/]run_a*_c*/mct_const_flat_none_*, stat/run_a*_c*/mct_const_none_*.
- Findings: flat ~ mct_const (+-1 s) -> priors not needed (experiments 'mct_const_flat run'); mct beats flat 10/10 in c1 (gap 0.7-3.8 s) = the cost model matters ('FIVE-SEED RANKING').

## aco / aco_const — online ant colony
- Status: seed 2037 only (2 runs). DROPPED from further work (user 11-01: report as-is, 'not good in this workload').
- How: p(site) ~ tau[show][site]^alpha x (1 / MCT est_finish)^beta (alpha 1, beta 2); tau moves to reward = predicted / actual service; clamp [0.25, 4]. aco_const = with constant work.
- Control: policy arg `aco` / `aco_const`; OBM_ACO_SEED (2026); cost model vars.
- Code: policy_aco.rs, lib_main_state.rs aco_balancer / aco_const_balancer / aco_seed.
- Data: stat/run_a*_c*/aco{,_const}_none_*.
- Findings: findings CURRENT 'ACO parameters' (pheromone has nothing to learn; only beta matters; best ACO ~ MCT); 'REPEATS' (2.6-5.1 s behind mct in c1).
- Tools: temp/aco_params.py, aco_signal2.py, aco_sim.py, aco_sim2.py; lb_log_check (p / tau / reward).

# 2. Computational features (Part 2 = mct + ONE feature; Part 3 later = combos)

## Slru / Slfu — segment cache, single (main server)
- Status: committed fe7ddfd (LRU) + b3750a1 (PolicyCache, lfu option). Part 2 runs: Slru (100 MiB lru) on 2037 + 2042 all 4 seqs. Planned: Slfu (todo), seeds 2043-45.
- How: cache of rendered segments on the main server, key = path; HIT checked before the LB (LB never sees hits), served by sendfile (worker 'cache', X-OBM-Cache hit). Miss: add after render if 200 (awaited when a C/P leader exists, else detached). Eviction by PolicyCache rank: Lru (last_use) | Lfu (uses while cached, last_use).
- Control: OBM_SEGMENT_CACHE_MB (unset/0 = off; Part 2 = 100), OBM_SEGMENT_CACHE_POLICY (lru|lfu), OBM_SEGMENT_CACHE_DIR. Label Slru / Slfu (cache_log_check checks the policy in the label against the startup line).
- Code: rust_obm/src/mod_feature/base_cache.rs (PolicyCache), segment_cache.rs (SegmentCache::Single), rust_obm/src/file_cache.rs (CacheTable key->file); lib_main_state.rs segment_cache_loader; lib_segment.rs segment_lookup / offload_cache_add; main_server.rs serve_offload_cache. Design: code.md '[11-05]' entries.
- Data: stat/[seed2042/]run_a*_c*/mct_Slru_1.*; smoke temp/cache_smoke/ (label S, old).
- Findings: 'FEATURES BATCH 1' (-55% mean / -53% p99 vs none); 'FEATURE DETAIL' (hit 20% req / 17% bytes, queue -72%); 'CACHE SIZE CHOICE' (why 100 MiB); 'WHY SEGMENT CACHE HITS LESS AT a1.2' (eviction, long f1_full reuse); 'SEGMENT CACHE LRU vs LFU SIM' (lfu +4.3 pts, 8/8); 'SITE CONFIG VIDEO CHECK' (renders from different configs interchangeable).
- Checks / tools: temp/cache_log_check.py (label vs startup line, [CACHE] outcome counts, LB select count); stat/feature_plot/segment_plot.py (Slru / Slfu / B6 / B8), feature_detail_plot.py, stat/latency_plot.py --features; sims temp/cache_size_sim.py, temp/seg_alpha_why/{rep,why}.py, temp/seg_lfu_sim/sim.py.
- Limits: capacity must stay << ~430 MB distinct per run or every policy ties; cold at each run start.

## B6 / B8 — bin (cost-partitioned) segment cache (paper A)
- Status: committed 4a07eb3. Part 2: B6 (50 + 50 MiB, threshold 6 s) on 2037 + 2042. Planned: B8 (threshold 8 s; sim +11% saved cost, todo).
- How: 2 LRU PolicyCaches (lower: cost < threshold, upper: >=), bin chosen once at insert; cost = (asset_ms + work_ms / render factor gpu 1, llvmpipe 1.33, cpu 4.6) / 1000 from X-Dana-Timings (independent of LB policy). Same feature as S (env sets both -> server exits).
- Control: OBM_BIN_CACHE_LOWER_MB, OBM_BIN_CACHE_UPPER_MB, OBM_BIN_CACHE_THRESHOLD_S (6), OBM_BIN_CACHE_DIR. Label B<threshold> (B6, B8; cache_log_check checks it against the startup line).
- Code: mod_feature/bin_cache.rs (BinCache, cost_s), segment_cache.rs (SegmentCache::Bin), lib_main_state.rs segment_cache_loader. Design: code.md 'BIN COST SIMPLE', 'BIN COST DATA', '[11-06] bin_cache.rs'.
- Data: stat/[seed2042/]run_a*_c*/mct_B6_1.*; tests temp/bin_smoke/, temp/bin_xbin/ (label B, old).
- Findings: 'WHY BIN CACHE (B) ONLY TIES LRU (S)' (threshold splits traffic ~half, small cost skew, a1.2 reuse > 50 MB; sweep thr8 50/50 +11% saved cost, fragile); 'FEATURES BATCH 1' (S beats B 6/8).
- Tools: segment_plot.py (bin hits [lower, upper]), cache_log_check (B<thr> label, ' cost N.NN s'), temp/bin_why/{sim,sweep}.py.

## Alru / Alfu — asset cache (per offload site)
- Status: committed b3750a1. Part 2: Alru + Alfu (50 MB per site) on 2037 + 2042.
- How: in each site's offload_server, cache of asset files Dana fetches from main (/assets, /shows): hit -> sendfile; miss -> stream to Dana + buffer, add if 200. Policy lru | lfu. Sites restart each run = cold.
- Control (SITE env, not main): OBM_ASSET_CACHE_MB (0 = off; Part 2 = 50), OBM_ASSET_CACHE_POLICY (lru|lfu; env_setting.txt sets lfu), OBM_ASSET_CACHE_DIR. om: shell -> run_all_offload.py -> run_sh.sh -e; rs: cluster_lib start_case ssh env. Labels Alru / Alfu.
- Code: mod_feature/asset_cache.rs (= PolicyCache alias), rust_server/src/offload_server.rs serve_main_proxy_cached, rust_server/src/lib/lib_server_offload.rs asset_cache_loader; site log [ASSET] hit / added / add failed / skip.
- Data: stat/[seed2042/]run_a*_c*/mct_A{lru,lfu}_1.* + _site_*.log; smoke temp/asset_smoke/.
- Findings: 'FEATURES BATCH 1' (Alfu -51%, Alru -47%; Alfu < Alru 7/8); 'FEATURE DETAIL' (file hit 56/63%, byte hit 27-28%, asset+net -51..-56%, rs gains 2x more); 'ASSET CACHE POLICY' (sim, model under-counts real fetches ~4x); asset time ~50 RTs (CURRENT).
- Tools: stat/feature_plot/asset_plot.py, feature_detail_plot.py (per-site asset share), cache_log_check (site startup + [ASSET] counts), temp/asset_cache_sim.py.
- Limits: asset fetch looks sequential in Dana (concurrent fetch = other idea, todo Later).

## C — request coalescing (main server, in-flight table)
- Status: committed e0ba1c1 (+ gap fix and fix 1 in 7c5b8d9). Tested only (coalesce_smoke, coalesce_race, prefetch_test CP, cp_break_test CP). NO Part 2 run yet: planned mct_C on 2037 + 2042 (run_table, todo NOW).
- How: Coalescer = map path -> slot; first request = Leader (renders via LB), identical requests while in flight = Followers (wait, then get the leader's bytes; worker 'coalesce', header X-OBM-Coalesce wait ms, timings 0). Leader failure -> followers 502 (no fallback, user). With S on: cache add is awaited BEFORE finish (gap fix).
- Control: OBM_COALESCE=1 (0/unset off; other value = exit). Label C.
- Code: mod_feature/coalescing.rs (Coalescer / Leader / Follower / Role, Slot{cell, joined, speculative}); lib_main_state.rs coalesce_loader, ServerState.coalescer + coalesce_real; lib_segment.rs segment_lookup (Follow / Lead); main_server.rs serve_offload_coalesce; log [COALESCE] add | N followers / hit | wait / skip / leader failed. Design: code.md '[11-06 DESIGN TALK]', '[11-06] Coalescer', 'COALESCING WIRED'.
- Data: tests temp/coalesce_smoke/, temp/coalesce_race/ (om only, c2), temp/prefetch_test/, temp/cp_break_test/.
- Findings: 'COALESCING POTENTIAL' (3.9-7.8% of requests in flight at arrival without cache; 1-5% left with S -> small add-on to S, bigger alone).
- Checks / tools: temp/coalesce_smoke/check.py, temp/coalesce_race/check.py (per-request replay), cache_log_check (followers subtracted from LB selects), feature_detail_plot (NON_SITE 'coalesce'). Missing: a C/P per-feature plot (todo).
- Limits: latent Drop-order race in Leader::drop (only with >1 executor thread; SMOL_THREADS unset = 1); no cache re-check after becoming leader.

## P — merged prefetch ('2 continue' predictor, main server)
- Status: committed 7c5b8d9; main image 11-07 00:27 = that code. Tested only (temp/prefetch_test 5 modes, temp/cp_break_test). NO Part 2 run yet: planned mct_P on 2037 + 2042; S vs SP comparison DEFERRED (run_table Part 3 note).
- How: observe every request; if segment k-1 and k of the same show + combo seen within WINDOW s -> issue prefetch of k+1 (skip if cached / in flight / last segment / unknown show). Prefetch = speculative leader in the C table (built even with C off); real requests for it follow (worker 'prefetch'). After publish it is HELD hold s (readable); at hold end added to the segment cache ONLY if read (joined > 0) and S/B on. Failed prefetch -> its followers 'lookup again'.
- P alone (Part 2): segment cache off -> prefetched bytes only readable in flight / during the 30 s hold; C off -> real renders not in the table, so a prefetch can duplicate one (design).
- Control: OBM_PREFETCH=1, OBM_PREFETCH_WINDOW_S (30), OBM_PREFETCH_HOLD_S (30). Label P (combos SP, CP, SCP).
- Code: mod_feature/merged_prefetch.rs (MergedPrefetch::observe), rust_obm/src/obm_parse.rs (SegmentPath parse / next_path; show durations from servers_benchmark/video_duration.csv), coalescing.rs (lead_speculative, try_follow, publish, joined), lib_segment.rs prefetch_task, lib_main_state.rs prefetch_loader. Log [PREFETCH] issue / skip: cached|in flight / done / failed / expire | N readers | cache added|cache add failed|not read|cache off / hit | in flight|held / leader failed, lookup again. Design: prediction.md (workload facts, user design, decisions, sim, research framing); code.md '[11-06] MERGED PREFETCH WIRED', '[11-07] C/P REVIEW' + FIXES.
- Data: tests temp/prefetch_test/run_a0.5_c1/, temp/cp_break_test/run_a0.5_c1/.
- Findings: prediction.md 'SIM temp/prefetch_sim2.py' (S 13.9 -> SP 11.3 s mean, +12.5% renders, no load feedback = optimistic); prefetch_sim.py rule coverage ('1 obs' 57-61% / '2 obs' 37-39% covered, ~67% precise); 'Review bug #4 sized' (<= 0.15 s, conservative).
- Checks / tools: temp/prefetch_test/check.py + temp/cp_break_test/check.py (sequence-specific; I3 lifecycle / I4 / I5 reusable), cache_log_check (prefetch issues + 'expire ... cache add(ed| failed)'), feature_detail_plot (NON_SITE 'prefetch'; follower rows shown as 0 s parts - decide). Missing: temp/prefetch_log_check.py for replay runs (must handle evictions at 100 MiB), C/P plot (todo).
- Limits: #4 hold reads invisible to cache policy (sized small); #5 joined() race at hold end (tiny); no prefetch priority (throttled queues them); held bytes unbounded (rate x hold x size); renders 25-35 s under c1 load (timing assumptions).

## (not built) Partial segment caching
- Ignored (user 11-04: needs a new Dana API). todo 'COMPUTATIONAL FEATURES' item 5.

# 3. Analysis tools index (what each reads / shows; CLI = prints the plotted numbers as text)
Plot style + outputs: plot.md. Feature PNGs deleted 11-07 (user regenerates for slides). CLI rule (user 11-07): every plot tool must also print its numbers (plot.md); 'no' below = next task.
| tool | for | shows / checks | CLI numbers |
|---|---|---|---|
| stat/latency_plot.py `<run dir> [--features ..]` | policies, features | mean/p50/p90/p99 per policy (or policy+feature) | yes (table ± std) |
| stat/latency_detail_plot.py `<run dir>` | policies | stacked queue / wait / work / asset+net | yes (table) |
| stat/feature_plot/feature_detail_plot.py `<run dir>` | all features | stacked s/request per run type, hit ratios, per-site share | yes (tables) |
| stat/feature_plot/segment_plot.py / asset_plot.py `<run dir>` (feature_breakdown.py) | Slru,Slfu,B6,B8 / Alru,Alfu | % of no feature per part + hit line | yes (via feature_breakdown) |
| temp/seed_rank.py | policies | 5-seed rank table, Kendall W, mct-best counts | yes (text only, no plot) |
| temp/case_wins_plot.py `--case c1/c2` | policies | best / worst counts per statistic | yes (tables) |
| temp/seed_compare_plot.py | policies | seed 2037 vs 2042 mean/p99 | NO (prints only the path) |
| highlight/make_figures.py | README figures | headline, latency, breakdown, cost model, offline, workload | PARTLY (cost model error only) |
| servers_benchmark/sequences/plot_seq.py | workload | arrival timeline, show share | yes (summary) |
| temp/plot_tier_comparison.py | old tiers | old | old, ignore |
| temp/lb_log_check.py (driver runs it) | policies | mct argmin, aco p/tau, throttled FIFO + LIMIT | checker |
| temp/cache_log_check.py `<csv>` | S B A C P | label vs ON lines, outcome counts, LB select count | checker |
| temp/coalesce_{smoke,race}/check.py, temp/prefetch_test/check.py, temp/cp_break_test/check.py | C, P | per-test invariants (sequence-specific) | checker |
| temp/analyse_v4.py, analyse_load.py, analyse_net.py, speedup.py, model_vs_real.py | policies / load | per-site / load / network / om-vs-rs CSVs in temp/analysis_v4/ | csv + text |
| sims: temp/aco_sim{,2,3}.py, offline_opt.py, cache_size_sim.py, asset_cache_sim.py, bin_why/{sim,sweep}.py, seg_alpha_why/*.py, seg_lfu_sim/sim.py, prefetch_sim{,2}.py | per card above | trace / event replays behind the findings | text / csv |
