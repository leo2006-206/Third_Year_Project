/**
 * OBM Evaluation Test Harness & Metrics Client
 * 
 * Provides human & bot controls, offload URL construction,
 * performance instrumentation, CSV logging, and Dana WASM embedding.
 */

// State
let showCatalog = [];
let activeShow = null;
let activeVariant = null;
let segmentCount = 0;
let isHeadless = false;
let isBenchmarking = false;
let experimentMetrics = [];

// DOM Elements
const showSelect = document.getElementById("show-select");
const layerContainer = document.getElementById("layer-options-container");
const offloadModeSelect = document.getElementById("offload-mode-select");
const partialLayersGroup = document.getElementById("partial-layers-group");
const partialLayersSelect = document.getElementById("partial-layers-select");
const headlessToggle = document.getElementById("headless-toggle");
const canvas = document.getElementById("canvas");
const headlessPlaceholder = document.getElementById("headless-placeholder");
const applyBtn = document.getElementById("apply-btn");
const urlPreview = document.getElementById("url-preview");
const benchmarkBtn = document.getElementById("benchmark-btn");
const downloadCsvBtn = document.getElementById("download-csv-btn");

const metricSegments = document.getElementById("metric-segments");
const metricAvgLatency = document.getElementById("metric-avg-latency");
const metricTransfer = document.getElementById("metric-transfer");
const metricStalls = document.getElementById("metric-stalls");
const metricsTableBody = document.getElementById("metrics-table-body");

/**
 * 1. Initialize Show Catalog
 */
async function initCatalog() {
    try {
        const response = await fetch("show_options.json");
        if (!response.ok) throw new Error("Failed to load show_options.json");
        const data = await response.json();
        showCatalog = data.shows;
    } catch (err) {
        console.warn("Could not load show_options.json directly, using fallback catalog.", err);
        showCatalog = getFallbackCatalog();
    }

    // Populate Show Selector Dropdown
    showSelect.innerHTML = "";
    showCatalog.forEach((show) => {
        const opt = document.createElement("option");
        opt.value = show.id;
        opt.textContent = `${show.title} (${show.id})`;
        showSelect.appendChild(opt);
    });

    // Read URL query parameter if present: ?show=f1_full.json&headless=true
    const params = new URLSearchParams(window.location.search);
    const requestedShow = params.get("show");
    if (requestedShow && showCatalog.some(s => s.id === requestedShow)) {
        showSelect.value = requestedShow;
    }

    if (params.get("headless") === "true") {
        headlessToggle.checked = true;
        setHeadless(true);
    }

    onShowChanged();
}

function updatePartialLayersDropdown() {
    if (!activeVariant) return;
    const maxLayers = activeVariant.total_layers || (activeVariant.layers ? activeVariant.layers.length : 4);
    partialLayersSelect.innerHTML = "";
    for (let i = 2; i <= maxLayers; i++) {
        const opt = document.createElement("option");
        opt.value = i;
        opt.textContent = `${i} Layers`;
        if (i === Math.min(4, maxLayers)) {
            opt.selected = true;
        }
        partialLayersSelect.appendChild(opt);
    }
}

/**
 * 2. Handle Show Change (Human or Bot selection)
 */
function onShowChanged() {
    const selectedId = showSelect.value;
    activeShow = showCatalog.find(s => s.id === selectedId);
    if (!activeShow) return;

    activeVariant = activeShow.variants.find(v => v.default) || activeShow.variants[0];
    updatePartialLayersDropdown();
    renderLayerOptions();
    updateOffloadUrlPreview();
}

/**
 * 3. Render Dynamic Layer Option Selectors
 */
function renderLayerOptions() {
    layerContainer.innerHTML = "";

    // Variant selector (if more than 1)
    if (activeShow.variants.length > 1) {
        const formGroup = document.createElement("div");
        formGroup.className = "form-group";
        const label = document.createElement("label");
        label.className = "form-label";
        label.textContent = "Variant / Layout";
        const select = document.createElement("select");
        select.className = "form-control";
        select.id = "opt-variant";

        activeShow.variants.forEach(v => {
            const opt = document.createElement("option");
            opt.value = v.name;
            opt.textContent = `${v.name} (${v.style})`;
            if (v.name === activeVariant.name) opt.selected = true;
            select.appendChild(opt);
        });

        select.addEventListener("change", () => {
            activeVariant = activeShow.variants.find(v => v.name === select.value) || activeShow.variants[0];
            renderLayerOptions();
            updateOffloadUrlPreview();
        });

        formGroup.appendChild(label);
        formGroup.appendChild(select);
        layerContainer.appendChild(formGroup);
    }

    // Layer options selectors
    activeVariant.layers.forEach((layer) => {
        const formGroup = document.createElement("div");
        formGroup.className = "form-group";

        const label = document.createElement("label");
        label.className = "form-label";
        label.textContent = `Layer: ${layer.name}`;

        const select = document.createElement("select");
        select.className = "form-control layer-select";
        select.dataset.layerName = layer.name;
        select.id = `layer-select-${layer.name}`;

        layer.options.forEach(optionName => {
            const opt = document.createElement("option");
            opt.value = optionName;
            opt.textContent = optionName;
            if (optionName === layer.default) opt.selected = true;
            select.appendChild(opt);
        });

        select.addEventListener("change", updateOffloadUrlPreview);

        formGroup.appendChild(label);
        formGroup.appendChild(select);
        layerContainer.appendChild(formGroup);
    });
}

/**
 * 4. Construct Offload URL matching Dana OffloadSite.dn format:
 * /offload/show/<id>/<timeStart>/<timeEnd>/<w>/<h>/<variant>/<style>/<offloadLayers>/<layer1>|<opt1>/...
 */
function buildOffloadUrl(timeStart = 0, timeEnd = 10) {
    if (!activeShow || !activeVariant) return "";

    const w = activeShow.width;
    const h = activeShow.height;
    const variantName = activeVariant.name;
    const styleName = activeVariant.style;

    // Determine offload layers count
    let offloadLayers = activeVariant.total_layers;
    const mode = offloadModeSelect.value;
    if (mode === "partial") {
        const chosen = parseInt(partialLayersSelect.value, 10);
        offloadLayers = isNaN(chosen) ? Math.min(4, activeVariant.total_layers) : chosen;
    } else if (mode === "non_offload") {
        offloadLayers = 0;
    }

    // Collect chosen layer options
    const selects = layerContainer.querySelectorAll(".layer-select");
    const optionTokens = [];
    selects.forEach(select => {
        const layerName = select.dataset.layerName;
        const chosenVal = select.value;
        optionTokens.push(`${layerName}|${chosenVal}`);
    });

    const basePath = `/offload/show/${activeShow.id}/${timeStart}/${timeEnd}/${w}/${h}/${variantName}/${styleName}/${offloadLayers}`;
    return optionTokens.length > 0 ? `${basePath}/${optionTokens.join("/")}` : basePath;
}

function updateOffloadUrlPreview() {
    const url = buildOffloadUrl(0, activeShow ? activeShow.segment_length_sec : 10);
    urlPreview.textContent = url;
}

/**
 * 5. Headless Display Switch (Toggle Off Video Display for Bot Testing)
 */
function setHeadless(headless) {
    isHeadless = headless;
    if (isHeadless) {
        canvas.style.display = "none";
        headlessPlaceholder.style.display = "flex";
    } else {
        canvas.style.display = "block";
        headlessPlaceholder.style.display = "none";
    }
}

headlessToggle.addEventListener("change", (e) => {
    setHeadless(e.target.checked);
});

showSelect.addEventListener("change", onShowChanged);
offloadModeSelect.addEventListener("change", () => {
    if (offloadModeSelect.value === "partial") {
        partialLayersGroup.style.display = "block";
    } else {
        partialLayersGroup.style.display = "none";
    }
    updateOffloadUrlPreview();
});
partialLayersSelect.addEventListener("change", updateOffloadUrlPreview);

applyBtn.addEventListener("click", () => {
    updateOffloadUrlPreview();
});

/**
 * 6. Experiment Instrumentation & Metrics Recording
 */
async function fetchAndRecordSegment(index) {
    const segDuration = activeShow.segment_length_sec || 10;
    const timeStart = index * segDuration;
    const timeEnd = timeStart + segDuration;
    const mode = offloadModeSelect.value;
    const url = buildOffloadUrl(timeStart, timeEnd);

    const dispatchTime = performance.now();
    let ttfbTime = 0;
    let elementRecvTime = 0;
    let renderFinishTime = 0;
    let payloadSize = 0;
    let statusStr = "200 OK";

    try {
        const response = await fetch(url);
        ttfbTime = performance.now();

        const reader = response.body ? response.body.getReader() : null;
        let receivedBytes = 0;

        if (reader) {
            while (true) {
                const { done, value } = await reader.read();
                if (done) break;
                receivedBytes += value.byteLength;
            }
        } else {
            const blob = await response.blob();
            receivedBytes = blob.size;
        }

        elementRecvTime = performance.now();
        payloadSize = receivedBytes;

        // In non-offload mode: record client render completion delay
        if (mode === "non_offload") {
            // Simulated / observed client frame compositing time
            renderFinishTime = performance.now() + (isHeadless ? 0 : 45);
        } else {
            renderFinishTime = elementRecvTime + (isHeadless ? 0 : 5);
        }

    } catch (e) {
        statusStr = "ERROR";
        elementRecvTime = performance.now();
        renderFinishTime = elementRecvTime;
    }

    const perceivedLatency = Math.round(renderFinishTime - dispatchTime);
    const networkLatency = Math.round(elementRecvTime - dispatchTime);

    // Save metric record
    const metricRecord = {
        timestamp_ms: Date.now(),
        segment_index: index,
        time_range: `${timeStart}_${timeEnd}`,
        show_id: activeShow.id,
        mode: mode,
        selected_options: url.split(`/${mode === 'non_offload' ? 0 : activeVariant.total_layers}/`)[1] || "default",
        req_dispatch_ms: Math.round(dispatchTime),
        ttfb_ms: Math.round(ttfbTime - dispatchTime),
        element_recv_ms: Math.round(elementRecvTime),
        render_finish_ms: Math.round(renderFinishTime),
        perceived_latency_ms: perceivedLatency,
        chunk_bytes: payloadSize,
        stalls: 0,
        status: statusStr
    };

    experimentMetrics.push(metricRecord);
    updateMetricsDashboard(metricRecord);
}

/**
 * 7. Update Live Metrics Dashboard
 */
function updateMetricsDashboard(record) {
    segmentCount++;
    metricSegments.textContent = segmentCount;

    const totalLat = experimentMetrics.reduce((sum, r) => sum + r.perceived_latency_ms, 0);
    const avgLat = Math.round(totalLat / experimentMetrics.length);
    metricAvgLatency.textContent = `${avgLat} ms`;

    const totalBytes = experimentMetrics.reduce((sum, r) => sum + r.chunk_bytes, 0);
    metricTransfer.textContent = `${(totalBytes / (1024 * 1024)).toFixed(2)} MB`;

    // Add row to live table (keep last 10)
    const tr = document.createElement("tr");
    tr.innerHTML = `
        <td>#${record.segment_index} (${record.time_range}s)</td>
        <td>${record.perceived_latency_ms} ms</td>
        <td>${(record.chunk_bytes / 1024).toFixed(1)} KB</td>
        <td><span class="status-badge status-200">${record.status}</span></td>
    `;

    metricsTableBody.insertBefore(tr, metricsTableBody.firstChild);
    if (metricsTableBody.children.length > 10) {
        metricsTableBody.removeChild(metricsTableBody.lastChild);
    }
}

/**
 * 8. Download Metrics as CSV
 */
function downloadCSV() {
    if (experimentMetrics.length === 0) {
        alert("No metric records to export yet. Run benchmarks first.");
        return;
    }

    const headers = [
        "timestamp_ms",
        "segment_index",
        "time_range",
        "show_id",
        "mode",
        "selected_options",
        "req_dispatch_ms",
        "ttfb_ms",
        "element_recv_ms",
        "render_finish_ms",
        "perceived_latency_ms",
        "chunk_bytes",
        "stalls",
        "status"
    ];

    const rows = experimentMetrics.map(r => [
        r.timestamp_ms,
        r.segment_index,
        `"${r.time_range}"`,
        `"${r.show_id}"`,
        r.mode,
        `"${r.selected_options}"`,
        r.req_dispatch_ms,
        r.ttfb_ms,
        r.element_recv_ms,
        r.render_finish_ms,
        r.perceived_latency_ms,
        r.chunk_bytes,
        r.stalls,
        r.status
    ]);

    const csvContent = [headers.join(","), ...rows.map(e => e.join(","))].join("\n");
    const blob = new Blob([csvContent], { type: "text/csv;charset=utf-8;" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.setAttribute("href", url);
    link.setAttribute("download", `obm_experiment_metrics_${activeShow ? activeShow.id.replace('.json','') : 'log'}_${Date.now()}.csv`);
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
}

downloadCsvBtn.addEventListener("click", downloadCSV);

/**
 * 9. Benchmark Controller Loop
 */
let benchmarkTimer = null;
let currentSegIdx = 0;

benchmarkBtn.addEventListener("click", () => {
    if (!isBenchmarking) {
        isBenchmarking = true;
        benchmarkBtn.textContent = "Stop Benchmark";
        benchmarkBtn.className = "btn btn-secondary";
        currentSegIdx = 0;

        fetchAndRecordSegment(currentSegIdx++);
        benchmarkTimer = setInterval(() => {
            fetchAndRecordSegment(currentSegIdx++);
        }, (activeShow ? activeShow.segment_length_sec : 10) * 1000);
    } else {
        isBenchmarking = false;
        clearInterval(benchmarkTimer);
        benchmarkBtn.textContent = "Start Benchmark";
        benchmarkBtn.className = "btn btn-primary";
    }
});

/**
 * 10. TODO: Automation Mode (Scheduled / Randomized Layer Switching for Bots)
 * - Future implementation: randomly switch option selects every N seconds.
 */

// Fallback Catalog if show_options.json is not reachable
function getFallbackCatalog() {
    return [
        {
            id: "f1_full.json",
            title: "Formula 1 Race (Full)",
            width: 1280,
            height: 720,
            segment_length_sec: 10,
            variants: [
                {
                    name: "race",
                    style: "landscape",
                    default: true,
                    total_layers: 4,
                    layers: [
                        { name: "race", options: ["race"], default: "race" },
                        { name: "driver", options: ["Off", "Sam"], default: "Sam" },
                        { name: "track", options: ["Off", "track"], default: "track" },
                        { name: "drivers", options: ["Off", "Sam,Alex"], default: "Sam,Alex" }
                    ]
                }
            ]
        }
    ];
}

// Start
window.addEventListener("DOMContentLoaded", initCatalog);

