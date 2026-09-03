# OBM Main Server (Stage 2 - Rust Load Balancer & Dana Web Server)

This directory contains the Docker configuration, entrypoint, and execution scripts for running the **OBM Main Server** container with:
- **Rust Main Server** (`smol` asynchronous server) listening on public port **7000**.
- **Dana Web Server** (`ws.core` and `ws/Web.o`) running on port **8000**.

---

## 1. Prerequisites

1. **Docker** installed and running on your system.
2. **Dana** installed on your host machine (e.g. at `$HOME/Application/Dana` with `DANA_HOME` set in your environment).
3. **Rust & Cargo** installed on your host machine (for compiling with release optimizations).

---

## 2. Quick Start (One-Command Automated Run)

Run the automated script from the repository root or from this directory:

```bash
# Option A: From repository root
./servers/main_server/run_sh.sh

# Option B: From servers/main_server directory
cd servers/main_server
./run_sh.sh
```

### What this script does automatically:
1. **Pre-compiles** the Dana web server component (`dnc ws/Web.dn` inside `obm/`).
2. **Ensures** the Dana runtime binary (`dana` and `components/`) is staged in `servers/dana_runtime_copy/` (via `servers/dana_runtime_copy/setup.sh`).
3. **Compiles** the Rust server with optimizations: `cargo build --release --bin main_server`.
4. **Builds** the Docker image: `obm-main-server`.
5. **Launches** the container in **interactive mode** mapping both ports (`-p 7000:7000 -p 8000:8000`).

---

## 3. Manual Step-by-Step Instructions

If you prefer to run each step manually:

### Step 1: Pre-compile Web Server Component (on Host)
```bash
cd obm
dnc ws/Web.dn
cd ..
```

### Step 2: Stage Shared Dana Runtime Files
Run the setup script in `servers/dana_runtime_copy/` to stage the Dana runtime:
```bash
./servers/dana_runtime_copy/setup.sh
```

### Step 3: Compile Rust Server with Release Optimizations
From the repository root:
```bash
cd obm_servers
cargo build --release --bin main_server
cd ..
```

### Step 4: Build the Docker Image
From the repository root:
```bash
docker build -f servers/main_server/dockerfile -t obm-main-server .
```

### Step 5: Run the Container Interactively
```bash
docker run -it --rm \
    -p 7000:7000 \
    -p 8000:8000 \
    --name obm-main-server \
    obm-main-server
```

---

## 4. Connecting Cloudflare Tunnel

To expose your main server to the public domain through Cloudflare Tunnel, ensure your Cloudflare tunnel routes traffic to `http://localhost:7000` (or `http://localhost:8000`). Then open a **second terminal window** and run:

```bash
cloudflared tunnel run --token eyJhIjoiMDQzNzQ1NjU2MDBhMzVlNjMyOWFkZGI2ZjFiNjI5Y2YiLCJ0IjoiYjFlOWQ0ZWItOTVmNC00NTc3LWJkMTgtZTRiNWZkNmYxZjZjIiwicyI6Ik5EQXpaams1WW1JdE1XSXpPUzAwWW1WaExUazFZekF0WlRjeU9UZ3pNek14TkRjNCJ9
```

---

## 5. Testing & Verification

1. **Test Rust Main Server (Port 7000)**:
   In a separate terminal on your host:
   ```bash
   echo "Hello OBM Rust Server" | nc localhost 7000
   ```
   Or send an HTTP request:
   ```bash
   curl -i http://localhost:7000/
   ```
   You will see live logs in the Docker terminal:
   ```text
   Server received: Hello OBM Rust Server
   ```

2. **Test Dana Web Server (Port 8000)**:
   Open your browser and navigate to:
   - `http://localhost:8000/`
   You will see the **Available shows** catalogue and can launch the WASM OBM player.

---

## 6. Stopping the Server

Since the container runs in **interactive mode** (`-it`), press `Ctrl + C` in the container terminal window. The entrypoint script traps the signal and gracefully terminates both the Dana process and the Rust server before removing the container.
