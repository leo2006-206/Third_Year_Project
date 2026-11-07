#!/usr/bin/env python3
"""
Figures for the project summary (README.md), written into highlight/.

Usage: highlight/make_figures.py

  headline.png         mean and p99 latency of the key policies on all four set 2 sequences
  latency_c1.png       mean / p50 / p90 / p99 of every policy, mixed cluster, alpha 0.5
  breakdown_c1.png     where the latency goes (queue / work / asset + net), same sequence
  cost_model.png       predicted vs measured work per request inside the running MCT
  offline_bracket.png  online MCT against the clairvoyant and offline bounds (trace simulator; not shown in README)

Data: the set 2 replay runs in stat/seed2037/run_a<alpha>_c<case>/, the cost model in stat_predict/, and
highlight/data/offline_opt.csv (output of the offline search, temp/offline_opt.py).
"""

import csv
import sys
from pathlib import Path
from urllib.parse import unquote

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(ROOT / "stat"))
import latency_detail_plot as ldp
import latency_plot as lp

SEQS = [
    ("run_a0.5_c1", "mixed\nα 0.5"),
    ("run_a1.2_c1", "mixed\nα 1.2"),
    ("run_a0.5_c2", "all-GPU\nα 0.5"),
    ("run_a1.2_c2", "all-GPU\nα 1.2"),
]
KEY = ["rr", "esce", "throttled2", "mct_const_flat", "mct"]
LABEL = {
    "rr": "round robin",
    "esce": "least connections (esce)",
    "throttled2": "throttled, limit 2",
    "mct_const_flat": "MCT, no cost model",
    "mct": "MCT + cost model",
}
INK, MUTED, GRID = "#1b1b1a", "#52514e", "#e4e3df"


def style(ax):
    ax.grid(axis="y", color=GRID, zorder=0)
    ax.set_axisbelow(True)
    for side in ("top", "right"):
        ax.spines[side].set_visible(False)


def headline():
    """Grouped bars: mean and p99 per sequence for the key policies; a bar above the axis is cut
    and its value written at the top."""
    summ = {d: lp.summarise(lp.load(ROOT / "stat" / "seed2037" / d)) for d, _ in SEQS}
    fig, axes = plt.subplots(1, 2, figsize=(16, 6.5))
    width = 0.8 / len(KEY)
    x = np.arange(len(SEQS))
    for ax, stat, cap in zip(axes, ["mean", "p99"], [32, 160]):
        for i, p in enumerate(KEY):
            colour, hatched = lp.STYLE[p]
            vals = np.array([summ[d].loc[p, stat] for d, _ in SEQS])
            sds = np.array([summ[d].loc[p, f"{stat}_sd"] for d, _ in SEQS])
            xs = x - 0.4 + (i + 0.5) * width
            ax.bar(
                xs,
                np.minimum(vals, cap),
                width * 0.9,
                yerr=np.where(vals > cap, 0, sds),
                color=colour,
                hatch="//" if hatched else None,
                edgecolor="#3b3a37" if hatched else "#fcfcfb",
                linewidth=0.6,
                error_kw={"elinewidth": 1, "capsize": 2, "ecolor": "#3b3a37"},
                label=LABEL[p],
                zorder=2,
            )
            for xi, v, s in zip(xs, vals, sds):
                cut = v > cap  # cut bar: value just above the axis top, in dark ink
                ax.text(
                    xi,
                    cap * 1.01 if cut else v + s + cap * 0.01,
                    f"{v:.0f}" + ("↑" if cut else ""),
                    ha="center",
                    va="bottom",
                    fontsize=9,
                    color=INK if cut else MUTED,
                    fontweight="bold" if cut else "normal",
                    clip_on=False,
                    zorder=3,
                )
        ax.set_ylim(0, cap)
        ax.set_xticks(x, [lab for _, lab in SEQS], fontsize=12)
        ax.set_title(f"{stat} latency (s)", fontsize=15)
        style(ax)
    axes[0].legend(fontsize=11, frameon=False, loc="upper right")
    fig.text(
        0.5,
        0.01,
        "Set 2 replays: mean over runs (error bar = std); ↑ = bar cut at the axis top, value above it",
        ha="center",
        va="bottom",
        fontsize=12,
        color=MUTED,
    )
    fig.tight_layout(rect=(0, 0.05, 1, 0.97))
    fig.savefig(HERE / "headline.png", dpi=150)
    plt.close(fig)


def sequence_figures():
    d = ROOT / "stat" / "seed2037" / "run_a0.5_c1"
    lp.plot(
        lp.summarise(lp.load(d)),
        "mixed cluster, α 0.5 (set 2)",
        HERE / "latency_c1.png",
    )
    ldp.plot(
        ldp.summarise(ldp.load(d)),
        "mixed cluster, α 0.5 (set 2)",
        HERE / "breakdown_c1.png",
    )
    plt.close("all")


def cost_model():
    """Replays MCT's learned machine factor over every set 2 mct run and plots the prediction it
    used (factor x cost model) against the measured work, per request."""
    coef = {
        (r["stage"], r["backend"]): r
        for r in csv.DictReader(
            (ROOT / "stat_predict/coef.csv").read_text().splitlines()
        )
    }
    feat = {
        r["path"]: r
        for r in csv.DictReader(
            (ROOT / "stat_predict/features.csv").read_text().splitlines()
        )
    }

    def model(path, render):
        x = feat.get(path) or feat.get(unquote(path))
        if x is None:
            return None

        def stage(s, b):
            c = coef[(s, b)]
            skip = ("stage", "backend", "a")
            return float(c["a"]) + sum(
                float(v) * float(x[k]) for k, v in c.items() if k not in skip
            )

        full = (
            stage("decode", "gpu")
            + stage("render", render)
            + stage("encode", f"{render}+cpu")
        )
        return full * float(x["seg_frac"])

    pts = []  # (predicted, actual, render)
    for f in sorted((ROOT / "stat" / "seed2037").glob("run_a*_c*/mct_none_*.csv")):
        case = f.parent.name.rsplit("_c", 1)[1]
        with open(ROOT / f"servers_container/cluster_endpoint_c{case}.csv") as fh:
            rows = csv.DictReader(line for line in fh if not line.startswith("#"))
            sites = {
                f"{r['ip']}:{r['port']}": (r["id"], r["composition"]) for r in rows
            }
        df = pd.read_csv(f)
        df = df[(df.status == 200) & (df.work_ms > 0)]
        df = df.assign(done=df.send_epoch_ms + df.latency_ms).sort_values("done")
        fac = {}
        for r in df.itertuples():
            sid, render = sites[r.worker]
            work = model(r.path, render)
            if not work:
                continue
            m = fac.setdefault(sid, 1.0 if sid.startswith("om") else 0.75)
            actual = r.work_ms / 1000
            pts.append((m * work, actual, render))
            fac[sid] = m + 0.1 * (min(max(actual / work, 0.25), 4.0) - m)
    p = pd.DataFrame(pts, columns=["pred", "actual", "render"])
    err = (p.pred - p.actual).abs() / p.actual
    fig, ax = plt.subplots(figsize=(8, 7.5))
    for render, colour in [
        ("gpu", "#2a78d6"),
        ("llvmpipe", "#1baf7a"),
        ("cpu", "#eb6834"),
    ]:
        q = p[p.render == render]
        ax.scatter(
            q.pred,
            q.actual,
            s=9,
            alpha=0.45,
            color=colour,
            label=f"{render} render",
            lw=0,
        )
    lim = [1, max(p.pred.max(), p.actual.max()) * 1.1]
    ax.plot(lim, lim, color=INK, lw=1)
    ax.fill_between(
        lim, [v * 0.8 for v in lim], [v * 1.2 for v in lim], color=GRID, zorder=0
    )
    ax.set_xscale("log")
    ax.set_yscale("log")
    ax.set_xlim(lim)
    ax.set_ylim(lim)
    ax.set_xlabel("predicted work (s) = machine factor × cost model", fontsize=12)
    ax.set_ylabel("measured work (s)", fontsize=12)
    ax.set_title(
        "Cost model inside the running MCT (set 2, every mct run)", fontsize=13
    )
    ax.text(
        0.03,
        0.97,
        f"{len(p)} requests\nmedian error {err.median():.1%}\np90 error {err.quantile(0.9):.1%}\n"
        f"within ±20%: {(err <= 0.2).mean():.0%} (grey band)",
        transform=ax.transAxes,
        va="top",
        fontsize=11,
        color=INK,
    )
    ax.legend(loc="lower right", fontsize=11, frameon=False, markerscale=2)
    style(ax)
    fig.tight_layout()
    fig.savefig(HERE / "cost_model.png", dpi=150)
    plt.close(fig)
    return err.median(), err.quantile(0.9)


def offline_bracket():
    """Mean latency per sequence: lower bound, best offline schedule, clairvoyant greedy, MCT."""
    d = pd.read_csv(HERE / "data/offline_opt.csv")
    g = d.groupby(["variant", "run"])["mean"].mean().unstack()
    rows = [
        ("mct (online, estimates)", "MCT (online, predicted times)", "#104281"),
        (
            "mct oracle (online, true times)",
            "greedy with true times (online)",
            "#6da7ec",
        ),
        (
            "offline iterated search, SPT order",
            "best offline schedule found",
            "#1baf7a",
        ),
        ("lower bound (no waiting)", "lower bound (no queueing)", "#c3c2b7"),
    ]
    runs = ["a0.5_c1", "a1.2_c1", "a0.5_c2", "a1.2_c2"]
    x = np.arange(len(runs))
    width = 0.8 / len(rows)
    fig, ax = plt.subplots(figsize=(11, 6))
    for i, (key, label, colour) in enumerate(rows):
        vals = g.loc[key, runs].to_numpy()
        xs = x - 0.4 + (i + 0.5) * width
        ax.bar(
            xs,
            vals,
            width * 0.9,
            color=colour,
            label=label,
            zorder=2,
            edgecolor="#fcfcfb",
        )
        for xi, v in zip(xs, vals):
            ax.text(
                xi,
                v + 0.2,
                f"{v:.1f}",
                ha="center",
                va="bottom",
                fontsize=10,
                color=MUTED,
            )
    ax.set_xticks(x, [lab for _, lab in SEQS], fontsize=12)
    ax.set_ylabel("mean latency (s)", fontsize=12)
    ax.set_title(
        "How far is online MCT from the best schedule? (trace simulator, set 2)",
        fontsize=13,
    )
    ax.legend(
        fontsize=11,
        frameon=False,
        loc="upper center",
        ncol=2,
        bbox_to_anchor=(0.5, -0.12),
    )
    style(ax)
    fig.tight_layout()
    fig.savefig(HERE / "offline_bracket.png", dpi=150)
    plt.close(fig)


def main():
    headline()
    sequence_figures()
    med, p90 = cost_model()
    offline_bracket()
    print(f"cost model live error: median {med:.1%}, p90 {p90:.1%}")
    print(f"figures -> {HERE}")


if __name__ == "__main__":
    main()
