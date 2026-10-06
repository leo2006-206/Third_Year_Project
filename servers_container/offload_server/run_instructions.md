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
Use `servers_container/run_all_offload.py` to parse `offload_endpoint.csv`, check for duplicate IDs/ports, pre-compile binaries, pre-build Docker images, and launch all offload worker nodes into separate tabs in a single `gnome-terminal` window:

```bash
./servers_container/run_all_offload.py
```

### B. Run a Specific Offload Instance Manually
You can launch a specific offload instance by passing its `ID`, `PORT`, and hardware acceleration tier (`gpu`, `llvmpipe`, or `cpu`):

```bash
./servers_container/offload_server/run_sh.sh <ID> <PORT> [gpu|llvmpipe|cpu]
```

*Example:*
```bash
./servers_container/offload_server/run_sh.sh 1 7010 gpu
./servers_container/offload_server/run_sh.sh 2 7020 llvmpipe
./servers_container/offload_server/run_sh.sh 3 7030 cpu
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

---

## 5. Component Configuration & Pipeline Analysis

Dana's offload rendering pipeline consists of several distinct stages: video decoding, 2D canvas composition, pixel readback, and video encoding. Below is a comprehensive breakdown of the available configurations, their performance/functional impacts, and their required dependencies:

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                                DANA OFFLOAD PIPELINE                                    │
│                                                                                        │
│  [Source Video] ──► 1. DECODING ──► 2. COMPOSITION ──► 3. READBACK ──► 4. ENCODING     │
│                     (GPU vs CPU)     (OpenGL vs CPU)    (GPU vs CPU)   (x264 vs VA-API)│
└────────────────────────────────────────────────────────────────────────────────────────┘
```

### A. 2D Scene Composition (Canvas Blitting & Scaling)
* **Available Configurations**:
  * **Tier 1: Pure CPU Software (`SDL_RENDER_DRIVER=software`)**: Uses SDL's CPU software rasterizer without OpenGL.
  * **Tier 2: CPU Software OpenGL (`SDL_RENDER_DRIVER=opengl`, `LIBGL_ALWAYS_SOFTWARE=1`)**: Uses Mesa's `llvmpipe` CPU rasterizer with OpenGL shaders.
  * **Tier 3: Physical GPU Direct Rendering (VirtualGL + Mesa DRI)**: Direct hardware-accelerated OpenGL rendering via VirtualGL (`vglrun -d "$GPU_CARD"`) communicating with the host KMS/DRI graphics card (`/dev/dri/card*`).
* **Performance / Functional Impact**:
  * Tier 1 (CPU Software): **~16,000 ms – 29,000 ms** (64 – 115 ms/frame) — major bottleneck, limits throughput to ~10 fps.
  * Tier 2 (Mesa llvmpipe): **~7,000 ms – 8,000 ms** (28 – 32 ms/frame) — multi-threaded software OpenGL rasterization.
  * Tier 3 (Physical GPU VirtualGL): **~3,200 ms – 4,700 ms** (13 – 19 ms/frame) — **fastest**, offloads 3D rendering and compositing to the physical GPU.
* **Strict Startup Pre-Flight Assertions (`entrypoint.sh`)**:
  To guarantee that containers strictly run on the requested device tier without silent software fallbacks:
  - When started with tier `gpu`, the container probes VirtualGL (`vglrun -d "$GPU_CARD" glxinfo -B`) and VA-API (`vainfo --display drm --device "$RENDER_DEV"`). If the renderer detects `llvmpipe`, `software`, or fails to find hardware decoding (`VAEntrypointVLD`), it **immediately prints a fatal error and terminates (`exit 1`)**.
  - When started with tier `llvmpipe`, it asserts that `glxinfo` reports `llvmpipe`. If not, it halts (`exit 1`).
* **Dependencies & Settings Required**:
  1. Virtual X11 display: `Xvfb :99 -screen 0 1920x1080x24 -ac +extension GLX +render -noreset &`
  2. Environment variables: `DISPLAY=:99`, `SDL_VIDEODRIVER=x11`, `SDL_RENDER_DRIVER=opengl`
  3. System packages: `virtualgl` (3.1.5), `xvfb`, `libgl1`, `libglu1-mesa`, `libgl1-mesa-dri`, `mesa-utils`, `libxtst6`, `libxv1`
  4. Docker device mount (Tier 3 only): `--device /dev/dri:/dev/dri`
  5. Native library patch: `UIPlaneLib[deb.x64].dnl` is patched at Docker build time to initialize at $1280 \times 720$.

### B. Video Decoding (Prepare Stage)
* **Available Configurations**:
  * **Option 1: CPU Software Decoder**: Default `media.video.Decoder:h264` (FFmpeg `libavcodec`).
  * **Option 2: GPU VA-API Hardware Decoder**: `media/video/Decoder.h264va.o` mapped via Dana's `-lc` component switch:
    ```bash
    dana -lc "media.video.Decoder:h264|media/video/Decoder.h264va.o|media.video.Decoder:h264va" OffloadSite_new
    ```
* **Performance / Functional Impact**:
  * Option 1 (CPU Decoder): **4,498 ms** (18.0 ms/frame).
  * Option 2 (GPU VA-API Decoder): **1,262 ms – 1,509 ms** (5.0 – 6.0 ms/frame) — **3x faster decoding**, offloading video decompression to the GPU hardware video processor (VPU).
* **Dependencies & Settings Required**:
  1. Host physical GPU device: `/dev/dri/renderD128` (Intel Quick Sync or AMD VA-API)
  2. Docker permission: `--device /dev/dri:/dev/dri`
  3. System packages: `libva2`, `libva-drm2`, `libvdpau1`, `intel-media-va-driver-non-free`, `mesa-va-drivers`, `vainfo`
  4. Compiled Dana component: `dnc media/video/Decoder.h264va.dn`

### C. Video Encoding (H.264 Segment Generation)
* **Available Configurations**:
  * **Option 1: CPU Software Encoder (`media.video.Encoder:h264` via `libx264`)**: [ACTIVE STANDARD]
  * **Option 2: GPU Hardware Encoder (`Encoder.h264va` via VA-API)**: [DEPRECATED / REMOVED]
* **Performance / Functional Impact**:
  * Option 1 (CPU `libx264`): **346 ms** for 250 frames (1.3 ms/frame) — **4.8x FASTER than hardware!**
  * Option 2 (GPU VA-API): **1,669 ms** (6.6 ms/frame) due to VA-API context recreation, surface synchronization, and packet extraction overhead for short 10-second segments.
  * **Standards Compliance & Stream Validity**:
    - `libx264`: Emits **100% standards-compliant** H.264 streams containing SPS/PPS parameter sets, IDR keyframes, and proper SEI units. Decodes with 0 errors in all browsers, `ffplay`, and VLC.
    - `Encoder.h264va`: Omits SPS/PPS headers on standalone segments, throwing `non-existing PPS 0 referenced` and failing 100% in `ffplay` and browser WebCodecs.
  * **Architecture Decision**: All offload acceleration tiers (`cpu`, `llvmpipe`, `gpu`) strictly use CPU `libx264` encoding.
* **Dependencies & Settings Required**:
  - `H264Lib[deb.x64].dnl` with `libx264` (packaged within Dana runtime). No GPU driver or extra dependencies required.

### D. Framebuffer Pixel Readback (`window.getPixels`)
* **Available Configurations**:
  * **Option 1: CPU Memory Blit**: Software surface clone (`SDL_BlitSurface` / `memcpy`).
  * **Option 2: OpenGL Framebuffer Readback**: GPU readback via `SDL_RenderReadPixels` $\to$ `glReadPixels`.
* **Performance / Functional Impact**:
  * Option 1 (CPU Blit): **4,002 ms** (16.0 ms/frame).
  * Option 2 (OpenGL Readback): **652 ms – 869 ms** (3.4 ms/frame) — **4.6x faster**.
* **Dependencies & Settings Required**:
  - Window initialization must match target stream resolution ($1280 \times 720$) to prevent reading beyond buffer bounds.

### E. Multi-Threading & Batching (`-batch`)
* **Available Configurations**:
  * **Option 1: Sequential Loop**: Default in REST server mode (`M_LISTEN`).
  * **Option 2: Multi-Threaded Batch Mode**: Activated via `-batch` in direct CLI mode (`M_DIRECT`).
* **Performance / Functional Impact**:
  * Spawns a pool of 5 asynchronous worker threads (`asynch::work`) to parallelize CPU-bound image transformations (chromakey, alphamask) over 50-frame chunks.
* **Dependencies & Settings Required**:
  - Multi-core CPU; Dana runtime thread management support.

---

## 6. Dana Code Changes & Engine Optimizations

Two specific code-level adaptations were made to Dana's engine and configuration to support high-performance containerized offloading:

### A. Window Initialization Resolution Patch ($640 \times 480 \to 1280 \times 720$)
* **Target File**: `servers_container/dana_runtime_copy/components/resources-ext/UIPlaneLib[deb.x64].dnl` (Byte offset `0x39b68`).
* **Original Code**:
  In `UIPlaneLib.flow_makeWindow`:
  ```asm
  movabs $0x1e000000280, %rcx   # 0x0280 = 640 width, 0x01e0 = 480 height
  call SDL_CreateWindow
  call SDL_CreateRenderer       # OpenGL viewport and glOrtho initialized to 640x480
  ```
* **Why the Change Was Needed**:
  Dana's `OffloadSite.dn` creates the window with `new FlowRender(25)` (which invoked `flow_makeWindow` at $640 \times 480$) and only called `window.setSize(1280, 720)` later.
  Under headless X11 (`Xvfb`), `SDL_SetWindowSize` only sends an asynchronous resize request to the X server. Because `OffloadSite.dn` never pumped the X11 event loop after resizing, SDL's OpenGL renderer never updated `glViewport` or `glOrtho`.
  This caused severe visual defects:
  1. Content was clamped/squashed into the bottom-left $640 \times 360$ quadrant.
  2. The top half was filled with uninitialized video RAM static noise.
  3. Offscreen chromakey surfaces read back upside down with horizontal scanline striations.
* **The Change**:
  Patched the hardcoded initial window dimension from $640 \times 480$ (`48 b9 80 02 00 00 e0 01 00 00`) to **$1280 \times 720$** (`48 b9 00 05 00 00 d0 02 00 00`).
* **Result**:
  The OpenGL viewport, projection matrix, sub-surface textures, and pixel readbacks match $1280 \times 720$ 1:1 right from initialization, rendering **100% pixel-perfect video with 0 visual artifacts** at full GPU speed.

### B. ASSET_HOST Dynamic Port Binding
* **Target File**: `servers_container/offload_server/OffloadSite_new.dn` (Line 80; keeping `obm/OffloadSite.dn` 100% untouched).
* **Original Code**:
  ```dana
  const char ASSET_HOST[] = "http://localhost:8080/"
  ```
* **Why the Change Was Needed**:
  In the OBM offload architecture, each container runs a co-located Rust offload server on an assigned port (e.g. `7010`, `7020`). Dana needs to request assets (source video chunks, metadata, overlays) from its co-located Rust proxy server, which acts as a local cache and asset provider.
* **The Change**:
  In `servers_container/offload_server/dockerfile` and `entrypoint.sh`, `ASSET_HOST` is dynamically rewritten in `OffloadSite_new.dn` to match the container's configured port and recompiled:
  ```bash
  sed -i "s|http://localhost:[0-9]*/|http://localhost:${PORT}/|g" /app/obm/OffloadSite_new.dn
  (cd /app/obm && dnc OffloadSite_new.dn)
  ```
* **Result**:
  Dana offload workers seamlessly route asset downloads through their local Rust offload proxy cache on their assigned port.

### C. Clock-Skew-Free Timing Instrumentation ($T_\text{queue}, T_\text{asset}, T_\text{work}$)
* **Target File**: `servers_container/offload_server/OffloadSite_new.dn`.
* **Metrics Recorded**:
  - **$T_\text{queue}$**: Elapsed time waiting in Dana's task queue between arrival (`queueRequest`) and being dequeued into service (`M_INIT`).
  - **$T_\text{asset}$**: Elapsed time between downloading spec JSON and all variant assets finishing download (`variantReady`).
  - **$T_\text{work}$**: Pure rendering, transformation, and H.264 video encoding execution time.
* **Response Headers**:
  Dana returns the timings via custom HTTP headers:
  ```http
  X-Dana-Timings: queue=1;asset=35;work=142
  Server-Timing: queue;dur=1, asset;dur=35, work;dur=142
  Access-Control-Expose-Headers: X-Dana-Timings, Server-Timing
  ```
* **Network Latency Estimation ($T_\text{net}$)**:
  Because all Dana durations are measured locally, $T_\text{net}$ is calculated on the client/main server without any clock skew:
  $$T_\text{net} = T_\text{total} - (T_\text{queue} + T_\text{asset} + T_\text{work})$$


