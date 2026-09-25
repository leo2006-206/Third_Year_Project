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

    showSelect.addEventListener("change", onShowChanged);
    modeSelect.addEventListener("change", () => {
        partialGroup.style.display = modeSelect.value === "partial" ? "block" : "none";
        updateUrlPreview();
    });
    partialSelect.addEventListener("change", updateUrlPreview);
    benchmarkBtn.addEventListener("click", toggleBenchmark);

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
        urlPreview.textContent = `/shows/${activeShow ? activeShow.id : "f1.json"} (Local Mode)`;
        return;
    }
    urlPreview.textContent = buildOffloadUrl(0, activeShow ? activeShow.segment_length_sec : 10);
}

async function sendSegment(index) {
    const dur = activeShow ? activeShow.segment_length_sec : 10;
    const url = buildOffloadUrl(index * dur, (index + 1) * dur);
    const sentTime = new Date().toLocaleTimeString();
    const t0 = performance.now();

    let recvTime = "Error";
    let duration = "-";

    try {
        const response = await fetch(url);
        recvTime = new Date().toLocaleTimeString();
        duration = `${Math.round(performance.now() - t0)} ms (${response.status})`;
    } catch (e) {
        duration = "Failed";
    }

    const tr = document.createElement("tr");
    tr.innerHTML = `<td>${url}</td><td>${sentTime}</td><td>${recvTime} (${duration})</td>`;
    logBody.insertBefore(tr, logBody.firstChild);
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
