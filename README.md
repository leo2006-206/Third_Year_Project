# OBM Video Offload & Load Balancing Project

A distributed Object-Based Media (OBM) system with dynamic server-side video rendering offloading and load balancing.

---

## 1. System Architecture & Tech Stack

- **Main Server (Rust + `smol`)** — Port `7000`:
  - Serves the testing web client UI and Dana WebAssembly player runtime (`dana.wasm`, `dana.js`, `file_system.js`).
  - Serves raw media assets (`/assets/...`) for client-side local rendering.
  - Acts as a reverse proxy / load balancer for `/offload/show/...`, dispatching video rendering requests to offload servers and streaming H.264 video back to the browser.
- **Offload Nodes (Rust + Dana)** — Configurable Ports (e.g. `7010`, `7020`):
  - **Rust Offload Server**: Exposed container entrypoint. Forwards offload requests to Dana and serves as a local asset cache (`ASSET_HOST`).
  - **Dana Offload Engine** (Port `9009` internal): Headless Mesa/GL rendering engine that composites active show layers and encodes them into H.264 video streams.
- **Networking**:
  - **Local (Same host)**: Docker network `obm-net` (containers resolve each other directly by name, e.g. `obm-offload-1:7010`, `obm-offload-2:7020`).
  - **Distributed (Multi-machine)**: Tailscale P2P WireGuard mesh (`100.x.y.z:<PORT>`) for direct peer-to-peer communication.
  - **Public Access**: Cloudflare Tunnel routing `https://obm_main.leowong.space/` to `localhost:7000`.

---

## 2. How to Run the Servers

### Prerequisites
1. **Docker Network** (create once):
   ```bash
   docker network create obm-net 2>/dev/null || true
   ```
2. **Cloudflare Tunnel** (`cloudflared` installed on the host).

### Starting the Services
1. **Start All Servers (Offload Instances + Main Server)**:
   Reads `servers_container/offload_endpoint.csv`, checks for duplicates, pre-compiles and pre-builds container images, and launches each offload instance plus the Main Server in separate tabs in a single `gnome-terminal` window:
   ```bash
   ./servers_container/run_all.py
   # Or using the bash wrapper:
   ./servers_container/run_all.sh
   ```
   *(Or run an individual offload node manually: `./servers_container/offload_server/run_sh.sh 1 7010`)*.

2. **Start Cloudflare Tunnel for Main Server**:
   Exposes `localhost:7000` to `https://obm_main.leowong.space/` with automatic HTTPS/SSL:
   ```bash
   cloudflared tunnel run --token eyJhIjoiMDQzNzQ1NjU2MDBhMzVlNjMyOWFkZGI2ZjFiNjI5Y2YiLCJ0IjoiYjFlOWQ0ZWItOTVmNC00NTc3LWJkMTgtZTRiNWZkNmYxZjZjIiwicyI6Ik5EQXpaams1WW1JdE1XSXpPUzAwWW1WaExUazFZekF0WlRjeU9UZ3pNek14TkRjNCJ9
   ```
   *(This command is also saved in `servers_container/main_server/local_host.txt`)*.

4. Open `https://obm_main.leowong.space/` (or `http://localhost:7000/` locally) in your browser.

---

## 3. How to Add a New Offload Server

1. **Add an endpoint to [`servers_container/offload_endpoint.csv`](servers_container/offload_endpoint.csv)**:
   ```csv
   id, port
   1, 7010
   2, 7020
   ```
2. **Register in Main Server** (`servers_rust/src/bin/main_server.rs`):
   - Add `"obm-offload-2:7020"` to your offload server pool / load-balancer list.
3. **Run or re-run**:
   ```bash
   ./servers_container/run_all.sh
   ```
   `run_all.sh` will validate that there are no duplicate IDs or ports, compile the Rust binary once, and launch each offload instance in its own tab.

---

## 4. `servers_container/` Directory Breakdown

| File / Directory | Status | Purpose |
| :--- | :--- | :--- |
| `main_server/` | **Required** | Builds & runs the Rust Main Server container on port `7000`. |
| `offload_server/` | **Required** | Generic Offload Node container configuration (Rust + Dana). Parameterized by `ID` and `PORT`. |
| `offload_endpoint.csv` | **Required** | Defines active offload instance mappings (`id, port`). |
| `run_all.sh` | **Required** | Reads CSV, validates duplicates, and launches all offload instances in `gnome-terminal` tabs. |
| `dana_runtime_copy/` | **Required** | Local Dana runtime binaries & compiler (`dana`, `dnc`) used during Docker builds. |
| `original_obm_main/` | **Optional (Baseline)** | Legacy standalone Dana server (`dana ws.core -p 7000`). Only needed for comparative baseline benchmarks. |
