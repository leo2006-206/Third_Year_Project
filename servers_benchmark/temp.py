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

    fig, ax = plt.subplots(figsize=(8, 6), dpi=300)

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


def main() -> None:
    repo_root = Path(__file__).resolve().parent.parent
    csv_path = repo_root / "servers_benchmark" / "request_timings.csv"
    plot_path = repo_root / "servers_benchmark" / "offload_time_comparison.png"

    records = load_dataset(csv_path)
    print(f"Loaded {len(records)} benchmark records from {csv_path.name}\n")

    stats = compute_statistics(records, SHOW_TYPES)

    # Print summary table to console
    header = f"{'Video Type':<16} | {'Count':<5} | {'Work Time (s)':<16} | {'Resource/Dec':<14} | {'Render (s)':<14} | {'Encode (s)':<14} | {'Duration (s)':<14}"
    print(header)
    print("-" * len(header))
    for label, s in stats.items():
        cnt = s["count"]
        w = f"{s['metrics']['work_ms']['mean']:.2f} ± {s['metrics']['work_ms']['std']:.2f}"
        dec = f"{s['metrics']['decode_ms']['mean']:.2f} ± {s['metrics']['decode_ms']['std']:.2f}"
        rnd = f"{s['metrics']['render_ms']['mean']:.2f} ± {s['metrics']['render_ms']['std']:.2f}"
        enc = f"{s['metrics']['encode_ms']['mean']:.2f} ± {s['metrics']['encode_ms']['std']:.2f}"
        dur = f"{s['metrics']['duration_ms']['mean']:.2f} ± {s['metrics']['duration_ms']['std']:.2f}"
        print(
            f"{label:<16} | {cnt:<5} | {w:<16} | {dec:<14} | {rnd:<14} | {enc:<14} | {dur:<14}"
        )

    plot_statistics(stats, plot_path)


if __name__ == "__main__":
    main()
