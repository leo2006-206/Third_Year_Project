# Results Report: Baseline Policies on the Two-Laptop Cluster

Data: `stat/run_a{0.5,1.2}_c{1,2}_p95_nonfifo/*.csv` and `*_main.log`, produced 2026-10-28/29. Analysis script (gitignored):
`temp/report_analysis.py`, full output in `temp/report_out.txt`. All times are in seconds unless stated otherwise.

**Summary**

- **Queueing dominates.** 82–88% of every request's latency is time waiting in a site's queue.
  Actual work is only 7–12%. The policy decides the result; the rendering speed matters much less.
- **The cluster ran much closer to full load than planned.** The sequences were built for a load of
  ρ = 0.7, but sites were really busy about 30% longer per request than the calibration assumed. The
  cause is network-bound overhead per request (asset fetch, connection set-up, sending the result). The
  realised load is about 0.95 overall, and individual sites run at 0.7–3.0.
- **Round robin.** Requests on gpu/llvm sites wait little (those sites run at ρ 0.6–0.9). But the two
  cpu sites run at ρ 1.7–3.0, so their queues grow without bound, and they cause *all* 17–49 failures
  (requests cut off at 600 s). A good median, a very bad tail.
- **iwrr 3:3:1.** Close to the real capacity ratio (about 3 : 2.7 : 1). Every site ends up near
  ρ ≈ 1. Failures disappear and the tail is halved, but the median doubles, because no site has
  spare capacity left.
- **The rs sites compute 16–31% faster than om, yet sit in a longer queue.** Each request costs rs
  about 2.3 s more in network round trips (60 ms vs 30 ms delay), which more than cancels the faster
  rendering.
- **Comparing α 1.2 with α 0.5 is confounded.** The α 1.2 sequences have about 7–10% higher realised
  load and more of the heavy `forecast` show. The α effect cannot be separated from the load effect.
- **Repeatability is good at run level** (p50 within 1–2 s, failures 17 vs 21). Per-site figures
  vary by ±10–20% between repeats.

---

## 1. Experiment Factors and Runs

| Factor | Levels | Notes |
|---|---|---|
| Popularity skew α | 0.5, 1.2 | Zipf exponent in the viewer model; one sequence per α and case |
| Cluster case | c1: gpu + llvm + cpu on each machine; c2: 2 gpu on each machine | 6 sites vs 4 |
| Policy | rr, iwrr331 (gpu:llvm:cpu = 3:3:1), iwrr551 (5:5:1, running) | In c2, iwrr ≡ rr (all weights equal) |
| Machine | om (near, 30±3 ms), rs (far, 60±6 ms, ~1.3× faster CPU/GPU) | Fixed in every run |
| Feature | none | No cache in any run |

| Run | Sequence | Requests | Planned / realised ρ (calibrated) |
|---|---|---|---|
| a0.5 c1 rr_none_1 | seq_a0.5_case1 | 370 | 0.7 / 0.689 (old run: 600 s client timeout, no main-server log) |
| a0.5 c1 rr_none_2, iwrr331_none_1 | seq_a0.5_case1 | 370 | 0.7 / 0.689 |
| a1.2 c1 rr_none_1, iwrr331_none_1 | seq_a1.2_case1 | 370 | 0.7 / 0.734 |
| a0.5 c2 rr_none_1, iwrr_none_1 | seq_a0.5_case2 | 336 | 0.7 / 0.669 |
| a1.2 c2 rr_none_1, iwrr_none_1 | seq_a1.2_case2 | 345 | 0.7 / 0.734 |

Arrivals span 0–600 s. The main server cuts a request off 600 s after it is sent (HTTP 504), so a
replay lasts at most about 20 min. The client never fell behind schedule (max `lag_ms` 2 ms).

## 2. Run Overview

Latency is end to end at the client, over successful requests. "Mean (all)" counts failures at
their 600 s cut-off. "Makespan" is when the last response arrived.

| Run | ok / fail | mean | mean (all) | p50 | p90 | p99 | max | makespan |
|---|---|---|---|---|---|---|---|---|
| a0.5 c1 rr_1 | 353 / 17 | 85.9 | 109.5 | 17.4 | 299.8 | 549.4 | 594 | 1189 |
| a0.5 c1 rr_2 | 349 / 21 | 79.2 | 108.6 | 16.7 | 274.9 | 497.5 | 599 | 1198 |
| a0.5 c1 iwrr331 | 370 / 0 | 53.8 | 53.8 | 37.4 | 121.7 | 267.5 | 517 | 708 |
| a1.2 c1 rr | 321 / 49 | 93.0 | 160.2 | 35.6 | 293.0 | 556.8 | 597 | 1196 |
| a1.2 c1 iwrr331 | 369 / 1 | 94.7 | 96.1 | 67.5 | 207.6 | 394.8 | 514 | 818 |
| a0.5 c2 rr | 336 / 0 | 41.0 | 41.0 | 26.7 | 89.8 | 193.8 | 218 | 637 |
| a0.5 c2 iwrr | 336 / 0 | 41.4 | 41.4 | 25.5 | 100.5 | 231.6 | 248 | 648 |
| a1.2 c2 rr | 345 / 0 | 61.9 | 61.9 | 40.4 | 145.1 | 260.1 | 411 | 742 |
| a1.2 c2 iwrr | 345 / 0 | 63.4 | 63.4 | 41.0 | 152.0 | 333.3 | 398 | 754 |

**Where latency goes** (share of total latency, successful requests):

| Run | queue | asset | work | net | body |
|---|---|---|---|---|---|
| a0.5 c1 rr_2 | 86.3% | 1.6% | 9.9% | 1.5% | 0.7% |
| a0.5 c1 iwrr331 | 81.8% | 2.4% | 12.3% | 2.4% | 1.1% |
| a1.2 c1 rr | 87.3% | 1.7% | 8.9% | 1.4% | 0.7% |
| a1.2 c1 iwrr331 | 88.4% | 1.7% | 7.9% | 1.5% | 0.6% |
| a0.5 c2 rr | 82.9% | 3.3% | 9.2% | 3.2% | 1.5% |
| a1.2 c2 rr | 87.3% | 2.6% | 6.8% | 2.2% | 1.0% |

Column definitions: `queue` is the wait in the Dana queue before the request starts; `asset` is the
asset fetch, and also contains a second wait (see §9); `work` is decode + render + encode; `net` is
the main server's elapsed time up to the response header minus queue, asset and work; `body` is the
transfer of the response body plus the hop from the main server to the client.

## 3. How Each Site Works

### 3.1 Site occupancy: the real service time

A Dana site handles one request at a time in a single loop. Two steps in that loop wait on the
network and are not counted in `work`:
- asset loading, where `resource()` fetches files from the main server one by one;
- sending the result, where `sendDataChunks` pushes about 2.3 MB before the next request starts.

There is also no asset cache on the site: the Rust proxy on each site opens a new TCP connection to
the main server for every asset file, every time.

I measured **occupancy** (how long a request really holds a site) as the gap between consecutive
response headers while the site had a backlog (both requests queued more than 3 s):

| Site | work | occupancy | overhead (occ − work) | calibrated work (idle) |
|---|---|---|---|---|
| om-gpu | 4.7–5.3 | 6.4–7.5 | ~1.7–2.2 | ≈ in-situ / 0.93 |
| rs-gpu | 3.3–4.3 | 7.0–8.9 | ~3.7–4.6 | ≈ in-situ / 0.91 |
| om-llvm | 5.3–6.4 | 7.5–8.7 | ~2.2–2.3 | ≈ in-situ / 1.15 |
| rs-llvm | 4.4–4.8 | 8.3–9.5 | ~3.9–4.7 | ≈ in-situ / 1.0 |
| om-cpu | 18–29 | 20.7–30.6 | ~2 | ≈ in-situ / 0.8 |
| rs-cpu | 13–18 | 16.9–22.1 | ~3.8–4.4 | ≈ in-situ / 0.75 |
| c2 om-gpu1/2 | 4.0–5.1 | 6.0–7.5 | ~2.0–2.3 | ≈ in-situ / 0.85 |
| c2 rs-gpu1/2 | 3.2–3.8 | 7.4–8.7 | ~4.1–4.9 | ≈ in-situ / 0.87 |

Consequences:
- **The real capacity ratio is about gpu : llvm : cpu = 3 : 2.7 : 1** (cpu occupancy 21 s ÷ gpu
  7 s, ÷ llvm 8 s). Calibrated work alone suggests about 5:5:1, but the fixed overhead per request
  compresses the ratio.
- **Real capacity is about 30% below the calibrated capacity** that the sequences were built for
  (`capacity_rps` in `meta.csv`). A planned ρ = 0.7 is in practice about 0.95.

### 3.2 Load per site (ρ = requests × occupancy / 600 s)

| Run | om-gpu | om-llvm | om-cpu | rs-gpu | rs-llvm | rs-cpu |
|---|---|---|---|---|---|---|
| a0.5 c1 rr_2 | 0.72 | 0.84 | **2.22** | 0.72 | 0.84 | **1.72** |
| a0.5 c1 iwrr331 | 0.85 | 1.00 | 0.90 | 0.98 | **1.15** | 0.86 |
| a1.2 c1 rr | 0.77 | 0.86 | **2.96** | 0.86 | 0.96 | **2.13** |
| a1.2 c1 iwrr331 | 0.95 | 1.15 | **1.33** | 1.17 | 1.15 | 0.96 |

| Run | om-gpu1 | om-gpu2 | rs-gpu1 | rs-gpu2 |
|---|---|---|---|---|
| a0.5 c2 rr | 0.90 | 0.85 | **1.06** | **1.03** |
| a1.2 c2 rr | **1.08** | 0.96 | **1.18** | **1.22** |

Requests per site: rr gives 61–62 to each site in c1 and 84–87 in c2; iwrr331 gives 79–80 to each
gpu/llvm site and 26 to each cpu site.

### 3.3 Per-site latency and stages

Means over successful requests; `p50` is latency.

| Run | Site | n | fail | p50 | queue | asset | work | net | body |
|---|---|---|---|---|---|---|---|---|---|
| a0.5 c1 rr_2 | om-gpu | 62 | 0 | 7.7 | 2.2 | 0.95 | 4.84 | 0.77 | 0.37 |
| | om-llvm | 62 | 0 | 12.3 | 10.2 | 0.90 | 6.14 | 0.83 | 0.33 |
| | om-cpu | 62 | 11 | 273.9 | 245.3 | 0.93 | 17.83 | 0.78 | 0.38 |
| | rs-gpu | 62 | 0 | 11.3 | 4.9 | 1.63 | 3.32 | 1.51 | 0.78 |
| | rs-llvm | 61 | 0 | 19.3 | 19.1 | 1.76 | 4.36 | 1.71 | 0.82 |
| | rs-cpu | 61 | 10 | 193.7 | 178.6 | 1.61 | 13.10 | 1.53 | 0.79 |
| a0.5 c1 iwrr331 | om-gpu | 80 | 0 | 13.5 | 13.1 | 0.90 | 4.83 | 0.78 | 0.39 |
| | om-llvm | 80 | 0 | 28.7 | 36.5 | 0.94 | 5.29 | 0.82 | 0.35 |
| | om-cpu | 26 | 0 | 52.9 | 45.1 | 0.96 | 21.22 | 0.63 | 0.34 |
| | rs-gpu | 79 | 0 | 35.8 | 40.3 | 1.72 | 3.55 | 1.69 | 0.81 |
| | rs-llvm | 79 | 0 | 86.9 | 95.8 | 1.72 | 4.65 | 1.93 | 0.83 |
| | rs-cpu | 26 | 0 | 35.1 | 15.1 | 1.55 | 17.10 | 1.68 | 0.93 |
| a1.2 c1 rr | om-gpu | 62 | 0 | 13.7 | 14.2 | 1.08 | 5.33 | 0.84 | 0.36 |
| | om-llvm | 62 | 0 | 27.7 | 27.4 | 1.09 | 6.32 | 0.79 | 0.40 |
| | om-cpu | 62 | 26 | 280.3 | 282.3 | 1.10 | 22.40 | 0.88 | 0.38 |
| | rs-gpu | 62 | 0 | 25.1 | 23.0 | 2.05 | 3.82 | 1.77 | 0.83 |
| | rs-llvm | 61 | 0 | 45.3 | 45.5 | 2.01 | 4.75 | 1.78 | 0.84 |
| | rs-cpu | 61 | 23 | 275.7 | 240.3 | 2.13 | 15.97 | 2.04 | 0.88 |
| a1.2 c1 iwrr331 | om-gpu | 80 | 0 | 32.4 | 34.1 | 1.15 | 4.68 | 0.80 | 0.39 |
| | om-llvm | 80 | 0 | 70.0 | 91.4 | 1.09 | 6.39 | 0.89 | 0.34 |
| | om-cpu | 26 | 1 | 167.0 | 166.9 | 1.03 | 28.55 | 1.14 | 0.34 |
| | rs-gpu | 79 | 0 | 81.2 | 100.0 | 2.01 | 4.28 | 1.96 | 0.87 |
| | rs-llvm | 79 | 0 | 75.0 | 97.5 | 2.02 | 4.36 | 1.88 | 0.67 |
| | rs-cpu | 26 | 0 | 59.4 | 41.2 | 2.06 | 18.27 | 2.04 | 0.86 |
| a0.5 c2 rr | om-gpu1 | 84 | 0 | 17.6 | 18.1 | 0.95 | 4.28 | 0.75 | 0.30 |
| | om-gpu2 | 84 | 0 | 19.7 | 17.1 | 1.04 | 4.10 | 0.83 | 0.37 |
| | rs-gpu1 | 84 | 0 | 47.5 | 54.9 | 1.66 | 3.38 | 1.92 | 0.82 |
| | rs-gpu2 | 84 | 0 | 43.6 | 45.6 | 1.75 | 3.31 | 1.72 | 0.89 |
| a1.2 c2 rr | om-gpu1 | 87 | 0 | 40.0 | 45.7 | 1.09 | 5.06 | 0.88 | 0.37 |
| | om-gpu2 | 86 | 0 | 26.1 | 27.0 | 1.11 | 4.45 | 0.92 | 0.36 |
| | rs-gpu1 | 86 | 0 | 52.3 | 65.1 | 2.11 | 3.52 | 1.86 | 0.86 |
| | rs-gpu2 | 86 | 0 | 68.0 | 78.4 | 2.12 | 3.77 | 1.90 | 0.93 |

(The c2 iwrr runs match their rr pair to within run-to-run noise. The full per-site table for every
run is in `temp/report_per_site.csv`.)

### 3.4 Queues over time

Mean queue time, by when the request arrived (rr runs):

```
a0.5 c1 rr_2   0-100  100-200 200-300 300-400 400-500 500-600 s
om-gpu           1.5     3.4     3.2     0.2     1.2     2.3     stable
om-llvm          4.0    21.0    20.9     0.6     2.4     1.4     burst, drains
om-cpu          89.0   236.1   326.6   369.6   353.0   328.0     grows until the 600 s cut-off
rs-gpu           4.7    10.3     6.7     0.0     0.5     1.5
rs-llvm          8.9    28.6    51.8    10.9     3.2     3.3
rs-cpu          42.5   201.8   236.7   212.7   390.0   198.6     grows until the 600 s cut-off

a0.5 c2 rr     0-100  100-200 200-300 300-400 400-500 500-600 s
om-gpu1          6.3     4.9    19.4    43.5    22.7     1.7     burst, drains
om-gpu2          5.7     7.7    24.8    33.7    15.9     0.3     burst, drains
rs-gpu1         13.5    23.7    59.7    95.5    92.6    47.5     never drains
rs-gpu2         11.2    19.8    55.0    85.9    64.2    29.4     never drains
```

Under iwrr331 (a0.5 c1), rs-llvm is the worst site: its mean queue is 66–143 s in every window, at
ρ 1.15. At α 1.2, every gpu/llvm site has a queue of 80–175 s from 100 s onward.

### 3.5 Failures

All 88 failures are 504s, cut off at 600 s on a **cpu site**.

| Run | om-cpu | rs-cpu | Arrival times | Shows |
|---|---|---|---|---|
| a0.5 c1 rr_1 | 11 | 6 | 47–589 s (median 313 s) | f1_full 7, forest720_leaves 4, f1 3, … |
| a0.5 c1 rr_2 | 11 | 10 | 110–598 s (median 399 s) | f1_full 8, f1 5, forecast 4, … |
| a1.2 c1 rr | 26 | 23 | 74–596 s (median 343 s) | f1_full 20, forecast 12, f1 9, … |
| a1.2 c1 iwrr331 | 1 | 0 | 218 s | forecast 1 |

The failures follow from the queue: by about 100 s a cpu site's queue passes 200 s, and from then on
any request routed to it is at risk. `rr_none_1` also has 5 client-side `ReadTimeout`s, from its
older 600 s client timeout.

## 4. Factor Effects

### 4.1 Policy (case 1)

| | α 0.5 rr → iwrr331 | α 1.2 rr → iwrr331 |
|---|---|---|
| failures | 21 → 0 | 49 → 1 |
| p50 | 16.7 → 37.4 (×2.2) | 35.6 → 67.5 (×1.9) |
| p90 | 275 → 122 (×0.44) | 293 → 208 (×0.71) |
| mean (all, failures at 600 s) | 108.6 → 53.8 | 160.2 → 96.1 |
| makespan | 1198 → 708 | 1196 → 818 |

- **Round robin** ignores capacity. The cpu sites receive a third of all requests at ρ 1.7–3.0, while
  the gpu/llvm sites run at 0.7–0.9 with capacity to spare. The two-thirds of requests that reach
  fast sites see short queues, so the median looks good.
- **iwrr331** matches the real capacity ratio (3 : 2.7 : 1). It spreads the load evenly, but the
  total load is about 0.95, so every site sits near ρ ≈ 1 and every queue is long. The failures
  disappear and the average and tail improve a lot, but the median rises.
- **A policy that balances capacity alone cannot beat this.** With realised ρ ≈ 0.95 there is no
  spare capacity to share out. Improvements must come from choosing *which* request goes where:
  - using known request costs (MCT, min-min);
  - avoiding the slower, network-heavy rs path for light requests;
  - or removing work entirely, with a cache.
- **Case 2:** iwrr and rr are the same policy here. Their p50s agree within 0.6–1.2 s, so this pair
  works as a repeatability check (§6).

### 4.2 α (popularity skew)

| Case / policy | p50 α 0.5 → 1.2 | mean | failures |
|---|---|---|---|
| c1 rr | 16.7 → 35.6 | 79 → 93 | 21 → 49 |
| c1 iwrr331 | 37.4 → 67.5 | 54 → 95 | 0 → 1 |
| c2 rr | 26.7 → 40.4 | 41 → 62 | 0 → 0 |

**Confounded.** The α 1.2 sequences differ in more than skew:
- **Higher realised load:** 0.734 vs 0.689 (c1) and 0.734 vs 0.669 (c2), measured with calibrated
  capacity. Near saturation, a 7–10% higher ρ alone can explain most of the difference.
- **Heavier mix:** α 1.2 is 47% `f1_full` and 28% `forecast`, against 27% / 19% at α 0.5.
  `forecast` is the heaviest show (gpu work 8.3 s vs 3.6 s for f1_full). Mean in-situ work rises
  about 10% (c2: 3.8 → 4.2 s) and asset time about 0.3 s.
- **Fewer exact repeats:** 43% vs 53% of requests repeat an earlier path in c1, and 37% vs 48% in
  c2. Concentrating on f1_full, which has many layer combinations, spreads requests over more paths.
  This matters for the cache experiments.

To isolate α, either regenerate the sequences with the *same realised ρ*, or report ρ alongside α.

### 4.3 Case (c1 vs c2)

- c2 has no failures in any run, and a much lower p90/p99 than c1 rr (90 vs 275 at α 0.5), because
  it has no slow cpu site.
- The c2 median (26.7) is *worse* than c1 rr (16.7). The c2 sequence is scaled to c2's own capacity,
  and that capacity was itself estimated from c1's co-located gpu timings (`meta.csv` marks it
  "provisional"). Every c2 site runs at ρ 0.85–1.2.
- The two cases use different sequences (336/345 vs 370 requests), so the comparison is between two
  systems, not one controlled change.

### 4.4 Machine (om near vs rs far)

Median rs/om ratio for the *same path* on the *same tier*, across all runs:

| Stage | rs / om | Driven by |
|---|---|---|
| work | 0.69–0.84 (llvm 0.69–0.76, gpu 0.73–0.82, cpu 0.76–0.84) | rs hardware is faster |
| asset | 1.79–1.90 | round trips (60 vs 30 ms) |
| net | 1.87–2.24 | round trips |
| body | 1.6–2.9 | TCP slow start on a new connection per response |

Body throughput averages only 6.5 MB/s on om and 2.8 MB/s on rs, far below the 1 Gbit link. Each
2.3 MB response uses a new connection, so its transfer is limited by TCP slow start: about 8 round
trips.

The `net` stage alone is 0.83 s on om and 1.83 s on rs, about 28–30 round trips. Something inside
`net` costs one round trip after another. A likely candidate is the spec/show fetch before
`t_arrive`, but this is **not yet measured**.

Overall, rs saves about 1 s of work but pays about 2.3 s more in round trips per request. Its
occupancy is therefore about 15–20% higher than om's on the same tier, and its queues are longer
(§3.2, §3.4).

### 4.5 Tier (gpu, llvm, cpu)

Mean in-situ work by show (s), all case 1 runs pooled:

| Show | om-gpu | om-llvm | om-cpu | rs-gpu | rs-llvm | rs-cpu |
|---|---|---|---|---|---|---|
| spiders | 2.6 | 3.3 | 9.2 | 1.8 | 2.4 | 7.0 |
| f1_full | 3.6 | 4.3 | 15.3 | 2.4 | 3.1 | 11.2 |
| f1 | 3.8 | 4.3 | 13.0 | 2.5 | 3.0 | 9.6 |
| forest720 | 4.8 | 5.9 | 17.7 | 3.5 | 3.6 | 12.8 |
| forest720_leaves | 6.3 | 7.1 | 23.6 | 4.5 | 6.1 | 15.0 |
| forecast | 8.3 | 10.3 | 39.6 | 6.7 | 7.8 | 31.5 |

- llvm is only 1.1–1.3× slower than gpu at the work itself; cpu is 3.5–4.8× slower. Rendering cost
  grows with layers and pixel effects, with `forecast` (chroma key) and the forest shows (alpha)
  heaviest.
- The cpu tier's cost per show varies much more (9–40 s), which makes it the riskiest place for a
  heavy request. That argues for a policy that sends only light requests to cpu sites.
- **Segment 0 vs later segments:** segment 0 has slightly more work (+0.3–0.9 s; it is the first
  decode of the clip), while asset time is the same. The sites fetch every asset again for every
  request.

## 5. Calibration vs In-Situ

Median in-situ work ÷ idle calibrated work, same kind, segment 0:

| Site | Ratio | Reading |
|---|---|---|
| om-gpu, rs-gpu (c1) | 0.90–0.95 | about equal; slightly faster in situ |
| c2 gpu sites | 0.81–0.90 | faster than the c1-based calibration |
| om-llvm | 1.07–1.26 | **slowed by co-location:** om also runs the cpu site, the main server and the client, and llvm renders on the CPU |
| rs-llvm | 0.97–1.05 | about equal |
| cpu sites | 0.73–0.87 | biased low: failed (slowest) requests are excluded, and n = 3–11 |

Calibrated *work* predicts in-situ work well enough: within about 10%, apart from om-llvm. It
**underestimates occupancy** by the fixed network overhead of about 2 s on om and about 4.3 s on
rs. Any cost model (MCT, capacity for sequence generation) needs to add a per-site overhead term.

## 6. Repeatability

| Pair | p50 | mean | p90 | failures | Per site |
|---|---|---|---|---|---|
| a0.5 c1 rr_1 vs rr_2 | 17.4 / 16.7 | 85.9 / 79.2 | 300 / 275 | 17 / 21 | gpu/llvm queues agree within a few s; cpu queue means differ by ~40 s |
| a0.5 c2 rr vs iwrr (same policy) | 26.7 / 25.5 | 41.0 / 41.4 | 90 / 101 | 0 / 0 | rs-gpu2 queue 45.6 / 54.9 (±10–20%) |
| a1.2 c2 rr vs iwrr (same policy) | 40.4 / 41.0 | 61.9 / 63.4 | 145 / 152 | 0 / 0 | rs-gpu2 queue 78.4 / 84.7 |

Run-level metrics repeat within about 5–10% (p99 and max less well). Treat differences smaller than
that as noise until you have 3+ repeats.

## 7. Workload Characteristics

| Sequence | n | Unique paths | Kinds | Repeated path | Top-10 path share | Arrivals per 100 s |
|---|---|---|---|---|---|---|
| a0.5 c1 | 370 | 174 | 63 | 53.0% | 27.8% | 79, 95, 57, 42, 42, 55 |
| a0.5 c2 | 336 | 176 | 65 | 47.6% | 28.6% | 61, 65, 79, 69, 37, 25 |
| a1.2 c1 | 370 | 210 | 73 | 43.2% | 19.5% | 72, 100, 69, 34, 48, 47 |
| a1.2 c2 | 345 | 218 | 72 | 36.8% | 18.0% | 56, 47, 62, 82, 66, 32 |

- **Arrivals are bursty.** The first 200 s carry 47–52% of c1 requests, which creates the
  200–400 s queue peak seen in every run.
- **Segment 0 is 41–46% of all requests**, because many viewers leave after the first segment.
- **Cache potential:** 37–53% of requests repeat an exact earlier path. That is an upper bound on
  the segment-cache hit rate, ignoring the time needed to fill the cache. The rate is higher at
  α 0.5 than at α 1.2 with these sequences.

## 8. iwrr551 (5:5:1): Running

The runs `stat/run_a{0.5,1.2}_c1_p95_nonfifo/iwrr551_none_1.csv` started at 12:19.

**Prediction made before the results arrive.** Using the real capacity ratio of 3 : 2.7 : 1,
weights of 5:5:1 give each gpu/llvm site about 84 requests (ρ ≈ 1.0–1.25) and each cpu site about
17 (ρ ≈ 0.6). Expected outcomes:
- a higher p50 and p90 than iwrr331;
- rs-llvm and rs-gpu become the bottleneck;
- the cpu sites sit idle part of the time.

If that holds, it confirms that the weights should come from *occupancy*, not from calibrated work.

**α 0.5 result: the prediction holds.**

| a0.5 c1 | ok / fail | mean | p50 | p90 | p99 | max | makespan |
|---|---|---|---|---|---|---|---|
| rr_2 | 349 / 21 | 79.2 | 16.7 | 274.9 | 497.5 | 599 | 1198 |
| iwrr331 | 370 / 0 | 53.8 | 37.4 | 121.7 | 267.5 | 517 | 708 |
| **iwrr551** | 370 / 0 | 63.1 | **47.9** | **146.9** | 243.2 | 349 | 693 |

| iwrr551 a0.5 | om-gpu | om-llvm | om-cpu | rs-gpu | rs-llvm | rs-cpu |
|---|---|---|---|---|---|---|
| requests | 85 | 85 | 16 | 84 | 84 | 16 |
| ρ (occupancy) | 1.00 | 1.11 | ≈0.66* | 1.06 | 1.14 | ≈0.48* |
| mean queue | 33.6 | 66.5 | 2.9 | 59.5 | 75.8 | 2.0 |
| p50 latency | 32.4 | 60.5 | 26.0 | 52.7 | 64.7 | 18.7 |

\*The cpu sites were never backlogged, so their ρ shown is busy time (asset + work) ÷ 600 s.

- The p50 rises by 10.5 s and the p90 by 25 s compared with iwrr331. The gpu/llvm sites are all
  over ρ = 1, while the cpu sites sit mostly idle (queue of 2–3 s).
- The p99 and max are *lower* than with iwrr331, because the slow cpu tail is gone (only 32 requests
  go to cpu sites). The worst requests are now the ones queued on rs-llvm.
- **Conclusion:** weights based on calibrated work (5:5:1) overload the fast sites. Occupancy-based
  weights (about 3 : 2.7 : 1, i.e. iwrr331) give a better median and p90 at this load.

**α 1.2 result: iwrr551 is worse than iwrr331 on every metric.**

| a1.2 c1 | ok / fail | mean | p50 | p90 | p99 | max | makespan |
|---|---|---|---|---|---|---|---|
| rr | 321 / 49 | 93.0 | 35.6 | 293.0 | 556.8 | 597 | 1196 |
| iwrr331 | 369 / 1 | 94.7 | 67.5 | 207.6 | 394.8 | 514 | 818 |
| **iwrr551** | 370 / 0 | 110.7 | **86.3** | 227.2 | 412.4 | 544 | 793 |

Mean queue time per site: om-gpu 65.8, om-llvm 82.8, rs-gpu 102.1, rs-llvm 115.7 s. The cpu sites
(16 requests each) waited 59 s (om) and 33 s (rs). At α 1.2 the om-cpu site still built up a
queue, because it got heavy `forecast` requests (32 s of work each).

## 9. Measurement Caveats

- **The `asset` stage contains a second queue wait.** Its timer starts at the first dequeue and stops
  at `variantReady`, which fires only when the re-queued render task is serviced. Part of `asset` is
  therefore waiting behind other requests, not fetching.
- **The `net` stage is not pure network time.** It is about 28–30 round trips; see §4.4.
- **Survivor bias:** stage means for cpu sites leave out the requests cut off at 600 s, which are
  the slowest ones.
- **The occupancy estimate** uses only gaps in which both requests were queued more than 3 s. Sites
  that are rarely backlogged (rr gpu) have fewer samples.
- **The realised ρ in `meta.csv`** is based on calibrated work, so it understates real load by about
  30% (§3.1).
- **`a0.5 c1 rr_none_1`** used a 600 s client timeout and has no main-server log. Its failures are
  attributed to sites by `req_idx mod 6`, the round-robin order checked on successful requests.
- **Single run per configuration,** apart from the repeats in §6.

## 10. Recommendations

1. **Re-estimate capacity from occupancy** (work plus a per-site overhead of about 2 s on om and
   4.3 s on rs), then regenerate the sequences, so that a planned ρ = 0.7 really is 0.7. As things
   stand, every result is measured close to saturation.
2. **Equalise realised ρ across α** (or across cases) so the α effect can be read on its own.
3. **Derive iwrr weights from occupancy** (about 3 : 2.7 : 1), or make them a main-server argument,
   so 331 and 551 can be rerun without changing code.
4. **Load balancer cost model:** occupancy = calibrated work × in-situ factor + site overhead. This
   is the input that MCT / min-min need.
5. **Cut the per-request network overhead** before comparing smarter policies:
   - an asset cache on the site, in the Rust proxy (assets are fetched again for every request);
   - sending responses asynchronously in Dana;
   - possibly keep-alive connections.

   Each change makes a clean experiment in itself.
6. **Run 3 repeats** of each final configuration. Differences under about 10% are within noise.
