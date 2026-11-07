#!/usr/bin/env python3
"""
Where the latency of each policy goes, for one sequence directory of replay runs.

Usage:
  stat/latency_detail_plot.py <run dir> [--out PNG] [--exclude POLICY ...]
  e.g. stat/latency_detail_plot.py stat/seed2037/run_a0.5_c1

Per request (status 200): latency = queue + asset + work + net + body, exactly. queue_ms already
holds the central wait (wait_ms: held at the main server until a site has a free slot; only the
throttled policies wait there, the others send at once) plus the wait in the site's own queue.
Body (sending the result back to the client) is left out: the policy doesn't change it.

Per run: the mean of each part over its requests; per policy: mean and std over runs. Prints a
table, and plots one stacked bar per policy (default stat/latency_detail_<run dir name>.png):
site queue + central wait (two shades of one colour: both are queueing), work, asset + net.
The error bar is the std over runs of the stacked total. The y axis stops at the second highest
bar + 10%: a worse bar (e.g. rr in c1) is cut and its total written inside. The five lowest
totals get a rank badge 1-5, and the space right of the bars lists the five lowest queue (central
wait + site queue) and the five lowest work.
"""

import argparse
import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
import pandas as pd
from latency_plot import RUN_FILE, STYLE

# (column, label, colour, hatch); stacked bottom to top
PARTS = [
    ("site_queue", "site queue", "#2a78d6", None),
    ("wait", "central wait", "#86b6ef", "//"),
    ("work", "work", "#eb6834", None),
    ("asset_net", "asset + net", "#1baf7a", None),
]
FS_TITLE, FS_LABEL, FS_TICK, FS_VALUE = 20, 18, 14, 11
TOP = 5  # lowest totals that get a rank badge; length of the queue / work lists


def run_parts(csv: Path) -> dict:
    """[Dishonest I/O] Mean of each part (s) over the successful requests of one run."""
    df = pd.read_csv(csv)
    df = df[df["status"] == 200]
    wait = df["wait_ms"].fillna(0)
    return {
        "site_queue": (df["queue_ms"] - wait).mean() / 1000,
        "wait": wait.mean() / 1000,
        "work": df["work_ms"].mean() / 1000,
        "asset_net": (df["asset_ms"] + df["net_ms"]).mean() / 1000,
    }


def load(run_dir: Path) -> pd.DataFrame:
    """[Dishonest I/O] One row per run: policy, run, the mean of each part."""
    rows = []
    for csv in sorted(run_dir.glob("*_none_*.csv")):
        m = RUN_FILE.match(csv.name)
        if m:
            rows.append({"policy": m["policy"], "run": int(m["run"]), **run_parts(csv)})
    return pd.DataFrame(rows)


def summarise(runs: pd.DataFrame) -> pd.DataFrame:
    """[Honest] Per policy: n runs, mean of each part over runs, total and its std over runs."""
    cols = [c for c, *_ in PARTS]
    runs = runs.assign(total=runs[cols].sum(axis=1))
    g = runs.groupby("policy")
    out = g[cols + ["total"]].mean()
    out["total_sd"] = g["total"].std(ddof=1).fillna(0.0)
    out.insert(0, "n", g.size())
    order = [p for p in STYLE if p in out.index] + sorted(set(out.index) - set(STYLE))
    return out.loc[order]


def print_table(summary: pd.DataFrame, title: str):
    print(f"Mean latency parts (s, mean over runs), body left out: {title}")
    labels = [label for _, label, *_ in PARTS]
    header = f"{'policy':16} {'n':>2}" + "".join(f"{s:>14}" for s in labels + ["total"])
    print(header)
    print("-" * len(header))
    for policy, r in summary.iterrows():
        cells = "".join(f"{r[c]:>14.2f}" for c, *_ in PARTS)
        total = f"{r['total']:.2f} ± {r['total_sd']:.2f}"
        print(f"{policy:16} {int(r['n']):>2}{cells}{total:>14}")


def plot(summary: pd.DataFrame, title: str, out: Path):
    policies = list(summary.index)
    x = np.arange(len(policies))
    total = summary["total"].to_numpy()
    sd = summary["total_sd"].to_numpy()
    tops = np.sort(total + sd)
    ymax = (tops[-2] if len(tops) > 1 else tops[-1]) * 1.1
    fig, ax = plt.subplots(figsize=(16, 8))
    bottom = np.zeros(len(policies))
    for col, label, colour, hatch in PARTS:
        h = summary[col].to_numpy()
        ax.bar(
            x,
            h,
            0.75,
            bottom=bottom,
            color=colour,
            hatch=hatch,
            edgecolor="#fcfcfb",  # 2px-style gap between stacked segments
            linewidth=1.2,
            label=label,
            zorder=2,
        )
        for i in range(len(policies)):
            mid = bottom[i] + h[i] / 2
            # label a segment only where it is tall enough to hold the text and not cut
            if h[i] > ymax * 0.045 and mid < ymax * 0.96:
                ax.text(
                    x[i],
                    mid,
                    f"{h[i]:.1f}",
                    ha="center",
                    va="center",
                    fontsize=FS_VALUE,
                    color="#1b1b1a" if col == "wait" else "#fcfcfb",
                    zorder=3,
                )
        bottom += h
    for i in range(len(policies)):
        text = f"{total[i]:.1f}"
        if total[i] > ymax:  # cut bar: total inside, under the top edge
            ax.text(
                x[i],
                ymax * 0.98,
                f"{text}\n↑",
                ha="center",
                va="top",
                fontsize=FS_VALUE,
                fontweight="bold",
                color="#fcfcfb",
                zorder=4,
                bbox={"boxstyle": "round,pad=0.2", "fc": "#3b3a37", "ec": "none"},
            )
        else:
            ax.errorbar(
                x[i], total[i], yerr=sd[i], color="#3b3a37", capsize=4, elinewidth=1.2
            )
            ax.text(
                x[i],
                total[i] + sd[i] + ymax * 0.01,
                text,
                ha="center",
                va="bottom",
                fontsize=FS_VALUE + 1,
                fontweight="bold",
                color="#1b1b1a",
                zorder=3,
            )
    # rank badges above the TOP lowest totals
    for rank, i in enumerate(np.argsort(total, kind="stable")[:TOP], start=1):
        ax.text(
            x[i],
            total[i] + sd[i] + ymax * 0.065,
            str(rank),
            ha="center",
            va="bottom",
            fontsize=FS_VALUE,
            fontweight="bold",
            color="#fcfcfb",
            bbox={"boxstyle": "circle,pad=0.25", "fc": "#1b1b1a", "ec": "none"},
            zorder=4,
        )
    # top lists in the free space right of the bars, under the legend
    queue = summary["site_queue"] + summary["wait"]
    lists = [("Lowest queue", queue), ("Lowest work", summary["work"])]
    lines = []
    for heading, values in lists:
        lines.append(f"{heading} (s)")
        for rank, (p, v) in enumerate(
            values.sort_values(kind="stable")[:TOP].items(), 1
        ):
            lines.append(f" {rank} {p:<15}{v:5.1f}")
        lines.append("")
    ax.text(
        1.02,
        0.62,
        "\n".join(lines).rstrip(),
        transform=ax.transAxes,
        ha="left",
        va="top",
        fontsize=FS_TICK - 1,
        family="monospace",
        color="#1b1b1a",
    )
    ax.set_ylim(0, ymax)
    ax.set_xlim(-0.6, len(policies) - 0.4)
    ax.set_xticks(
        x, policies, rotation=45, ha="right", rotation_mode="anchor", fontsize=FS_TICK
    )
    ax.tick_params(axis="y", labelsize=FS_TICK)
    ax.set_ylabel("Mean time per request (s)", fontsize=FS_LABEL)
    ax.set_title(f"Where the latency goes: {title}", fontsize=FS_TITLE)
    ax.grid(axis="y", color="#e4e3df", zorder=0)
    ax.set_axisbelow(True)
    for side in ("top", "right"):
        ax.spines[side].set_visible(False)
    ax.legend(
        handles=ax.get_legend_handles_labels()[0][
            ::-1
        ],  # top segment first, as stacked
        labels=ax.get_legend_handles_labels()[1][::-1],
        fontsize=FS_TICK,
        loc="upper left",
        bbox_to_anchor=(1.0, 1.0),
        frameon=False,
    )
    fig.text(
        0.99,
        0.01,
        "mean over runs; bold = total without body, error bar = its std over runs;"
        " ↑ = bar cut, total inside; numbered = 5 lowest totals",
        ha="right",
        fontsize=FS_TICK - 3,
        color="#52514e",
    )
    # fixed margins: tight_layout gives up when a rank badge sits above the axes; right = lists
    fig.subplots_adjust(left=0.07, right=0.74, top=0.9, bottom=0.2)
    fig.savefig(out, dpi=150)
    print(f"plot -> {out}")


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("run_dir", type=Path)
    ap.add_argument(
        "--out",
        type=Path,
        help="PNG path (default stat/latency_detail_<run dir name>.png)",
    )
    ap.add_argument(
        "--exclude", nargs="+", default=[], help="policies left out of the plot"
    )
    args = ap.parse_args()
    runs = load(args.run_dir)
    if runs.empty:
        sys.exit(f"no <policy>_none_<N>.csv files in {args.run_dir}")
    summary = summarise(runs)
    # stat/seed2042/run_a0.5_c1 -> seed2042_run_a0.5_c1 (every seed in its own folder, 2037 too)
    d = args.run_dir.resolve()
    title = d.name if d.parent.name == "stat" else f"{d.parent.name}_{d.name}"
    print_table(summary, title)
    shown = summary.drop(index=args.exclude, errors="ignore")
    out = args.out or Path(__file__).parent / f"latency_detail_{title}.png"
    plot(shown, title, out)


if __name__ == "__main__":
    main()
