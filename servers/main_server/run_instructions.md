# OBM Main Server (Stage 1 - Dana Web Server)

This directory contains the Docker configuration and execution scripts for running the **OBM Main Server** using the Dana web server (`ws.core` and `ws/Web.o`) on port **7000**.

---

## 1. Prerequisites

1. **Docker** installed and running on your system.
2. **Dana** installed on your host machine (e.g. at `$HOME/Application/Dana` with `DANA_HOME` set in your environment).

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
3. **Builds** the Docker image: `obm-main-server`.
4. **Launches** the container in **interactive mode** with port mapping (`-p 7000:7000`).

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

### Step 3: Build the Docker Image
From the repository root:
```bash
docker build -f servers/main_server/dockerfile -t obm-main-server .
```

### Step 4: Run the Container Interactively
```bash
docker run -it --rm \
    -p 7000:7000 \
    --name obm-main-server \
    obm-main-server
```

---

## 4. Connecting Cloudflare Tunnel

To expose your local main server to the public domain through Cloudflare Tunnel (as defined in `local_host.txt`), ensure your Cloudflare tunnel routes traffic to `http://localhost:7000`. Then open a **second terminal window** and run:

```bash
cloudflared tunnel run --token eyJhIjoiMDQzNzQ1NjU2MDBhMzVlNjMyOWFkZGI2ZjFiNjI5Y2YiLCJ0IjoiYjFlOWQ0ZWItOTVmNC00NTc3LWJkMTgtZTRiNWZkNmYxZjZjIiwicyI6Ik5EQXpaams1WW1JdE1XSXpPUzAwWW1WaExUazFZekF0WlRjeU9UZ3pNek14TkRjNCJ9
```

---

## 5. Testing & Verification

1. Open your web browser and navigate to:
   - Local access: `http://localhost:7000/`
   - Or your Cloudflare domain URL.
2. You will see the **Available shows** web catalogue (e.g. `forecast`, `f1`, `spiders`, `forest720`).
3. Click on any show to launch the WebAssembly (WASM) OBM player in your browser.
4. You will see live HTTP request logs in your interactive Docker terminal window.

---

## 6. Stopping the Server

Since the container runs in **interactive mode** (`-it`), press `Ctrl + C` in the container terminal window to cleanly stop and remove the container.

