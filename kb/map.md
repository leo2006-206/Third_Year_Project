Where to find things in the repo (pointer map, not content). updated 2026-11-07
Trust order: code > KB / files changed this week (2026-10-28..31) > older docs. Anything <= 2026-10-21 predates the cable cluster / replay work; verify before quoting.

## Top level
- kb/ (moved from temp/kb 11-07; temp/kb = symlink to it) — this KB (Claude's notes). NOT gitignored -> shows as untracked; commit only when the user asks.
- README.md (11-03) — CV-style project summary + highlights (figures in highlight/, make_figures.py). breakdown.md (old README, git mv) — §1 architecture (§1 networking text about Tailscale/VPS is OLD, cluster now uses cable), §2 host GPU checklist, §3 two-laptop cluster guide, §4 servers_container file reference, §5 syn shows.
- Cargo.toml workspace: rust_obm, rust_server, rust_sim. local_ci.sh / .github/workflows/ci.yml: fmt, clippy -D warnings, test, ruff.
- obm/ (gitignored, from barryfp/obm): Dana OBM engine, obm/shows/*.json (f1, f1_full, forecast, forest720, forest720_leaves, spiders), obm/assets.
- set_ssh.sh, servers_container/main_server/local_host.txt — gitignored secrets, never touch/commit.
- .claude/settings.json (gitignored): SessionStart hook printing kb/INDEX.md + newest handoff.

## Rust
- rust_server/src/main_server.rs — main server :7000: web UI, /assets /shows serving, /offload LB proxy (store-then-forward). Policy arg: rr, iwrr, esce, mct, mct_const, aco, aco_const, throttled, throttled2. serve_offload: select_wait (throttled waits here) -> X-OBM-Wait; CONST_WORK_S 5.59; load_cost_model (OBM_COST_COEF/FEATURES, default /app/stat_predict/*.csv); aco seed OBM_ACO_SEED (2026).
- rust_server/src/offload_server.rs — per-site proxy in container: /offload -> FIFO channel -> single dana_worker -> Dana :9009 (read_to_end, reply sent by spawned dana_sendback, 840a927); /assets,/shows -> MAIN_SERVER_ADDR (plain proxy, NO cache, new TCP connect per asset).
- rust_server/src/lib/: lib_http.rs, lib_server_main.rs (cluster CSV, time_append_header incl. X-OBM-Wait, gateway error, aco_seed/load_cost_*/segment_cache_loader/is_status_ok moved/added 11-05), lib_util.rs, lib_core_type.rs, lib_server_offload.rs (DanaJob).
- rust_obm/src/mod_load_balancer/: base_balancer.rs (LoadBalancer, select + select_wait, OffloadSite, SelectionGuard, TimesFeedback), policy_robin.rs, policy_iwrr.rs (weights gpu 3 : llvm 3 : cpu 1), policy_esce.rs, policy_mct.rs (MCT<F> generic predict fn, EWMA f/n per site), policy_aco.rs (reuses MCT parts, pheromone show x site, seeded), policy_throttled.rs (Throttled<const LIMIT>: central FIFO, token deque; ThrottledLB limit 1, Throttled2LB limit 2), mod.rs (POLICIES).
- rust_obm/src/cost_predict.rs — CostPredictor (loads stat_predict coef.csv + features.csv, predict(path, site) -> machine-neutral work s).
- rust_obm/src/file_cache.rs — CacheTable (key->hashed file). rust_obm/src/mod_feature/: base_cache.rs PolicyCache(byte_cap, Lru|Lfu) (b3750a1), bin_cache.rs BinCache (2 LRU bins), segment_cache.rs SegmentCache::{Single,Bin}, asset_cache.rs AssetCache (site), coalescing.rs Coalescer/Leader/Follower/Role (e0ba1c1, 11-06; + publish/joined/try_follow/lead_speculative 11-06), merged_prefetch.rs MergedPrefetch (2-continue predictor, 7c5b8d9). rust_server/src/lib/lib_main_state.rs (ServerState, env loaders, cluster csv, cost model, LB builders; 11-06), lib_server_main.rs now only response helpers; rust_server/src/lib/lib_segment.rs (segment lookup/render/share/prefetch_task, split from main_server 11-06). rust_obm/src/obm_parse.rs SegmentPath parse/with_segment/next_path + show durations from servers_benchmark/video_duration.csv (include_str!, 7c5b8d9). temp/prefetch_sim2.py prefetch design sim. rust_sim/ — stub.
- rust_server/src/main_server.rs (11-06 e0ba1c1): serve_offload (cache hit -> coalesce follower -> fetch_offload leader), fetch_offload (LB + site only), offload_cache_add (cache write + leader.finish), serve_offload_coalesce; lib_server_main.rs ServerState {balancer, segment_cache, coalescer}, *_loader per feature, coalesce_response_head.

## Containers (servers_container/)
- cluster_endpoint_c{1,2}.csv (main server site list), offload_{om,rs}_c{1,2}.csv (per machine: tiers, cpu/mem caps, delay/jitter 30/3 om, 60/6 rs).
- run_all_offload.py [CSV] [-d] [-w] — builds offload binary + image and starts the sites of a CSV (rs builds from its own repo copy ~/SCC/Third_Year_Project -> needs git pull).
- offload_server/: dockerfile, entrypoint.sh (netem rate 1gbit, GPU checks, prefers Intel iGPU), run_sh.sh, OffloadSite_new.dn (Dana site; asset timer now ends after first-frame resources, 0ad7767), docs_detail.md.
- main_server/: dockerfile (COPY target/release/main_server -> cargo build --release first; COPY stat_predict csvs), entrypoint.sh, run_sh.sh.

## Benchmark (servers_benchmark/)
- generate_request_seq.py + detail_seq.md — viewer model; capacity from replay runs (run_globs stat/set1_unload, stat/old_nonfifo; calibration fills unseen kinds); rate fitted on window's own mix; representative window (show share tol 0.03); seed 2037; rho 0.8 all cases (e0946eb, 621d6fc).
- sequences/: set 2 = seq_a{0.5,1.2}_case{1,2}.csv + *_viewers.csv + meta.csv (attempt, show_share_dev, first_half); plot_seq.py (timeline PNG, show share legend; PNGs gitignored); set1_unload/ = set 1 seqs (seed 2026).
- stat/latency_plot.py (11-02) — <run dir> [--exclude rr]: latency table mean/p50/p90/p99 ± std over runs + 4-panel bar PNG (default stat/latency_<run dir name>.png).
- sequences/seed2042/ (11-02) — second sequence set (seed 2042), runs -> stat/seed2042/run_a*_c*/; generator --seed/--out-dir, seq_bench --run-root.
- seq_bench.py — open-loop replayer -> stat/run_a<α>_c<c>/<label>_none_N.csv (cols incl. wait_ms; queue_ms = site queue + wait).
- stat_collect.py — calibration (first segment of each kind) + parse_timing_headers. request_kind.txt, show_options.json, video_duration.csv.

## Cost model (stat_predict/)
- features.py, model.py (NNLS per stage/backend), evaluate.py, export.py -> coef.csv + features.csv (1362 paths: every kind x segment). README.md. GOTCHA: evaluate/export glob stat/run_* (only set-2 runs there now).

## Data (stat/)
- timings/{om,rs}_7010_gpu/7020_llvm/7030_cpu_timing.csv — calibration (no delay); timings/syn_timings.csv (moved to stat/timings/ in 564e989).
- run_a{0.5,1.2}_c{1,2}/ — set 2 runs: esce_none_1 (96b275a) + full batch 8 policies, esce_none_2 (445b6c5) + _main.log + _site_<id>.log (logs gitignored).
- feature runs (568700f, 11-06): stat/[seed2042/]run_a*_c*/mct_{Slru,B6,Alru,Alfu}_1.csv (S/B renamed 11-07); plot programs stat/feature_plot/ (153ae53, see plot.md; PNGs deleted 11-07, user regenerates for slides). rr c1 seeds 2043-45 in stat/seed204{3,4,5}/run_a*_c1/.
- set1_unload/run_a*_c*/ — set 1: FIFO era, 8 policies (c2 no iwrr), old seqs (cfe3319, moved b0109ed).
- old_nonfifo/ — old semaphore-gate era: run_a*_c1, run_a*_c2, _p95 (first seqs, real ~0.95), _p57/_p60 (c1 iwrr), _p75 (c2 at rho 0.8).

## Docs
- docs/next_experiment.md — experiment record (run log not updated past 10-29). docs/results_report.md (10-29 analysis, old gate era).
- docs/next_cites.md, docs/2_Planning.md (pre-cluster), docs/Planning.md (OLD), docs/*.pdf papers. reports/proposal/main.tex.

## temp/ (gitignored)
- logs/ (batch_*.log, driver_*.log, replay_current.log), analysis_v4/ (summary + load/net/speedup CSVs).
- Drivers: run_driver.sh <case> "<alphas>" "<policies>" (saves _main.log, _site_*.log, runs lb_log_check), cluster_lib.sh (start_case, stop_all, save_site_logs), batch_fifo1.sh, batch_set2_esce.sh, batch_v3_rest.sh, batch_v4.sh, watchdog.sh (follows newest batch log, [check] PROBLEM, temps), syn_driver.sh.
- Checks / analysis: lb_log_check.py (mct argmin, aco p/tau/reward, throttled FIFO + max in flight per site <= LIMIT), analyse_v4.py, analyse_load.py (busy incl. net), analyse_net.py, model_vs_real.py, true_load_one.py, speedup.py, aco_params.py / aco_signal2.py / aco_sim.py (ACO tuning: log stats + trace sim), aco_sim2.py (sim with f/n learning, LEARNING rate test), aco_sim3.py (event sim: oracle mct, late binding, SJF), offline_opt.py (offline best case: no-wait LB + iterated local search), report_analysis.py, netem_test.py (+json: only delay 100 kept).
- Older (10-22..27): capacity.py, sim_load.py, analyse_tiers.py, analyse_timings.py, timeline.py, compare_machines.py, om_analysis.py, rs_analysis.py, batch_check.py, cap_bench.py, plot_tier_comparison.py, interference/, cpu_vs_gpu/, statv/, seq_flat/.
- syn_test/ (232 MB) — host Dana test bench: obm/ copy, test_render.py, www/, out/ (renders).
- syn_video/ (repo, not temp) — synthetic shows build.py, docs.md, shows/, assets/syn.
- [11-05] main_server.rs: serve_offload (cache check) -> serve_offload_cache (hit, send_file) | serve_offload_request (miss, adds to cache, logs [CACHE]). temp/cache_smoke.sh + temp/cache_smoke/ (100-req smoke). temp/site_video_check/ (run.sh, analyse.py, out/, grids, summary.csv). kb/env.md = all env vars. Scratchpad zcbench (sendfile bench, not kept).
- 11-06 analysis/test dirs: seg_alpha_why/ (a0.5 vs a1.2 segment repeats + LRU sim), seg_lfu_sim/ (LRU vs LFU sim, result.csv), coalesce_smoke/ (50-req om-only smoke: run.sh, check.py, seq/, run_a0.5_c2/), coalesce_race/ (236-req race test: run.sh with site-kill watcher, check.py, seq/phases.csv, run_a0.5_c2/).
- temp/cp_break_test/ (11-07): C/P break test - make_seq.py, kill_watch.py, check.py, run.sh, run_a0.5_c1/mct_{P,CP,SP,SCP}_1.* (experiments.md 'C/P break test')
