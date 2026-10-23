`generate_request_seq.py` builds **one ordered list of requests per α**: which user asks for which video segment, and in what order. It then **stretches that list in time separately for each cluster**, so each cluster receives 70% of what it can serve. There are no command-line arguments: run `python3 servers_benchmark/generate_request_seq.py`, and change behaviour by editing the settings at the top of `main()`.

## How a sequence is built

**Step 1: arrival times, in abstract units.**
- It draws 1000 arrival times from a Poisson process where the average gap between requests is 1 "unit".
- Times have no seconds yet; step 5 converts them for each cluster.

**Step 2: video popularity, two levels.**
- **Video:** the six shows get Zipf probabilities in your `show_rank` order, for example f1_full 47.6% … forest720 5.0% at α = 1.2.
- **Combination:** within each show, its request kinds (layer-option combinations) get another Zipf. Their ranking comes from a seeded shuffle.
- **Result:** each kind's probability = P(show) × P(combination | show).

**Step 3: assign each arrival to a user.** At each arrival time t:
1. Find users who are waiting to make their next request and whose last request was at least `min_gap_units` (5) ago.
2. If any are ready, the one who has waited longest gets this arrival and asks for its next segment.
3. If nobody is ready, a **new user** arrives and picks a video by step 2's probabilities. They start at segment 0, or at a random segment 10% of the time (`p_seek`).

So the number of users isn't an input; it comes out of the process.

**Step 4: decide that user's next move.**

| Roll | Action |
|---|---|
| 70% (`p_like`) | continue to the next segment of the same video (if it was the last segment, switch instead) |
| 15% (`p_switch`) | switch: pick a new video by popularity, segment 0 |
| 15% | leave: never seen again |

A user who stays becomes ready again at t + 5 units.

A small example:
```
t=0.0  nobody ready  -> new user 0 picks f1_full/race…  seg 0-10
t=0.7  nobody ready  -> new user 1 picks forecast/BSL…  seg 0-10
t=5.2  user 0 ready (0.0+5) -> user 0 asks           seg 10-20
t=5.9  user 1 ready         -> user 1 asks           seg 10-20
t=6.3  nobody ready  -> new user 2 ...
```

**Step 5: scale to each cluster.**
1. **Cost per site:** for each site in the cluster, a request's cost is its busy time (asset + work) from your measured CSVs in `stat/`. The cpu sites estimate unmeasured kinds from the 8 representative ones.
2. **Capacity:** cluster capacity C is the sum over sites of 1 / (that site's mean cost over the requests in the window).
3. **Arrival rate:** λ = ρ × C, for example 0.71 requests/s for case 1 at α 0.5.
4. **Seconds:** each request's time is `t_ms = t_unit / λ × 1000`. Only requests before `duration_s` (300 s) are kept, giving 194–217 requests.
5. **Fixed point:** which requests fall inside 300 s depends on λ, and λ depends on them, so it repeats this a few times until the window stops changing.

**Independent random seeds.** `seed` is split into three streams: arrivals, ranking, and user decisions. Changing α changes only which videos get picked; the arrival times and continue/switch/leave rolls stay identical. All clusters get exactly the same order of requests, just at different speeds.

## Inputs (files it reads)

| File | Used for |
|---|---|
| `servers_benchmark/request_kind.txt` | the 145 request kinds (URL templates with `t_start/t_end`) |
| `servers_benchmark/video_duration.csv` | number of 10 s segments per show |
| `stat/{lap,lab}_{7010_gpu,7020_llvm,7030_cpu}_timing.csv` | per-kind busy time on each site type, for capacity |

## Settings you can change (in `main()`)

| Setting | Current value | What it controls |
|---|---|---|
| `alphas` | `[0.5, 1.2]` | Zipf skew; one sequence per value |
| `show_rank` | your 6-show ranking | video popularity order (`None` = random order from the seed) |
| `zipf_cutoff` | `50.0` | exponential cutoff; matters for combinations (up to 66 per show), not for the 6 shows |
| `n_requests` | `1000` | length of the full sequence; needs to be larger than what fits in the window |
| `seed` | `2026` | change for a different repeat of the same setup |
| `session.p_like` | `0.70` | chance to continue to the next segment |
| `session.p_switch` | `0.15` | chance to switch video (the rest leave) |
| `session.p_seek` | `0.10` | chance a new video starts mid-way |
| `session.min_gap_units` | `5.0` | minimum gap between one user's requests, in units (about 7 s at case 1's rate) |
| `duration_s` | `300` | replay window |
| `rho` | `0.7` | target load as a fraction of cluster capacity |
| `clusters` | case 1 (6 sites), case 2 (4 gpu, provisional) | which `(machine, tier)` sites each cluster has |

**Arguments to `main()`**, for use from Python rather than the command line:
- `popularity_mode`: `"two_level"` (default) or `"flat"` (one Zipf over all 145 kinds).
- `out_dir`: where to write the files (default `servers_benchmark/sequences/`).

## Outputs (`servers_benchmark/sequences/`)

| File | Contents |
|---|---|
| `seq_a{α}.csv` | full 1000-request sequence, with times in units (`t_unit`) |
| `seq_a{α}_{case}.csv` | **the replay file:** `req_idx, t_ms, user_id, kind_idx, show, seg_start, seg_end, path`, truncated at 300 s |
| `meta.csv` | capacity, λ and request count for each α and cluster |

A replayer only needs to send `path` at `t_ms`; the other columns are for analysis.