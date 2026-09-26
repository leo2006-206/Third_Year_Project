# OBM Video Offload & Load Balancing Project

A distributed Object-Based Media (OBM) system with server-side video rendering offloading and load balancing.

______________________________________________________________________

## 1. System Architecture & What the Servers Do

- **Main Server (Rust + `smol`)** — Port `7000`:
  - Serves the testing web client UI (benchmark and evaluation harness).
  - Serves raw media assets (`/assets/...`) and show specifications (`/shows/...`).
  - Acts as a reverse proxy / load balancer for `/offload/shows/...`: receives segment rendering requests, routes them to offload worker servers, and streams generated H.264 video chunks back to the client.
- **Offload Nodes (Rust + Dana Engine)** — Configurable Ports (e.g. `7010`, `7020`):
  - **Rust Offload Server**: Exposed container entrypoint. Acts as a reverse proxy, forwards offload requests to the local Dana engine, and serves as a local asset cache (`ASSET_HOST`) for Dana.
  - **Dana Offload Engine** (Port `9009` internal): Headless Mesa/GL rendering engine that composites active show layers and encodes them into H.264 video streams using hardware acceleration (Intel VA-API GPU via `/dev/dri`) or CPU fallback.
- **Networking**:
  - **Local (Same host)**: Docker network `obm-net` (containers resolve each other directly by name, e.g. `obm-offload-1:7010`, `obm-offload-2:7020`).
  - **Distributed (Multi-machine)**: Tailscale P2P WireGuard mesh (`100.x.y.z:<PORT>`) for direct peer-to-peer communication across hosts.
  - **Public Access**: Cloudflare Tunnel routing `https://obm_main.leowong.space/` to `localhost:7000`.

______________________________________________________________________

## 2. How to Run the Docker Containers

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
You can launch an individual offload node by passing `ID`, `PORT`, and optional device (`gpu` or `cpu`):

```bash
./servers_container/offload_server/run_sh.sh <ID> <PORT> [gpu|cpu]
```

*Examples:*
```bash
./servers_container/offload_server/run_sh.sh 1 7010 gpu
./servers_container/offload_server/run_sh.sh 2 7020 cpu
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

## 3. How to Add a New Offload Server

1. **Add an endpoint to [`servers_container/offload_endpoint.csv`](servers_container/offload_endpoint.csv)**:
   ```csv
   id, port, device
   1, 7010, gpu
   2, 7020, cpu
   3, 7030, cpu
   ```
2. **Register in Main Server** (`servers_rust/src/bin/main_server.rs`):
   - Add `"obm-offload-3:7030"` to the offload server pool / load balancer list.
3. **Launch the cluster**:
   ```bash
   ./servers_container/run_all_offload.py
   ```

______________________________________________________________________

## 4. `servers_container/` Directory Breakdown

| File / Directory | Purpose |
| :--- | :--- |
| `main_server/` | Builds and runs the Rust Main Server container on port `7000`. |
| `offload_server/` | Offload Node container configuration (Rust proxy + Dana engine). Parameterized by `ID`, `PORT`, and `DEVICE` (`gpu`/`cpu`). |
| `offload_endpoint.csv` | Active offload instance mappings (`id, port, device`). |
| `run_all_offload.py` | Validates endpoints CSV and launches all offload worker containers into terminal tabs. |
| `dana_runtime_copy/` | Local Dana runtime binaries and compiler (`dana`, `dnc`) used during Docker builds. |
| `original_obm_main/` | Legacy standalone Dana server (`dana ws.core -p 7000`) used for comparative baseline benchmarks. |
