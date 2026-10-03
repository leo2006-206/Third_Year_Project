# Dana Offload Server Rendering Pipeline: Profiling Report

This benchmark profiles the rendering latency of Dana's offload pipeline when generating a **10-second segment (250 frames at 25 fps, 1280x720)** for the test show (`f1_full.json`).

---

## 1. Empirical Results: GPU vs. CPU Offload

| Pipeline Stage | GPU Node (VA-API) | CPU Node (libx264) | Per-Frame Avg | % Total Time |
| :--- | :---: | :---: | :---: | :---: |
| **1. Resource Wait (HTTP / Network)** | 12 ms | 25 ms | 0.1 ms | **0.1%** |
| **2. Prepare (Decode Video + Scene)** | 1,262 ms | 4,077 ms | 5.0 – 16.3 ms | **5.4% – 15.9%** |
| **3. Render (Canvas 2D Compositing)** | **16,086 ms** | **16,639 ms** | **64.3 ms** | **65.0% – 69.4% 🔥** |
| **4. Readback (`window.getPixels` to RAM)** | **4,002 ms** | **4,053 ms** | **16.0 ms** | **15.8% – 17.2% 🔥** |
| **5. YUV Convert (`rgbaToYUV`)** | 314 ms | 304 ms | 1.2 ms | **1.2%** |
| **6. H.264 Encoder (Frame Encode)** | 1,461 ms | 172 ms | 0.6 – 5.8 ms | **0.6% – 6.3%** |
| **7. Encoder Finish (Flush Buffer)** | 0 ms | 291 ms | — | **1.1%** |
| **8. Frame Packaging (IHDR/NAL)** | 2 ms | 2 ms | — | **0.0%** |
| **Total Frame Processing Loop** | **23,148 ms** | **25,575 ms** | **92.6 – 102 ms** | **100.0%** |
| **Pre-Loop Overhead (Spec & Variant)** | 248 ms | 301 ms | — | — |
| **Overall Wall-Clock Duration** | **23,407 ms (23.4s)** | **25,881 ms (25.8s)** | — | — |
| **Processing Throughput** | **10.8 fps** | **9.7 fps** | — | (Real-time is 25 fps) |

---

## 2. Key Findings: Where the Latency Comes From

Encoding, decoding, and network downloads are **not** the main bottlenecks:
```
Total Frame Loop: 23,148 ms
├── 3. Canvas 2D Compositing (UIPlaneLib) : 16,086 ms (69.4%) ─── 64.3 ms/frame
├── 4. Framebuffer Readback (getPixels)   :  4,002 ms (17.2%) ─── 16.0 ms/frame
└── Everything else (decode, encode, net) :  3,060 ms (13.4%)
```

- **Compositing + Readback account for 20.1s (86.6%) of the 23s total rendering time**.

### Why Compositing is Slow (~16.1s / 64.3 ms per frame)
In headless Docker containers, there is no hardware display server (X11/Wayland). Dana falls back to `UIPlaneLib`'s software CPU rasterizer (`flow_bitmap_yuv`). For each frame, it software-rescales and alpha-blends:
1. 1080p race video down to 720p.
2. 360p driver video down to 422x238 (PiP).
3. Animated track map and driver vector marker.

### Why Readback is Slow (~4.0s / 16.0 ms per frame)
`window.getPixels()` copies the rendered 2D canvas buffer back to RAM as a Dana `PixelMap` ($1280 \times 720 \times 4\text{ bytes} \approx 3.68\text{ MB/frame}$). Over 250 frames, this copies **920 MB of uncompressed RGBA pixel data** in RAM.

---

## 3. Why Local Mode Takes 10s vs. Offload Mode 23s

| Phase | Local Mode (WASM / Browser / Native Window) | Offload Mode (Headless Docker Server) |
| :--- | :--- | :--- |
| **Execution Model** | Real-time playback (25 fps). | Headless batch transcoding pipeline. |
| **Compute Time** | **< 1.5s total compute**; CPU/GPU **sleeps 8.5s** (`window.wait()` throttled to 25 fps clock). | **23+ seconds** running at 100% CPU capacity with zero sleeping. |
| **Compositing** | **Hardware GPU Canvas**: Browser WebGL/Canvas2D scales and blits layers in **< 1 ms per frame**. | **CPU Software Rasterizer (`UIPlaneLib`)**: Pure CPU loops scale and blit pixels in **64.3 ms per frame**. |
| **Readback** | **Zero readback**: Surface displayed directly to screen by GPU. | **920 MB readback**: Canvas copied to host RAM via `window.getPixels()` (**16 ms/frame, 4.0s total**). |
| **Encoding** | **Zero encoding**: Decodes and displays; never encodes. | **Full H.264 Re-encode**: Compresses all 250 frames into a new H.264 bitstream. |

---

## 4. Hardware vs. Software Codec Impact

- **VA-API Decoder (`Decoder.h264va`)**: Decodes 250 frames in **1.26s** vs. **4.08s** in software (`libavcodec`), saving **2.8 seconds**.
- **Encoder**: Software `libx264` (ultrafast) takes **463 ms**, while VA-API hardware encoder takes **1,461 ms**.
- **Conclusion**: GPU codecs provide a minor speedup (~2.4s overall), but cannot overcome the ~20s CPU compositing and readback bottleneck without offscreen GPU rasterization (e.g. EGL / GBM).
