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
  - **Dana Offload Engine** (Port `9009` internal): Headless Mesa/GL rendering engine supporting 3 strict acceleration tiers:
    - **`gpu` (Level 3)**: Physical GPU direct rendering via VirtualGL (`vglrun -d "$GPU_CARD"`) + VA-API hardware video decoding (`Decoder.h264va`). Pre-flight verified at startup.
    - **`llvmpipe` (Level 2)**: CPU Software OpenGL rasterizer (`Mesa llvmpipe`) + CPU video decoding.
    - **`cpu` (Level 1)**: Pure CPU software pipeline (SDL software blitter) + CPU video decoding.
  - **Pre-Flight Device Verification**: Each offload container asserts and verifies its active graphics driver against the requested tier at startup; if a device tier mismatch occurs or falls back to software on `gpu`, the container prints a fatal error and terminates immediately (`exit 1`).
- **Networking**:
  - **Local (Same host)**: Docker network `obm-net` (containers resolve each other directly by name, e.g. `obm-offload-1:7010`, `obm-offload-2:7020`, `obm-offload-3:7030`).
  - **Distributed (Multi-machine)**: Tailscale P2P WireGuard mesh (`100.x.y.z:<PORT>`) for direct peer-to-peer communication across hosts.
  - **Public Access**: Cloudflare Tunnel routing `https://obm_main.leowong.space/` to `localhost:7000`.

______________________________________________________________________

## 2. Host Machine Requirements for the 3 Offload Tiers

To run all 3 acceleration tiers (`gpu`, `llvmpipe`, `cpu`) correctly on a machine without startup assertions failing, the host system must meet the following requirements:

### Tier Requirements Matrix

| Requirement | Tier 3: `gpu` (Hardware Accelerated) | Tier 2: `llvmpipe` (Software OpenGL) | Tier 1: `cpu` (Pure Software) |
| :--- | :--- | :--- | :--- |
| **Physical Hardware** | Dedicated or integrated GPU with H.264 decode (Intel UHD/Iris/Arc or AMD) | Any multi-core x86_64 CPU (No GPU required) | Any multi-core x86_64 CPU (No GPU required) |
| **Linux DRM Subsystem** | `/dev/dri/card*` (for VirtualGL) and `/dev/dri/renderD*` (for VA-API) | Not required | Not required |
| **Docker Device Mount** | Mandatory: `--device /dev/dri:/dev/dri` | None | None |
| **Host User Groups** | User in `video` and `render` groups (`sudo usermod -aG video,render $USER`) | Standard Docker access | Standard Docker access |
| **Compositing Driver** | VirtualGL 3.1.5 + Mesa DRI direct hardware rendering | CPU Mesa software rasterizer (`llvmpipe`) | Pure SDL2 software blitter |
| **Video Decoding** | GPU VA-API hardware acceleration (`Decoder.h264va`) | Software CPU decoding (`libavcodec`) | Software CPU decoding (`libavcodec`) |
| **Video Encoding** | Multi-threaded CPU `libx264` | Multi-threaded CPU `libx264` | Multi-threaded CPU `libx264` |
| **Avg Render Time** | **~3.2s – 4.7s** | **~7.0s – 8.5s** | **~25s – 35s** |
| **Strict Assertion** | Fails startup if `/dev/dri` missing or renderer is `llvmpipe`/software | Asserts active renderer is `llvmpipe` | Sets software driver |

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
You can launch an individual offload node by passing `ID`, `PORT`, and acceleration tier (`gpu`, `llvmpipe`, or `cpu`):

```bash
./servers_container/offload_server/run_sh.sh <ID> <PORT> [gpu|llvmpipe|cpu]
```

*Examples:*
```bash
./servers_container/offload_server/run_sh.sh 1 7010 gpu
./servers_container/offload_server/run_sh.sh 2 7020 llvmpipe
./servers_container/offload_server/run_sh.sh 3 7030 cpu
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
   id, port, device
   1, 7010, gpu
   2, 7020, llvmpipe
   3, 7030, cpu
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
| `offload_server/` | Offload Node container configuration (Rust proxy + Dana engine). Parameterized by `ID`, `PORT`, and `DEVICE` (`gpu`/`llvmpipe`/`cpu`). |
| `offload_endpoint.csv` | Active offload instance mappings (`id, port, device`). |
| `run_all_offload.py` | Validates endpoints CSV and launches all offload worker containers into terminal tabs. |
| `dana_runtime_copy/` | Local Dana runtime binaries and compiler (`dana`, `dnc`) used during Docker builds. |
| `original_obm_main/` | Legacy standalone Dana server (`dana ws.core -p 7000`) used for comparative baseline benchmarks. |

***

## 6. Play Video

1. Open a directory
2. run `curl -s "https://obm_main.leowong.space/offload/show/f1_full.json/0/10/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam" -o segment.h264 `
3. run `ffplay -f h264 -autoexit segment.h264`
