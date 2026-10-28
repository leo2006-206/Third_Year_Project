# OBM Video Offload & Load Balancing Project

A distributed Object-Based Media (OBM) system with server-side video rendering offloading and load balancing.

______________________________________________________________________

## 1. System Architecture & What the Servers Do

- **Main Server (Rust + `smol`)** — Port `7000`:
  - Serves the testing web client UI (benchmark and evaluation harness).
  - Serves raw media assets (`/assets/...`) and show specifications (`/shows/...`).
  - Acts as a reverse proxy / load balancer for `/offload/shows/...`: receives segment rendering requests, routes them to offload worker servers, and streams generated H.264 video chunks back to the client.
- **Offload Nodes (Rust + Dana Engine)** — Configurable Ports (e.g. `7010`, `7020`, `7030`):
  - **Rust Offload Server**: Exposed container entrypoint. Acts as a reverse proxy, forwards offload requests to the local Dana engine, and serves as a local asset cache (`ASSET_HOST`) for Dana.
  - **Dana Offload Engine** (Port `9009` internal): Headless Mesa/GL rendering engine structured into a modular 3-stage pipeline matching the project presentation:
    - **1. Decoding (`cpu`, `gpu`)**: Software CPU decoding via `libavcodec` vs hardware GPU decoding via VA-API (`Decoder.h264va`, saving 3–4s).
    - **2. Composition & Readback (`cpu`, `llvmpipe`, `gpu`)**: Pure CPU SDL blitter (21–25s) vs Mesa LLVMpipe CPU rasterizer (5–8s across multi-threaded cores) vs physical GPU direct rendering via VirtualGL (3–7s).
    - **3. Encoding (`cpu`)**: Standards-compliant multi-threaded H.264 CPU software encoding via `libx264` (GPU encoding disabled).
    - **Fastest Combination**: `gpu, gpu, cpu` => ~3.2s per 10s segment.
  - **Pre-Flight Hardware Verification**: Each offload container asserts and verifies its active graphics driver and decoding pipeline against the requested configuration at startup; if an acceleration mismatch occurs or falls back to software unexpectedly on `gpu`, the container prints a fatal error and terminates immediately (`exit 1`).
- **Networking**:
  - **Local (Same host)**: Docker network `obm-net` (containers resolve each other directly by name, e.g. `obm-offload-1:7010`, `obm-offload-2:7020`, `obm-offload-3:7030`).
  - **Distributed (Multi-machine)**: Tailscale P2P WireGuard mesh (`100.x.y.z:<PORT>`) for direct peer-to-peer communication across hosts.
  - **Public Access (Self-Hosted Edge Tunnel)**: Cloud VPS (Caddy on `195.58.134.234`) with automatic Let's Encrypt TLS terminating at `https://obm-main.leowong.space/` and proxying over Tailscale to local port `7000` (bypassing Cloudflare's 100 MB cap and 100s streaming timeouts).

______________________________________________________________________

## 2. Host Machine Requirements for Offload Pipeline Acceleration

To run the full modular pipeline (`decoding: cpu|gpu`, `composition: cpu|llvmpipe|gpu`, `encoding: cpu`) correctly on a machine without startup assertions failing, the host system must meet the following requirements:

### Pipeline Acceleration Matrix

| Stage / Component | Hardware Accelerated (`gpu`) | Software OpenGL (`llvmpipe`) | Pure Software (`cpu`) |
| :--- | :--- | :--- | :--- |
| **Decoding** | GPU VA-API hardware acceleration (`Decoder.h264va`, ~1.3s) | Software CPU decoding (`libavcodec`, ~4.5s) | Software CPU decoding (`libavcodec`, ~4.5s) |
| **Composition & Readback** | VirtualGL 3.1.5 + Mesa DRI hardware OpenGL (~3.5s) | CPU Mesa software rasterizer (`llvmpipe`, ~6.5s) | Pure SDL2 software blitter (~23s) |
| **Encoding** | Multi-threaded CPU `libx264` (~0.35s) | Multi-threaded CPU `libx264` (~0.35s) | Multi-threaded CPU `libx264` (~0.35s) |
| **Total Segment Time** | **~3.2s – 4.5s** (Fastest: `gpu,gpu,cpu`) | **~7.0s – 8.5s** (`cpu,llvmpipe,cpu`) | **~21s – 28s** (`cpu,cpu,cpu`) |
| **Physical Hardware** | Dedicated or integrated GPU with H.264 decode (Intel/AMD) | Any multi-core x86_64 CPU (No GPU required) | Any multi-core x86_64 CPU (No GPU required) |
| **Linux DRM Subsystem** | `/dev/dri/card*` (VirtualGL) and `/dev/dri/renderD*` (VA-API) | Not required | Not required |
| **Docker Device Mount** | Mandatory: `--device /dev/dri:/dev/dri` | None | None |
| **Strict Assertion** | Fails startup if `/dev/dri` missing or renderer is software | Asserts active renderer is `llvmpipe` | Disables GLX / sets software driver |

### Host System Checklist

1. **Operating System**: Linux x86_64 with kernel 5.4+ (Ubuntu 20.04/22.04/24.04, Debian 11+, Arch, etc.).
2. **DRM Device Access (for `gpu` tier)**:
   - Ensure the kernel DRM driver exposes both card and render nodes:
     ```bash
     ls -la /dev/dri
     # Expected: card0/card1 and renderD128/renderD129
     ```
   - Ensure your host user has read/write permissions to `/dev/dri`:
     ```bash
     sudo usermod -aG video,render $USER
     ```
3. **CPU & Memory Sizing**:
   - **RAM**: Minimum 8 GB (each active rendering worker container consumes ~200MB – 460MB during 1080p/720p compositing).
   - **CPU**: Minimum 4 physical cores recommended (8+ vCPUs recommended for multi-node clusters, as `libx264` utilizes 3–4 threads per active encoding segment).
4. **Docker Engine**: Docker 20.10+ with BuildKit support.

______________________________________________________________________

## 3. Two-Laptop Cluster

The experiments run on two laptops joined by one Ethernet cable. `om` runs the main server (load balancer, assets) and the near offload sites; `rs` runs the far offload sites.

| Machine | Role | Cable interface | Cable IP | Site delay (netem) |
|---|---|---|---|---|
| `om` (main laptop, weaker) | main server `:7000` + near sites | `eno1` | `10.10.0.1` | 30 ± 3 ms |
| `rs` (second laptop, ~1.3× faster) | far sites | `enp6s0` | `10.10.0.2` | 60 ± 6 ms |

```
                        om (10.10.0.1)                                 rs (10.10.0.2)
 client ──:7000──> obm-main-server ──obm-net──> obm-offload-om1..3     obm-offload-rs1..3 (:7010..)
                        ▲   │                                               ▲      │
                        │   └──────────────── cable: offload requests ──────┘      │
                        └────────────── cable: /shows + /assets from 10.10.0.1:7000 ┘
```

- The main server reaches om's sites by **container name** on the Docker network `obm-net`, and rs's sites by **cable IP**.
- Every offload site (both machines) downloads `/shows` and `/assets` from the main server at **`10.10.0.1:7000`** (default of `MAIN_SERVER_ADDR`).
- The cable itself adds < 1 ms; near / far delays come only from each site's netem setting (egress-only, applied to everything the site sends, incl. asset downloads, so every round trip the site makes grows by `delay_ms`). netem runs with `rate 1gbit`: per-packet jitter alone reorders packets, which TCP treats as loss (large responses ~10× slower); the rate keeps packets in order without limiting bandwidth.

### 3.1 One-Time Setup (each laptop)

**1. Software and access** (both): Docker usable without `sudo` (`docker ps` works; user in the `docker` group), Rust/cargo, the user in `video` and `render` for the gpu sites (`sudo usermod -aG video,render $USER`), and the host's `sch_netem` module (a site with a delay aborts at startup without it).

**2. Project and media** (both):

```bash
git clone <this repository> ThirdYearProject && cd ThirdYearProject
# obm/ (assets + shows) is gitignored: copy it into the project root (USB / scp / clone barryfp/obm)
```

**3. Cable link** (NetworkManager profile; saved, so afterwards just plug the cable in):

```bash
# om
sudo nmcli con add type ethernet ifname eno1 con-name obm-link \
  ipv4.method manual ipv4.addresses 10.10.0.1/24 ipv6.method disabled \
  connection.autoconnect-priority 10
sudo nmcli con up obm-link

# rs
sudo nmcli con add type ethernet ifname enp6s0 con-name obm-link \
  ipv4.method manual ipv4.addresses 10.10.0.2/24 ipv6.method disabled \
  connection.autoconnect-priority 10
sudo nmcli con up obm-link
```

No gateway is set, so internet stays on Wi-Fi. `autoconnect-priority 10` makes `obm-link` win over NetworkManager's default DHCP profile (`Wired connection 1`), which would wait for a router that isn't there.

**4. Firewall on om** (`ufw` is active there and drops these by default):

```bash
docker network create obm-net 2>/dev/null || true
docker network inspect obm-net -f '{{range .IPAM.Config}}{{.Subnet}}{{end}}'   # 172.18.0.0/16 on om
sudo ufw allow from 172.18.0.0/16 to any port 7000 proto tcp   # om's own site containers -> 10.10.0.1:7000
sudo ufw allow from 10.10.0.0/24 to any port 7000 proto tcp    # rs's sites over the cable
```

If `ufw` is active on rs too: `sudo ufw allow from 10.10.0.0/24` (om -> rs offload ports).

**5. Check** (once, and whenever something looks wrong):

```bash
ping -c 3 10.10.0.2                                                  # on om: < 1 ms
curl -s -o /dev/null -w "%{http_code}\n" http://10.10.0.1:7000/     # on rs (main server running): 200
```

### 3.2 Cluster Cases and Config Files

| Case | Sites per machine | rs runs | om runs | Main server (on om) |
|---|---|---|---|---|
| 1 (heterogeneous) | gpu + llvmpipe + cpu | `offload_rs_c1.csv` | `offload_om_c1.csv` | `cluster_endpoint_c1.csv` |
| 2 (homogeneous) | 2 × gpu | `offload_rs_c2.csv` | `offload_om_c2.csv` | `cluster_endpoint_c2.csv` |

All files are in `servers_container/`. Each `offload_*` file holds the same rows as its `cluster_endpoint_*` file without the `ip` column; keep them in sync when changing a case.

- **`offload_{om,rs}_{c1,c2}.csv`** (sites one machine starts): `id,port,decoding,composition,encoding,cpu,memory,delay_ms,jitter_ms`.
  - `id`: machine + number (`om1`, `rs2`; letters, digits, `-` only). The container is `obm-offload-<id>`.
  - `decoding` `cpu|gpu`, `composition` `cpu|llvmpipe|gpu`, `encoding` `cpu` (gpu encoding disabled).
  - `cpu` / `memory`: docker `--cpus` / `--memory` (GB) caps, `-1` = no cap.
  - `delay_ms` / `jitter_ms`: netem delay / jitter (normal distribution), `0` = none.
- **`cluster_endpoint_{c1,c2}.csv`** (sites the load balancer uses): same columns plus `ip` after `id`. The endpoint is `ip:port`, and the site name in the logs is every `col=value` joined by `_` (e.g. `id=rs1_ip=10.10.0.2_port=7010_decoding=gpu_…`).
  - **`ip` rule:** om's sites by container name (`obm-offload-om1`), rs's by `10.10.0.2`. The main server runs in a container on om, and om's containers cannot reach om's own `10.10.0.1` on the site ports.
- Lines starting with `#` are comments (each file documents its columns).

The load-balancing policy is chosen in `main()` of `rust_server/src/main_server.rs` (`RoundRobinLB` / `InterleavedWeightedRRLB`); changing it needs a main server rebuild (run `run_sh.sh` without `--skip-build`).

### 3.3 Running a Case (in this order)

```bash
# 0. both: plug the cable in, same commit on both machines
git pull

# 1. rs: start the far sites (builds the offload binary + image, one gnome-terminal tab per site; -d = detached)
./servers_container/run_all_offload.py servers_container/offload_rs_c1.csv

# 2. om: start the near sites
./servers_container/run_all_offload.py servers_container/offload_om_c1.csv

# 3. om: start the main server (needs a real terminal; Ctrl+C stops it)
./servers_container/main_server/run_sh.sh servers_container/cluster_endpoint_c1.csv [--skip-build]
```

`run_all_offload.py` checks the CSV (duplicate ids / ports, values), compiles the Rust offload server, builds the image `obm-offload-server`, removes old containers of the same ids, and starts the sites. `main_server/run_sh.sh` compiles `main_server`, builds `obm-main-server`, and runs it on `obm-net` with port `7000`, mounting read-only:
- `obm/assets/` → `/app/obm/assets`, `obm/shows/` → `/app/obm/shows`
- the cluster CSV → `/app/cluster_endpoint.csv` (passed as the server's argument; changing the cluster needs only a restart).

**Main server in the background** (instead of step 3, e.g. over SSH; image already built):

```bash
docker run -d --rm --init --network obm-net -p 7000:7000 \
    -v "$PWD/obm/assets:/app/obm/assets:ro" \
    -v "$PWD/obm/shows:/app/obm/shows:ro" \
    -v "$PWD/servers_container/cluster_endpoint_c1.csv:/app/cluster_endpoint.csv:ro" \
    --name obm-main-server obm-main-server /app/cluster_endpoint.csv
docker logs -f obm-main-server
```

**Verify before a benchmark** (on om):

```bash
docker logs obm-main-server | grep -E "Offload site|Load balancer"          # all sites of the case, the policy
docker logs obm-offload-om1 | grep "Assets from"                             # 10.10.0.1:7000 (same on rs)
curl -s -o seg.h264 -D - "http://localhost:7000/offload/show/spiders.json/0/10/1280/720/main/landscape/2/main|race/spider_blocker|Off" \
  | grep -E "HTTP|X-OBM-Worker|X-Dana-Timings"                               # 200, the chosen site, timings
ffplay -f h264 -autoexit seg.h264                                            # optional: watch the segment
```

Repeating the `curl` once per site should walk through every site (round robin). Expected work time: om ≈ 8.7 s gpu / 10.7 s llvm / 34 s cpu, rs ≈ 1.3× faster; rs's asset and net times are ~0.6 s higher (far delay).

**4. Benchmark** (on om): replay a generated sequence open-loop; the two labels name the run.

```bash
python3 servers_benchmark/seq_bench.py servers_benchmark/sequences/seq_a0.5_case1.csv rr SA
# -> stat/run_a0.5_c1/rr_SA_1.csv (then _2, _3, ... for repeats); --limit 50 for a smoke test
```

Use `seq_a*_case1.csv` with case 1 and `seq_a*_case2.csv` with case 2. The policy label must match the policy compiled into the main server.

### 3.4 Switching Case and Stopping

- **Switch case:** rerun steps 1–3 with the other case's files (step 1/2 replace the old sites only if the ids match, so stop the old sites first). No rebuild needed for the main server: `--skip-build`.
- **Stop** (each machine):

```bash
docker rm -f $(docker ps -q --filter name=obm-)     # offload sites (+ main server on om)
```

### 3.5 Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `ping 10.10.0.2` fails after plugging in | NetworkManager picked `Wired connection 1`: `sudo nmcli con up obm-link` |
| Need the Ethernet port on a normal network | `sudo nmcli con up "Wired connection 1"`; remove the cable setup with `sudo nmcli con delete obm-link` |
| Requests to a site never return (connect works) | the site can't download assets: check `docker logs obm-offload-<id> \| grep "Assets from"` shows `10.10.0.1:7000`, and from inside it `docker exec obm-offload-<id> curl -s -o /dev/null -w "%{http_code}\n" http://10.10.0.1:7000/` gives `200` (else the om `ufw` rules) |
| Main server requests to om's sites hang | the cluster CSV uses `10.10.0.1` for om; use the container names |
| `the input device is not a TTY` / `cannot attach stdin` | `main_server/run_sh.sh` needs a terminal; use the background `docker run` above |
| A site with a delay exits at startup | netem could not be applied (missing `sch_netem` on the host) |
| A gpu site exits at startup | the hardware check failed (`/dev/dri` missing or software renderer); see section 2 |

______________________________________________________________________

## 4. `servers_container/` Directory Breakdown

| File / Directory | Purpose |
| :--- | :--- |
| `main_server/` | Main server image and `run_sh.sh [CLUSTER_CSV] [--skip-build]` (port `7000`). |
| `offload_server/` | Offload site image (Rust proxy + Dana engine) and `run_sh.sh <ID> <PORT> <dec> <comp> <enc> [CPU] [MEMORY_GB] [DELAY_MS] [JITTER_MS]` for a single site (`DETACH=1` = background, `MAIN_SERVER_ADDR` = asset source). |
| `run_all_offload.py` | `[ENDPOINT_CSV] [-d] [-w]`: validates an endpoint CSV and starts all its sites (default CSV `offload_endpoint.csv`). |
| `offload_{om,rs}_{c1,c2}.csv` | Sites each machine starts for case 1 / 2. |
| `cluster_endpoint_{c1,c2}.csv` | Sites the main server load-balances over for case 1 / 2. |
| `offload_endpoint.csv` | Default endpoint CSV of `run_all_offload.py` (not used by the cluster cases). |
