# OBM Offload-1 Node (Rust Offload Server on Port 7010 & Dana Offload Site)

This directory contains the Docker configuration and execution scripts for running **OBM Offload-1 Node**, which co-locates the **Rust Offload Server** and the **Dana Offload Engine**.

---

## 1. Architecture Overview

```
[Main Server / Cloudflare Tunnel]
              │
              │  Port 7010 (https://obm_offload_1.leowong.space/)
              ▼
┌─────────────────────────────────────────────────────────────┐
│ CONTAINER: obm-offload-1                                    │
│                                                             │
│  ┌───────────────────────────────────────────────────────┐  │
│  │ Rust Offload Server (Port 7010 - EXPOSED)              │  │
│  │   - Receives offload & asset proxy requests           │  │
│  │   - Acts as local cache / asset provider for Dana     │  │
│  └───────────────────────────▲───────────────────────────┘  │
│                              │ ASSET_HOST (localhost:7010) │
│  ┌───────────────────────────┴───────────────────────────┐  │
│  │ Dana Offload Site (Port 9009 - INTERNAL ONLY)         │  │
│  │   - Receives video rendering tasks                    │  │
│  │   - Renders frames and encodes H.264 video streams    │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

- **Rust Offload Server**: Listens on port `7010` (the only exposed port). It is launched with `LOCAL_URL` (`https://obm_offload_1.leowong.space/`) and `PORT` (`7010`).
- **Dana Offload Site**: Listens on internal port `9009`. Its `ASSET_HOST` is configured to `http://localhost:7010/` to request assets directly from the co-located Rust server.
- **Fast Build / Export Layer**: Large static media assets (`obm/assets/`, ~2GB) and shows (`obm/shows/`) are **excluded** from this container. Only the Dana runtime and OBM component code (~64MB) are packaged, keeping build and export times down to a few seconds.

---

## 2. Prerequisites

1. **Docker** installed and running.
2. **Rust & Cargo** installed on the host machine.
3. *(Optional)* **Cloudflare Tunnel** for public routing:
   ```bash
   cloudflared tunnel run --token eyJhIjoiMDQzNzQ1NjU2MDBhMzVlNjMyOWFkZGI2ZjFiNjI5Y2YiLCJ0IjoiMmU5NzI2MjEtY2E5MC00MjY5LTljZWQtZTM2M2ZkZWQyODIzIiwicyI6Ik4yWTROR1EzWm1VdE9Ua3pZaTAwTjJKbUxUbG1Zemt0TmpBMFltUmtNamxsTVRsaCJ9
   ```

---

## 3. Quick Start (One-Command Automated Run)

Run the automated script from the repository root:

```bash
./servers_container/offload_1/run_sh.sh
```

### What this script does automatically:
1. **Compiles** the Rust offload server with optimizations:
   ```bash
   cargo build --release --bin offload_server
   ```
2. **Builds** the Docker image with fast layer caching:
   ```bash
   docker build --build-arg PORT=7010 -f servers_container/offload_1/dockerfile -t obm-offload-1 .
   ```
   During build, `OffloadSite.dn` is updated with `ASSET_HOST = "http://localhost:7010/"` and recompiled with `dnc`.
3. **Launches** the container interactively with SIGINT/SIGTERM forwarding:
   ```bash
   docker run -it --rm --init -p 7010:7010 --name obm-offload-1 obm-offload-1 "https://obm_offload_1.leowong.space/" "7010"
   ```

---

## 4. Endpoints & Ports

| Service | Container Port | Host Port | Accessibility | URL / Endpoint |
| :--- | :--- | :--- | :--- | :--- |
| **Rust Offload Server** | `7010` | `7010` | **Exposed** (Public) | `https://obm_offload_1.leowong.space/` or `http://localhost:7010/` |
| **Dana Offload Engine** | `9009` | None | **Internal Only** | `http://localhost:9009/` (Inside container only) |

---

## 5. Stopping the Servers

Press `Ctrl + C` in the terminal. The container entrypoint traps the interrupt signal and cleanly shuts down both the Rust offload server and the Dana offload engine.
