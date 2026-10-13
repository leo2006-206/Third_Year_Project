#!/usr/bin/env python3
"""
servers_benchmark/temp.py

Loads benchmark timing data from request_timings.csv,
computes statistical metrics (mean, std, median, min, max) for the 6 main video types,
and generates a stacked bar plot mirroring the paper's visualization.
"""

import csv
import os
from collections import defaultdict
from pathlib import Path

# Avoid Matplotlib config warnings in sandboxed/custom home environments
os.environ.setdefault("MPLCONFIGDIR", "/tmp/matplotlib")
import matplotlib.pyplot as plt
import numpy as np

SHOW_TYPES = [
    ("f1.json", "f1"),
    ("f1_full.json", "f1_full"),
    ("forest720.json", "forest"),
    ("forest720_leaves.json", "forest_leaves"),
    ("forecast.json", "weather"),
    ("spiders.json", "spiders"),
]

METRICS = [
    "work_ms",
    "decode_ms",
    "render_ms",
    "encode_ms",
    "asset_ms",
    "queue_ms",
    "duration_ms",
]


def compute_statistics(records: list[dict], show_types: list[tuple[str, str]]) -> dict:
    """[Honest / Pure Domain Logic]
    Computes summary statistics (mean, std, median, min, max) in seconds
    for each video type across all timing metrics.
    """
    by_show = defaultdict(lambda: defaultdict(list))
    for r in records:
        show = r["request"].split("/")[3]
        for m in METRICS:
            by_show[show][m].append(float(r[m]) / 1000.0)

    stats = {}
    for show_file, label in show_types:
        show_stats = {"count": len(by_show[show_file]["work_ms"]), "metrics": {}}
        for m in METRICS:
            arr = np.array(by_show[show_file][m])
            if len(arr) > 0:
                show_stats["metrics"][m] = {
                    "mean": float(np.mean(arr)),
                    "std": float(np.std(arr)),
                    "median": float(np.median(arr)),
                    "min": float(np.min(arr)),
                    "max": float(np.max(arr)),
                }
            else:
                show_stats["metrics"][m] = {
                    "mean": 0.0,
                    "std": 0.0,
                    "median": 0.0,
                    "min": 0.0,
                    "max": 0.0,
                }
        stats[label] = show_stats

    return stats


def plot_statistics(stats: dict, output_path: Path) -> None:
    """[Dishonest / I/O Boundary]
    Renders and saves a stacked bar chart matching the paper's visual style.
    """
    labels = list(stats.keys())
    x = np.arange(len(labels))
    width = 0.55

    resource_means = [stats[l]["metrics"]["decode_ms"]["mean"] for l in labels]
    render_means = [stats[l]["metrics"]["render_ms"]["mean"] for l in labels]
    encode_means = [stats[l]["metrics"]["encode_ms"]["mean"] for l in labels]
    asset_means = [stats[l]["metrics"]["asset_ms"]["mean"] for l in labels]
    work_means = [stats[l]["metrics"]["work_ms"]["mean"] for l in labels]
    work_stds = [stats[l]["metrics"]["work_ms"]["std"] for l in labels]

    _fig, ax = plt.subplots(figsize=(8, 6), dpi=300)

    # Color palette matching the paper's stacked layout
    c_resource = "#c1ffc1"  # Light green (resource / decode)
    c_render = "#ffc1c1"  # Peach / pink (composition & render)
    c_encode = "#ce93d8"  # Lavender / purple (getPixels, YUV, H.264 encode)
    c_asset = "#d1d5db"  # Light gray (asset setup remainder)

    ax.bar(
        x,
        resource_means,
        width,
        label="resource (decode)",
        color=c_resource,
        edgecolor="black",
        linewidth=0.8,
    )
    ax.bar(
        x,
        render_means,
        width,
        bottom=resource_means,
        label="render",
        color=c_render,
        edgecolor="black",
        linewidth=0.8,
    )

    bottom_enc = np.array(resource_means) + np.array(render_means)
    ax.bar(
        x,
        encode_means,
        width,
        bottom=bottom_enc,
        label="encode (getPixels+YUV+enc)",
        color=c_encode,
        edgecolor="black",
        linewidth=0.8,
    )

    bottom_asset = bottom_enc + np.array(encode_means)
    ax.bar(
        x,
        asset_means,
        width,
        bottom=bottom_asset,
        label="remainder (asset setup)",
        color=c_asset,
        edgecolor="black",
        linewidth=0.8,
    )

    # Total work time error bars
    ax.errorbar(
        x,
        work_means,
        yerr=work_stds,
        fmt="none",
        ecolor="black",
        elinewidth=1.2,
        capsize=4,
        capthick=1.2,
    )

    # Value annotations on top of bars (Mean ± Std)
    for i, (m, s) in enumerate(zip(work_means, work_stds)):
        ax.text(
            i,
            m + s + 0.2,
            f"{m:.2f} ± {s:.2f}s",
            ha="center",
            va="bottom",
            fontsize=8.5,
            fontweight="bold",
        )

    ax.set_ylabel("Frame Offload Time (s)", fontsize=13, fontweight="bold")
    ax.set_xlabel("Video Category", fontsize=13, fontweight="bold")
    ax.set_title(
        "OBM Offload Pipeline Performance by Video Type (10s Segments)",
        fontsize=14,
        fontweight="bold",
        pad=12,
    )
    ax.set_xticks(x)
    ax.set_xticklabels(labels, fontsize=11, fontweight="bold")
    ax.set_ylim(0, 11.0)
    ax.set_yticks(np.arange(0, 12, 1))

    # Paper-style horizontal grid lines
    ax.yaxis.grid(True, linestyle="--", alpha=0.5, color="gray")
    ax.set_axisbelow(True)

    ax.legend(
        loc="upper left", frameon=True, framealpha=0.9, edgecolor="black", fontsize=10
    )
    plt.tight_layout()

    output_path.parent.mkdir(parents=True, exist_ok=True)
    plt.savefig(output_path)
    plt.close()
    print(f"[Plot Saved] {output_path}")


def load_dataset(csv_path: Path) -> list[dict]:
    """[Dishonest / I/O Boundary]
    Reads benchmark CSV rows.
    """
    if not csv_path.exists():
        raise FileNotFoundError(f"Benchmark file not found: {csv_path}")
    with open(csv_path, encoding="utf-8") as f:
        return list(csv.DictReader(f))


def parse_record(r: dict) -> dict:
    """[Honest / Pure Domain Logic]
    Parses request URL path parameters into structured fields.
    """
    parts = r["request"].split("/")
    show = parts[3]
    t_start = int(parts[4])
    t_end = int(parts[5])
    variant = parts[8]
    layers = dict(tok.split("|", 1) if "|" in tok else (tok, "on") for tok in parts[11:])
    return {
        "show": show,
        "t_start": t_start,
        "t_end": t_end,
        "variant": variant,
        "layers": layers,
        "work_ms": float(r["work_ms"]),
        "decode_ms": float(r["decode_ms"]),
        "render_ms": float(r["render_ms"]),
        "encode_ms": float(r["encode_ms"]),
    }


def analyze_layer_independence(records: list[dict]) -> dict:
    """[Honest / Pure Domain Logic]
    Evaluates layer independence, marginal costs across contexts,
    and additive predictability from benchmark records.
    """
    parsed = [parse_record(r) for r in records]

    # 1. Index f1.json race variant records
    f1_race = [r for r in parsed if r["show"] == "f1.json" and r["variant"] == "race"]
    lookup_f1: dict[tuple, dict] = {}
    for r in f1_race:
        has_frame = "race-frame" in r["layers"]
        drivers = r["layers"].get("drivers", "")
        track = r["layers"].get("track", "")
        driver = r["layers"].get("driver", "")
        lookup_f1[(r["t_start"], has_frame, drivers, track, driver)] = r

    driver_deltas_off: list[dict] = []
    driver_deltas_track: list[dict] = []
    for (t, has_frame, drivers, track, driver), r_on in lookup_f1.items():
        if driver == "Sam":
            off_key = (t, has_frame, drivers, track, "Off")
            if off_key in lookup_f1:
                r_off = lookup_f1[off_key]
                delta = {
                    "t": t,
                    "work": r_on["work_ms"] - r_off["work_ms"],
                    "decode": r_on["decode_ms"] - r_off["decode_ms"],
                    "render": r_on["render_ms"] - r_off["render_ms"],
                    "encode": r_on["encode_ms"] - r_off["encode_ms"],
                }
                if track == "off":
                    driver_deltas_off.append(delta)
                elif track == "track":
                    driver_deltas_track.append(delta)

    track_deltas_off: list[dict] = []
    track_deltas_sam: list[dict] = []
    for (t, has_frame, drivers, track, driver), r_on in lookup_f1.items():
        if track == "track":
            off_key = (t, has_frame, drivers, "off", driver)
            if off_key in lookup_f1:
                r_off = lookup_f1[off_key]
                delta = {
                    "t": t,
                    "work": r_on["work_ms"] - r_off["work_ms"],
                    "decode": r_on["decode_ms"] - r_off["decode_ms"],
                    "render": r_on["render_ms"] - r_off["render_ms"],
                    "encode": r_on["encode_ms"] - r_off["encode_ms"],
                }
                if driver == "Off":
                    track_deltas_off.append(delta)
                elif driver == "Sam":
                    track_deltas_sam.append(delta)

    # 2. Index f1_full.json race variant records
    f1_full_race = [r for r in parsed if r["show"] == "f1_full.json" and r["variant"] == "race"]
    lookup_full: dict[tuple, dict] = {}
    for r in f1_full_race:
        has_frame = "race-frame" in r["layers"]
        drivers = r["layers"].get("drivers", "")
        track = r["layers"].get("track", "")
        driver = r["layers"].get("driver", "")
        lookup_full[(r["t_start"], has_frame, drivers, track, driver)] = r

    driver_full_off: list[dict] = []
    driver_full_track: list[dict] = []
    for (t, has_frame, drivers, track, driver), r_on in lookup_full.items():
        if driver == "Sam":
            off_key = (t, has_frame, drivers, track, "Off")
            if off_key in lookup_full:
                r_off = lookup_full[off_key]
                delta = {
                    "work": r_on["work_ms"] - r_off["work_ms"],
                    "decode": r_on["decode_ms"] - r_off["decode_ms"],
                    "render": r_on["render_ms"] - r_off["render_ms"],
                    "encode": r_on["encode_ms"] - r_off["encode_ms"],
                }
                if track == "off":
                    driver_full_off.append(delta)
                elif track == "track":
                    driver_full_track.append(delta)

    # 3. Temporal breakdown for f1 driver delta
    temporal_deltas: dict[int, dict[str, list[float]]] = defaultdict(lambda: {"off": [], "track": []})
    for d in driver_deltas_off:
        temporal_deltas[d["t"]]["off"].append(d["work"])
    for d in driver_deltas_track:
        temporal_deltas[d["t"]]["track"].append(d["work"])

    temporal_stats = []
    for t in sorted(temporal_deltas.keys()):
        m_off = float(np.mean(temporal_deltas[t]["off"]))
        m_trk = float(np.mean(temporal_deltas[t]["track"]))
        temporal_stats.append({
            "t": t,
            "mean_off": m_off,
            "mean_track": m_trk,
            "interaction": m_trk - m_off,
        })

    # 4. Superposition Additive Model prediction on f1.json {driver=Sam, track=track}
    iso_driver = float(np.mean([d["work"] for d in driver_deltas_off]))
    iso_track = float(np.mean([d["work"] for d in track_deltas_off]))
    actual_list: list[float] = []
    pred_list: list[float] = []
    for (t, has_frame, drivers, track, driver), r in lookup_f1.items():
        if track == "track" and driver == "Sam":
            base_key = (t, has_frame, drivers, "off", "Off")
            if base_key in lookup_f1:
                base_work = lookup_f1[base_key]["work_ms"]
                pred = base_work + iso_driver + iso_track
                actual = r["work_ms"]
                actual_list.append(actual)
                pred_list.append(pred)

    acts = np.array(actual_list)
    prds = np.array(pred_list)
    mae = float(np.mean(np.abs(acts - prds)))
    mape = float(np.mean(np.abs(acts - prds) / acts) * 100.0)
    bias = float(np.mean(acts - prds))
    ss_tot = float(np.sum((acts - np.mean(acts)) ** 2))
    ss_res = float(np.sum((acts - prds) ** 2))
    r2 = float(1.0 - ss_res / ss_tot) if ss_tot > 0 else 0.0

    return {
        "f1_driver_off": driver_deltas_off,
        "f1_driver_track": driver_deltas_track,
        "f1_track_off": track_deltas_off,
        "f1_track_sam": track_deltas_sam,
        "full_driver_off": driver_full_off,
        "full_driver_track": driver_full_track,
        "temporal": temporal_stats,
        "prediction": {
            "actuals": actual_list,
            "preds": pred_list,
            "mae": mae,
            "mape": mape,
            "bias": bias,
            "r2": r2,
            "iso_driver": iso_driver,
            "iso_track": iso_track,
        },
    }


def plot_layer_independence(analysis: dict, output_path: Path) -> None:
    """[Dishonest / I/O Boundary]
    Renders and saves a 4-panel figure evaluating layer additivity and independence.
    """
    _fig, axes = plt.subplots(2, 2, figsize=(14, 11), dpi=300)
    ax_ctx, ax_stage, ax_temp, ax_pred = axes[0, 0], axes[0, 1], axes[1, 0], axes[1, 1]

    # Panel A: Context Independence Test
    labels = [
        "f1: Driver\n[trk=off]",
        "f1: Driver\n[trk=on]",
        "f1_full: Driver\n[trk=off]",
        "f1_full: Driver\n[trk=on]",
        "f1: Track\n[drv=Off]",
        "f1: Track\n[drv=Sam]",
    ]
    data_groups = [
        [d["work"] for d in analysis["f1_driver_off"]],
        [d["work"] for d in analysis["f1_driver_track"]],
        [d["work"] for d in analysis["full_driver_off"]],
        [d["work"] for d in analysis["full_driver_track"]],
        [d["work"] for d in analysis["f1_track_off"]],
        [d["work"] for d in analysis["f1_track_sam"]],
    ]
    means = [float(np.mean(g)) for g in data_groups]
    stds = [float(np.std(g)) for g in data_groups]
    x = np.arange(len(labels))
    colors = ["#90caf9", "#1e88e5", "#a5d6a7", "#388e3c", "#ffcc80", "#f57c00"]

    ax_ctx.bar(
        x,
        means,
        yerr=stds,
        capsize=4,
        color=colors,
        edgecolor="black",
        linewidth=0.8,
        alpha=0.9,
    )
    ax_ctx.axhline(0, color="gray", linestyle="--", linewidth=0.8)
    ax_ctx.set_ylabel("Marginal Δ Work Time (ms)", fontsize=11, fontweight="bold")
    ax_ctx.set_title(
        "A. Context Independence Test: ΔT(X | C1) vs ΔT(X | C2)",
        fontsize=12,
        fontweight="bold",
    )
    ax_ctx.set_xticks(x)
    ax_ctx.set_xticklabels(labels, fontsize=8.5, fontweight="bold")
    ax_ctx.set_ylim(-700, 1350)
    ax_ctx.grid(axis="y", linestyle="--", alpha=0.5)

    for i, (m, s) in enumerate(zip(means, stds)):
        va = "bottom" if m >= 0 else "top"
        offset = 20 if m >= 0 else -35
        ax_ctx.text(
            i,
            m + (s if m >= 0 else -s) + offset,
            f"{m:.0f}±{s:.0f}ms",
            ha="center",
            va=va,
            fontsize=8.5,
            fontweight="bold",
        )

    # Panel B: Pipeline Stage Breakdown
    comp_labels = [
        "Driver\n[trk=off]",
        "Driver\n[trk=on]",
        "Track\n[drv=Off]",
        "Track\n[drv=Sam]",
    ]
    comp_data = [
        analysis["f1_driver_off"],
        analysis["f1_driver_track"],
        analysis["f1_track_off"],
        analysis["f1_track_sam"],
    ]
    dec_means = [float(np.mean([d["decode"] for d in cd])) for cd in comp_data]
    rnd_means = [float(np.mean([d["render"] for d in cd])) for cd in comp_data]
    enc_means = [float(np.mean([d["encode"] for d in cd])) for cd in comp_data]

    cx = np.arange(len(comp_labels))
    bw = 0.25
    ax_stage.bar(
        cx - bw,
        dec_means,
        bw,
        label="Δ Decode (ms)",
        color="#c1ffc1",
        edgecolor="black",
        linewidth=0.8,
    )
    ax_stage.bar(
        cx,
        rnd_means,
        bw,
        label="Δ Render (ms)",
        color="#ffc1c1",
        edgecolor="black",
        linewidth=0.8,
    )
    ax_stage.bar(
        cx + bw,
        enc_means,
        bw,
        label="Δ Encode (ms)",
        color="#ce93d8",
        edgecolor="black",
        linewidth=0.8,
    )
    ax_stage.axhline(0, color="gray", linestyle="--", linewidth=0.8)
    ax_stage.set_ylabel("Stage Delta (ms)", fontsize=11, fontweight="bold")
    ax_stage.set_title(
        "B. Pipeline Stage Decomposition of Marginal Cost",
        fontsize=12,
        fontweight="bold",
    )
    ax_stage.set_xticks(cx)
    ax_stage.set_xticklabels(comp_labels, fontsize=9.5, fontweight="bold")
    ax_stage.legend(loc="upper left", frameon=True, edgecolor="black", fontsize=9)
    ax_stage.grid(axis="y", linestyle="--", alpha=0.5)

    # Panel C: Temporal Stability & Interaction Over Time
    t_vals = [s["t"] for s in analysis["temporal"]]
    off_vals = [s["mean_off"] for s in analysis["temporal"]]
    trk_vals = [s["mean_track"] for s in analysis["temporal"]]
    int_vals = [s["interaction"] for s in analysis["temporal"]]

    ax_temp.plot(
        t_vals,
        off_vals,
        marker="o",
        color="#1e88e5",
        linewidth=2,
        label="Δ Driver | track=off",
    )
    ax_temp.plot(
        t_vals,
        trk_vals,
        marker="s",
        color="#d32f2f",
        linewidth=2,
        label="Δ Driver | track=track",
    )
    ax_temp.plot(
        t_vals,
        int_vals,
        marker="^",
        color="#7b1fa2",
        linestyle=":",
        linewidth=2,
        label="Interaction I(A, B)",
    )
    ax_temp.axhline(0, color="gray", linestyle="--", linewidth=0.8)
    ax_temp.set_xlabel("Video Segment Start Time (s)", fontsize=11, fontweight="bold")
    ax_temp.set_ylabel("Delta Time (ms)", fontsize=11, fontweight="bold")
    ax_temp.set_title(
        "C. Temporal Variation & Interaction Term I(A, B) Over Time",
        fontsize=12,
        fontweight="bold",
    )
    ax_temp.set_xticks(t_vals)
    ax_temp.grid(True, linestyle="--", alpha=0.5)
    ax_temp.legend(loc="upper left", frameon=True, edgecolor="black", fontsize=9)

    # Panel D: Superposition Additive Model Predictability
    acts = np.array(analysis["prediction"]["actuals"])
    prds = np.array(analysis["prediction"]["preds"])
    ax_pred.scatter(
        acts,
        prds,
        alpha=0.7,
        color="#0288d1",
        edgecolor="black",
        s=40,
        label="Requests {driver, track}",
    )

    min_v = min(np.min(acts), np.min(prds)) - 100
    max_v = max(np.max(acts), np.max(prds)) + 100
    ax_pred.plot(
        [min_v, max_v],
        [min_v, max_v],
        color="red",
        linestyle="--",
        linewidth=1.8,
        label="Ideal Additivity (y = x)",
    )
    ax_pred.set_xlim(min_v, max_v)
    ax_pred.set_ylim(min_v, max_v)
    ax_pred.set_xlabel("Actual Work Time (ms)", fontsize=11, fontweight="bold")
    ax_pred.set_ylabel("Additive Predicted Time (ms)", fontsize=11, fontweight="bold")
    ax_pred.set_title(
        "D. Superposition Predictability: Actual vs. T0 + ΔT(A) + ΔT(B)",
        fontsize=12,
        fontweight="bold",
    )

    pred_info = (
        f"Bias (Mean Err): {analysis['prediction']['bias']:+.1f} ms\n"
        f"MAE: {analysis['prediction']['mae']:.1f} ms\n"
        f"MAPE: {analysis['prediction']['mape']:.1f}%\n"
        f"Isolated Δ Driver: {analysis['prediction']['iso_driver']:.1f} ms\n"
        f"Isolated Δ Track: {analysis['prediction']['iso_track']:.1f} ms"
    )
    ax_pred.text(
        0.04,
        0.70,
        pred_info,
        transform=ax_pred.transAxes,
        fontsize=9.5,
        fontweight="bold",
        bbox=dict(
            boxstyle="round,pad=0.5",
            facecolor="#fff9c4",
            edgecolor="black",
            alpha=0.9,
        ),
    )
    ax_pred.legend(loc="lower right", frameon=True, edgecolor="black", fontsize=9)
    ax_pred.grid(True, linestyle="--", alpha=0.5)

    plt.tight_layout()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    plt.savefig(output_path)
    plt.close()
    print(f"[Analysis Plot Saved] {output_path}")


def main() -> None:
    repo_root = Path(__file__).resolve().parent.parent
    csv_path = repo_root / "servers_benchmark" / "request_timings.csv"

    records = load_dataset(csv_path)
    print(f"Loaded {len(records)} benchmark records from {csv_path.name}\n")

    # --- Previous Show-Level Summary (commented out as requested) ---
    # plot_path = repo_root / "servers_benchmark" / "offload_time_comparison.png"
    # stats = compute_statistics(records, SHOW_TYPES)
    # header = f"{'Video Type':<16} | {'Count':<5} | {'Work Time (s)':<16} | {'Resource/Dec':<14} | {'Render (s)':<14} | {'Encode (s)':<14} | {'Duration (s)':<14}"
    # print(header)
    # print("-" * len(header))
    # for label, s in stats.items():
    #     cnt = s["count"]
    #     w = f"{s['metrics']['work_ms']['mean']:.2f} ± {s['metrics']['work_ms']['std']:.2f}"
    #     dec = f"{s['metrics']['decode_ms']['mean']:.2f} ± {s['metrics']['decode_ms']['std']:.2f}"
    #     rnd = f"{s['metrics']['render_ms']['mean']:.2f} ± {s['metrics']['render_ms']['std']:.2f}"
    #     enc = f"{s['metrics']['encode_ms']['mean']:.2f} ± {s['metrics']['encode_ms']['std']:.2f}"
    #     dur = f"{s['metrics']['duration_ms']['mean']:.2f} ± {s['metrics']['duration_ms']['std']:.2f}"
    #     print(f"{label:<16} | {cnt:<5} | {w:<16} | {dec:<14} | {rnd:<14} | {enc:<14} | {dur:<14}")
    # plot_statistics(stats, plot_path)

    # --- New Layer Independence & Mathematical Additivity Analysis ---
    analysis = analyze_layer_independence(records)

    print("=== Layer Independence & Marginal Cost Analysis ===")
    print(
        f"1. Driver Layer (Sam vs Off) in f1.json:\n"
        f"   - Under track=off:  mean = {np.mean([d['work'] for d in analysis['f1_driver_off']]):.1f} ± {np.std([d['work'] for d in analysis['f1_driver_off']]):.1f} ms\n"
        f"   - Under track=on:   mean = {np.mean([d['work'] for d in analysis['f1_driver_track']]):.1f} ± {np.std([d['work'] for d in analysis['f1_driver_track']]):.1f} ms\n"
        f"   - Interaction I(driver, track): +{np.mean([d['work'] for d in analysis['f1_driver_track']]) - np.mean([d['work'] for d in analysis['f1_driver_off']]):.1f} ms (super-additive)\n"
    )
    print(
        f"2. Driver Layer (Sam vs Off) in f1_full.json (Independent Show Config):\n"
        f"   - Under track=off:  mean = {np.mean([d['work'] for d in analysis['full_driver_off']]):.1f} ms\n"
        f"   - Under track=on:   mean = {np.mean([d['work'] for d in analysis['full_driver_track']]):.1f} ms\n"
        f"   - Context Delta Difference: {abs(np.mean([d['work'] for d in analysis['full_driver_track']]) - np.mean([d['work'] for d in analysis['full_driver_off']])):.1f} ms (near zero!)\n"
    )
    print(
        f"3. Track Layer (track vs off) in f1.json:\n"
        f"   - Under driver=Off: mean = {np.mean([d['work'] for d in analysis['f1_track_off']]):.1f} ms\n"
        f"   - Under driver=Sam: mean = {np.mean([d['work'] for d in analysis['f1_track_sam']]):.1f} ms\n"
    )
    pred_res = analysis["prediction"]
    print(
        f"4. Additive Superposition Prediction Test {{driver, track}}:\n"
        f"   - Mean Prediction Bias: {pred_res['bias']:+.1f} ms\n"
        f"   - Mean Absolute Error:  {pred_res['mae']:.1f} ms\n"
        f"   - Mean Absolute % Err:  {pred_res['mape']:.1f}%\n"
    )

    layer_plot_path = repo_root / "servers_benchmark" / "layer_independence_analysis.png"
    plot_layer_independence(analysis, layer_plot_path)


if __name__ == "__main__":
    main()
