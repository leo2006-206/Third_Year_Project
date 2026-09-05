# Evaluation Client & Test Harness Design (Client Testing)

## 1. Overview & Motivation

The default Object-Based Media (OBM) web interface renders controls directly onto WebGL `<canvas>` pixels via WebAssembly (Dana engine). While suitable for manual viewer interaction, it creates significant challenges for experimental evaluation:
- Headless test bots (Playwright, Puppeteer, Selenium, Python scripts) cannot inspect or trigger canvas-pixel buttons via standard DOM selectors.
- Decoding multi-layer 720p streams in software WebAssembly on client devices introduces heavy CPU throttling, obscuring network and load-balancing latency measurements.

The **Evaluation Test Page** (`/client_testing`) is a lightweight, static HTML/JS evaluation harness served directly by the Rust server. It provides a standardized interface for both human evaluation and automated bot benchmarks, with zero modifications required in the Dana OBM core.

---

## 2. Core Metadata: `show_options.json`

At initialization, the test page fetches `show_options.json` from the Rust server (`GET /api/show_options.json`). This file provides a unified catalog of all available shows, variants, and layer options:

```json
{
  "shows": [
    {
      "id": "f1_full.json",
      "title": "Formula 1 Race",
      "width": 1280,
      "height": 720,
      "fps": 25,
      "segment_length_sec": 10,
      "variants": [
        {
          "name": "race",
          "style": "landscape",
          "default": true,
          "layers": [
            {
              "name": "race",
              "options": ["race"],
              "default": "race"
            },
            {
              "name": "driver",
              "options": ["Off", "Sam"],
              "default": "Sam"
            },
            {
              "name": "track",
              "options": ["Off", "track"],
              "default": "track"
            },
            {
              "name": "drivers",
              "options": ["Off", "Sam,Alex"],
              "default": "Sam,Alex"
            }
          ]
        }
      ]
    },
    {
      "id": "forest720_leaves.json",
      "title": "Forest 720p Leaves",
      "width": 1280,
      "height": 720,
      "fps": 25,
      "segment_length_sec": 10,
      "variants": [ ... ]
    }
  ]
}
```

---

## 3. Web Page UI & Component Architecture

```text
+---------------------------------------------------------------------------------------+
|  OBM Evaluation Testbed & Metrics Harvester                     [ Status: STREAMING ] |
+---------------------------------------------------------------------------------------+
| Show Selector: [ Forest 720p Leaves (forest720_leaves.json) v ]                       |
| Offload Mode:  [ Full Offload (All Layers)                  v ]                       |
| Display Mode:  [x] Disable Video Display (Headless / Bot Mode)                        |
+-------------------------------------------+-------------------------------------------+
|                                           |  Active Layer Configuration               |
|                                           |  ---------------------------------------  |
|             VIDEO DISPLAY                 |  Variant:     [ core                  v ] |
|                                           |  Background:  [ background_default    v ] |
|  (Canvas / WebCodecs / Hidden if Headless)|  Creature:    [ creature_default      v ] |
|                                           |  Bird:        [ bird_default          v ] |
|                                           |  Leaves:      [ leaves_default        v ] |
|                                           |                                           |
|                                           |  [ Apply Changes ]      [ Reset ]         |
+-------------------------------------------+-------------------------------------------+
| Live Experiment Metrics & Instrumentation                                             |
| Segment: #2 (20_30s) | Latency: 124ms | Transfer: 1.84MB | Render/Frame Finish: 38ms   |
| [ Start Benchmark ]      [ Stop Benchmark ]      [ Download Metrics (CSV) ]           |
+---------------------------------------------------------------------------------------+
| TODO: Automation Mode (Scheduled / Randomized Layer Switching)                        |
+---------------------------------------------------------------------------------------+
```

### Key UI Features:
1. **Show Selector Dropdown**:
   - Standard `<select id="show_select">` populated dynamically from `show_options.json`.
   - Allows instant switching between different shows (`f1`, `forest`, `forecast`) by both human testers and automated bots.
2. **Dynamic Layer Option Selectors**:
   - Selecting a show generates an array of `<select id="layer_<name>">` elements for each layer.
   - Defaults are pre-selected based on `show_options.json`.
3. **"Apply Changes" Button (`<button id="apply_btn">`)**:
   - Commits the selected options. The next 10-second segment request automatically uses the updated option string.
4. **"Disable Video Display" Switch (Headless Bot Mode)**:
   - Checkbox / toggle switch (`<input type="checkbox" id="disable_display">`).
   - When enabled: The client skips canvas drawing and video decoding. It only receives the data, calculates the timings, and logs metrics.
   - **Purpose**: Enables testing high client counts (e.g. 20–50 parallel bot instances) without saturating client CPU on video decoding.
5. **Automation Mode (TODO)**:
   - Reserved for future in-browser scripted bot behavior (e.g., automated random option switches every $N$ seconds).

---

## 4. Request Generation & Server Routing

Requests sent by the test page strictly follow the OBM offload URL standard expected by Dana's `OffloadSite.dn`:

```http
GET /offload/show/<showID>/<timeStart>/<timeEnd>/<width>/<height>/<variant>/<style>/<offloadLayers>/<layer1>|<opt1>/<layer2>|<opt2>/...
```

- **Example Request**:
  ```http
  GET /offload/show/forest720_leaves.json/0/10/1280/720/core/landscape/7/background|default/creature|default/bird|default
  ```
- **Rust Main Server Role**:
  - Serves `client_testing.html` and `show_options.json`.
  - Intercepts `/offload/...` requests.
  - Applies load balancing policies (Round Robin, least-loaded, latency-based).
  - Forwards the request to the target Dana `OffloadSite` worker.
  - Streams the resulting chunk back to the client.

---

## 5. Metrics Instrumentation & CSV Recording

The test harness uses high-resolution timestamps (`performance.now()`) to track exact performance metrics across every segment:

### Metric Columns in `metrics.csv`:
| Column Name | Description |
| :--- | :--- |
| `timestamp_ms` | Absolute wall-clock epoch timestamp (ms). |
| `segment_index` | 0, 1, 2... index of the 10-second segment. |
| `time_range` | Video timestamp interval, e.g. `0_10`, `10_20`. |
| `show_id` | Identifier of the active show. |
| `mode` | `offload` or `non_offload`. |
| `active_options` | Serialized options string, e.g. `driver|Sam/track|track`. |
| `req_dispatch_ms` | Timestamp when the HTTP request was sent. |
| `ttfb_ms` | Time To First Byte received from server. |
| `element_recv_ms` | Timestamp when the video segment or layer element was 100% downloaded. |
| `render_finish_ms` | Timestamp when client finished decoding/rendering the segment (or frame in non-offload). |
| `perceived_latency_ms`| `render_finish_ms - req_dispatch_ms` (total client delay). |
| `chunk_bytes` | Payload size in bytes. |
| `stalls` | Count of buffer underflow stalls during this segment. |

### Non-Offload Mode Specifics:
When benchmarking in non-offload mode:
1. The client records the timestamp when each layer element (`.h264`, `.mask`) is fully received (`element_recv_ms`).
2. The client records the timestamp when the client CPU actually finishes compositing/rendering that segment or frame on the screen (`render_finish_ms`).
3. This directly captures the **local compute latency penalty**, providing empirical ground truth to compare against server offloading.

---

## 6. Testing Workflows

### A. Human Interactive Testing
1. Navigate to `http://localhost:7000/client_testing`.
2. Select a show and desired initial options from the dropdowns.
3. Click **Start Benchmark**.
4. During playback, switch layer options (e.g. change driver or toggle overlay) and click **Apply Changes**.
5. Observe real-time latency fluctuations on the dashboard.
6. Click **Download Metrics (CSV)** to save the evaluation session.

### B. Automated Bot Testing
1. Enable **"Disable Video Display"** (Headless mode).
2. Bot script (Playwright/Puppeteer/Selenium or HTTP runner):
   - Opens the page with query params: `?show=f1_full.json&mode=headless`.
   - Programmatically selects `<select>` options.
   - Triggers `apply_btn.click()`.
   - Automatically calls the CSV export trigger at the end of the test run.

