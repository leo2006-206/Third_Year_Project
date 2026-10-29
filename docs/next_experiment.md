# Experiment Record: Round Robin Baseline, Case 1, α = 0.5

Written 2026-10-28. This file records how the next experiment is set up and run. It also records the pilot run that was discarded and why.

> **Note (2026-10-29):** the runs recorded here used the first sequences (370 requests). Planned ρ = 0.7 turned out to be ≈0.95 real, because calibration has no network overhead. Their results were moved to `stat/run_*_p95/`; see `docs/results_report.md`. The sequences were then regenerated with a network overhead per request (`net_round_trips` in `generate_request_seq.py`), so `seq_a*_case*.csv` are now the new ones (239 requests in case 1). The old ones are in git history (commit `68b4419`).

| Field | Value |
|---|---|
| Cluster | case 1 (heterogeneous): gpu + llvmpipe + cpu site on each of two laptops |
| Workload | `servers_benchmark/sequences/seq_a0.5_case1.csv` (Zipf α = 0.5, offered load ρ = 0.7) |
| Load-balancing policy | round robin (`rr`) |
| Features | none (no segment cache, no asset cache), label `none` |
| Output | `stat/run_a0.5_c1/rr_none_<n>.csv` |
| Code version | fill in at run time (`git log --oneline -1`); needs netem `rate 1gbit` and the buffered forwarding below |

---

## 1. Aim

Measure how round robin performs on a heterogeneous two-machine cluster under a realistic viewer workload at 70% offered load. This is the baseline that the other policies (iwrr, MCT, min-min, max-min) and the cache features are compared against.

Round robin sends each site the same number of requests regardless of its speed. The cpu sites are about 3–4× slower than the gpu and llvmpipe sites, so the expectation is:
- the cpu sites are overloaded and their queues grow throughout the window;
- the gpu and llvmpipe sites have spare capacity;
- the latency tail is dominated by the cpu sites.

---

## 2. Hardware and Software

| | om (main laptop) | rs (second laptop) |
|---|---|---|
| Hostname | `leowong-OMEN-by-HP-Gaming-Laptop-16-wf0xxx` | `leowong-RS` |
| CPU | Intel Core i9-13900HX, 24 cores / 32 threads | Intel Core i9-13900HX, 24 cores / 32 threads |
| RAM | 16 GB | 32 GB |
| GPUs | Intel Raptor Lake-S UHD (integrated) + NVIDIA RTX 4070 Max-Q | Intel Raptor Lake-S UHD (integrated) + NVIDIA (device 2f18) |
| GPU used by the gpu site | Intel integrated (VA-API decode + Mesa OpenGL); the container exposes only the Intel GPU | same |
| OS / kernel | Linux Mint 22.3, kernel 7.0.0-42-generic | Linux Mint 22.3, kernel 7.0.0-38-generic |
| Docker | 29.9.2 | 29.9.2 |
| Rust | 1.97.1 | (built by the scripts on rs) |
| Measured speed | baseline | about 1.3–1.4× faster work time per segment |
| Role | main server (port 7000) + near sites | far sites |

Both laptops are on mains power for the run, with no other heavy programs open (browsers with video, builds).

---

## 3. Network

### 3.1 Physical link

- One Ethernet cable between the laptops, 1000 Mb/s, no switch or router.
- NetworkManager profile `obm-link` on each laptop, with a fixed address, no gateway and IPv6 off. Internet stays on Wi-Fi.

| | Interface | Address |
|---|---|---|
| om | `eno1` | `10.10.0.1/24` |
| rs | `enp6s0` | `10.10.0.2/24` |

- Measured round trip over the bare cable: 0.44–0.50 ms (`ping`).

### 3.2 Emulated distance (netem)

Each offload site adds a delay to everything it sends, using `tc qdisc ... netem` inside its container. This affects its responses to the main server and its requests for assets.

| Sites | Delay | Jitter | netem command (in `offload_server/entrypoint.sh`) |
|---|---|---|---|
| om1–om3 (near) | 30 ms | 3 ms, normal distribution | `netem delay 30ms 3ms distribution normal rate 1gbit` |
| rs1–rs3 (far) | 60 ms | 6 ms, normal distribution | `netem delay 60ms 6ms distribution normal rate 1gbit` |

**Why `rate 1gbit`:** with jitter alone, netem gives every packet its own random delay, so later packets overtake earlier ones. TCP treats the reordering as packet loss: it resends data and keeps shrinking its sending window. Large transfers become about 10× slower. Setting a rate makes netem send each packet no earlier than the previous one. Packets stay in order, while the delay of each request still varies by the jitter. 1 Gbit/s is the speed of the cable and far above what a site sends, so it doesn't limit bandwidth.

Measured on om (2.7 MB transfer, throwaway container, 3–5 tries each):

| netem setting | Transfer time | TCP connect time |
|---|---|---|
| none | 0.006 s | – |
| 30 ms | 0.5–0.8 s | – |
| 30 ± 3 ms (no rate, used in the pilot) | 4.2–8.2 s | – |
| 30 ± 3 ms + rate 1gbit | 0.4–0.8 s | 26–40 ms |
| 60 ms | 0.8–1.0 s | – |
| 60 ± 6 ms (no rate, used in the pilot) | 5.4–8.5 s | – |
| 60 ± 6 ms + rate 1gbit | 0.8–2.2 s | 58–67 ms |

### 3.3 Who connects to whom

```
                        om (10.10.0.1)                                 rs (10.10.0.2)
 seq_bench ──:7000──> obm-main-server ──obm-net──> obm-offload-om1..3   obm-offload-rs1..3 (:7010..7030)
                        ▲   │                                               ▲      │
                        │   └──────────────── cable: offload requests ──────┘      │
                        └────────────── cable: /shows + /assets from 10.10.0.1:7000 ┘
```

- **The main server reaches om's sites by container name** (`obm-offload-om1:7010`) over the Docker network `obm-net`, and rs's sites by cable address (`10.10.0.2:7010`). om's firewall blocks containers from reaching om's own address on the site ports.
- **Every site downloads `/shows` and `/assets` from `10.10.0.1:7000`.** This is the default of `MAIN_SERVER_ADDR` in `offload_server.rs`, `offload_server/run_sh.sh` and `run_all_offload.py`. So assets also cross the netem delay; for rs they also cross the cable.
- **om firewall (`ufw`), one-time rules:**
  ```bash
  sudo ufw allow from 172.18.0.0/16 to any port 7000 proto tcp   # om's containers (obm-net subnet) -> 10.10.0.1:7000
  sudo ufw allow from 10.10.0.0/24 to any port 7000 proto tcp    # rs over the cable
  ```
- **SSH from om to rs** over the cable (`ssh rs ...`), with key-only login, for checking rs during a run. Details are in `temp/ssh_rs_setup.txt` (local, gitignored).

---

## 4. Cluster: Case 1

### 4.1 Sites

| id | Machine | Endpoint used by main server | decoding / composition / encoding | CPU cap | Memory cap | Delay ± jitter |
|---|---|---|---|---|---|---|
| om1 | om | `obm-offload-om1:7010` | gpu / gpu / cpu | 4 | 2 GB | 30 ± 3 ms |
| om2 | om | `obm-offload-om2:7020` | gpu / llvmpipe / cpu | 8 | 2 GB | 30 ± 3 ms |
| om3 | om | `obm-offload-om3:7030` | gpu / cpu / cpu | 2 | 2 GB | 30 ± 3 ms |
| rs1 | rs | `10.10.0.2:7010` | gpu / gpu / cpu | 4 | 2 GB | 60 ± 6 ms |
| rs2 | rs | `10.10.0.2:7020` | gpu / llvmpipe / cpu | 8 | 2 GB | 60 ± 6 ms |
| rs3 | rs | `10.10.0.2:7030` | gpu / cpu / cpu | 2 | 2 GB | 60 ± 6 ms |

- **CPU cap** is Docker `--cpus`. It limits total CPU time, not which cores are used, so the work spreads over all cores.
- **Memory cap** is Docker `--memory` with swap disabled.
- **Each site processes one request at a time.** Dana queues the others in arrival order, renders, and sends the response from inside the same loop. So a slow send also blocks the site's next request.

### 4.2 Config files (`servers_container/`)

| File | Used by | Contents |
|---|---|---|
| `offload_om_c1.csv` | `run_all_offload.py` on om | om1–om3 |
| `offload_rs_c1.csv` | `run_all_offload.py` on rs | rs1–rs3 |
| `cluster_endpoint_c1.csv` | main server on om | all six, with `ip` (container names for om, `10.10.0.2` for rs) |

Each site's name in the main server log is all of its CSV columns joined, e.g. `id=rs1_ip=10.10.0.2_port=7010_decoding=gpu_composition=gpu_encoding=cpu_cpu=4_memory=2_delay_ms=60_jitter_ms=6`.

### 4.3 Single-request reference times

Measured 2026-10-28 with one `forecast` segment, 1280×720, through the main server with the cluster idle. This was with the old netem (no rate), so the net times include some reordering slowdown.

| Site | Total | Asset | Work | Net (X-OBM-Net) | Size |
|---|---|---|---|---|---|
| om1 gpu | 12.8 s | 0.89 s | 8.7 s | 675 ms | 929,887 B |
| om2 llvm | 13.1 s | 0.86 s | 10.7 s | 674 ms | 930,746 B |
| om3 cpu | 37.7 s | 0.83 s | 34.0 s | 702 ms | 937,749 B |
| rs1 gpu | 12.8 s | 1.52 s | 6.6 s | 1206 ms | 929,887 B |
| rs2 llvm | 10.8 s | 1.58 s | 7.6 s | 1252 ms | 930,746 B |
| rs3 cpu | 30.0 s | 1.54 s | 24.7 s | 1286 ms | 937,749 B |

Sites of the same composition type return byte-identical sizes on both machines.

---

## 5. Main Server and Policy

- **Main server:** Rust (`rust_server/src/main_server.rs`) in container `obm-main-server`, port 7000, Docker network `obm-net`. It mounts:
  - `obm/assets`, `obm/shows` → served at `/assets`, `/shows`;
  - `servers_container/cluster_endpoint_c1.csv` → `/app/cluster_endpoint.csv`, passed as its argument.
- **Policy:** `lb::RoundRobinLB::new(offload_url)` in `main()` of `main_server.rs`. Requests go to sites in CSV order (om1, om2, om3, rs1, rs2, rs3, om1, …), whatever the sites' load.
- **Per request,** the main server:
  1. picks a site;
  2. opens a new TCP connection to it and forwards the GET;
  3. reads until the end of the response head (`split_header_body`);
  4. adds `X-OBM-Worker` (site endpoint), `X-OBM-Net` and `Server-Timing` (`time_append_header`), and parses the site's timings into a `TimesFeedback`;
  5. reads the **whole body into memory** and checks it against `Content-length` (a mismatch is a 502);
  6. reports the site as done to the load balancer, with the timings (`finish(Some(&times))`);
  7. sends head + body to the client;
  8. logs `[OFFLOAD] -> <site name> | <net + queue + asset + work ms> | <path>`.

  Store-then-forward is deliberate: the segment cache will keep the body from step 5. It adds about 1 ms to the full response time, but the client's first byte now comes only after the whole segment has reached the main server.

  There is no queue in the main server.
- **Timeouts towards the sites** (`serve_offload` in `main_server.rs`):

  | Step | Limit | On expiry |
  |---|---|---|
  | TCP connect | 5 s | `504 Gateway Timeout` to the client, `[OFFLOAD FAIL] -> <site> \| timed out after 5s \| <path>` in the log |
  | waiting for the response headers (= site queue + asset + render) | 600 s | `504`, logged the same way |
  | body streaming | 30 s without any data (not a limit on the whole transfer) | connection closed, the client gets a short body |

  Other connection errors (e.g. refused, no route to host) give `502 Bad Gateway`. Either way the site is reported as done to the load balancer: the selection guard does this when it's dropped.
- **Features:** none. The current code has no segment cache and no asset cache, so the label is `none`.

---

## 6. Workload

**Sequence file:** `servers_benchmark/sequences/seq_a0.5_case1.csv` (`req_idx, t_ms, path`), made by `servers_benchmark/generate_request_seq.py` (method in `servers_benchmark/detail_seq.md`).

| Parameter | Value |
|---|---|
| Popularity | Zipf with exponential cutoff, α = 0.5, cutoff c = 50; shows in fixed rank order f1_full, forecast, f1, forest720_leaves, spiders, forest720; combinations per layer with the producer default first |
| Viewer model | a video's first decision: continue 0.40 / switch 0.45 / leave 0.15, combination change 0.50; later: 0.80 / 0.10 / 0.10, combination change 0.10; max 30 segments |
| Player timing | a video's segment 0 at t, next request at t + 5 s, then every 10 s |
| Viewer arrivals | Poisson, pre-roll 600 s, rate fitted so the window holds exactly N requests |
| Cluster capacity C | 0.8813 req/s (529 per 600 s), from popularity-weighted busy times of the six sites |
| Target load ρ | 0.7 |
| Window T | 600 s |
| Requests N | 370 (= ρ·C·T) |
| Request rate | 0.617 req/s |
| Viewers in window | 53 (arrival rate 0.068/s), mean 7.9 segments per viewer |
| Realised load | 0.689 (capacity under the window's actual mix) |
| Seed | 2026 (the same sequence for every policy: paired comparison) |

---

## 7. Replayer and Measurements

**Script:** `servers_benchmark/seq_bench.py SEQ POLICY FEATURES [--base-url http://localhost:7000] [--timeout S] [--limit N]`

- **Open loop.** Each request is sent at its scheduled `t_ms` on its own thread, whether or not earlier requests have finished. Latency is measured from the scheduled send time, so if the client itself fell behind, the delay still counts against the system (no coordinated omission).
- **Timeout.** The effective limit is the **main server's 600 s header timeout** (§5). A request that waits longer than that at its site comes back as `status 504`, `error HTTP 504`, at about 600 s. `seq_bench --timeout 3600` keeps the client from giving up first, so every cut-off is recorded consistently by the server as a 504 rather than a client `ReadTimeout`. For round robin, expect 504s from the cpu sites: the pilot had 44 requests over 600 s. Report them as failed / censored at 600 s, not as latencies. The site still finishes a request after the main server gives up, and logs `Broken pipe in send()`.
- **Output** `stat/run_a0.5_c1/rr_none_<n>.csv`, one row per request. `<n>` counts up for repeats.

| Column | Meaning |
|---|---|
| `req_idx`, `path` | request from the sequence |
| `sched_ms` | scheduled send time from run start |
| `lag_ms` | actual − scheduled send (client lateness, should be ~0) |
| `ttfb_ms` | scheduled send → response headers. Because of store-then-forward this is ≈ `latency_ms`: the headers are sent only after the whole body has reached the main server |
| `latency_ms` | scheduled send → full body received (what the viewer experiences) |
| `status`, `size_bytes`, `error` | result |
| `worker` | `X-OBM-Worker`: the site chosen |
| `queue_ms`, `asset_ms`, `work_ms`, `decode_ms`, `render_ms`, `encode_ms` | Dana's own timings (`X-Dana-Timings`) |
| `net_ms` | main server: time to headers − (queue + asset + work) |
| `client_net_ms` | latency − (work + asset + queue + net) |
| `send_epoch_ms` | wall clock at scheduled send, to line up with server logs |

**Derived:**
- site → main server body transfer: `latency_ms − ttfb_ms` no longer shows it (store-then-forward makes the two ≈ equal). It is inside `net_ms` + the gap between `X-OBM-Net` and the end; compare total latency with queue + asset + work instead. After the netem fix, `latency − (queue + asset + work)` should be about 1–3 s per request, not 5–6 s as in the pilot.
- per-site utilisation = Σ(asset + work) / window.

---

## 8. Procedure

### 8.1 Before the first run (one-time; already done 2026-10-28)

1. Cable profiles `obm-link` on both laptops (README §3.1).
2. om `ufw` rules (§3.3).
3. SSH key from om to rs (`temp/ssh_rs_setup.txt`).
4. `obm/` (assets and shows) copied to both machines.

### 8.2 Prepare (each run)

```bash
# both laptops: cable in, on mains power, same commit
git pull                                  # rs: must include the netem 'rate 1gbit' change
ping -c 3 10.10.0.2                       # om: < 1 ms
```

### 8.3 Start the sites: fresh containers, so no leftover queues

`run_all_offload.py` always rebuilds the offload binary and image, removes containers with the same ids, and starts new ones. That picks up the netem change and empties any queue from an earlier run.

```bash
# rs (in a terminal on rs, or: ssh rs, then cd ~/SCC/Third_Year_Project)
./servers_container/run_all_offload.py servers_container/offload_rs_c1.csv -d

# om
./servers_container/run_all_offload.py servers_container/offload_om_c1.csv -d
```

- `-d` starts the containers in the background. Without it, each site opens in a gnome-terminal tab.
- If the previous run was case 2, remove its sites first: `docker rm -f $(docker ps -q --filter name=obm-offload-)` on each machine.

### 8.4 Start the main server: fresh, so its counter and log start empty

```bash
# om, in its own terminal (Ctrl+C stops it)
docker rm -f obm-main-server 2>/dev/null
./servers_container/main_server/run_sh.sh servers_container/cluster_endpoint_c1.csv
```

In the background instead (image already built):

```bash
docker rm -f obm-main-server 2>/dev/null
docker run -d --rm --init --network obm-net -p 7000:7000 \
    -v "$PWD/obm/assets:/app/obm/assets:ro" \
    -v "$PWD/obm/shows:/app/obm/shows:ro" \
    -v "$PWD/servers_container/cluster_endpoint_c1.csv:/app/cluster_endpoint.csv:ro" \
    --name obm-main-server obm-main-server /app/cluster_endpoint.csv
```

### 8.5 Checks before the replay (all on om)

```bash
# 1. main server: six sites, round robin
docker logs obm-main-server 2>&1 | grep -E "Offload site|Load balancer"

# 2. netem on every site, with the rate (expect: delay 30ms 3ms ... rate 1Gbit; rs: 60ms 6ms)
for c in om1 om2 om3; do docker exec obm-offload-$c tc qdisc show | grep netem; done
ssh rs 'for c in rs1 rs2 rs3; do docker exec obm-offload-$c tc qdisc show | grep netem; done'

# 3. asset source on every site (expect 10.10.0.1:7000)
for c in om1 om2 om3; do docker logs obm-offload-$c 2>&1 | grep -m1 "Assets from"; done
ssh rs 'for c in rs1 rs2 rs3; do docker logs obm-offload-$c 2>&1 | grep -m1 "Assets from"; done'

# 4. smoke test: 6 requests walk through every site once (one curl at a time; the cpu sites take ~25-35 s)
for i in 1 2 3 4 5 6; do
  curl -s -o /dev/null -D - "http://localhost:7000/offload/show/spiders.json/0/10/1280/720/main/landscape/2/main|race/spider_blocker|Off" \
    | grep -E "^HTTP|X-OBM-Worker"
done
```

The smoke test puts six requests into the sites, and moves the round-robin counter on by 6, so it starts again at om1. Wait until all six have returned before starting the replay.

### 8.6 Run

```bash
# om, project root
python3 servers_benchmark/seq_bench.py servers_benchmark/sequences/seq_a0.5_case1.csv rr none --timeout 3600 \
  | tee temp/rr_none_console.log
```

- Sending lasts 600 s. Draining the cpu sites' queues takes longer: in the pilot the run lasted 1199 s, capped by the 600 s timeout. With `--timeout 3600`, expect roughly 20–40 min in total.
- Don't use either laptop for anything heavy during the run.
- Optionally, watch rs: `ssh rs 'docker stats --no-stream'`.

### 8.7 After the run

```bash
# 1. summary printed by seq_bench (requests, ok/failed, max send lag, latency percentiles)

# 2. no site was killed for memory, and all are still up
docker inspect -f '{{.Name}} oom={{.State.OOMKilled}} {{.State.Status}}' obm-offload-om1 obm-offload-om2 obm-offload-om3
ssh rs 'docker inspect -f "{{.Name}} oom={{.State.OOMKilled}} {{.State.Status}}" obm-offload-rs1 obm-offload-rs2 obm-offload-rs3'

# 3. client stayed on schedule (max lag_ms ~ 0), body transfer is fast again, per-site breakdown
python3 - <<'EOF'
import csv, statistics as st
rows = list(csv.DictReader(open(sorted(__import__("glob").glob("stat/run_a0.5_c1/rr_none_*.csv"))[-1])))
ok = [r for r in rows if not r["error"]]
print("rows", len(rows), "ok", len(ok), "max lag", max(int(r["lag_ms"]) for r in rows if r["lag_ms"]))
by = {}
for r in ok: by.setdefault(r["worker"], []).append(r)
for w, rs in sorted(by.items()):
    m = lambda k: st.mean(int(r[k]) for r in rs) / 1000
    body = st.mean((int(r["latency_ms"]) - int(r["queue_ms"]) - int(r["asset_ms"]) - int(r["work_ms"])) / 1000 for r in rs)
    print(f"{w:22} n={len(rs):3} lat={m('latency_ms'):6.1f}s queue={m('queue_ms'):6.1f}s "
          f"asset={m('asset_ms'):4.2f}s work={m('work_ms'):5.1f}s rest={body:4.2f}s")
EOF

# 4. save the server logs next to the result
docker logs obm-main-server > stat/run_a0.5_c1/rr_none_<n>_main.log 2>&1
```

### 8.8 Repeats and the next policy

- **Repeat** (`rr_none_2`, …): restart the sites (§8.3) and the main server (§8.4), then run again.
- **Next policy:**
  1. change `main()` in `main_server.rs`;
  2. restart the main server without `--skip-build` so it's rebuilt;
  3. restart the sites;
  4. run with that policy's label (e.g. `iwrr none`).

  The label must match the compiled policy.

---

## 9. Pilot Run (Discarded)

**Run:** `rr_none_1`, 2026-10-28, same setup as above except netem had no `rate` and the timeout was 600 s. The data was deleted.

- **Result:** 370 requests; 326 succeeded and 44 timed out at 600 s (all on the cpu sites). Successful requests: median 68 s, 90th percentile 304 s, maximum 599 s. Run time 1199 s. The client sent every request on schedule (at most 1 ms late).
- **Per site:**

  | Site | n | Mean queue | Asset | Work |
  |---|---|---|---|---|
  | om1 | 61 | 54 s | 0.9 s | 4.9 s |
  | om2 | 62 | 50 s | 0.9 s | 5.9 s |
  | om3 | 36 ok | 247 s | 0.9 s | 20.4 s |
  | rs1 | 62 | 74 s | 1.6 s | 3.6 s |
  | rs2 | 62 | 44 s | 1.5 s | 4.0 s |
  | rs3 | 43 ok | 240 s | 1.7 s | 15.2 s |

- **Why it was discarded:**
  - Every response body took 4–5 s on average (up to 24 s) to transfer about 2.5 MB, i.e. about 0.5 MB/s, on every site.
  - Dana sends inside its processing loop, so this added about 4–5 s to every request's time at its site. That roughly doubled the gpu and llvmpipe sites' time per request and produced 50–74 s queues on sites that should have been about half busy.
  - Cause: netem jitter without a rate reorders packets, and TCP reacts as if they were lost (§3.2).
  - **Fixed** by adding `rate 1gbit` in `offload_server/entrypoint.sh`.

---

## 10. Known Limitations (affect interpretation)

- **The main server has no queue.** It picks a site the moment a request arrives.
- **Requests are cut off at 600 s** (main server header timeout). Round robin's latency tail is censored there; count the 504s.
- **Dana sends the response inside its single processing loop.** Transfer time blocks the next request, so network conditions act on site throughput, not just on latency.
- **Abandoned requests aren't cancelled.** When the main server times out (504), the site still renders the segment; the work is wasted and its queue doesn't shrink.
- **Duplicate `Access-Control-Expose-Headers`** in main server responses: harmless to the measurements.
- **The first main-server log line always shows `00:00:00.000`.** The log clock starts at the first message, so use `send_epoch_ms` in the CSV to line up times.
- **Case 2 timings are provisional.** Its capacity uses case-1 co-located gpu timings, not a measured 4-gpu run.

---

## 11. Run Log (fill in)

| Date / time | Run file | Commit | Timeout | Requests ok / failed | Run time | Max lag | Notes |
|---|---|---|---|---|---|---|---|
| | `rr_none_1.csv` | | 3600 | | | | |
