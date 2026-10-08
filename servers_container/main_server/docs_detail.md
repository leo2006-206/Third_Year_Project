# OBM Main Server (Rust Server on Port 7000)

This directory contains the Docker configuration and execution scripts for running the **Rust Main Server** on port **7000**.

Dana has been completely decoupled from this container:
- **Rust Main Server**: Runs in this container on port `7000`.
- **Original Dana Web Server**: Available in `servers_container/original_obm_main` on port `7000`.

---

## 1. Prerequisites

1. **Docker** installed and running on your system.
2. **Rust & Cargo** installed on your host machine (for building with `--release`).

---

## 2. Quick Start (One-Command Automated Run)

Run the automated script from the repository root:

```bash
./servers_container/main_server/run_sh.sh
```

### What this script does automatically:
1. **Compiles** the Rust server with optimizations: `cargo build --release --bin main_server`.
2. **Builds** the Docker image: `obm-main-server`.
3. **Launches** the container in interactive mode on `obm-net` mapping port `7000`:
   ```bash
   docker run -it --rm --init --network obm-net -p 7000:7000 \
       -v "$PWD/obm/assets:/app/obm/assets:ro" \
       -v "$PWD/obm/shows:/app/obm/shows:ro" \
       --name obm-main-server obm-main-server
   ```

---

## 3. Endpoints & Ports

| Service | Container Port | Host Port | URL |
| :--- | :--- | :--- | :--- |
| **Rust Main Server** | `7000` | `7000` | `http://localhost:7000/` |

---

## 4. Container Structure & Assets

Assets and show specifications are mounted directly from the host at runtime via read-only volumes (keeping the Docker image small and build times under 1 second):
- `obm/assets/` -> `/app/obm/assets:ro`: Cached video, mask, and media assets.
- `obm/shows/` -> `/app/obm/shows:ro`: Show definition files (`.json`).
- `/usr/local/bin/main_server`: The compiled Rust binary.
