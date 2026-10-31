#!/usr/bin/env python3
"""
Plots how a request sequence is spread over its replay window.

Usage:
  servers_benchmark/sequences/plot_seq.py <seq.csv> [<seq.csv> ...] [--bin 10] [--window 60] [--out-dir DIR]
  e.g. servers_benchmark/sequences/plot_seq.py servers_benchmark/sequences/seq_a1.2_case1.csv

One PNG per sequence (default: next to the CSV, seq_..._timeline.png) with three panels:
  1. requests per --bin seconds, against the cluster's capacity per bin (from meta.csv) and the
     mean arrival rate
  2. count load over a sliding --window: requests in the last window / capacity in a window.
     It counts requests, not their work, so it is an approximation; above 1 = more requests
     than the cluster finishes on average (the queue grows)
  3. requests per show per --window seconds (which content the bursts are made of)
Also prints a summary: mean rate, inter-arrival CV (Poisson = 1, bursty > 1), the busiest
window and how long the count load stays above 1.
"""

import argparse
import re
import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
import numpy as np  # noqa: E402
import pandas as pd  # noqa: E402

# Reference palette (dataviz skill, light mode): categorical slots in fixed order + chrome
SERIES = ["#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300", "#6250d6", "#e34948"]
SURFACE, INK, INK_2, MUTED = "#fcfcfb", "#0b0b0b", "#52514e", "#898781"
GRID, AXIS = "#e1e0d9", "#c3c2b7"


# ==============================================================================
# 1. Honest Functions (Pure Domain Logic)
# ==============================================================================


def seq_key(path: Path) -> tuple[str, str]:
    """[Honest] seq_a1.2_case1.csv -> ("1.2", "case1"), the meta.csv row of this sequence."""
    m = re.match(r"seq_a([0-9.]+)_(case\d+)\.csv$", path.name)
    if not m:
        raise ValueError(f"{path.name}: expected seq_a<alpha>_case<n>.csv")
    return m.group(1), m.group(2)


def counts_per_bin(t_s: np.ndarray, bin_s: float, duration_s: float) -> tuple[np.ndarray, np.ndarray]:
    """[Honest] Bin start times and the number of arrivals in each bin."""
    edges = np.arange(0.0, duration_s + bin_s, bin_s)
    counts, _ = np.histogram(t_s, bins=edges)
    return edges[:-1], counts


def sliding_count(t_s: np.ndarray, window_s: float, step_s: float, duration_s: float) -> tuple[np.ndarray, np.ndarray]:
    """[Honest] At each time t (from window_s on), the arrivals in (t - window_s, t]."""
    ts = np.arange(window_s, duration_s + step_s / 2, step_s)
    sorted_t = np.sort(t_s)
    counts = np.searchsorted(sorted_t, ts, side="right") - np.searchsorted(sorted_t, ts - window_s, side="right")
    return ts, counts


def summary(t_s: np.ndarray, capacity_rps: float, window_s: float, duration_s: float) -> dict[str, float]:
    """[Honest] Headline numbers of one sequence."""
    gaps = np.diff(np.sort(t_s))
    ts, win = sliding_count(t_s, window_s, 1.0, duration_s)
    load = win / (capacity_rps * window_s)
    peak = int(np.argmax(win))
    return {
        "n": len(t_s),
        "mean_rps": len(t_s) / duration_s,
        "capacity_rps": capacity_rps,
        "count_load": len(t_s) / (capacity_rps * duration_s),
        "gap_cv": float(gaps.std() / gaps.mean()) if len(gaps) > 1 else float("nan"),
        "peak_count": int(win[peak]),
        "peak_end_s": float(ts[peak]),
        "peak_load": float(load[peak]),
        "over_1_s": float((load > 1.0).sum()),  # 1 s steps -> seconds
        "first_half": int((t_s < duration_s / 2).sum()),
    }


# ==============================================================================
# 2. Dishonest Functions (I/O, plotting)
# ==============================================================================


def load_seq(path: Path) -> pd.DataFrame:
    """[Dishonest I/O] Reads a sequence CSV: t_s (s from window start) and the show of each request."""
    d = pd.read_csv(path)
    d["t_s"] = d.t_ms / 1000
    d["show"] = d.path.str.split("/").str[3].str.replace(".json", "", regex=False)
    return d


def load_capacity(meta_path: Path, alpha: str, cluster: str) -> float:
    """[Dishonest I/O] The cluster's modelled capacity (requests/s) for this sequence, from meta.csv."""
    meta = pd.read_csv(meta_path, dtype={"alpha": str})
    row = meta[(meta.alpha == alpha) & (meta.cluster == cluster)]
    if row.empty:
        raise ValueError(f"{meta_path}: no row for alpha {alpha}, {cluster}")
    return float(row.capacity_rps.iloc[0])


def style_axis(ax: plt.Axes) -> None:
    """[Dishonest Plot] Recessive chrome: hairline grid, muted ticks, no top/right frame."""
    ax.set_facecolor(SURFACE)
    ax.grid(axis="y", color=GRID, linewidth=0.8)
    ax.set_axisbelow(True)
    for side in ("top", "right"):
        ax.spines[side].set_visible(False)
    for side in ("left", "bottom"):
        ax.spines[side].set_color(AXIS)
    ax.tick_params(colors=MUTED, labelcolor=INK_2, labelsize=9)
    ax.yaxis.label.set_color(INK_2)
    ax.xaxis.label.set_color(INK_2)


def plot_seq(d: pd.DataFrame, title: str, capacity_rps: float, bin_s: float, window_s: float,
             duration_s: float, out: Path) -> None:
    """[Dishonest Plot] Writes the three-panel timeline PNG."""
    t = d.t_s.to_numpy()
    fig, axes = plt.subplots(3, 1, figsize=(11, 9), sharex=True, gridspec_kw={"height_ratios": [3, 2, 3]})
    fig.patch.set_facecolor(SURFACE)
    fig.suptitle(title, color=INK, fontsize=13, x=0.06, ha="left")

    # 1. arrivals per bin vs capacity per bin
    ax = axes[0]
    starts, counts = counts_per_bin(t, bin_s, duration_s)
    ax.bar(starts, counts, width=bin_s * 0.85, align="edge", color=SERIES[0], label=f"requests per {bin_s:g} s")
    cap = capacity_rps * bin_s
    mean = len(t) / duration_s * bin_s
    # the two labels would overlap when mean ~ capacity: lower one below its line, other above
    close = abs(mean - cap) < 0.06 * max(counts.max(), cap)
    cap_va, mean_va = ("bottom", "top") if mean <= cap else ("top", "bottom")
    ax.axhline(cap, color=INK_2, linestyle="--", linewidth=1.2)
    ax.text(duration_s, cap, f" capacity {cap:.1f}", color=INK_2, va=cap_va if close else "center",
            fontsize=9)
    ax.axhline(mean, color=MUTED, linestyle=":", linewidth=1.2)
    ax.text(duration_s, mean, f" mean {mean:.1f}", color=MUTED, va=mean_va if close else "center",
            fontsize=9)
    ax.set_ylabel(f"requests / {bin_s:g} s")
    ax.set_title("Arrivals over the window", color=INK, fontsize=10, loc="left")
    style_axis(ax)

    # 2. sliding count load
    ax = axes[1]
    ts, win = sliding_count(t, window_s, 1.0, duration_s)
    load = win / (capacity_rps * window_s)
    ax.plot(ts, load, color=SERIES[1], linewidth=2)
    ax.fill_between(ts, 1.0, load, where=load > 1.0, color=SERIES[1], alpha=0.18, linewidth=0)
    ax.axhline(1.0, color=INK_2, linestyle="--", linewidth=1.2)
    ax.text(duration_s, 1.0, " = capacity", color=INK_2, va="center", fontsize=9)
    ax.set_ylabel(f"load (last {window_s:g} s)")
    ax.set_title(f"Request-count load over a sliding {window_s:g} s window (above 1: queue grows)",
                 color=INK, fontsize=10, loc="left")
    style_axis(ax)

    # 3. requests per show per window, stacked, fixed show order (most requested first)
    ax = axes[2]
    share = d.show.value_counts(normalize=True)
    shows = share.index.tolist()
    edges = np.arange(0.0, duration_s + window_s, window_s)
    bottom = np.zeros(len(edges) - 1)
    for i, show in enumerate(shows):
        c, _ = np.histogram(d.t_s[d.show == show], bins=edges)
        ax.bar(edges[:-1], c, width=window_s * 0.9, align="edge", bottom=bottom,
               label=f"{show} ({share[show]:.0%})",
               color=SERIES[i % len(SERIES)], edgecolor=SURFACE, linewidth=1)
        bottom += c
    ax.set_ylabel(f"requests / {window_s:g} s")
    ax.set_xlabel("time in the replay window (s)")
    ax.set_title("Which shows the requests are", color=INK, fontsize=10, loc="left")
    leg = ax.legend(ncol=min(len(shows), 6), fontsize=9, frameon=False, loc="upper left",
                    bbox_to_anchor=(0, -0.18))
    for text in leg.get_texts():
        text.set_color(INK_2)
    style_axis(ax)

    axes[-1].set_xlim(0, duration_s)
    fig.tight_layout(rect=(0, 0, 0.95, 0.97))
    fig.savefig(out, dpi=130, facecolor=SURFACE)
    plt.close(fig)


def main() -> None:
    """[Dishonest Orchestrator]"""
    parser = argparse.ArgumentParser(description="Plot a request sequence's arrivals over its window")
    parser.add_argument("seq", nargs="+", type=Path, help="seq_a<alpha>_case<n>.csv file(s)")
    parser.add_argument("--bin", type=float, default=10.0, help="bar width of panel 1 (s), default 10")
    parser.add_argument("--window", type=float, default=60.0, help="sliding / show window (s), default 60")
    parser.add_argument("--duration", type=float, default=600.0, help="replay window length (s), default 600")
    parser.add_argument("--out-dir", type=Path, help="where the PNGs go (default: next to each CSV)")
    args = parser.parse_args()

    for path in args.seq:
        try:
            alpha, cluster = seq_key(path)
            capacity = load_capacity(path.with_name("meta.csv"), alpha, cluster)
        except (ValueError, FileNotFoundError) as e:
            print(f"[Error] {e}", file=sys.stderr)
            sys.exit(1)
        d = load_seq(path)
        s = summary(d.t_s.to_numpy(), capacity, args.window, args.duration)
        out_dir = args.out_dir or path.parent
        out_dir.mkdir(parents=True, exist_ok=True)
        out = out_dir / f"{path.stem}_timeline.png"
        title = f"{path.name}: {s['n']} requests, alpha {alpha}, {cluster}"
        plot_seq(d, title, capacity, args.bin, args.window, args.duration, out)

        print(f"== {path.name}")
        print(f"  requests {s['n']}, mean {s['mean_rps']:.3f}/s, capacity {s['capacity_rps']:.3f}/s "
              f"-> count load {s['count_load']:.2f}")
        print(f"  inter-arrival CV {s['gap_cv']:.2f} (Poisson 1.0, bursty > 1)")
        print(f"  first half {s['first_half']} / second half {s['n'] - s['first_half']} requests")
        print(f"  busiest {args.window:g} s: {s['peak_count']} requests ending at {s['peak_end_s']:.0f} s "
              f"(load {s['peak_load']:.2f}); load > 1 for {s['over_1_s']:.0f} s of the window")
        print(f"  -> {out}")


if __name__ == "__main__":
    main()
