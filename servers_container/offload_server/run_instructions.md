# OBM Offload Server Node (Rust Offload Server & Dana Offload Site)

This directory contains the Docker configuration and execution scripts for running **OBM Offload Server Nodes**, co-locating the **Rust Offload Server** and the **Dana Offload Engine**.

---

## 1. Architecture Overview

```
[Main Server / Client]
         │
         │  Port 7010, 7020, ...
         ▼
┌─────────────────────────────────────────────────────────────┐
│ CONTAINER: obm-offload-<ID>                                 │
│                                                             │
│  ┌───────────────────────────────────────────────────────┐  │
│  │ Rust Offload Server (Port <PORT> - EXPOSED)           │  │
│  │   - Receives offload & asset proxy requests           │  │
│  │   - Acts as local cache / asset provider for Dana     │  │
│  └───────────────────────────▲───────────────────────────┘  │
│                              │ ASSET_HOST (localhost:<PORT>)│
│  ┌───────────────────────────┴───────────────────────────┐  │
│  │ Dana Offload Site (Port 9009 - INTERNAL ONLY)         │  │
│  │   - Receives video rendering tasks                    │  │
│  │   - Renders frames and encodes H.264 video streams    │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

- **Rust Offload Server**: Listens on the specified port (e.g. `7010`, `7020`). It receives video offloading tasks and forwards them to Dana, and serves cached assets to Dana.
- **Dana Offload Site**: Listens on internal port `9009`. Its `ASSET_HOST` is configured to `http://localhost:<PORT>/` to request assets directly from the co-located Rust server.
- **Fast Build / Export Layer**: Large static media assets (`obm/assets/`, ~2GB) and shows (`obm/shows/`) are **excluded** from this container. Only the Dana runtime and OBM component code (~64MB) are packaged, keeping build times under 2 seconds.

---

## 2. Running an Offload Server

### A. Run All Configured Servers Automatically
Use `servers_container/run_all.py` (or `servers_container/run_all.sh`) to parse `offload_endpoint.csv`, check for duplicate IDs/ports, pre-compile binaries, pre-build Docker images, and launch all offload servers and the main server into separate tabs in a single `gnome-terminal` window:

```bash
./servers_container/run_all.py
# or:
./servers_container/run_all.sh
```

### B. Run a Specific Offload Instance Manually
You can launch a specific offload instance by passing its `ID` and `PORT`:

```bash
./servers_container/offload_server/run_sh.sh <ID> <PORT>
```

*Example:*
```bash
./servers_container/offload_server/run_sh.sh 1 7010
./servers_container/offload_server/run_sh.sh 2 7020
```

---

## 3. Endpoints & Ports

| Service | Container Port | Host Port | Accessibility | URL / Endpoint |
| :--- | :--- | :--- | :--- | :--- |
| **Rust Offload Server** | `<PORT>` (e.g. 7010) | `<PORT>` | **Exposed** | `obm-offload-<ID>:<PORT>` on `obm-net` |
| **Dana Offload Engine** | `9009` | None | **Internal Only** | `http://localhost:9009/` (Inside container only) |

---

## 4. Stopping the Server

Press `Ctrl + C` in the terminal tab. The container entrypoint traps the interrupt signal and cleanly shuts down both the Rust offload server and the Dana offload engine.
