`generate_request_seq.py` builds **one set of viewers per α** (what each viewer watches, segment by segment), then **places those viewers in time separately for each cluster**, so each cluster's 600 s (10 min) replay window offers about 70% of what it can serve. Every cluster sees the same viewers watching the same things; only how densely they arrive differs. There are no command-line arguments: run `python3 servers_benchmark/generate_request_seq.py` and change behaviour in the settings at the top of `main()`.

## How a sequence is built

**Step 1: popularity, two levels (once per α).** Zipf with exponential cutoff:

$$p(r) = \frac{r^{-\alpha}\, e^{-r/c}}{\sum_{j=1}^{N} j^{-\alpha}\, e^{-j/c}}$$

- $r$: popularity rank (1 = most popular), $N$: number of items ranked, $\alpha$: skew (0.5 or 1.2), $c = 50$: cutoff rank.
- **Show:** ranked in the fixed `show_rank` order (f1_full, forecast, f1, forest720_leaves, spiders, forest720), e.g. f1_full 28.6% … forest720 10.6% at α = 0.5.
- **Combination within a show, per layer (default effect):** each layer's options, and the show's layouts (e.g. race/landscape vs portrait), are Zipf-ranked with the **producer's default first** (from `show_options.json`), then the rest in spec order. A combination's popularity is the product of its choices, with layers assumed independent:

$$P(\text{combination} \mid \text{show}) = P(\text{layout}) \prod_{\ell \in \text{layers}} P_\ell(\text{option}_\ell)$$

  Combinations sharing options are therefore similarly popular, and a shared part's popularity is just the product of its options. E.g. f1 at α = 1.2: all defaults 11.3%; the 7 combinations sharing "race + driver PIP + track" (differing only in the cheap driver marker) total 24.6%. That stable "heavy shared trunk" is what partial-segment caching relies on; a random ranking of whole combinations would put that share anywhere from 1.6% to 40.6% depending on the seed.
- $P(\text{kind}) = P(\text{show}) \times P(\text{combination} \mid \text{show})$.

**Step 2: viewer paths (once per α, no times yet).** Each viewer starts on a show and combination (step 1) at segment 0. While a segment plays, the player buffers the next one, so the viewer decides at that point (step 5 gives the times):

```
viewer decides (while watching)       a video's first decision │ later decisions
 ├─ leave                                                0.15 │ 0.10
 ├─ switch video                                         0.45 │ 0.10 → new show + combination (Zipf), segment 0
 └─ continue (next segment)                              0.40 │ 0.80
       ├─ keep combination                               0.50 │ 0.90
       └─ change combination                             0.50 │ 0.10 → new combination of the same show (Zipf, not the current one)
```

- A video's first decision behaves differently ("video surfing", Krishnan & Sitaraman 2012): viewers leave or switch much more often, and those who stay often try other options (change combination 0.50) before settling (0.10).
- At a video's last segment, "continue" becomes "switch video".
- Changing combination does not restart the video: the next segment is requested in the new combination.
- A session is capped at 30 segments.

Typical result: 8 segments per viewer (median 6, P90 19), about 3.3 videos and 0.9 combination changes per viewer, a 65 s session.

**Step 3: capacity (per cluster, computed once from the step-2 paths).** For each site $j$, $S_{j,k}$ is the busy time of kind $k$ on that site type, measured in the cluster's own replay runs (`run_globs`: `stat/set1_unload/run_*`, `stat/old_nonfifo/run_*`, folders `run_a<α>_<c1|c2>*`) as the mean of asset + work + net. Net counts because it is almost all Dana fetching the requested layer options' resources, which holds the site (the FIFO queue shows a site is occupied for asset + work + net). The runs therefore already include the netem delay, the load and the co-located containers, and case 2 uses its own 4-gpu runs. Kinds a site type never ran (45–60 of 145) use the no-delay calibration time (`stat/<machine>_<port>_<tier>_timing.csv`; cpu sites extrapolate from 8 measured kinds) times that site type's median run/calibration ratio (om gpu 1.35, llvm 1.31, cpu 0.79; rs gpu 2.20, llvm 1.92, cpu 1.06). On the set 1 sequences this predicts the measured busy fraction within 0.001–0.03 (the earlier calibration + 50 round-trip model was off by up to 0.07). Each site's mean busy time weights every kind by how often it is *requested*, $q(k)$, its share of all requests in the step-2 paths (about 24,000 requests from 3,000 viewers):

$$E_j = \sum_k q(k)\, S_{j,k}, \qquad q(k) \approx \frac{P(\text{show})\, \mathbb{E}[\text{segments} \mid \text{show}]\, P(\text{combination} \mid \text{show})}{\sum (\cdot)}, \qquad C = \sum_{j} \frac{1}{E_j}$$

$q$ differs from $P(\text{kind})$ because a viewer requests several segments of a show: the 30 s forest shows (at most 3 segments) get fewer requests than their popularity, the 90 s shows more (α = 0.5: forest720 0.106 → 0.074, f1_full 0.286 → 0.328). Counting $q$ from the simulated paths gives the exact value for the viewer model without deriving $\mathbb{E}[\text{segments} \mid \text{show}]$, and it does not depend on $N$ (paths come before timing), so there is no loop.

Each site serves one request at a time, so it completes $1/E_j$ requests per second; the cluster's capacity is the sum.

**Step 4: target load (per cluster).** The window's realised load must be $\rho$ ($\rho = 0.8$, $T = 600$ s): its request count divided by $C_w T$, where $C_w$ is the capacity (step 3) under the window's *own* mix rather than $q$.

**Step 5: place viewers in time (per cluster).** Viewer arrivals are a Poisson process, i.e. exponential gaps between viewers, $f(\Delta t) = \lambda_v e^{-\lambda_v \Delta t}$:

$$T_v = -T_{\text{pre}} + \frac{1}{\lambda_v}\sum_{i \le v} \Delta u_i, \quad \Delta u_i \sim \text{Exp}(1), \qquad t_{v,k} = T_v + \sum_{i < k} g_i, \quad g_i = \begin{cases} 5 \text{ s} & \text{step } i \text{ is a video's segment 0} \\ 10 \text{ s} & \text{otherwise} \end{cases}$$

- Each viewer follows its own playback clock, independent of other viewers. As in the OBM player, which buffers the next segment while the current one plays, a video's first decision comes 5 s after its segment 0 request and later ones every 10 s: a video started at $t$ requests at $t$, $t+5$, $t+15$, $t+25$, … A switch or leave at the first decision also happens at $t+5$.
- The two requests 5 s apart at every video start make arrivals burstier than evenly paced viewers (variance/mean of arrivals per 10 s rises from 0.93 to about 1.2; Poisson = 1), so queues form more often at the same load.
- The same $\text{Exp}(1)$ draws are used for every cluster; only $\lambda_v$ differs.
- **Pre-roll** $T_{\text{pre}} = 600$ s: viewers start before $t = 0$, so the window opens on an audience already mid-session (no empty start). Requests after $T$ are simply dropped, so there is no pile-up at the end.
- $\lambda_v$ is fitted by bisection so the window's realised load is $\rho$ (within 0.002).

**Representative window.** Only about 35–50 viewers fall in the window and each stays on a show for many segments, so the window is effectively 40–50 show choices, and its mix can be far from $q$. In set 1 (`sequences/set1_unload/`, fitted on a request count), α 0.5 had forecast at 12% against 22% expected and f1 at 23% against 17%, which made it about 0.05 lighter than its target. Two fixes: step 4 fits the load on the window's own mix, so it is $\rho$ whatever the mix, and steps 2–5 are repeated with a fresh viewer population (the next child of the content and decision random streams; the arrival gaps stay the same) until, in every cluster's window, each show's share is within `show_share_tol` (0.03) of its share of all generated requests. The search takes a few to a few dozen attempts (seconds). It is a curated sample: the window is chosen to match the model, which should be stated with the results. `meta.csv` records `real_rho`, the accepted `attempt`, the largest show-share gap (`show_share_dev`) and the share of requests in the first half (`first_half`).

**Independent random seeds.** `seed` is split into four streams: viewer arrival gaps, show ranking (only used if `show_rank` is `None`), content choices (show/combination), and viewer decisions (leave/switch/continue/change). Changing α only changes which shows and combinations are drawn.

## Example (case 1 = main laptop + second laptop, gpu/llvm/cpu sites each)

| α | Capacity (req / 600 s) | Requests in window | Viewers in window | Real load | λ_req | λ_viewers |
|---|---|---|---|---|---|---|
| 0.5 | 379 | 303 | 35 | 0.75 | 0.51 /s | 0.044 /s (one every ~23 s) |
| 1.2 | 379 | 303 | 33 | 0.82 | 0.51 /s | 0.044 /s |

## Inputs (files it reads)

| File | Used for |
|---|---|
| `servers_benchmark/request_kind.txt` | the 145 request kinds (URL templates with `t_start/t_end`) |
| `servers_benchmark/video_duration.csv` | number of 10 s segments per show |
| `servers_benchmark/show_options.json` | layouts, layers, options and **defaults** per show, for combination popularity |
| `stat/<run_glob>/run_a<α>_<c1\|c2>*/*.csv` | seq_bench replay results: busy time (asset + work + net) per kind on each site type, for capacity |
| `stat/<machine>_{7010_gpu,7020_llvm,7030_cpu}_timing.csv` | no-delay calibration: only fills kinds a site type never ran in the replay runs (scaled by its run/calibration ratio) |

## Settings you can change (in `main()`)

| Setting | Current value | What it controls |
|---|---|---|
| `alphas` | `[0.5, 1.2]` | Zipf skew; one set of viewers per value |
| `show_rank` | f1_full, forecast, f1, forest720_leaves, spiders, forest720 | show popularity order (`None` = seeded random order) |
| `zipf_cutoff` | `50.0` | exponential cutoff; barely matters here (at most 7 options per layer, 6 shows) |
| `viewer.p_first_continue` / `p_first_switch` | `0.40` / `0.45` | a video's first decision (rest leave) |
| `viewer.p_continue` / `p_switch` | `0.80` / `0.10` | later decisions (rest leave) |
| `viewer.p_combo_first` / `p_combo` | `0.50` / `0.10` | chance of changing combination when continuing, at a video's first / later decisions (no published data: sensitivity parameters) |
| `viewer.max_segments` | `30` | session cap |
| `viewer.first_gap_s` / `segment_gap_s` | `5.0` / `10.0` | time from a video's segment 0 request to its first decision / between later requests |
| `n_viewers` | `3000` | viewers generated (must outlast pre-roll + window; the script warns otherwise) |
| `seed` | `2037` | picked from 2000–2047 so all four sequences share one shape (two waves: rise to ~1.0, drain near 350 s, second rise to 1.3–1.55; pairwise correlation of the 60 s load ≥ 0.78; set 1 used 2026, whose sequences did not share a shape). Change for a different repeat of the same setup |
| `duration_s` / `pre_roll_s` | `600` / `600` | replay window / warm-up before it |
| `rho` | `0.8` | target offered load as a fraction of cluster capacity (realised value in `meta.csv`). Was 0.7; raised because the realised load came out lower (window mix) |
| `run_globs` | `set1_unload/run_*`, `old_nonfifo/run_*` (under `stat/`) | replay runs whose asset + work + net per kind give the busy times; all eras agree within ~2% per kind (om cpu ~8%). Replaces the old network allowance (`net_round_trips` 50 × netem delay = 1.5 s om, 3.0 s rs on top of calibration) |
| `show_share_tol` / `max_attempts` | `0.03` / `2000` | representative window: largest allowed gap between a show's share in a cluster's window and its share of all generated requests; viewer populations tried before giving up |
| `clusters` | case 1: gpu+llvm+cpu on `om` and `rs`; case 2: 2 gpu on each | which `(machine, tier)` sites each cluster has, in port order per machine (7010, 7020, ...), so a run's worker maps to its tier |

## Outputs (`servers_benchmark/sequences/`)

| File | Contents |
|---|---|
| `seq_a{α}_viewers.csv` | every viewer's path: `user_id, step, offset_s, kind_idx, show, seg_start, seg_end` (`offset_s` = time after the viewer's start) |
| `seq_a{α}_{case}.csv` | **the replay file:** `req_idx, t_ms, path`, within 0–600 s |
| `meta.csv` | target and realised load (`real_rho`), capacity, λ_req, λ_viewers, mean segments per viewer, requests and viewers in window, accepted attempt, largest show-share gap, first-half share, per α and cluster |

A replayer sends `path` at `t_ms`.
