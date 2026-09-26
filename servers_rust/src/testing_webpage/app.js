/**
 * OBM Minimal Benchmark Harness
 * Strictly records: Request URL, Sent Time, and Receive Time (Duration)
 */

let showCatalog = [];
let activeShow = null;
let activeVariant = null;
let benchmarkTimer = null;
let currentSegIdx = 0;

const showSelect = document.getElementById("show-select");
const modeSelect = document.getElementById("offload-mode-select");
const partialGroup = document.getElementById("partial-layers-group");
const partialSelect = document.getElementById("partial-layers-select");
const layerContainer = document.getElementById("layer-options-container");
const urlPreview = document.getElementById("url-preview");
const benchmarkBtn = document.getElementById("benchmark-btn");
const clearBtn = document.getElementById("clear-btn");
const logBody = document.getElementById("log-body");

async function initCatalog() {
    try {
        const response = await fetch("show_options.json");
        const data = await response.json();
        showCatalog = data.shows || [];
    } catch (err) {
        console.error("Failed to load show_options.json", err);
        return;
    }

    showSelect.innerHTML = showCatalog.map(s => `<option value="${s.id}">${s.title || s.id}</option>`).join("");

    const params = new URLSearchParams(window.location.search);
    const reqShow = params.get("show");
    if (reqShow && showCatalog.some(s => s.id === reqShow)) {
        showSelect.value = reqShow;
    }

    showSelect.addEventListener("change", () => {
        onShowChanged();
        const url = new URL(window.location);
        url.searchParams.set("show", showSelect.value);
        window.history.replaceState(null, "", url.href);
    });

    modeSelect.addEventListener("change", () => {
        partialGroup.style.display = modeSelect.value === "partial" ? "block" : "none";
        updateUrlPreview();
    });

    partialSelect.addEventListener("change", updateUrlPreview);
    benchmarkBtn.addEventListener("click", toggleBenchmark);
    if (clearBtn) {
        clearBtn.addEventListener("click", () => {
            logBody.innerHTML = "";
        });
    }

    onShowChanged();
}

function onShowChanged() {
    activeShow = showCatalog.find(s => s.id === showSelect.value);
    if (!activeShow) return;

    activeVariant = activeShow.variants.find(v => v.default) || activeShow.variants[0];

    const maxLayers = activeVariant.total_layers || (activeVariant.layers ? activeVariant.layers.length : 4);
    partialSelect.innerHTML = Array.from({ length: maxLayers - 1 }, (_, i) => `<option value="${i + 2}">${i + 2} Layers</option>`).join("");

    layerContainer.innerHTML = activeVariant.layers.map(layer => `
        <label>Layer: ${layer.name}
            <select class="layer-select" data-layer-name="${layer.name}">
                ${layer.options.map(opt => `<option value="${opt}" ${opt === layer.default ? "selected" : ""}>${opt}</option>`).join("")}
            </select>
        </label>
    `).join("");

    layerContainer.querySelectorAll("select").forEach(s => s.addEventListener("change", updateUrlPreview));
    updateUrlPreview();
}

function getSelectedOptionTokens() {
    return Array.from(document.querySelectorAll(".layer-select")).map(s => `${s.dataset.layerName}|${s.value}`);
}

function buildOffloadUrl(timeStart = 0, timeEnd = 10) {
    if (!activeShow || !activeVariant) return "";

    const mode = modeSelect.value;
    if (mode === "non_offload") {
        return `/shows/${activeShow.id}`;
    }

    const layers = mode === "partial" ? parseInt(partialSelect.value, 10) : activeVariant.total_layers;
    const tokens = getSelectedOptionTokens();
    const base = `/offload/show/${activeShow.id}/${timeStart}/${timeEnd}/${activeShow.width}/${activeShow.height}/${activeVariant.name}/${activeVariant.style}/${layers}`;
    return tokens.length > 0 ? `${base}/${tokens.join("/")}` : base;
}

function updateUrlPreview() {
    const mode = modeSelect.value;
    if (mode === "non_offload") {
        urlPreview.textContent = `/shows/${activeShow ? activeShow.id : "f1.json"} (Non-Offload / Local)`;
        return;
    }
    urlPreview.textContent = buildOffloadUrl(0, activeShow ? activeShow.segment_length_sec : 10);
}

function formatBytes(bytes) {
    if (!bytes || bytes <= 0) return "0 B";
    if (bytes < 1024) return bytes + " B";
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
    return (bytes / (1024 * 1024)).toFixed(2) + " MB";
}

async function sendSegment(index) {
    const dur = activeShow ? activeShow.segment_length_sec : 10;
    const url = buildOffloadUrl(index * dur, (index + 1) * dur);
    const sentTime = new Date().toLocaleTimeString();
    const t0 = performance.now();

    // Immediately render request row with empty Receive Time and Size
    const tr = document.createElement("tr");
    tr.innerHTML = `<td>${url}</td><td>${sentTime}</td><td class="res-time"></td><td class="res-size"></td>`;
    logBody.insertBefore(tr, logBody.firstChild);

    try {
        const response = await fetch(url);
        const blob = await response.blob();
        const duration = Math.round(performance.now() - t0);
        const recvTime = new Date().toLocaleTimeString();

        tr.querySelector(".res-time").textContent = `${recvTime} (${duration} ms, ${response.status})`;
        tr.querySelector(".res-size").textContent = formatBytes(blob.size);
    } catch (e) {
        tr.querySelector(".res-time").textContent = "Failed";
        tr.querySelector(".res-size").textContent = "-";
    }
}

function toggleBenchmark() {
    if (!benchmarkTimer) {
        benchmarkBtn.textContent = "Stop Benchmark";
        benchmarkBtn.classList.add("active");
        currentSegIdx = 0;

        sendSegment(currentSegIdx++);
        benchmarkTimer = setInterval(() => {
            sendSegment(currentSegIdx++);
        }, (activeShow ? activeShow.segment_length_sec : 10) * 1000);
    } else {
        clearInterval(benchmarkTimer);
        benchmarkTimer = null;
        benchmarkBtn.textContent = "Start Benchmark";
        benchmarkBtn.classList.remove("active");
    }
}

window.addEventListener("DOMContentLoaded", initCatalog);
