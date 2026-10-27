`generate_request_seq.py` builds **one set of viewers per α** (what each viewer watches, segment by segment), then **places those viewers in time separately for each cluster**, so each cluster's 300 s replay window offers exactly 70% of what it can serve. Every cluster sees the same viewers watching the same things; only how densely they arrive differs. There are no command-line arguments: run `python3 servers_benchmark/generate_request_seq.py` and change behaviour in the settings at the top of `main()`.

## How a sequence is built

**Step 1: popularity, two levels (once per α).** Zipf with exponential cutoff:

$$p(r) = \frac{r^{-\alpha}\, e^{-r/c}}{\sum_{j=1}^{N} j^{-\alpha}\, e^{-j/c}}$$

- $r$: popularity rank (1 = most popular), $N$: number of items ranked, $\alpha$: skew (0.5 or 1.2), $c = 50$: cutoff rank.
- **Show:** ranked in the fixed `show_rank` order (f1_full, forecast, f1, forest720_leaves, spiders, forest720), e.g. f1_full 28.6% … forest720 10.6% at α = 0.5.
- **Combination within a show:** ranked by a seeded shuffle (no popularity data exists for combinations).
- $P(\text{kind}) = P(\text{show}) \times P(\text{combination} \mid \text{show})$.

**Step 2: viewer paths (once per α, no times yet).** Each viewer starts on a show and combination (step 1) at segment 0. At the end of every segment it decides:

```
viewer finishes a segment
 ├─ leave                     first segment of a video: 0.15 │ later segments: 0.10
 ├─ switch video                                        0.45 │                 0.10 → new show + combination (Zipf), segment 0
 └─ continue (next segment)                             0.40 │                 0.80
       ├─ keep combination    0.90
       └─ change combination  0.10 → new combination of the same show (Zipf, not the current one)
```

- Abandonment is much higher after the first segment of a video ("video surfing", Krishnan & Sitaraman 2012).
- At a video's last segment, "continue" becomes "switch video".
- Changing combination does not restart the video: playback continues at the next segment.
- A session is capped at 30 segments (5 min of video).

Typical result: 8 segments per viewer (median 6, P90 19), about 3.3 videos and 0.45 combination changes per viewer.

**Step 3: capacity (per cluster).** For each site $j$, $S_{j,k}$ is the measured busy time (asset + work) of kind $k$ from `stat/`; cpu sites extrapolate unmeasured kinds from 8 representative ones. With $w_k$ the share of kind $k$ among requests in the window:

$$E_j = \sum_k w_k\, S_{j,k}, \qquad C = \sum_{j} \frac{1}{E_j}$$

**Step 4: target load (per cluster).** The window must hold exactly $N = \rho\, C\, T$ requests ($\rho = 0.7$, $T = 300$ s).

**Step 5: place viewers in time (per cluster).** Viewer arrivals are a Poisson process, i.e. exponential gaps between viewers, $f(\Delta t) = \lambda_v e^{-\lambda_v \Delta t}$:

$$T_v = -T_{\text{pre}} + \frac{1}{\lambda_v}\sum_{i \le v} \Delta u_i, \quad \Delta u_i \sim \text{Exp}(1), \qquad t_{v,k} = T_v + 10\,k \text{ s}$$

- Each viewer requests its $k$-th segment 10 s after the previous one (own playback clock, independent of other viewers).
- The same $\text{Exp}(1)$ draws are used for every cluster; only $\lambda_v$ differs.
- **Pre-roll** $T_{\text{pre}} = 600$ s: viewers start before $t = 0$, so the window opens on an audience already mid-session (no empty start). Requests after $T$ are simply dropped, so there is no pile-up at the end.
- $\lambda_v$ is fitted by bisection so the window holds exactly $N$ requests. Sessions make the realised count vary a lot (±22% between seeds), so fitting keeps every run at exactly ρ = 0.7.
- The window's mix ($w_k$) depends on $\lambda_v$ and vice versa, so steps 3–5 repeat until the window stops changing.

**Independent random seeds.** `seed` is split into four streams: viewer arrival gaps, combination ranking, content choices (show/combination), and viewer decisions (leave/switch/continue/change). Changing α only changes which shows and combinations are drawn.

## Example (case 1 = main laptop + second laptop, gpu/llvm/cpu sites each)

| α | Capacity (req / 300 s) | Requests in window | Viewers in window | λ_req | λ_viewers |
|---|---|---|---|---|---|
| 0.5 | 237 | 166 | 24 | 0.55 /s | 0.054 /s (one every ~19 s) |
| 1.2 | 264 | 185 | 25 | 0.62 /s | 0.054 /s |

About 55–59% of requests are a viewer's next segment, and about 40% of requests repeat an earlier (combination, segment).

## Inputs (files it reads)

| File | Used for |
|---|---|
| `servers_benchmark/request_kind.txt` | the 145 request kinds (URL templates with `t_start/t_end`) |
| `servers_benchmark/video_duration.csv` | number of 10 s segments per show |
| `stat/<machine>_{7010_gpu,7020_llvm,7030_cpu}_timing.csv` | per-kind busy time on each site type, for capacity; loaded for every machine the clusters use (now `om`, `rs`) |

## Settings you can change (in `main()`)

| Setting | Current value | What it controls |
|---|---|---|
| `alphas` | `[0.5, 1.2]` | Zipf skew; one set of viewers per value |
| `show_rank` | f1_full, forecast, f1, forest720_leaves, spiders, forest720 | show popularity order (`None` = seeded random order) |
| `zipf_cutoff` | `50.0` | exponential cutoff; matters for combinations (up to 66 per show) |
| `viewer.p_first_continue` / `p_first_switch` | `0.40` / `0.45` | decision after a video's first segment (rest leave) |
| `viewer.p_continue` / `p_switch` | `0.80` / `0.10` | decision after later segments (rest leave) |
| `viewer.p_combo` | `0.10` | chance of changing combination when continuing (no published data: a sensitivity parameter) |
| `viewer.max_segments` | `30` | session cap |
| `viewer.segment_gap_s` | `10.0` | time between a viewer's requests |
| `n_viewers` | `3000` | viewers generated (must outlast pre-roll + window; the script warns otherwise) |
| `seed` | `2026` | change for a different repeat of the same setup |
| `duration_s` / `pre_roll_s` | `300` / `600` | replay window / warm-up before it |
| `rho` | `0.7` | offered load as a fraction of cluster capacity |
| `clusters` | case 1: gpu+llvm+cpu on `om` and `rs`; case 2: 2 gpu on each (provisional) | which `(machine, tier)` sites each cluster has |

## Outputs (`servers_benchmark/sequences/`)

| File | Contents |
|---|---|
| `seq_a{α}_viewers.csv` | every viewer's path: `user_id, step, kind_idx, show, seg_start, seg_end` (no times) |
| `seq_a{α}_{case}.csv` | **the replay file:** `req_idx, t_ms, user_id, step, kind_idx, show, seg_start, seg_end, path`, within 0–300 s |
| `meta.csv` | capacity, λ_req, λ_viewers, mean segments per viewer, requests and viewers in window, per α and cluster |

A replayer only needs to send `path` at `t_ms`; `seq_bench.py` also copies `user_id` and `step` into its results for per-viewer analysis.
