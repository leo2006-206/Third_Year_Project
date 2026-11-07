Prediction / prefetch feature: design notes (todo item 4). updated 2026-11-07
## Workload facts the design rests on (generate_request_seq.py ViewerParams)
- Viewer = Markov chain: after seg 0: continue 0.40 (combo change 0.50), switch 0.45, leave 0.15; after seg k>=1: continue 0.80 (combo change 0.10), switch 0.10, leave 0.10.
- => P(same viewer asks seg k+1 same combo): after seg 0 = 0.20, after seg k>=1 = 0.72. Depth 2 after k>=1 ~0.52.
- Next request comes 5 s after seg 0, 10 s after later segs = max lead time of a 1-ahead prefetch. Latency 16-29 s, queue 82-88% -> prefetch saves at most the head start; extra work at rho 0.8 is the main risk.
- Sim (temp/prefetch_sim.py): '1 obs' 57-61% coverage, +35-40% work; '2 obs in 30 s' 37-39% coverage, +17-20% work, ~67% precise (matches 0.72).
## Real-world designs reviewed (11-06 discussion)
- Web: Padmanabhan & Mogul 1996 dependency graph + probability threshold; PPM / Markov orders. HTTP/2 server push removed from Chrome 2022 (low accuracy, wasted bytes) -> replaced by hints (103 Early Hints, Speculation Rules: prerender only when confident).
- Streaming: CMCD (CTA-5004) 'nor' = client tells CDN its next object -> CDN prefetches. We have no per-user signal, only aggregate.
- OS: Linux readahead: detect sequential, start small window, grow when async marker page is hit (confirmed), stop on break. DB: InnoDB linear read-ahead threshold (56 pages of extent seen sequentially -> fetch next extent) = confirmation rule.
- Feedback throttling (Srinath HPCA 2007): measure accuracy, lateness, pollution online -> adjust aggressiveness.
## Design axes (to decide with user)
predictor (rule / online-learned Markov p per seg position|show) ; trigger + depth ; admission gate (p threshold x load: idle site / in-flight cap / work budget) ; priority (main-server pending queue, sites are FIFO, can't preempt) ; dedup (cached / in flight / recently prefetched) ; storage (segment cache insert vs staging store TTL, pollution) ; coalescing join + promotion ; LB placement ; metrics (accuracy, coverage, timeliness, extra work, real-request latency) ; logging.
## User expectation + rule (11-06)
- User: prefetch alone not expected to win; design it to PAIR with segment cache. Rule = '2 continue': seg k-1 and seg k of same (show, combo) seen within a window (any users, no tracking) -> issue seg k+1. Only REAL requests trigger (no chaining off prefetches).
## Where the prefetched video goes (my proposal 11-06, awaiting user)
- Timeline: trigger t (real seg k); real seg k+1 at ~t+10 s; render latency 16-29 s -> most useful prefetches still IN FLIGHT when the real request comes -> must be a coalescing leader, else double render.
- Prefetch always registers in the coalescer (its own leader); OBM_COALESCE only controls real-real joining. Avoids P depending on C flag.
- On finish: followers joined -> proven real demand -> normal segment-cache insert (same as a miss). No followers -> probation/staging slot (Linux inactive list / 2Q A1in idea), TTL ~20-30 s; hit -> serve + promote to segment cache; expiry -> drop, count wasted. Staging budget carved out of the cache cap (e.g. 90 + 10 MiB) for a fair ablation.
- Open: prefetch leader fails -> followers should render themselves (not 502), since the request was speculative.
## USER DESIGN (11-06, replaces my probation/promotion proposal) - I agreed with notes
- Predictor built on the coalescer: trigger -> coalescer.join -> prefetch = Leader -> LB + render -> finish. No promotion step.
- After finish: cache ON -> add to segment cache, cache policy decides (no special insert position). Cache OFF -> hold 30 s, then drop; readers in the window get it.
- OBM_COALESCE=0 + OBM_PREFETCH=1: only prefetches can be leaders; real requests may only FOLLOW an existing (prefetch) entry, never insert.
- My notes: (1) Leader::finish consumes self -> Drop removes map entry, so holding an Arc of the slot is NOT findable; hold = keep the Leader alive (set value without dropping, sleep 30 s in the same detached prefetch task, then drop). Needs a non-consuming set / finish_hold. (2) real request with coalesce off needs lookup-only join (try_follow). (3) prefetch leader failure -> followers render themselves, not 502. (4) cache ON: await cache add before dropping leader (no gap). (5) trigger on every real request incl. cache hits + followers; dedup = skip if cached or entry exists. (6) last segment of a show: main server must know segment count (else prefetch a non-existent seg).
## Code structure Q (11-06): copy serve_offload vs separate stages
- Already half-split: fetch_offload (LB + site, no client) + offload_cache_add (cache + leader.finish) in rust_server/src/main_server.rs. Prefetch reuses both; no copy.
- My rec: one shared front step `lookup(state, path, mode) -> Lookup { Cached(File), Follow(Follower), Lead(Option<Leader>) }`, mode Real (join / try_follow when coalesce off) vs Prefetch (skip on cache hit or existing entry). Back: real = fetch_offload -> share -> client write; prefetch = fetch_offload -> share (await cache add) -> hold Leader 30 s if cache off. Don't unify payloads into one File|Buffer enum (headers/logs differ per source).
## Decision 11-06: cache add AFTER the 30 s hold (user), + my add: only if read during flight/hold (admission filter; LRU never refuses)
## SIM temp/prefetch_sim2.py (11-06): 20 mct_none_1 traces, 5890 req, window 30 / hold 30, LRU 100 MiB, NO load feedback (latency gain = upper bound; renders = cost)
| config | mean | p90 | renders vs none | hit% | pf issued | acc | cover (in-flt/held/cache) |
| none | 17.5 | 35.6 | 0 | 0 | - | - | - |
| S | 13.9 | 33.0 | -20.4% | 20.4 | - | - | - |
| P | 14.7 | 32.5 | +10.1% | 0 | 2302 | 71% | 29.0% (1105/601/0) |
| S+P before | 11.6 | 30.2 | -9.2% | 28.3 | 2155 | - | 28.4% (1028/0/646) |
| S+P after all | 11.5 | 30.1 | -10.1% | 19.8 | 2129 | 70% | 28.7% |
| S+P after read | 11.3 | 30.1 | -10.5% | 20.2 | 2153 | 70% | 28.5% (1028/554/99) |
| S+C | 13.5 | 32.3 | -23.6% | 20.4 | - | - | - |
| S+C+P after read | 11.0 | 29.0 | -13.2% | 20.2 | 2143 | 70% | 28.4% |
- 'after read' best of the 3 add modes but only ~1-1.5% renders; S+P = +12.5% renders vs S alone (+13.6% S+C+P vs S+C) -> at rho 0.8 the latency gain is optimistic. ~1/2 of prefetch reads are joins while rendering, ~1/3 during the hold.
- Insensitive: window 15/30/60, hold 10/30/60 change renders <= 1.5 pts, acc 70-71%.
- Next: sim with load feedback (queue model) or real run.
## Research framing (11-06, user asked 'does P replace coalescing?') -> NO
- Overlap sim: real-real merges C 343, S+C 186, S+C+P 152 (P absorbs only 34 = 18% of C's merges); seg 0 = 42% of requests, never predictable by P but C dedups them.
- Framing: coalescer = shared in-flight table MECHANISM (like hardware MSHR: demand miss merges into in-flight prefetch; nginx proxy_cache_lock). Features = policies on it: C = real may lead + merge; P = predictor may lead, real merges into prefetch (P useless without that merge -> part of P by definition, state it).
- Report: P benefit split (in-flight merge / hold / cache), C-P overlap, cost under load; optional 2nd P variant with idle gate (aggressive vs throttled) as research comparison.
## IMPLEMENTATION PLAN (explained to user 11-06, not coded)
1. coalescing.rs (honest): try_follow(key) lookup-only; Leader::publish(&self, v) non-consuming (finish = publish + drop); cumulative joined counter (AtomicUsize in slot, followers() is only 'now'); speculative flag on slot (join_speculative) so a follower of a failed prefetch renders itself.
2. NEW mod_feature/prefetch.rs (honest): parse_segment(path) -> (show, combo-rest, seg = t_start/10); with_segment(path, k) (replace fields 4/5 of the RAW path, keep encoding); Predictor{last_seen HashMap<(show,combo,seg), t>, max_seg HashMap<show,u32>, window}; observe(path, now: f64) -> Option<target> (time passed in = honest, testable); prune > window. max_seg built from features.csv paths (show JSON has length_frames but no fps).
3. lib_server_main: in-flight table exists if C or P; flags coalesce_real; ServerState.prefetch Option<Prefetcher{predictor, hold}>; env OBM_PREFETCH 0/1, OBM_PREFETCH_WINDOW_S 30, OBM_PREFETCH_HOLD_S 30, OBM_PREFETCH_SEGMENTS (default features.csv). Startup print.
4. main_server: lookup(state,path,Mode) -> Lookup; serve_offload: observe at arrival -> smol::spawn(prefetch_task(Arc state, target)).detach(); prefetch_task: cache check -> join_speculative -> fetch_offload -> publish -> Timer 30 s -> joined()>0 && cache -> await add -> drop leader. Follower of prefetch: worker 'prefetch' header; None + speculative -> fetch_offload itself.
5. Logs [PREFETCH] issue/skip(reason)/done(site, t_work, t_asset)/failed/expire(N readers, cache added|dropped)/hit(wait, inflight|held). 6. temp/prefetch_log_check.py (issued = done+failed, readers == CSV worker=prefetch rows, cache add iff readers>0, LB selects = [OFFLOAD] site + prefetch renders, trigger rule re-derived from CSV) + sim on same trace for expected counts. 7. local fake-site e2e, then real S vs S+P on 8 seqs.
## STATUS 11-07: COMMITTED 7c5b8d9 (was IMPLEMENTED 11-06) - see code.md '[11-06] MERGED PREFETCH WIRED'. Next: commit (when user asks), rebuild main image, temp/prefetch_log_check.py, real runs S vs S+P (label e.g. SP) on the 8 seqs.
## Review bug #4 sized (11-07 sim, S+P after-read, LRU by last-use time, 20 traces): stamping the cache entry at hold end (current) vs at its real last read: 50 MiB hit 12.0 vs 12.8%, renders 5770 vs 5728, mean 12.65 vs 12.51 s; 100 MiB 20.2 vs 20.7%, 5271 vs 5241, 11.35 vs 11.21 s; 200 MiB no difference. Direction: current code makes S+P slightly WORSE (too-fresh prefetched entries push out real ones) -> conservative. Size ~0.1-0.15 s vs S->S+P gap ~2.6 s (<6%). LFU: readers per prefetch ~1.1 -> count 1 already right. Verdict: not needed before S vs S+P; note as known limitation (user decides).
