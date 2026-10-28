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

## 3. How to Run the Docker Containers

### Prerequisites

1. **Docker Network** (create once):
   ```bash
   docker network create obm-net 2>/dev/null || true
   ```
2. **Tailscale & Edge VPS** (for remote workers and public HTTPS):
   - Local: Ensure Tailscale is running (`sudo systemctl start tailscaled && tailscale up`).
   - Cloud Edge VPS (`195.58.134.234`): Runs Caddy reverse-proxying `https://obm-main.leowong.space/` to your local Tailscale IP (`100.x.y.z:7000`).

### A. Running the Offload Servers

#### Cluster Mode (All Offload Nodes Automatically)
Reads an endpoint CSV (default `servers_container/offload_endpoint.csv`), checks for duplicate IDs/ports and valid CPU/memory caps, compiles the Rust offload server once, builds the Docker image, and launches all worker containers into separate tabs in `gnome-terminal` (`-d` for detached containers):

```bash
./servers_container/run_all_offload.py [ENDPOINT_CSV] [-d]
```

For the two-laptop clusters, each machine runs its own sites of a case (same rows as `cluster_endpoint_c1/c2.csv`, without `ip`):

| | case 1 (gpu + llvm + cpu each) | case 2 (2 gpu each) |
|---|---|---|
| om | `servers_container/offload_om_c1.csv` | `servers_container/offload_om_c2.csv` |
| rs | `servers_container/offload_rs_c1.csv` | `servers_container/offload_rs_c2.csv` |

#### Network Emulation (Near / Far Site Groups)
A non-zero `delay_ms` makes the container apply `tc netem delay <delay_ms>ms <jitter_ms>ms distribution normal` to everything it sends, **including asset downloads** (asset acquisition is a large share of offload time). It is egress-only, so every round trip the site makes (main server, asset server) grows by `delay_ms`. The container gets `--cap-add NET_ADMIN`; startup aborts if netem cannot be applied (e.g. the host lacks the `sch_netem` module), so a site never silently runs without its delay.

#### Individual Node (Manual Run)
You can launch an individual offload node by passing `ID`, `PORT`, `DECODING` (`cpu`|`gpu`), `COMPOSITION` (`cpu`|`llvmpipe`|`gpu`), and `ENCODING` (`cpu`):

```bash
./servers_container/offload_server/run_sh.sh <ID> <PORT> [cpu|gpu] [cpu|llvmpipe|gpu] [cpu] [CPU] [MEMORY_GB] [DELAY_MS] [JITTER_MS]
# CPU / MEMORY_GB: docker --cpus / --memory caps, -1 = no cap (defaults: -1 / 3.5)
# DELAY_MS / JITTER_MS: tc netem delay/jitter on ALL traffic the site sends, incl. asset downloads (default 0 = none)
# DETACH=1 runs the container in the background instead of interactively
```

*Examples:*
```bash
# Fastest combo: GPU decode, GPU composition, CPU encode (~3.2s)
./servers_container/offload_server/run_sh.sh 1 7010 gpu gpu cpu 2 1.5

# Software OpenGL combo: CPU decode, llvmpipe composition, CPU encode (~7.5s)
./servers_container/offload_server/run_sh.sh 2 7020 cpu llvmpipe cpu 8 1.5

# Pure software baseline: CPU decode, CPU SDL composition, CPU encode (~25s)
./servers_container/offload_server/run_sh.sh 3 7030 cpu cpu cpu 2 1.5
```

### B. Running the Main Server

The Main Server is a high-performance Rust web server that serves the UI harness, static show/asset files, and load-balances offload segment requests across available rendering worker nodes on port `7000`.

#### 1. Interactive Mode (Compile, Build & Run)
Run the automated runner from the project root:

```bash
./servers_container/main_server/run_sh.sh [CLUSTER_CSV]   # default: servers_container/cluster_endpoint_c1.csv
```

**What this script does:**
1. Compiles the Rust server binary with `--release` optimizations (`cargo build --release --bin main_server`).
2. Builds the lightweight Docker image `obm-main-server`.
3. Cleans up any existing `obm-main-server` container.
4. Mounts host directories directly via read-only volumes (no asset rebuilds required):
   - `obm/assets/` $\rightarrow$ `/app/obm/assets:ro`
   - `obm/shows/` $\rightarrow$ `/app/obm/shows:ro`
   - `CLUSTER_CSV` $\rightarrow$ `/app/cluster_endpoint.csv:ro` (offload sites for the load balancer, passed to `main_server` as its argument; changing the cluster needs no rebuild)
5. Connects to `obm-net` and exposes host port `7000`.

#### 2. Fast Startup (Skip Recompilation)
If the Rust binary and Docker image are already compiled:

```bash
./servers_container/main_server/run_sh.sh [CLUSTER_CSV] --skip-build
```

#### 3. Background / Daemon Mode
To run the Main Server continuously in the background:

```bash
# Start container in detached mode
docker run -d --rm --init \
    --network obm-net \
    -p 7000:7000 \
    -v "$PWD/obm/assets:/app/obm/assets:ro" \
    -v "$PWD/obm/shows:/app/obm/shows:ro" \
    -v "$PWD/servers_container/cluster_endpoint_c1.csv:/app/cluster_endpoint.csv:ro" \
    --name obm-main-server \
    obm-main-server /app/cluster_endpoint.csv

# Inspect live logs
docker logs -f obm-main-server

# Stop the container
docker stop obm-main-server
```

### C. Accessing the System

- **Local Machine**: `http://localhost:7000/`
- **Private Tailnet (P2P)**: `http://<LOCAL_TAILSCALE_IP>:7000/`
- **Public Domain (via Edge VPS)**: `https://obm-main.leowong.space/`

### D. Two-Laptop Cluster (Direct Ethernet Cable)

The two cluster machines are joined by one Ethernet cable as a private link with fixed addresses (no router, no gateway; internet stays on Wi-Fi). It only uses NetworkManager, so nothing registers.

| Machine | Role | Interface | Cable IP |
|---|---|---|---|
| `om` (main laptop) | main server + near sites | `eno1` | `10.10.0.1` |
| `rs` (second laptop) | far sites | `enp6s0` | `10.10.0.2` |

**One-time setup** (the profile is saved; afterwards just plug the cable in):

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

`autoconnect-priority 10` makes `obm-link` win over NetworkManager's default DHCP profile (`Wired connection 1`), which would otherwise wait for a router that isn't there.

**Check the link:**

```bash
ping -c 3 10.10.0.2                                          # from om, expect < 1 ms
curl -s -o /dev/null -w "%{http_code}\n" http://10.10.0.2:7010/   # rs offload site reachable (any code but 000)
```

**Cluster CSV:** use the cable IPs in the `ip` column (`10.10.0.1` for om's sites, `10.10.0.2` for rs's); the main server container reaches om's own sites through their published host ports.

**Troubleshooting:**
- Ping fails after plugging in: the default profile took the port; run `sudo nmcli con up obm-link`.
- Using the port on a normal network (uni / home): `sudo nmcli con up "Wired connection 1"`; remove the cable setup with `sudo nmcli con delete obm-link`.
- If `ufw` is active: `sudo ufw allow from 10.10.0.0/24`.

The cable adds well under 1 ms, so the near / far delays (om 30 ms, rs 60 ms) come only from the per-site `DELAY_MS` / `JITTER_MS` netem settings (see Network Emulation above).

______________________________________________________________________

## 4. How to Add a New Offload Server

1. **Add an endpoint to [`servers_container/offload_endpoint.csv`](servers_container/offload_endpoint.csv)**:
   ```csv
   id,port,decoding,composition,encoding,cpu,memory,delay_ms,jitter_ms
   1,7010,gpu,gpu,cpu,4,2,50,5
   2,7020,gpu,llvmpipe,cpu,8,2,50,5
   3,7030,gpu,cpu,cpu,2,2,100,10
   ```
2. **Register in Main Server** (`rust_server/src/bin/main_server.rs`):
   - Add `"obm-offload-3:7030"` to the offload server pool / load balancer list.
3. **Launch the cluster**:
   ```bash
   ./servers_container/run_all_offload.py
   ```

______________________________________________________________________

## 5. `servers_container/` Directory Breakdown

| File / Directory | Purpose |
| :--- | :--- |
| `main_server/` | Builds and runs the Rust Main Server container on port `7000`. |
| `offload_server/` | Offload Node container configuration (Rust proxy + Dana engine). Parameterized by `ID`, `PORT`, `DECODING`, `COMPOSITION`, and `ENCODING`. |
| `offload_endpoint.csv` | Active offload instance mappings (`id,port,decoding,composition,encoding,cpu,memory`); `cpu`/`memory` are docker `--cpus`/`--memory` (GB) caps, `-1` = no cap; `delay_ms`/`jitter_ms` add `tc netem` delay to all site traffic (near 30 ms / far 60 ms groups), `0` = none. |
| `offload_{om,rs}_{c1,c2}.csv` | Per-machine offload sites for cluster case 1 / 2 (input to `run_all_offload.py`). |
| `run_all_offload.py` | Validates an endpoint CSV (default `offload_endpoint.csv`) and launches all offload worker containers into terminal tabs. |

***

## 6. Play Video (CLI Verification)

Test offloaded segment generation and video playback directly from the command line:

1. Request and stream a 10-second composited H.264 segment:
   ```bash
   curl -s "https://obm-main.leowong.space/offload/show/f1_full.json/0/10/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam" -o segment.h264
   ```
2. Play the downloaded segment with `ffplay`:
   ```bash
   ffplay -f h264 -autoexit segment.h264
   ```

______________________________________________________________________

## 7. Self-Hosted Edge Tunnel (VPS + Caddy + Tailscale)

To eliminate third-party tunnel quotas (such as Cloudflare Tunnel's 100 MB request ceiling and 100-second execution timeouts), public access is routed through a private cloud edge VPS:

```
[Public Client] ──HTTPS──> [VPS: 195.58.134.234 (Caddy)] ──Tailscale (WireGuard)──> [Local Host: 100.x.y.z:7000]
```

### VPS Configuration Reference:
- **Cloudflare DNS**: An `A` record pointing `obm-main.leowong.space` to `195.58.134.234` with **DNS Only** (Grey Cloud).
- **Caddy Configuration** (`/etc/caddy/Caddyfile` on the VPS):
  ```caddyfile
  obm-main.leowong.space {
      reverse_proxy 100.75.221.63:7000 {
          transport http {
              response_header_timeout 600s
          }
      }
  }
  ```
- **Management Commands on VPS**:
  ```bash
  systemctl restart caddy    # Apply config changes
  systemctl status caddy     # Verify TLS certificates and proxy status
  journalctl -u caddy -f     # Inspect live access and error logs
  tailscale status           # Verify P2P connection to local machine
  ```


______________________________________________________________________

## 8. Lab Machine Setup (No Root, Redo Every Session)

The university lab PC blocks software that registers itself with the system (apt packages, system services, Tailscale's `install.sh`). Everything below installs into your home directory and runs as your normal user, so it is allowed. If the lab wipes your home directory between sessions, repeat these steps every time.

The lab PC only needs to **accept** connections from the main server on the laptop (offload ports `7010`, `7020`, ...). Assets are served by a local `obm-main-server` container on the lab PC, so nothing large crosses the network except the request and the returned video segment.

### Step 1: Install Rust (user-local, via rustup)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
source "$HOME/.cargo/env"
cargo --version
```

### Step 2: Run Tailscale from the standalone binaries (no install, no root)

Do **not** use Tailscale's `install.sh` (it adds an apt repository and a systemd service). The standalone tarball is just two programs; `--tun=userspace-networking` needs no root and creates no network interface. Run this inside `tmux` so it keeps running after you disconnect:

```bash
tmux new -s ts

mkdir -p ~/ts && cd ~/ts
curl -LO https://pkgs.tailscale.com/stable/tailscale_1.104.2_amd64.tgz
tar xzf tailscale_1.104.2_amd64.tgz && cd tailscale_1.104.2_amd64

# userspace networking: no root, no network interface created
./tailscaled --tun=userspace-networking \
  --state=$HOME/ts/tailscaled.state --socket=$HOME/ts/tailscaled.sock &

./tailscale --socket=$HOME/ts/tailscaled.sock up --hostname=lab-pc   # prints a login URL
./tailscale --socket=$HOME/ts/tailscaled.sock ip -4                  # the lab PC's 100.x address
```

- In userspace mode, incoming connections to the lab PC's `100.x` address are forwarded to its local ports, so Docker's published ports (`0.0.0.0:7010`) are reachable from the laptop at `<lab-100.x>:7010`.
- **Skip the browser login:** create a reusable, **ephemeral** auth key in the Tailscale admin console and run `./tailscale --socket=$HOME/ts/tailscaled.sock up --hostname=lab-pc --authkey=tskey-...`. Ephemeral nodes are removed automatically when they go offline, so a fresh session does not end up as `lab-pc-1`, `lab-pc-2`, ...
- **Check the connection type** (record it for the experiment): `./tailscale --socket=$HOME/ts/tailscaled.sock netcheck`. From the laptop, `tailscale ping lab-pc` shows `via <ip>:<port>` for a direct link or `via DERP(...)` when relayed (slower, bandwidth-limited).
- Detach from tmux with `Ctrl+b d`; reattach with `tmux attach -t ts`.

### Step 3: Get the project and the `obm/` folder

```bash
git clone <this repository> ThirdYearProject && cd ThirdYearProject
# obm/ is gitignored: copy it into the project root (USB / scp from the laptop / clone barryfp/obm)
```

Docker must be usable without `sudo` (your user is in the `docker` group). Check with `docker ps`.

### Step 4: Start the local asset server and the offload sites

```bash
./servers_container/main_server/run_sh.sh        # local obm-main-server (assets only), in its own terminal/tmux window
./servers_container/run_all_offload.py           # builds and starts the sites in offload_endpoint.csv (or pass a CSV)
```

Edit `servers_container/offload_endpoint.csv` first if the lab PC should run a different set of sites or caps.

### Step 5: Verify from the laptop

```bash
tailscale ping lab-pc
curl -s -o /dev/null -w "%{http_code} %{time_total}s\n" \
  "http://<lab-100.x>:7010/offload/show/spiders.json/0/10/1280/720/main/landscape/2/main|race/spider_blocker|Off"
```

Then add the lab sites (`<lab-100.x>:7010`, ...) to the main server's offload list.

### Fallback: reverse tunnel through the VPS

If Tailscale cannot connect from the campus network, the lab PC can open an **outgoing** SSH tunnel to the VPS (already on the tailnet), and the main server connects to the VPS instead:

```bash
ssh -N -o ServerAliveInterval=30 -o ExitOnForwardFailure=yes \
  -R 17010:localhost:7010 -R 17020:localhost:7020 -R 17030:localhost:7030 root@195.58.134.234
```

The main server then uses `<vps-100.x>:17010`, ... Every request and segment goes through the VPS, so this adds latency; note it in the experiment method. For a tunnel that reconnects automatically, `cargo install rathole` (server on the VPS, client on the lab PC) works the same way.

### Teardown

```bash
docker rm -f $(docker ps -q --filter name=obm-)          # offload sites + asset server
./tailscale --socket=$HOME/ts/tailscaled.sock down        # from ~/ts/tailscale_1.104.2_amd64
pkill -u "$USER" tailscaled
```
