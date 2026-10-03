# Dana Offload Server Rendering Pipeline: Profiling & OpenGL Acceleration Report

This document benchmarks Dana's offload rendering pipeline before and after enabling **OpenGL hardware acceleration** for a **10-second segment (250 frames at 25 fps, 1280x720)** (`f1_full.json`).

---

## 1. Executive Summary: The Massive Breakthrough

By enabling headless OpenGL via `Xvfb` + Mesa DRI, offload rendering went from **slower than real-time (10.8 fps, 23.4s)** to **super real-time (39.0 fps, 6.7s)**:

| Metric | Before: Software Fallback | After: OpenGL Hardware (GPU) | After: OpenGL (CPU llvmpipe) | Improvement |
| :--- | :---: | :---: | :---: | :---: |
| **Total Request Wall-Clock** | **23,407 ms (23.4s)** | **6,705 ms (6.7s)** | **8,243 ms (8.2s)** | **3.5x Faster! 🚀** |
| **Processing Throughput** | **10.8 fps** | **39.0 fps** | **31.3 fps** | **Super Real-Time!** |
| **2D Canvas Compositing** | **16,086 ms (16.1s)** | **1,881 ms (1.88s)** | **1,826 ms (1.83s)** | **8.5x Faster! 🔥** |
| **Framebuffer Readback** | **4,002 ms (4.0s)** | **869 ms (0.87s)** | **652 ms (0.65s)** | **4.6x Faster! 🔥** |
| **Combined Composition Bottleneck** | **20,088 ms (20.1s)** | **2,750 ms (2.75s)** | **2,478 ms (2.48s)** | **7.3x Speedup** |

---

## 2. Granular Stage-by-Stage Profiling Breakdown (250 Frames)

| Pipeline Stage | Before: Software GPU (ms) | After: OpenGL GPU (ms) | After: OpenGL CPU (ms) | Per-Frame Avg (OpenGL GPU) |
| :--- | :---: | :---: | :---: | :---: |
| **1. Resource Wait (HTTP / Network)** | 12 ms | 9 ms | 17 ms | 0.0 ms |
| **2. Prepare (Decode Video + Scene)** | 1,262 ms | 1,509 ms | 4,498 ms | 6.0 ms (GPU) / 18 ms (CPU) |
| **3. Render (Canvas 2D Compositing)** | **16,086 ms** | **1,881 ms** | **1,826 ms** | **7.5 ms (was 64.3 ms!)** |
| **4. Readback (`glReadPixels` to RAM)** | **4,002 ms** | **869 ms** | **652 ms** | **3.4 ms (was 16.0 ms!)** |
| **5. YUV Convert (`rgbaToYUV`)** | 314 ms | 436 ms | 368 ms | 1.7 ms |
| **6. H.264 Encoder (Frame Encode)** | 1,461 ms | 1,669 ms | 346 ms | 6.6 ms (VA-API) / 1.3 ms (CPU) |
| **7. Encoder Finish (Flush Buffer)** | 0 ms | 0 ms | 247 ms | — |
| **8. Frame Packaging (IHDR/NAL)** | 2 ms | 7 ms | 2 ms | — |
| **Total Frame Processing Loop** | **23,148 ms** | **6,408 ms** | **7,980 ms** | **25.6 ms / frame** |
| **Pre-Loop Overhead (Spec & ResLoad)** | 248 ms | 297 ms | 263 ms | — |
| **Overall Wall Clock Duration** | **23,407 ms (23.4s)** | **6,705 ms (6.7s)** | **8,243 ms (8.2s)** | — |

---

## 3. What Was Solved

### The Problem
1. Dana's native library (`UIPlaneLib`) requests an OpenGL hardware renderer by default (`SDL_CreateRenderer(..., SDL_RENDERER_ACCELERATED)`).
2. However, it was compiled against the **X11 video driver**.
3. In headless Docker, with no X11 display server running, the OpenGL context creation silently failed, forcing Dana to fall back to a single-threaded CPU software rasterizer (`SDL_CreateRenderer(..., 0)`), which took **64.3 ms per frame** just to scale and blit the video layers.

### The Solution
1. Added `xvfb`, `libgl1-mesa-dri`, and `mesa-utils` into the container.
2. In `entrypoint.sh`, started a virtual headless X11 display in memory:
   ```bash
   Xvfb :99 -screen 0 1920x1080x24 -ac +extension GLX +render -noreset &
   export DISPLAY=:99
   export SDL_VIDEODRIVER=x11
   export SDL_RENDER_DRIVER=opengl
   ```
3. The dummy X11 handshake succeeds $\rightarrow$ SDL attaches directly to Mesa DRI / Intel GPU (`/dev/dri/renderD128`) $\rightarrow$ OpenGL hardware shaders composite and scale video frames in **7.5 ms per frame** instead of 64.3 ms!
