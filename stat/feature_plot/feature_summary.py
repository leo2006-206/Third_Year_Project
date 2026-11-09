#!/usr/bin/env python3
"""Cross-sequence summary of every feature run (mct + one feature / pair), both site eras.

Usage: python3 stat/feature_plot/feature_summary.py [--rescan] [--era fixed]
--era fixed (11-09): FIXED site only (every single now rerun there: batch_singles), noise = fixed none_1 vs none_2,
pair interaction same-era (exact), x separators = families; output temp/figs/features_fixed/.
Data: OLD site stat/pre-sitefix/seed{2037,2042}/run_a*_c*/mct_<label>_<N>.csv (baseline mct_none_1 + _2),
      FIXED site stat/sitefix/seed{2037,2042}/run_a*_c*/ (baseline mct_none_1). 8 sequences per era.
Every feature is compared ONLY with the none of its own era and sequence (ratio = feature / none;
several runs -> mean of the run statistics first). Fixed-site labels get a '*' (e.g. B8lfu*).
Noise floor per sequence and statistic = max(|ln none_1/none_2|, |ln Slru_1/Slru_2|) of the OLD era
(the only repeats; used for both eras). Latency = status 200 requests only.

Outputs (temp/figs/features/): runs.csv (one row per run: stats + parts), feature_ratio.png,
feature_wins.png, feature_where.png, feature_load.png; every table also printed on stdout.
"""

import argparse
import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import matplotlib.ticker
import numpy as np
import pandas as pd

ROOT = (
    Path(__file__).resolve().parents[2]
)  # stat/feature_plot/ -> repo root (moved from temp/ 11-09)
sys.path.insert(0, str(ROOT / "stat" / "feature_plot"))
sys.path.insert(0, str(ROOT / "stat"))
from feature_detail_plot import PARTS, run_metrics  # noqa: E402
from latency_plot import FEATURE_FILE, FEATURE_STYLE  # noqa: E402

OUT = ROOT / "stat" / "figs" / "features"
SEEDS, SEQS = (2037, 2042), ("a0.5_c1", "a0.5_c2", "a1.2_c1", "a1.2_c2")
ERAS = {"old": "stat/pre-sitefix/seed{s}", "fixed": "stat/sitefix/seed{s}"}
STATS = ["mean", "p50", "p90", "p99"]
# x order: old-era features by family, then fixed-era ones
ORDER = [
    ("old", f)
    for f in ["Slru", "Slfu", "B6lru", "B8lru", "Alru", "Alfu", "C", "P", "SlruP"]
] + [
    ("fixed", f)
    for f in ["B8lfu", "PSC1small", "PSC0.5small", "PSC0.5save", "SlfuAlfu", "CP"]
]
FIXED_ORDER = [
    ("fixed", f)
    for f in [
        "Slru",
        "Slfu",
        "B6lru",
        "B8lru",
        "B8lfu",
        "Alru",
        "Alfu",
        "C",
        "P",
        "PSC1small",
        "PSC0.5small",
        "PSC0.5save",
        "SlfuAlfu",
        "CP",
    ]
]
SEP = [8.5]
FOOT_RATIO = None
STAR = "'*' = fixed site vs its own none; "  # x of dotted separators (old | fixed); fixed mode: family borders
NOISE_SRC = "old-era none / Slru repeats"
PAIRS = {"SlruP": ("Slru", "P"), "SlfuAlfu": ("Slfu", "Alfu"), "CP": ("C", "P")}
INK, MUTED, GRID, SURF = "#3b3a37", "#52514e", "#e4e3df", "#fcfcfb"
WIN_C = {"better": "#0e7d55", "within noise": "#c3c2b7", "worse": "#c0392b"}


def tick(era, f):
    return f + ("*" if era == "fixed" and len(ERAS) > 1 else "")


def scan() -> pd.DataFrame:
    """[Dishonest I/O] One row per run: era, seed, seq, label, run, latency stats, parts (s/req)."""
    rows = []
    for era, root in ERAS.items():
        for s in SEEDS:
            for q in SEQS:
                d = ROOT / root.format(s=s) / f"run_{q}"
                for csv in sorted(d.glob("mct_*.csv")):
                    m = FEATURE_FILE.match(csv.name)
                    if not m or m["policy"] != "mct":
                        continue
                    df = pd.read_csv(csv)
                    lat = df.loc[df["status"] == 200, "latency_ms"].to_numpy() / 1000
                    met = run_metrics(csv)
                    rows.append(
                        {
                            "era": era,
                            "seed": s,
                            "seq": q,
                            "label": m["features"],
                            "run": csv.stem.rsplit("_", 1)[1],
                            "fail": int((df["status"] != 200).sum()),
                            "mean": lat.mean(),
                            **{f"p{p}": np.percentile(lat, p) for p in (50, 90, 99)},
                            **{c: met[f"all_{c}"] for c, *_ in PARTS},
                        }
                    )
    return pd.DataFrame(rows)


def ratios(runs: pd.DataFrame) -> tuple[pd.DataFrame, pd.DataFrame]:
    """[Honest] Per (era, seed, seq, label): mean over runs, ratio to own none per statistic;
    noise per (seed, seq) and statistic (log units)."""
    m = (
        runs.groupby(["era", "seed", "seq", "label"])
        .mean(numeric_only=True)
        .reset_index()
    )
    base = m[m["label"] == "none"].set_index(["era", "seed", "seq"])
    r = m[m["label"] != "none"].copy()
    for c in STATS + [c for c, *_ in PARTS]:
        b = base.loc[list(zip(r["era"], r["seed"], r["seq"])), c].to_numpy()
        r[f"r_{c}" if c in STATS else f"d_{c}"] = (
            r[c].to_numpy() / b if c in STATS else r[c].to_numpy() - b
        )
    old = runs[runs["era"] == ("old" if "old" in ERAS else "fixed")]
    noise = []
    for (s, q), g in old.groupby(["seed", "seq"]):
        row = {"seed": s, "seq": q}
        for c in STATS:
            n = [
                abs(np.log(v[0] / v[1]))
                for lab in ("none", "Slru")
                if len(v := g.loc[g["label"] == lab].sort_values("run")[c].to_numpy())
                >= 2
            ]
            row[c] = max(n) if n else np.nan
        noise.append(row)
    return r, pd.DataFrame(noise).set_index(["seed", "seq"])


def gmean(x):
    return float(np.exp(np.mean(np.log(x))))


def style_ax(ax, title=None, ylabel=None):
    if title:
        ax.set_title(title, fontsize=14, color=INK, loc="left")
    if ylabel:
        ax.set_ylabel(ylabel, fontsize=12, color=INK)
    ax.set_facecolor(SURF)
    ax.grid(axis="y", color=GRID, zorder=0)
    ax.set_axisbelow(True)
    for side in ("top", "right"):
        ax.spines[side].set_visible(False)
    for side in ("left", "bottom"):
        ax.spines[side].set_color(MUTED)
    ax.tick_params(colors=MUTED, labelsize=11)


def colour(f):
    return FEATURE_STYLE.get(f, ("#c3c2b7", False))[0]


def plot_ratio(r, noise):
    fig, axes = plt.subplots(2, 2, figsize=(20, 11), facecolor=SURF, sharex=True)
    x = np.arange(len(ORDER))
    print(
        "\n== feature_ratio: latency / own-era none per sequence; geomean over 8 seqs (range)"
    )
    print(
        f"{'feature':13}"
        + "".join(f"{s + ' gm':>9}{s + ' min':>9}{s + ' max':>9}" for s in STATS)
    )
    table = {}
    for ax, st in zip(axes.flat, STATS):
        band = float(np.exp(noise[st].mean()))
        ax.axhspan(
            1 / band,
            band,
            color="#e4e3df",
            zorder=0,
            label=f"noise (mean ±{100 * (band - 1):.0f}%)",
        )
        ax.axhline(1, color=INK, linewidth=1, linestyle="--", zorder=1)
        for i, (era, f) in enumerate(ORDER):
            g = r[(r["era"] == era) & (r["label"] == f)]
            for _, row in g.iterrows():
                a12, c2 = row["seq"].startswith("a1.2"), row["seq"].endswith("c2")
                off = (-0.15 if not a12 else 0.15) + (
                    0.05 if row["seed"] == 2042 else -0.05
                )
                ax.scatter(
                    i + off,
                    row[f"r_{st}"],
                    marker="^" if a12 else "o",
                    s=55,
                    zorder=3,
                    facecolor=SURF if c2 else colour(f),
                    edgecolor=colour(f),
                    linewidth=1.6,
                )
            gm = gmean(g[f"r_{st}"])
            table.setdefault(f, []).extend([gm, g[f"r_{st}"].min(), g[f"r_{st}"].max()])
            ax.hlines(gm, i - 0.32, i + 0.32, color=INK, linewidth=2.6, zorder=4)
            ax.text(
                i,
                max(g[f"r_{st}"].max(), gm) * 1.06,
                f"{gm:.2f}",
                ha="center",
                va="bottom",
                fontsize=10,
                color=INK,
                fontweight="bold",
            )
        ax.set_yscale("log")
        ax.set_yticks(
            [0.2, 0.3, 0.5, 0.7, 1, 1.4], ["0.2", "0.3", "0.5", "0.7", "1", "1.4"]
        )
        ax.set_ylim(0.13, 1.9)
        ax.yaxis.set_minor_formatter(matplotlib.ticker.NullFormatter())
        for sx in SEP:
            ax.axvline(sx, color=MUTED, linewidth=1, linestyle=":")
        style_ax(ax, f"{st}", "latency / own none (log)")
        ax.set_xticks(x, [tick(*o) for o in ORDER], rotation=30, ha="right")
    for f, v in table.items():
        print(f"{f:13}" + "".join(f"{y:>9.2f}" for y in v))
    h = [
        plt.Line2D([], [], marker="o", ls="", mfc=INK, mec=INK, label="a0.5 c1"),
        plt.Line2D([], [], marker="o", ls="", mfc=SURF, mec=INK, label="a0.5 c2"),
        plt.Line2D([], [], marker="^", ls="", mfc=INK, mec=INK, label="a1.2 c1"),
        plt.Line2D([], [], marker="^", ls="", mfc=SURF, mec=INK, label="a1.2 c2"),
        plt.Line2D([], [], color=INK, lw=2.6, label="geometric mean (8 seqs)"),
        plt.Rectangle((0, 0), 1, 1, color=GRID, label=f"noise floor ({NOISE_SRC})"),
    ]
    fig.legend(
        handles=h,
        loc="upper center",
        ncol=6,
        frameon=False,
        fontsize=12,
        bbox_to_anchor=(0.5, 0.945),
    )
    fig.suptitle(
        "Each feature on mct: latency relative to mct with no feature, per sequence (below 1 = faster)",
        fontsize=18,
        color=INK,
    )
    fig.text(
        0.01,
        0.01,
        FOOT_RATIO
        if FOOT_RATIO
        else "left of the dotted line: OLD site (none = 2 runs); '*' = FIXED site (6da8258 + 1730623), "
        "compared with its own fresh none (1 run) - absolute seconds of the two eras are NOT comparable. "
        "1 run per feature cell, seeds 2037 + 2042 (2042 dots right of 2037), status 200 only.",
        fontsize=10,
        color=MUTED,
    )
    fig.subplots_adjust(
        left=0.05, right=0.99, top=0.88, bottom=0.12, hspace=0.12, wspace=0.1
    )
    fig.savefig(OUT / "feature_ratio.png", dpi=130)
    print(f"plot -> {OUT / 'feature_ratio.png'}")


def classify(r, noise, st):
    """[Honest] Per row: better / within noise / worse than own none on statistic st (exact values)."""
    n = noise.loc[list(zip(r["seed"], r["seq"])), st].to_numpy()
    lr = np.log(r[f"r_{st}"].to_numpy())
    return np.where(lr < -n, "better", np.where(lr > n, "worse", "within noise"))


def plot_wins(r, noise):
    fig, axes = plt.subplots(1, 2, figsize=(20, 7), facecolor=SURF, sharey=True)
    print(
        "\n== feature_wins: sequences (of 8) better than own none by more than that sequence's noise / within / worse"
    )
    for ax, st in zip(axes, ("mean", "p99")):
        print(f"-- {st}\n{'feature':13}{'better':>8}{'within':>8}{'worse':>8}")
        x = np.arange(len(ORDER))
        cnt = {k: [] for k in WIN_C}
        for era, f in ORDER:
            g = r[(r["era"] == era) & (r["label"] == f)]
            c = pd.Series(classify(g, noise, st)).value_counts()
            for k in WIN_C:
                cnt[k].append(int(c.get(k, 0)))
            print(f"{tick(era, f):13}" + "".join(f"{cnt[k][-1]:>8}" for k in WIN_C))
        bottom = np.zeros(len(ORDER))
        for k, col in WIN_C.items():
            v = np.array(cnt[k])
            ax.bar(
                x, v, 0.7, bottom=bottom, color=col, label=k, edgecolor=SURF, zorder=2
            )
            for i, (b, h) in enumerate(zip(bottom, v)):
                if h:
                    ax.text(
                        i,
                        b + h / 2,
                        str(h),
                        ha="center",
                        va="center",
                        fontsize=11,
                        color=SURF if k != "within noise" else INK,
                        fontweight="bold",
                    )
            bottom += v
        for sx in SEP:
            ax.axvline(sx, color=MUTED, linewidth=1, linestyle=":")
        ax.set_xticks(x, [tick(*o) for o in ORDER], rotation=30, ha="right")
        ax.set_ylim(0, 8.6)
        style_ax(ax, f"{st} latency", "sequences (of 8)")
    h, l = axes[0].get_legend_handles_labels()
    fig.legend(
        h,
        l,
        loc="upper center",
        ncol=3,
        frameon=False,
        fontsize=12,
        bbox_to_anchor=(0.5, 0.92),
    )
    fig.suptitle(
        "How often each feature beats mct with no feature by more than run-to-run noise",
        fontsize=18,
        color=INK,
    )
    fig.text(
        0.01,
        0.01,
        f"noise per sequence = max |ln run1/run2| of {NOISE_SRC} (same sequence); "
        + (
            "'*' = fixed site vs its own none; "
            if len(ERAS) > 1
            else "all runs on the FIXED site; "
        )
        + "whole counts on exact values",
        fontsize=10,
        color=MUTED,
    )
    fig.subplots_adjust(left=0.05, right=0.99, top=0.82, bottom=0.17, wspace=0.05)
    fig.savefig(OUT / "feature_wins.png", dpi=130)
    print(f"plot -> {OUT / 'feature_wins.png'}")


def plot_where(r):
    parts = [(c, lab, col, hat) for c, lab, col, hat in PARTS]
    fig, ax = plt.subplots(figsize=(20, 7.5), facecolor=SURF)
    x = np.arange(len(ORDER))
    w = 0.8 / (len(parts) + 1)
    print(
        "\n== feature_where: change vs own none, s per request (all requests, mean over 8 seqs; < 0 = saved)"
    )
    print(
        f"{'feature':13}"
        + "".join(f"{lab.split(' (')[0]:>14}" for _, lab, *_ in parts)
        + f"{'total':>10}"
    )
    for j, (c, lab, col, hat) in enumerate(parts + [("total", "total", INK, None)]):
        vals = []
        for era, f in ORDER:
            g = r[(r["era"] == era) & (r["label"] == f)]
            vals.append(
                sum(g[f"d_{p}"].mean() for p, *_ in parts)
                if c == "total"
                else g[f"d_{c}"].mean()
            )
        ax.bar(
            x - 0.4 + w * (j + 0.5),
            vals,
            w * 0.92,
            color=col,
            hatch=hat,
            edgecolor=SURF,
            label=lab,
            zorder=2,
        )
        if c == "total":
            for i, v in enumerate(vals):
                ax.text(
                    x[i] - 0.4 + w * (j + 0.5),
                    v + (0.3 if v > 0 else -0.3),
                    f"{v:+.1f}",
                    ha="center",
                    va="bottom" if v > 0 else "top",
                    fontsize=9,
                    color=INK,
                    fontweight="bold",
                )
    for era, f in ORDER:
        g = r[(r["era"] == era) & (r["label"] == f)]
        d = [g[f"d_{p}"].mean() for p, *_ in parts]
        print(
            f"{tick(era, f):13}"
            + "".join(f"{v:>14.2f}" for v in d)
            + f"{sum(d):>10.2f}"
        )
    ax.axhline(0, color=INK, linewidth=1)
    for sx in SEP:
        ax.axvline(sx, color=MUTED, linewidth=1, linestyle=":")
    ax.set_xticks(x, [tick(*o) for o in ORDER], rotation=30, ha="right")
    style_ax(ax, None, "change vs none (s per request)")
    ax.legend(
        loc="upper center",
        ncol=len(parts) + 1,
        frameon=False,
        fontsize=12,
        bbox_to_anchor=(0.5, 1.08),
    )
    fig.suptitle(
        "Where each feature changes latency: per part, vs mct with no feature (mean of 8 sequences)",
        fontsize=18,
        color=INK,
    )
    fig.text(
        0.01,
        0.01,
        "all requests (segment cache hit = 0 s, C / P follower = its whole latency as follower "
        "wait); " + STAR + "total = sum of the parts (body left out)",
        fontsize=10,
        color=MUTED,
    )
    fig.subplots_adjust(left=0.05, right=0.99, top=0.84, bottom=0.17)
    fig.savefig(OUT / "feature_where.png", dpi=130)
    print(f"plot -> {OUT / 'feature_where.png'}")


def plot_load(r):
    groups = [
        ("a0.5", lambda q: q.startswith("a0.5"), "o", True),
        ("a1.2", lambda q: q.startswith("a1.2"), "^", True),
        ("c1", lambda q: q.endswith("c1"), "s", False),
        ("c2", lambda q: q.endswith("c2"), "D", False),
    ]
    fig, axes = plt.subplots(1, 2, figsize=(20, 7), facecolor=SURF, sharey=True)
    print(
        "\n== feature_load: geomean ratio vs own none (4 seqs each) by load alpha and by cluster case"
    )
    for ax, st in zip(axes, ("mean", "p99")):
        print(f"-- {st}\n{'feature':13}" + "".join(f"{g[0]:>8}" for g in groups))
        x = np.arange(len(ORDER))
        for k, (name, sel, mk, filled) in enumerate(groups):
            vals = []
            for era, f in ORDER:
                g = r[(r["era"] == era) & (r["label"] == f) & r["seq"].map(sel)]
                vals.append(gmean(g[f"r_{st}"]))
            ax.scatter(
                x - 0.24 + 0.16 * k,
                vals,
                marker=mk,
                s=70,
                zorder=3,
                label=name,
                facecolor="#104281" if k % 2 == 0 else "#eb6834",
                edgecolor=INK,
                linewidth=0.6,
            )
            if k == 0:
                rows = [[v] for v in vals]
            else:
                for row, v in zip(rows, vals):
                    row.append(v)
        for (era, f), row in zip(ORDER, rows):
            print(f"{tick(era, f):13}" + "".join(f"{v:>8.2f}" for v in row))
        ax.axhline(1, color=INK, linewidth=1, linestyle="--")
        for sx in SEP:
            ax.axvline(sx, color=MUTED, linewidth=1, linestyle=":")
        ax.set_yscale("log")
        ax.set_yticks(
            [0.2, 0.3, 0.5, 0.7, 1, 1.4], ["0.2", "0.3", "0.5", "0.7", "1", "1.4"]
        )
        ax.yaxis.set_minor_formatter(matplotlib.ticker.NullFormatter())
        ax.set_xticks(x, [tick(*o) for o in ORDER], rotation=30, ha="right")
        style_ax(ax, f"{st} latency", "geomean latency / own none (log)")
    h, l = axes[0].get_legend_handles_labels()
    fig.legend(
        h,
        [
            "a0.5 (flatter popularity)",
            "a1.2 (skewed popularity, ~0.1 more load)",
            "case 1",
            "case 2",
        ],
        loc="upper center",
        ncol=4,
        frameon=False,
        fontsize=12,
        bbox_to_anchor=(0.5, 0.92),
    )
    fig.suptitle(
        "When each feature helps: ratio vs none split by workload alpha and by cluster case",
        fontsize=18,
        color=INK,
    )
    fig.text(
        0.01,
        0.01,
        "each marker = geometric mean over the 4 sequences of that group (2 seeds x 2 of the other "
        "axis); " + STAR,
        fontsize=10,
        color=MUTED,
    )
    fig.subplots_adjust(left=0.05, right=0.99, top=0.82, bottom=0.17, wspace=0.05)
    fig.savefig(OUT / "feature_load.png", dpi=130)
    print(f"plot -> {OUT / 'feature_load.png'}")


def interaction(r, noise):
    """[Honest] Pair interaction on log mean: ln(pair/none) - ln(A/none) - ln(B/none); < 0 = the pair is
    better than the two parts multiplied. SlfuAlfu / CP parts are OLD-site runs -> cross-era, approximate."""
    print(
        "\n== pair interaction (log mean, %; < 0 = synergy). "
        + (
            "all FIXED site: pair and parts same era (exact)"
            if "old" not in ERAS
            else "SlfuAlfu / CP: pair fixed site, parts old site (approximate)"
        )
    )
    print(
        f"{'seq':16}"
        + "".join(f"{p:>10}" for p in PAIRS if "old" in ERAS or p != "SlruP")
        + f"{'noise%':>8}"
    )
    for s in SEEDS:
        for q in SEQS:
            v = []
            for p, (a, b) in PAIRS.items():
                if "old" not in ERAS and p == "SlruP":
                    continue

                def get(lab):
                    return r[
                        (r["seed"] == s)
                        & (r["seq"] == q)
                        & (r["label"] == lab)
                        & (
                            r["era"]
                            == (
                                "fixed"
                                if "old" not in ERAS
                                else "old"
                                if lab != p or p == "SlruP"
                                else "fixed"
                            )
                        )
                    ]["r_mean"].iloc[0]

                v.append(
                    100 * (np.exp(np.log(get(p)) - np.log(get(a)) - np.log(get(b))) - 1)
                )
            print(
                f"{s} {q:11}"
                + "".join(f"{x:>+10.0f}" for x in v)
                + f"{100 * (np.exp(noise.loc[(s, q), 'mean']) - 1):>8.1f}"
            )


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument(
        "--rescan",
        action="store_true",
        help="re-read every run (else use runs.csv if present)",
    )
    ap.add_argument("--era", choices=["both", "fixed"], default="both")
    args = ap.parse_args()
    global OUT, ERAS, ORDER, SEP, NOISE_SRC, FOOT_RATIO, STAR
    if args.era == "fixed":
        OUT = OUT.parent / "features_fixed"
        ERAS = {"fixed": ERAS["fixed"]}
        ORDER, SEP = FIXED_ORDER, [4.5, 6.5, 8.5, 11.5]
        NOISE_SRC = "fixed-site none_1 / none_2"
        STAR = "all runs FIXED site; "
        FOOT_RATIO = (
            "ALL runs on the FIXED site (6da8258 + 1730623); none = 2 runs (mean), features 1 run per sequence; "
            "dotted lines = families (segment | asset | in-flight | PSC | pairs); seeds 2037 + 2042 "
            "(2042 dots right of 2037), status 200 only."
        )
    OUT.mkdir(parents=True, exist_ok=True)
    cache = OUT / "runs.csv"
    runs = scan() if args.rescan or not cache.exists() else pd.read_csv(cache)
    runs.to_csv(cache, index=False)
    print(
        f"runs: {len(runs)} ({runs.groupby('era').size().to_dict()}), failed requests {runs['fail'].sum()}"
    )
    r, noise = ratios(runs)
    print(
        f"noise floor per sequence ({NOISE_SRC}, % = exp(max |ln run1/run2|) - 1)\n"
        + (100 * (np.exp(noise) - 1)).round(1).to_string()
    )
    plot_ratio(r, noise)
    plot_wins(r, noise)
    plot_where(r)
    plot_load(r)
    interaction(r, noise)
    r.to_csv(OUT / "ratios.csv", index=False)


if __name__ == "__main__":
    main()
