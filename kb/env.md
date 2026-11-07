Every environment variable that changes an experiment: where it is read, default, effect, how it reaches the process. updated 2026-11-07
RULE: a new env var -> add a row here in the same step (user 11-05). Each run's main log prints the values (driver 'main server env:' + main server startup lines).

## Main server (rust_server/src/main_server.rs + lib/lib_server_main.rs; runs in container obm-main-server)
How it gets there: temp/cluster_lib.sh start_case passes EVERY OBM_* var set in the driver's shell into the container (`-e NAME`, since 11-05) and logs them ('main server env: ...'). servers_container/main_server/run_sh.sh (interactive) does NOT pass any -> add -e by hand there.
| var | default | effect | read in |
|---|---|---|---|
| OBM_SEGMENT_CACHE_MB | unset = OFF (0 = OFF too) | segment cache (LruCache) byte cap in MiB; bad value = server exits with error; startup prints 'Segment cache ON: N MB' / 'OFF'; log lines [CACHE] added/add failed/skip | lib_main_state::segment_cache_loader |
| OBM_SEGMENT_CACHE_DIR | /tmp/obm_cache/segment (inside container, wiped at start) | segment cache folder | lib_main_state::segment_cache_loader |
| OBM_BIN_CACHE_LOWER_MB | unset = 0 | bin cache lower bin (cost < threshold) MiB; bins ON if LOWER+UPPER > 0; set together with OBM_SEGMENT_CACHE_MB = server exits (same feature); startup 'Segment cache ON: bins lower X MB, upper Y MB, threshold T s'; [CACHE] lines add ' cost N.NN s'; driver refuses with FEATURES=none | lib_main_state::segment_cache_loader |
| OBM_BIN_CACHE_UPPER_MB | unset = 0 | bin cache upper bin (cost >= threshold) MiB; see LOWER | lib_main_state::segment_cache_loader |
| OBM_BIN_CACHE_THRESHOLD_S | 6.0 | bin split, cost = (asset_ms + work_ms/render factor gpu 1, llvm 1.33, cpu 4.6)/1000; bad value / <=0 = server exits | lib_main_state::segment_cache_loader, BinCache::new |
| OBM_BIN_CACHE_DIR | /tmp/obm_cache/bin (subdirs lower/ upper/, wiped at start) | bin cache folder | lib_main_state::segment_cache_loader |
| OBM_COST_COEF | /app/stat_predict/coef.csv | cost model coefficients (mct, aco, mct_lookup fill) | lib_main_state::load_cost_model |
| OBM_COST_FEATURES | /app/stat_predict/features.csv | cost model feature list | lib_main_state::load_cost_model |
| OBM_COST_LOOKUP | /app/stat_predict/lookup.csv | mct_lookup table | lib_main_state::load_cost_lookup |
| OBM_MCT_FALLBACK | unset (only "1" = on) | mct/mct_lookup: fall back to const work when the model fails to load (else exit) | main_server.rs main() |
| OBM_ACO_SEED | 2026 | ACO random seed | lib_main_state::aco_seed |
All planned feature vars are in code: OBM_ASSET_CACHE_* (site, b3750a1), OBM_BIN_CACHE_* (4a07eb3), OBM_COALESCE (e0ba1c1), OBM_PREFETCH* (7c5b8d9). Loaders live in rust_server/src/lib/lib_main_state.rs (moved from lib_server_main 11-06).

## Replay driver (temp/run_driver.sh, temp/cluster_lib.sh; gitignored)
| var | default | effect |
|---|---|---|
| FEATURES | none | seq_bench feature label -> <policy>_<FEATURES>_N.csv. Driver REFUSES OBM_SEGMENT_CACHE_MB (non-0) with FEATURES=none (mislabel guard). Naming idea: S = segment cache (seq_bench help: SA = segment + asset). |
| SEQ_DIR | servers_benchmark/sequences | sequence set (seed dirs servers_benchmark/sequences/seedS) |
| RUN_ROOT | stat | output root (e.g. stat/seed2042) |
| START_RETRY_S | 600 | start_case_retry: keep retrying site start every 60 s for this long |
Example feature run: `OBM_SEGMENT_CACHE_MB=45 FEATURES=S temp/run_driver.sh 2 0.5 mct`

## Offload sites (servers_container/offload_server/run_sh.sh -> container env)
| var | default | effect |
|---|---|---|
| MAIN_SERVER_ADDR | 10.10.0.1:7000 | where the site fetches assets/shows (offload_server.rs); run_all_offload.py forwards it to rs too |
| NETEM_DELAY_MS / NETEM_JITTER_MS | from offload_*_c*.csv delay_ms/jitter_ms (om 30/3, rs 60/6) | netem in entrypoint.sh (with rate 1gbit) |
| LP_NUM_THREADS | ceil(cpu cap) | llvmpipe threads, set by run_sh.sh when a CPU cap is given |
(GPU_CARD / RENDER_DEV / SDL_* / LIBGL_* are internal to entrypoint.sh, not experiment knobs.)

## Gotchas
- Container image must be REBUILT (cargo build --release, then docker build main_server) before feature vars do anything: the image binary is from before the segment cache code (as of 11-05 evening).
- Cache hit rows in seq_bench CSV: worker=cache, all timing cols 0, client_net_ms empty -> per-site analysis / lb_log_check must skip them.

## servers_container/env_setting.txt (11-06, user asked)
- Docker env-file format (KEY=value, '#' comment lines only, no quotes/inline comments). Holds OBM_SEGMENT_CACHE_MB=0, OBM_BIN_CACHE_LOWER_MB=0, OBM_BIN_CACHE_UPPER_MB=0, OBM_BIN_CACHE_THRESHOLD_S=6, OBM_MCT_FALLBACK=0, OBM_ACO_SEED=2026 (all = old defaults -> no-feature runs unchanged).
- temp/cluster_lib.sh load_env_setting (at source time): exports every OBM_* line NOT already set in the shell (shell wins); file path ENV_SETTING (default servers_container/env_setting.txt). run_driver now sources cluster_lib BEFORE the FEATURES guard so file cache values are guarded too. servers_container/main_server/run_sh.sh: docker run --env-file env_setting.txt.
- Side effect: 'main server env:' log now always lists these 6 vars (was 'none'); nothing parses it.
- Tested 11-06: defaults, shell override, guard (rc 2), docker --env-file parse, server reads (bins ON / OFF).

## Asset cache + cache policy vars (11-06, committed b3750a1)
| OBM_COALESCE | 0 (env_setting.txt) / unset = OFF | 1 = request coalescing on the main server (followers worker 'coalesce', header X-OBM-Coalesce ms); other value = main exits; startup 'Coalescing ON/OFF'; run_driver refuses it with FEATURES=none (added to guard list 11-06) | lib_main_state::coalesce_loader |
| OBM_SEGMENT_CACHE_POLICY | lru | lru or lfu for the single segment cache (SegmentCache::Single); bad value = main exits | lib_main_state::segment_cache_loader |
| OBM_PREFETCH | unset/0 = OFF | merged prefetch ('2 continue': k-1 + k of same show+combo within window -> prefetch k+1); prints 'Prefetch ON: window W s, hold H s' / 'OFF'; builds the in-flight table even with OBM_COALESCE=0 (real requests then only FOLLOW prefetches); main log [PREFETCH] issue/skip/done/failed/expire/hit/leader failed; CSV worker 'prefetch'; driver guard refuses with FEATURES=none | lib_main_state::prefetch_loader |
| OBM_PREFETCH_WINDOW_S | 30 | k-1 / k max gap (s) | lib_main_state::prefetch_loader |
| OBM_PREFETCH_HOLD_S | 30 | a finished prefetch stays in the table (readable) this long; then cache add only if read (joined > 0) | lib_main_state::prefetch_loader |
| OBM_ASSET_CACHE_MB | unset/0 = OFF | per-site asset cache cap MiB (offload_server, AssetCache = PolicyCache); site prints 'Asset cache ON: N MB, <policy>' / 'OFF'; site log [ASSET] hit / added / add failed / skip; driver refuses with FEATURES=none | offl::asset_cache_loader |
| OBM_ASSET_CACHE_POLICY | lru (env_setting.txt sets lfu) | lru or lfu | offl::asset_cache_loader |
| OBM_ASSET_CACHE_DIR | /tmp/obm_cache/asset (in site container, wiped at start) | asset cache folder | offl::asset_cache_loader |
Path to the site container: om = shell env -> run_all_offload.py (os.environ) -> offload run_sh.sh (forwards OBM_ASSET_* with -e) -> container; rs = cluster_lib start_case puts 'env OBM_ASSET_...=..' into the ssh command (logs 'site env:'). rs only has the code after commit + user push + rs pull.
