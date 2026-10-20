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
Reads `servers_container/offload_endpoint.csv`, checks for duplicate IDs/ports, compiles the Rust offload server once, builds the Docker image, and launches all worker containers into separate tabs in `gnome-terminal`:

```bash
./servers_container/run_all_offload.py
```

#### Individual Node (Manual Run)
You can launch an individual offload node by passing `ID`, `PORT`, `DECODING` (`cpu`|`gpu`), `COMPOSITION` (`cpu`|`llvmpipe`|`gpu`), and `ENCODING` (`cpu`):

```bash
./servers_container/offload_server/run_sh.sh <ID> <PORT> [cpu|gpu] [cpu|llvmpipe|gpu] [cpu]
```

*Examples:*
```bash
# Fastest combo: GPU decode, GPU composition, CPU encode (~3.2s)
./servers_container/offload_server/run_sh.sh 1 7010 gpu gpu cpu

# Software OpenGL combo: CPU decode, llvmpipe composition, CPU encode (~7.5s)
./servers_container/offload_server/run_sh.sh 2 7020 cpu llvmpipe cpu

# Pure software baseline: CPU decode, CPU SDL composition, CPU encode (~25s)
./servers_container/offload_server/run_sh.sh 3 7030 cpu cpu cpu
```

### B. Running the Main Server

The Main Server is a high-performance Rust web server that serves the UI harness, static show/asset files, and load-balances offload segment requests across available rendering worker nodes on port `7000`.

#### 1. Interactive Mode (Compile, Build & Run)
Run the automated runner from the project root:

```bash
./servers_container/main_server/run_sh.sh
```

**What this script does:**
1. Compiles the Rust server binary with `--release` optimizations (`cargo build --release --bin main_server`).
2. Builds the lightweight Docker image `obm-main-server`.
3. Cleans up any existing `obm-main-server` container.
4. Mounts host directories directly via read-only volumes (no asset rebuilds required):
   - `obm/assets/` $\rightarrow$ `/app/obm/assets:ro`
   - `obm/shows/` $\rightarrow$ `/app/obm/shows:ro`
5. Connects to `obm-net` and exposes host port `7000`.

#### 2. Fast Startup (Skip Recompilation)
If the Rust binary and Docker image are already compiled:

```bash
./servers_container/main_server/run_sh.sh --skip-build
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
    --name obm-main-server \
    obm-main-server

# Inspect live logs
docker logs -f obm-main-server

# Stop the container
docker stop obm-main-server
```

### C. Accessing the System

- **Local Machine**: `http://localhost:7000/`
- **Private Tailnet (P2P)**: `http://<LOCAL_TAILSCALE_IP>:7000/`
- **Public Domain (via Edge VPS)**: `https://obm-main.leowong.space/`

______________________________________________________________________

## 4. How to Add a New Offload Server

1. **Add an endpoint to [`servers_container/offload_endpoint.csv`](servers_container/offload_endpoint.csv)**:
   ```csv
   id,port,decoding,composition,encoding
   1,7010,gpu,gpu,cpu
   2,7020,gpu,gpu,cpu
   3,7030,cpu,llvmpipe,cpu
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
| `offload_endpoint.csv` | Active offload instance mappings (`id,port,decoding,composition,encoding`). |
| `run_all_offload.py` | Validates endpoints CSV and launches all offload worker containers into terminal tabs. |
| `dana_runtime_copy/` | Local Dana runtime binaries and compiler (`dana`, `dnc`) used during Docker builds. |
| `original_obm_main/` | Legacy standalone Dana server (`dana ws.core -p 7000`) used for comparative baseline benchmarks. |

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

