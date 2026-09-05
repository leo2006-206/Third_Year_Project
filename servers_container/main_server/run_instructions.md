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
3. **Launches** the container in interactive mode mapping port `7000`:
   ```bash
   docker run -it --rm -p 7000:7000 --name obm-main-server obm-main-server
   ```

---

## 3. Endpoints & Ports

| Service | Container Port | Host Port | URL |
| :--- | :--- | :--- | :--- |
| **Rust Main Server** | `7000` | `7000` | `http://localhost:7000/` |

---

## 4. Container Structure & Assets

The container preserves all OBM asset directories needed by the Rust server:
- `/app/obm/assets/`: Cached video, mask, and media assets (~2GB).
- `/app/obm/shows/`: Show definition files (`.json`).
- `/app/obm/std_ui/`: Standard UI elements and icons.
- `/usr/local/bin/main_server`: The compiled Rust binary.
