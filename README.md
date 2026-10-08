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
  - **Public Access**: Cloudflare Tunnel routing `https://obm_main.leowong.space/` to `localhost:7000`.

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
2. **Cloudflare Tunnel** (`cloudflared` installed on the host, if public access is needed).

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

Run the automated build and startup script from the project root:

```bash
./servers_container/main_server/run_sh.sh
```

This compiles the Rust `main_server`, builds the lightweight Docker image, mounts `obm/assets` and `obm/shows` directly as read-only volumes, and starts the container on port `7000`.

### C. Accessing the System

- **Local**: Open `http://localhost:7000/` in your browser.
- **Public / Remote**: Start your Cloudflare Tunnel to access `https://obm_main.leowong.space/`.

______________________________________________________________________

## 4. How to Add a New Offload Server

1. **Add an endpoint to [`servers_container/offload_endpoint.csv`](servers_container/offload_endpoint.csv)**:
   ```csv
   id,port,decoding,composition,encoding
   1,7010,gpu,gpu,cpu
   2,7020,gpu,gpu,cpu
   3,7030,cpu,llvmpipe,cpu
   ```
2. **Register in Main Server** (`servers_rust/src/bin/main_server.rs`):
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

## 6. Play Video

1. Open a directory
2. run `curl -s "https://obm_main.leowong.space/offload/show/f1_full.json/0/10/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam" -o segment.h264 `
3. run `ffplay -f h264 -autoexit segment.h264`
