# Handoff 22 — 2026-11-07 ~00:45 (PREFETCH built, reviewed, tested on c1, COMMITTED 7c5b8d9; session end)
- [11-07 update] USER SWITCHED PLAN: next = Part 2 mct + one feature incl. C and P (never run alone; 2037 + 2042 first, 16 runs) then seeds 2043-45 (run_table row 2043-2045, todo NOW). The S vs SP experiment below is DEFERRED, not dropped.
- COMMITTED 7c5b8d9 (not pushed): merged prefetch. rust_obm: obm_parse.rs (SegmentPath parse / with_segment / next_path on RAW path; show durations from servers_benchmark/video_duration.csv via include_str!), mod_feature/merged_prefetch.rs (MergedPrefetch '2 continue': k-1 + k same show+combo within window -> k+1; observe(path, now_s) honest), coalescing.rs (+ try_follow, lead_speculative, Leader::publish/joined, Follower::speculative/ready), SegmentCache/BinCache::contains_key. rust_server: lib_segment.rs (segment_lookup Real/Prefetch, fetch_offload, offload_cache_add, cache_add_task -> Task<bool>, prefetch_task), lib_main_state.rs (ServerState{coalescer if C or P, coalesce_real, prefetch}, env loaders, cluster csv, cost model, LB builders), lib_server_main.rs = response helpers only, main_server.rs = client flow (serve_offload loop: Hit / Follow / Lead; follower of a failed prefetch -> lookup again). env_setting.txt OBM_PREFETCH=0, _WINDOW_S=30, _HOLD_S=30. feature_detail_plot NON_SITE. Design + sims: prediction.md; code: code.md '[11-06] MERGED PREFETCH WIRED', SPLIT, SPLIT 2, '[11-07] C/P REVIEW' (+ FIXES 1-3).
- Behaviour: prefetch = speculative leader in the in-flight table; real requests follow it (worker 'prefetch', X-OBM-Coalesce wait) also with OBM_COALESCE=0; after publish it is held 30 s; at hold end added to the segment cache ONLY if read (joined > 0), awaited; logs [PREFETCH] issue / skip: cached|in flight / done / failed / expire | N readers | cache added|cache add failed|not read|cache off / hit | in flight|held / leader failed, lookup again. Real leaders now AWAIT the cache write before finish (gap fix).
- TESTED: unit 83 + 25 (+2), clippy 0; local fake-site e2e temp/prefetch_e2e/ (run.sh none|P|SP|SCP; needs target/release/main_server, ports 7000/17010/17020 free); c1 temp/prefetch_test (5 modes ALL OK, check.py) + temp/cp_break_test by peer session thirdyearproject-8e (P/CP/SP/SCP ALL OK, killed prefetch site -> CP/SCP 1 shared re-render). Analysis scripts C/P aware: cache_log_check (combined labels CS/SP/SCPAlfu, prefetch adds, LB selects), lb_log_check throttled, feature_detail_plot.
- Main image obm-main-server built 11-07 00:27 from the committed code (rebuild = harmless: cargo build --release --bin main_server && docker build -f servers_container/main_server/dockerfile -t obm-main-server .). rs at b3750a1: offload code unchanged since -> no pull needed (user pushes/pulls anyway).
- NEXT (user, new session): EXPERIMENT S vs SP on the 8 Part-2 seqs (run_table 'PLANNED NEXT'): OBM_SEGMENT_CACHE_MB=100 OBM_PREFETCH=1 FEATURES=SP temp/run_driver.sh per seq set (SEQ_DIR/RUN_ROOT as batch_features1, read batch_run.md: pre-flight, watchdog + cron). After each run: temp/cache_log_check.py + lb_log_check (driver does lb). check.py of prefetch_test is sequence-specific (scenarios.csv) - for replay runs use its rule/lifecycle part or write temp/prefetch_log_check.py. Sim expectation (prediction.md): S 13.9 -> SP 11.3 s mean, +12.5% renders vs S (no load feedback = optimistic).
- KNOWN LIMITATIONS (recorded, not fixed): #4 hold reads invisible to cache policy = SIZED SMALL (sim: <=0.15 s, makes SP slightly WORSE -> conservative; LFU unaffected) - mention in write-up; #5 joined() read at hold end can miss a joining reader (tiny); Drop-order (Leader::drop sets None before removing entry; only with >1 executor thread, SMOL_THREADS unset = 1); C off + P: real renders not in table -> prefetch may duplicate (design); no prefetch priority (throttled policies); LB has no dead-site detection (cp_break_test).
- GOTCHAS for the SP runs (added 11-07 KB check): temp/cache_log_check.py counts '[PREFETCH] expire ... cache add(ed| failed)' as a [CACHE] outcome (regex fixed 11-07; before it, every refused prefetch add gave a false '34 site answers but 35 outcomes' PROBLEM). cp_break_test check.py rules I3-I5 are the reusable part for a prefetch_log_check, but its I5 assumes NO eviction (200 MiB, 50 req) - at 100 MiB on full seqs track evictions first. Renders take 25-35 s under c1 load: any timing-based expectation (hold outlives a re-trigger, re-requests still in flight) must allow that. Verified 11-07: HEAD 7c5b8d9, rust tree clean, image 00:27 >= binary 00:15 = committed code.
- feature_detail_plot shows follower rows as 0 s parts (wait only in latency) - decide how to show for SP.

# Handoff 21 — 2026-11-06 evening (cache analysis + COALESCING built and tested, session end)
- Analysis (findings.md): WHY SEGMENT CACHE HITS LESS AT a1.2 (repeat supply equal ~0.38; gap = LRU eviction at 100 MiB, f1_full long reuse distance; temp/seg_alpha_why/); SEGMENT CACHE LRU vs LFU SIM (lfu100 +4.3 pts hit 8/8, +23% saved demand; bins add nothing; temp/seg_lfu_sim/); COALESCING POTENTIAL (4-8% of req no cache, 1-5% with cache).
- COMMITTED: 568700f data (32 feature runs + rr c1 2043-45), 153ae53 plots. Not pushed.
- COMMITTED e0ba1c1 (not pushed): coalescing = rust_obm/src/mod_feature/coalescing.rs (Coalescer: Mutex<HashMap<String, Arc<async_lock::OnceCell<Option<T>>>>>, Leader guard Drop removes entry / releases followers with None, followers()), mod.rs export, async-lock dep, Cargo.lock; main server: ServerState.coalescer (OBM_COALESCE 0/1), serve_offload_request SPLIT -> fetch_offload (no client I/O) + serve_offload (gateway errors, client write), offload_cache_add(state, path, leader, &seg) async (cache write detached, then leader.finish), serve_offload_coalesce (worker 'coalesce', X-OBM-Coalesce header, 502 when leader failed), sm::coalesce_response_head + test; env_setting.txt OBM_COALESCE=0; temp/run_driver.sh guard (gitignored). Details code.md '[11-06] COALESCING WIRED'.
- TESTED: smoke temp/coalesce_smoke/ (50 req C + CS, ALL OK) and race test temp/coalesce_race/ (236 req, phases dense/spread/same-ms/hammer/eviction/site-kill; C + CS ALL OK; evictions + 502 path exercised; gap race finish->cache write 0-27 ms reproduced once locally = harmless duplicate render 'already cached'). experiments.md has both. Main image currently = 153ae53 + coalescing (rebuilt 20:51).
- User terms: honest / dishonest functions (user.md) - tag /// [Honest] / [Dishonest I/O].
- om-only runs: export MAIN_SERVER_ADDR=obm-main-server:7000 (cluster.md).
- NEXT (user, new session): PREDICTION / prefetch feature (todo item 4 'Proactive Request Prediction': window of recent requests, seg k seen -> pre-render seg k+1 same combo; temp/prefetch_sim.py results in todo). Also open: LFU single segment real run (todo top); coalescing analysis-script follow-ups (worker 'coalesce' in feature_detail_plot site_of, cache_log_check); optional gap fix (await cache write before finish); bin 8 s run.

# Handoff 20 — 2026-11-06 (feature analysis + plots, session end)
- Plots: case_wins c1/c2 redone (rr c1 now 5 seeds). stat/latency_plot.py --features [LABEL] --policy mct (opt-in, no bar cut) -> stat/feature_plot/latency_features_*.png. NEW stat/feature_plot/: feature_detail_plot.py (moved from stat/, ONE common panel = mean time per request all runs), segment_plot.py + asset_plot.py (shared feature_breakdown.py: left parts % of no feature, right stacked s, bottom colour-square hit line '<run>: (hit X / miss Y req|files) = Z% hit (Q% of bytes)'). 8 seqs each regenerated. plot.md has all rows.
- Findings added: FEATURE DETAIL (queue is where all win; segment hits take load off LB/sites; asset byte hit 27% vs file hit 60%), % OF NO FEATURE, WHY BIN CACHE TIES LRU (threshold 6 s splits traffic ~50/50, cost skew 1.7x, reuse dist ~ cap; sim temp/bin_why/ validated, thr8 +11% sim), WHY NO CLEAR STRONGEST FEATURE (all cut service demand 20-30% -> nonlinear queue collapse; winner flips with alpha; noise 1-2 s, 1 run each).
- COMMITTED 11-06: 568700f data (32 mct feature runs + rr c1 2043-45), 153ae53 plots (latency_plot --features, stat/feature_plot/*.py). Not pushed. PNGs gitignored.
- NEXT: todo top = bin cache 8 s run (8 runs; extend cache_log_check label B* first); then part 2 other seeds / part 3 S+Alfu combo; seed_rank add rr c1; commit when asked.

# Handoff 19 — 2026-11-06 10:50 (features batch 1 DONE)
- temp/batch_features1.sh DONE 03:10-10:31: 38/38 runs, 1 failed request (Dana restart > 5 s, not cache), all driver + cache checks OK. Cron 34768d21 deleted. Containers 0 on om + rs. DATA UNCOMMITTED: stat/seed204{3,4,5}/run_a*_c1/rr_none_1* and stat/run_a*_c*/mct_{S,B,Alru,Alfu}_1*, stat/seed2042/run_a*_c*/mct_{S,B,Alru,Alfu}_1*.
- RESULT (findings FEATURES BATCH 1): every feature alone -35..-67% mean latency vs mct_none; avg S -55%, B -51%, Alfu -51%, Alru -47%. S >= B (6/8), Alfu > Alru (7/8).
- Tools: temp/cache_log_check.py (cache log checker), temp/feat_batch_status.py (per-run status), temp/feat_batch_summary.py (feature vs none table). run_table.md updated.
- NEXT: todo top item (part 2 on other seeds? part 3 combos S+Alfu; bin tuning; connect budget; commit data).

# Handoff 18 — 2026-11-06 03:15 (batch running, user asleep)
- RUNNING: temp/batch_features1.sh (nohup setsid, started 03:10, ETA ~11:25), log temp/logs/batch_features1.log. 38 runs = run_table.md part 1 rest (rr c1 seeds 2043-45) + part 2 (mct + S/B/Alru/Alfu on seeds 2037 + 2042). Cron 34768d21 :13/:43 session-only -> if this session died, re-create it (prompt in experiments 'Features batch 1' intent: watchdog.sh + cache_log_check.py per finished run + run_table update; on BATCH DONE summarise per feature vs mct_none, update KB, delete cron).
- NEW temp/cache_log_check.py <run csv/base>: cache log checker (ALL OK / WARN / PROBLEM). NEW kb/run_table.md (which runs done + pointers).
- Don't edit run_driver.sh / cluster_lib.sh while the batch runs. Data stays uncommitted until asked.

# Handoff 17 — 2026-11-06 (session long, 3 MB)
- COMMITTED (not pushed): fe7ddfd LRU segment cache; 4a07eb3 bin cache + ServerState + builders; b3750a1 PolicyCache (LRU/LFU) everywhere + site asset cache (OBM_ASSET_CACHE_MB/POLICY/DIR) + servers_container/env_setting.txt (all feature settings, env-file format; cluster_lib loads it, shell wins).
- NOT DONE for b3750a1: main image rebuild (SegmentCache::Single + policy), cluster test of the asset cache. rs needs user push + rs pull first (rs builds sites from its repo; check `ssh rs 'cd ~/SCC/Third_Year_Project && git log --oneline -1'` = b3750a1).
- Asset cache choice (findings 'ASSET CACHE POLICY'): 50 MB/site, LFU (+3.4 pts file hits vs LRU, sim temp/asset_cache_sim.py, model not validated -> the [ASSET] site log lines now allow validation).
- Segment cache size (findings 'CACHE SIZE CHOICE'): 100 MiB single, sweep 50/100/200.
- temp/ (gitignored) changed: cluster_lib.sh (load_env_setting, site env to rs via ssh 'env ...'), run_driver.sh (source before guard, guard incl OBM_ASSET_CACHE_MB), bin_smoke.sh, bin_xbin.sh, cache_size_sim.py, asset_cache_sim.py.
- NEXT: after push/pull: rebuild main image, asset cache smoke on c1 (FEATURES=A?), check [ASSET] hit counts vs sim; todo 'SEGMENT CACHE next steps' (a) analysis skips worker=cache; user's rr c1 seeds 2043-45 pending.

# Handoff 16 — 2026-11-06 (mid-session)
- COMMITTED (not pushed): fe7ddfd LRU segment cache; 4a07eb3 bin cache (BinCache 2 LRU, cost = (asset + work/render factor)/1000, threshold OBM_BIN_CACHE_THRESHOLD_S default 6, OBM_BIN_CACHE_{LOWER,UPPER}_MB, mutually exclusive with OBM_SEGMENT_CACHE_MB via SegmentCache enum) + ServerState<P> + policy builders in lib_server_main.
- MAIN IMAGE rebuilt 11-06 ~01:47 from 4a07eb3 code (caches off by default).
- TESTS: temp/bin_smoke.sh (c1 mct 50 req, FEATURES=B) OK; temp/bin_xbin.sh (c1 rr custom seq, cross-bin refusals) OK twice. experiments 'Bin cache smoke' + 'cross-bin test'. run_driver guard covers OBM_BIN_CACHE_*_MB.
- Side finding: llvmpipe segments ~half the bytes of gpu/cpu for the same path (cache keeps first finisher's version).
- NEXT: todo 'SEGMENT CACHE next steps' (a) analysis scripts skip worker=cache, (c) Policy trait?, (d) capacity sweep, (e) coalescing; user's rr c1 seeds 2043-45 still pending.

# Handoff 15 — 2026-11-06 (end of session)
- CODE (COMMITTED fe7ddfd 11-06, user pushes): rust_obm/src/mod_feature/base_cache.rs LruCache(byte_cap) over CacheTable (one Mutex; file write + victim delete outside lock; 5 tests) + rust_obm/src/lib.rs `pub mod mod_feature` (user's line). rust_server: lib_server_main.rs gets aco_seed/load_cost_lookup/load_cost_model (moved) + segment_cache_loader (OBM_SEGMENT_CACHE_MB, unset/0 = off; OBM_SEGMENT_CACHE_DIR) + is_status_ok; main_server.rs serve_offload -> serve_offload_cache (hit: send_file, X-OBM-Cache hit, worker cache, zero timings; LB untouched) / serve_offload_request (miss: add only 200, smol::unblock after client write, every outcome logged '[CACHE] added N B (used M B) | add failed (N B): reason | skip: status not 200'). clippy/fmt/tests OK.
- DECISIONS (code.md): features don't share files (own CacheTable/subdir; prefetch->segment = move); LRU = HashMap + linear min scan (n small); planned Policy trait (on_insert/on_hit/on_evict/priority f64); bins = 2 caches + router.
- MAIN IMAGE REBUILT 11-05 23:30 (264b0b4) with cache code, off by default -> no-feature runs unchanged.
- DRIVER (temp/, gitignored): cluster_lib start_case forwards all OBM_* into main container + logs 'main server env:'; run_driver FEATURES label (default none), refuses cache var with FEATURES=none. New KB env.md lists every env var (rule: add a row for each new one).
- RESULTS: cache smoke c1 a0.5 set2 mct 100 req 45 MB -> 100/100 ok, 14 hits/23 possible, 3 concurrent-miss 'already cached', LB check OK, mean 6.85 vs 9.6-9.8 s (experiments 'Segment cache smoke'). sendfile bench: LAN link-bound, zc 2.5-5x less CPU; localhost zc 1.5-3x faster (findings 'SENDFILE'). Site video check: all 6 configs (dec cpu|gpu x comp gpu|llvmpipe|cpu, enc cpu only) playable, visually identical (SSIM >= 0.983/frame); caveats llvmpipe forecast slice errors, green line under f1 inset in all (findings 'SITE CONFIG VIDEO CHECK'). User may say 'all ok, small difference' with scope.
- transport-prod container removed (user's old one). Cluster stopped, no crons.
- USER TONIGHT: runs rr on c1 for seeds 2043-45 themselves (todo); afterwards seed_rank (add rr to c1) + case_wins_plot --case c1.
- NEXT: todo top 'SEGMENT CACHE next steps' (commit, analysis scripts skip worker=cache, bins, sweep, coalescing).

# Handoff 14 — 2026-11-05 (end of session)
- DONE: 3-seed batch (2043/2044/2045, 54 runs, 0 failed, checks 54/54 OK; 24 re-run after rs eduroam/DNS drop 05:33). Data committed 1d687a3, seqs 4ccf1de (NOT pushed). Tree clean, no crons, no containers.
- RESULT (findings 'FINAL LB RESULT', user-agreed wording): c1 mct best mean + p99 10/10, model > no model 10/10; throttled2 longer p50 / shorter p99 vs esce; esce short p50 / long tail; rr ~best p50, worst tail. c2: informed policies tie (noise), mct most frequent winner (mean 4/10, p99 6/10, never worst), rr worst p99 10/10. Why mct loses some c2 seqs: 'WHY MCT LOSES SOME c2 SEQS' (queueing in busy stretch, flips by seq).
- Tools: temp/seed_rank.py (rank table + Kendall W), temp/case_wins_plot.py [--case c1|c2] -> temp/figs/<case>_wins.png. NEW KB file plot.md = user's plot style rules + all plot programs.
- Driver change: temp/cluster_lib.sh start_case_retry (site start retried every 60 s up to 10 min), used by run_driver.sh; resume-style script optional.
- Plots: seed latency PNGs moved to stat/seed<S>/; latency_detail_seed* deleted (keep only set-2 detail plots).
- NEXT: (1) todo top: rr on c1 for seeds 2043-45 (6 runs ~2 h, needs rs OK) then rerun seed_rank (add rr to c1) + case_wins_plot --case c1; (2) optional 5-seed slide figure (extend seed_compare_plot.py); (3) segment cache work (todo COMPUTATIONAL FEATURES). User pushes commits.

# Handoff 13 — 2026-11-05 00:17
- RUNNING: temp/batch_seeds3.sh started 00:14:45 (03c9a1f, rs 0e5ded6, om performance / rs balanced): seeds 2043 2044 2045 (pre-declared), mct mct_const_flat throttled2 esce on 4 seqs + rr c2 only, 1 round = 54 runs, ETA ~11:15, -> stat/seed204{3,4,5}/. 05:33 rs wifi/DNS drop skipped 24 runs -> temp/batch_seeds3_resume.sh 05:56 (DNS guard), ETA ~10:45, cron c48aab10 (was d36aee23) :13/:43 (watchdog; on BATCH DONE: plots, 5-seed rank table, KB, report). Rules kb/batch_run.md.
- Claim to test (findings 'RANK CLAIM CHECK'): strict 5-way order does not hold (esce~throttled2 swap, c2 tie); viva claim = mct best + rr worst in c1, mct <= flat, c2 tie.
- Seqs seed 2043-2045 committed 11-05 (not pushed; timelines gitignored). Segment-cache design still next after this.

# Handoff 12 — 2026-11-04
- Since handoff 11: committed f0a68d2 (seed 2042 data) + 03c9a1f (plot margin fix); tree clean. Analyses (findings): 'SEED 2042 DEEPER' (MCT wins c1 by sending only SMALL jobs to cpu sites), 'TIER CONSTANT PREDICTOR' (per-tier const ~ flat, must rank jobs), 'WHY SEED 2042 LATENCIES HIGHER' (burstier, Little's law). Slide fig temp/figs/seed_compare.png (temp/seed_compare_plot.py; rr left out, offered rr on c2 panels - no answer yet). Week-5 progress report text given.
- New hook: .claude/kb_reminder.sh (UserPromptSubmit; >3 MB transcript -> suggest new session on topic change).
- TODO added: 3 more seeds for viva (todo.md top). Caching = 5 caches (todo.md 'COMPUTATIONAL FEATURES').
- NEXT (new session): read docs/Render_Cost_Driven_Caching_for_Object-Based_Media.pdf + docs/Weighted_Greedy_Dual_Size_Frequency_Based_Caching_Replacement_Algorithm.pdf -> ideas for the Segment Cache replacement policy (frequency + computation cost), write to KB (code.md / findings), then design with user.

# Handoff 11 — 2026-11-03 ~10:00
- DONE: seed 2042 batch 01:47-09:45, 40/40 runs -> stat/seed2042/run_a*_c*/ (committed f0a68d2). 0 Dana resends, [check] 40/40 OK, failures only rr c1 22x HTTP 504 at the 600 s timeout (overload). Cron ece2075e deleted.
- RESULT: set 2 ranking holds on seed 2042 (findings 'SEED 2042 REPLICATION'): c1 MCT+model best mean + p99 (lead 1.3-4.5 s mean, p99 51/74 vs 81-200); c2 tie within ~1.7 s; throttled2 tail << esce; rr best-ish p50, catastrophic c1 tail.
- committed 03c9a1f: stat/latency_plot.py + latency_detail_plot.py layout fix (fixed subplots_adjust; tight_layout clipped x labels / right-side lists). Set 2 + seed 2042 PNGs regenerated (gitignored).
- NEXT: ask user to commit data + plot fix; README line 'ranking holds on a 2nd seed' (offered); then caching (stage 2 remainder) in a NEW session (user agreed sessions per deliverable).

# Handoff 10 — 2026-11-03 ~02:30
- RUNNING (02:23 check ALL OK, 3/40 done, 0 fail; c1 a0.5 run1 mct 21.1 vs flat 22.5 vs throttled2 25.6 mean): temp/batch_seed2042.sh started 11-03 01:47 (48ada55 om, rs 0e5ded6): mct mct_const_flat throttled2 esce rr x 4 seed-2042 seqs x 2 rounds = 40 runs, ETA ~09:20, -> stat/seed2042/run_a*_c*/. Cron ece2075e at :13/:43 (watchdog; on BATCH DONE: plots + compare with set 2 + KB + report). Rules: kb/batch_run.md.
- Commits since handoff 9: e784ef1 repeat data, 564e989 timings -> stat/timings/ (+ path fixes), be37586 stat/latency_plot.py + latency_detail_plot.py, 7eedaf7 generator --seed/--out-dir + seq_bench --run-root + seed2042 sequences, 48ada55 README ffplay line.
- COMMITTED 13152f3 (user pushes): README.md rewritten as CV/project summary following the proposal (reports/proposal/main.tex) — results up front, MCT/EWMA/cost model, findings, offline bracket, plan by proposal stage, planned optimisations; old README git mv -> breakdown.md; highlight/ (make_figures.py + 6 PNGs + data/offline_opt.csv); .gitignore !highlight/*.png.
- KB added: findings (final latency summary, breakdown, noise, const instability, EWMA ends, c2 per-site cost), batch_run.md (how to run batches), experiments 'SEED 2042 SET'.
- Gotcha: live cost-model error is 8.5% median / 25% p90 in the full replay (highlight/cost_model.png) vs 7.5% / 29% quoted earlier (fewer runs); user's slide 23 uses the old numbers.
- NEXT: batch report; README line on whether ranking holds on seed 2042; computational optimisations (caching first) = stage 2 remainder; simulator = stage 3.

# Handoff 9 — 2026-11-02 ~11:40
- Repeat batch DONE 11:23: 46 runs (45 after restart on 0e5ded6 + iwrr #2), 0 failed requests, checks ALL OK (1 benign throttled note), Dana resends 0. Summary temp/analysis_v4/set2_repeat_summary.csv + set2_repeat_runs.csv; claims in findings 'REPEATS (11-02)'. COMMITTED e784ef1 (was uncommitted): new stat/run_a*_c*/*_none_{2,3}.csv + mct_lookup/flat _1/_2 + earlier mct_lookup c1a0.5 #1 and iwrr #2.
- Commits since handoff 8: 0e5ded6 Dana resend fix (rs pulled by user). temp/lb_log_check.py: site-queue >3 s is a note when in-flight OK.
- Headline: mct best in c1 (2.2-2.3 s over mct_const, p99 51/60 vs 64-130); lookup slight edge (c1 a0.5 only); c2 ~tie; throttled2 beats throttled except c1 a0.5; flat ~ mct_const.
- NEXT: commit data when asked; stacked breakdown figure with error bars (offered); per-site analysis; batching sim (offered); informed throttled (offered).

# Handoff 8 — 2026-11-02
- Commits (not pushed by me): af6a449 mct_lookup (stat_predict/lookup_table.py -> lookup.csv, rust_obm cost_lookup.rs, cost-model fill for misses) + mct_const_flat (MCT::with_prior); 2213e1b plot_seq bigger/wider; 17183ce docs/next_cites.md (11 entries, 4 'from memory: check'); a13970c data throttled2 x4 + mct_const_flat c1 a0.5. Working tree clean (temp/ gitignored).
- Main image already rebuilt with af6a449 code (includes lookup.csv).
- Findings 11-01/02 (findings CURRENT): ACO only BETA matters; LEARNING rate no effect; flat priors fine (19.6 vs mct_const 21.6, variance); throttled2 -2 s vs limit 1, c1 behind mct (speed-blind); practice vs papers; offline best case 10-11 s vs mct ~18 (prediction 3-4 s + online 2-4 s).
- NEXT: run mct_lookup (c1 a0.5 ~12 min or all 4 ~50 min; offered, not started); batching sim offered; informed throttled offered; Dana recycle race fix; per-site set 2 analysis.

# Handoff 7 — 2026-11-01 ~16:20
- Commits today (not pushed by me): 445b6c5 set 2 full batch data, cdf7ddc Throttled<const LIMIT> + throttled2 (token deque, ties rotate; limit-1 tie rule changed vs set 1/2 runs).
- ACO / MCT analysis (findings CURRENT): only BETA matters for ACO (high = MCT), pheromone has ~no signal, LEARNING rate no effect; user: report ACO as-is, no retune. Sims temp/aco_sim{,2,3}.py: clairvoyant mct -3..4 s = prediction bound; SJF better mean, bad p99.
- throttled2 batch DONE 16:04: 4/4, 0 fail, checks ALL OK. 21.8/24.3/18.8/18.5 mean (c1a0.5/c1a1.2/c2a0.5/c2a1.2) vs throttled 24.0/26.3/20.7/22.9, mct 18.0/19.2/16.5/18.7. c1 gap = speed-blind slots on cpu sites. UNCOMMITTED: stat/run_a*_c*/throttled2_none_1.csv (+ logs gitignored); temp/smoke/ (smoke script + run).
- temp/lb_log_check.py: throttled checks max in flight per site <= LIMIT (from type line); 3 s site-queue alarm only for limit 1.
- NEXT (todo): commit throttled2 data when asked; informed throttled (offered); Dana recycle race fix; set 2 per-site analysis; scripts into repo.

# Handoff 6 — 2026-11-01 ~09:00
- Set 2 full batch DONE 08:50 (30 runs, 1 failed request = Dana recycle race, decision checks all OK after checker threshold 2->3 s). Results + reading: experiments.md 'Set 2 full batch', temp/analysis_v4/set2_summary.csv. Data committed 445b6c5 (11-01); temp/lb_log_check.py change (3 s threshold) is in temp, untracked.
- Commits since handoff 5 (not pushed by me): 0852f30 .gitignore .claude/, 96b275a set-2 esce check.
- Headline: mct best mean+p99 in all 4; c2 close; throttled worst dynamic mean; esce run-to-run noise mean <=0.8 s, p99 up to 28 s.
- NEXT (todo.md): throttled limit 1+2 (token deque + waiter queue), Dana recycle race fix (serve_dana retry on reset-before-first-byte), set 2 per-site analysis, move report analysis scripts from temp/ into repo + zip logs for archive (offered), repeats.
- SessionStart hook in .claude/settings.json shows INDEX + this handoff on startup/resume/compact.

# Handoff 5 — 2026-10-31 ~20:00
- Commits today (user pushed; rs on 621d6fc): cfe3319 set-1 data, 0ad7767 Dana asset timer (stops after first-frame resources -> asset_ms includes option fetches, net_ms ~2-3 delays), b0109ed set 1 moved (servers_benchmark/sequences/set1_unload/, stat/set1_unload/), e0946eb generator (capacity from replay runs asset+work+net; rate fitted on window's own mix; representative window show share tol 0.03) + plot_seq.py, 621d6fc set 2 seqs seed 2037 (all rho 0.8, one two-wave shape).
- Set 1 = FIFO-era 30 runs on old seqs: valid per-sequence policy ranking, but true loads 0.71/0.75/0.83/0.88, unrepresentative show mix, n=1. Use as robustness/backup.
- Set 2 esce load check DONE (temp/batch_set2_esce.sh, stat/run_a*_c*/esce_none_1.csv, uncommitted): busy 0.84/0.82/0.83/0.80 vs target 0.80, 0 fail, queue builds in both waves + drains ~min 6, net_ms now 88/150 ms.
- NEXT: user says "run all later" -> full set 2 batch (8 policies x 4 seqs, ~5h40, watchdog + 30-min decision checks; esce rerun gives a 2nd esce sample). Optional later: 0.65 sensitivity (rr/esce/mct), swapped-delay c2 (de-confound machine vs delay), cost model retrain (fix stat_predict globs first).
- Slides: user writing; network + speedup slides reviewed (fix "60 ms adds 1.9 s" -> extra 30 ms vs om costs ~1.8 s; rs slower only for gpu/llvm, faster for cpu; MCT sends cpu work to rs).
- GOTCHA: stat_predict evaluate/export glob stat/run_* (stale after moves). Cost model kept frozen for set 2 (all set-2 paths in features.csv).

# Handoff 4 — 2026-10-31 ~10:05
- FIFO era: offload 840a927 (FIFO worker + async send-back, om + rs), lb 522c7d9 (Throttled + ACO), data move 06645a0 (old runs in stat/old_nonfifo/). Not pushed past what the user pushed.
- batch_fifo1 DONE: 30 runs (c1 x8 policies, c2 x7, both alphas), 0 failures, decision checks 30/30 OK. Summary temp/analysis_v4/fifo1_summary.csv + experiments.md "FIFO era". UNCOMMITTED: stat/run_a*_c*/, servers_benchmark/sequences/plot_seq.py.
- Tools: temp/lb_log_check.py (decision checker), temp/batch_fifo1.sh, run_driver saves _site_*.log + runs checker, watchdog live decision check + median temp.
- Sequences kept as is (user: shared ramp is good); temp rho 1.0 test showed regenerating changes the ramp (time-compress instead if a 2nd load level is wanted).
- Next ideas: repeats (1 run each; p99 spread ~30 s), per-site/decision analysis of FIFO data (analyse_v4 on new folders), oracle MCT, cache work (c2 first).

# Handoff 3 — 2026-10-30 ~16:15
- Commits: 348869c (v3 data, old c2 -> old_nonfifo/run_a*_c2_p75), 551c70e (per-cluster rho, case2 0.88), 55167bc (MCT<F> generic predict fn, mct_const = Some(5.59 s)). Not pushed.
- Batch v4 done (temp/batch_v4.sh): c1 mct_const x2, c2 mct_const/rr/esce/mct x2, all 0 fail. Results + ablation reading in experiments.md "Batch v4". UNCOMMITTED: stat/old_nonfifo/run_a*_c1/mct_const_none_1.csv, stat/old_nonfifo/run_a0.5_c2/, stat/old_nonfifo/run_a1.2_c2/.
- GOTCHA: main image COPYs target/release/main_server -> cargo build --release --bin main_server before docker build.
- Watchdog (temp/watchdog.sh) follows newest temp/logs/batch_*.log.
- Next ideas: repeats (1 run each so far, c2 gaps ~noise), per-site/prediction-error analysis from MCT logs, caching in c2 first.

Session handoffs: newest first. Each one = state at the end of a session, so the next session can resume without re-deriving. updated 2026-10-30

## Handoff 2 — 2026-10-30 ~07:05 (session 9754c517, overnight batch)

### State right now
- Nothing running; no obm containers on om or rs; watchdog cron deleted. Git: HEAD 2d8ff06 (MCT) + UNCOMMITTED data: stat/old_nonfifo/run_a0.5_c1/mct_none_1.csv, stat/old_nonfifo/run_a1.2_c1/{iwrr331,esce,mct}_none_1.csv, stat/old_nonfifo/run_a{0.5,1.2}_c2/ (rr/esce/mct). Commit only when the user asks.
- Power profiles changed 10-30 05:24: om performance, rs balanced (rs was overheating: acpitz 85-89 C, crit 100). c1 a1.2 iwrr331 straddles the change.
- Results table of all 14 v3 runs: experiments.md "Batch v3_rest DONE". MCT best in c1 on all percentiles; c2 mixed (esce ~ mct at a0.5).

### What was done this session
1. MCT policy (policy_mct.rs): EWMA machine factor (t_work/pred, clamp) + network (net+asset), makespan = sum of in-flight costs, Ticket type in LoadBalancerPolicy/SelectionGuard, fallback off by default (OBM_MCT_FALLBACK=1), 2 log lines per request (mct select/done). main_server "mct" arm, POLICIES const, model baked into image. Commit 2d8ff06.
2. Smoke test temp/mct_smoke/run.sh; batch temp/batch_v3_rest.sh; watchdog temp/watchdog.sh (now incl. acpitz >= 95).
3. Standing rule saved: long run = user asleep -> watchdog + 30-min cron (memory long-run-watchdog.md).

### Next steps
1. Commit the new run data (when asked). 2. Per-site analysis of v3 incl. MCT logs (prediction error, factor convergence: rs network ~3.8-4.3 s vs prior 3.0). 3. Decide on rerunning c1 under the new power profiles / repeats. 4. Oracle / min-min / max-min / cache (todo.md).

### Gotchas
- Cluster site names are "id=om1_ip=..." (MCT short_id). Process checks: match exact argv fields, not grep on args. User keeps interactive `docker stats` in terminals: leave them.

## Handoff 1 — 2026-10-30 ~02:30 (end of session c95e93dc)

### State right now
- Nothing running. Both laptops idle, no obm containers on om or rs, no leftover monitor loops (check: `ssh rs 'ps -eo args | grep "[d]ocker stats"'`).
- Git: working tree clean on `main`, ahead of origin (the user pushes and pulls — never push). Latest commits:
  `0e51909` cost model + Rust predictor · `65a6896` v3 replays + syn_timings + syn_bench · `6dcea4b` syn_video · `49c6a77` load-corrected sequences · `667b73f` ESCE policy.
- iwrr weights in code are 3:3:1 (policy_iwrr.rs). Main server image built from current code.
- Sequences in servers_benchmark/sequences/ = v3 (rho 0.8, network overhead 50 RT = 1.5 s om / 3.0 s rs, request mix from paths): case 1 = 303 requests.

### What was done this session (details in the topic files)
1. Overnight + iwrr331/551 runs -> real load was ~1.0, not 0.7 (stat/run_*_p95/). docs/results_report.md.
2. Generator fixed (network overhead, request mix, rho 0.8); runs at real load 0.57/0.60 (p57/p60).
3. ESCE policy; v3 case-1 runs: a0.5 rr/iwrr331/esce + a1.2 rr (a1.2 iwrr331, esce NOT run).
4. Synthetic shows (syn_video/, 27 request kinds, OBM H2i format) + host Dana test bench (temp/syn_test/) + cluster timings (stat/syn_timings.csv).
5. cpu-vs-gpu burst test (temp/cpu_vs_gpu/); GPU-native idea tested and PARKED (not the user's direction).
6. Cost model (stat_predict/): generic features, NNLS per (stage, backend), seg_frac, replay data; Rust CostPredictor in rust_obm/src/cost_predict.rs (matches Python).

### Next steps (user's priorities: cost prediction -> LB policies; deadline = supervisor meeting next week)
1. MCT policy in rust_obm using `CostPredictor`: est. finish = site busy_until + network_estimate[site] + machine_factor[machine] × predict(path, site); learn machine factor + network from `TimesFeedback` (EWMA); `None` -> fall back to esce. Add `"mct"` to main_server policy match. Needs the predictor files mounted into the main server container (stat_predict/coef.csv, features.csv).
2. Finish v3 batch: `temp/run_driver.sh 1 "1.2" "iwrr esce"` (+ mct when ready), then per-site analysis (temp/report_analysis.py "stat/old_nonfifo/run_a*_c1/*.csv").
3. Validate partial offload predictions (requests with smaller root count N).
4. Oracle / best case, min-min / max-min, caches (see todo.md).

### Must-know rules / gotchas
- User rules (user.md): commit only when asked; never push; ask before changing anything on rs (reading is fine); never touch servers_container/main_server/local_host.txt; baseline stays "normal default level", Dana core unmodified; keep KB + history.md updated every step (memory keep-kb-updated).
- Request path: number after orientation = layer-graph ROOT nodes (not layer count); layer tokens start at split index 11; paths may contain spaces ("Weather Icons") -> split files by lines, not whitespace.
- OBM video assets use H2i framing; undecodable input still renders 250 black frames with exit 0 -> always check picture content.
- Last segment of a show is often short -> seg_frac.
- Scratchpad gets wiped: keep scripts in temp/ (gitignored). Drivers: temp/run_driver.sh, temp/syn_driver.sh, temp/cluster_lib.sh.
- Never `pkill -f` with a pattern in your own command line; use `[x]yz` patterns or PIDs. Remote monitor loops must be killed by name.
- "gpu" sites use the Intel iGPU (entrypoint prefers 0x8086); NVIDIA dGPUs unused.
