#!/usr/bin/env python3
"""
End-to-end latency per policy for one sequence directory of replay runs.

Usage:
  stat/latency_plot.py <run dir> [--out PNG] [--exclude POLICY ...]
  stat/latency_plot.py <run dir> --features [LABEL ...] [--policy mct] [--out PNG]
  e.g. stat/latency_plot.py stat/pre-sitefix/seed2037/run_a0.5_c1 --exclude rr
       stat/latency_plot.py stat/pre-sitefix/seed2037/run_a0.5_c1 --features --policy mct   (mct alone vs mct + each feature)

Reads every <policy>_none_<N>.csv in the directory (one replay run each; files marked _e are
skipped), takes the latency of the requests that succeeded (status 200) and computes per run the
mean, p50, p90 and p99 in seconds. Prints a table of each statistic as mean ± std over the runs of
that policy (std is 0 with one run), plus the run count and failed requests.

Plot (default stat/latency_<run dir name>.png): one bar chart per statistic, each with its own
y axis, one bar per policy: height = mean over runs, error bar = std over runs, value written on
the bar, the five lowest ranked 1-5. The y axis stops at the second highest bar + 10%: a worse
bar (e.g. rr in c1, p99 ~385 s) is cut and its value written inside. Policies are coloured by
family (static, count-based, ACO, MCT) and shaded within it; hatched bars use no cost model
(constant work). --exclude leaves policies out of the plot; the table always lists all of them.

--features also reads the feature runs <policy>_<features>_<N>.csv (e.g. mct_Slru_1, mct_Alfu_1;
labels have no '_'); a bar is then '<policy>' (no feature) or '<policy>+<features>' (e.g. mct+Slru).
With no LABEL every feature run is taken, otherwise only those labels. --policy keeps one policy.
Default output for --features: stat/feature_plot/latency_features_<run dir name>.png; bars are
never cut there (the no-feature bar is the baseline).
"""

import argparse
import re
import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

RUN_FILE = re.compile(r"^(?P<policy>.+)_none_(?P<run>\d+)\.csv$")
# <policy>_<features>_<N>.csv; the feature label has no '_' (mct_const_flat_none_1 -> mct_const_flat, none)
FEATURE_FILE = re.compile(r"^(?P<policy>.+)_(?P<features>[^_]+)_(?P<run>\d+)\.csv$")
STATS = ["mean", "p50", "p90", "p99"]
TOP = 5  # lowest bars per statistic that get a rank badge
FS_TITLE, FS_LABEL, FS_TICK, FS_VALUE = 20, 18, 13, 11

# policy -> (colour, hatched = no cost model); order = bar order inside a group (by family)
STYLE = {
    "rr": ("#8a8984", False),
    "iwrr331": ("#5c5b57", False),
    "esce": ("#0e7d55", False),
    "throttled": ("#1baf7a", False),
    "throttled2": ("#6fd3ab", False),
    "aco": ("#eb6834", False),
    "aco_const": ("#f2a07c", True),
    "mct": ("#104281", False),
    "mct_lookup": ("#2a78d6", False),
    "mct_const": ("#6da7ec", True),
    "mct_const_flat": ("#9ec5f4", True),
}
OTHER = ("#c3c2b7", False)  # a policy not listed above
# feature bars '<policy>+<label>': segment caches purple (single darker, bins lighter), asset
# caches amber (lru lighter, lfu darker), in-flight features rose (coalescing C darker, prefetch P
# lighter); order = bar order after the plain policy bars. Labels: S<policy> = single segment cache,
# B<threshold s><policy> = bin cache (B6lru, B8lru, B8lfu), A<policy> = asset cache.
FEATURE_STYLE = {
    "Slru": ("#6b3fa0", False),
    "Slfu": ("#4a2675", False),
    "B6lru": ("#b28ad9", False),
    "B8lru": ("#d2b8ec", False),
    "B8lfu": ("#8f63c4", False),
    "Alru": ("#e8b74a", False),
    "Alfu": ("#b9770e", False),
    "C": ("#b0306a", False),
    "P": ("#e27fa9", False),
    "SlruP": ("#8e5a8a", False),  # pair (old site)
    "PSC1small": ("#0f6b6b", False),  # partial segment cache: teal family (11-08)
    "PSC0.5small": ("#3a9e9e", False),
    "PSC0.5save": ("#86cccc", False),
    "SlfuAlfu": ("#5a3a7a", False),  # pairs (fixed site, 11-08)
    "CP": ("#7a1f4c", False),
}
# Per-sequence feature plots show SINGLE-feature runs only (user 11-09); pair / multi-feature runs
# (two features on at once) are analysed separately (stat/feature_plot/feature_summary.py).
PAIRS = ("SlruP", "SlfuAlfu", "CP")
SINGLES = tuple(f for f in FEATURE_STYLE if f not in PAIRS)


def single(label: str) -> bool:
    """[Honest] True if a feature label is one known single feature (unknown labels -> False)."""
    return label in SINGLES


def run_stats(csv: Path) -> dict:
    """[Dishonest I/O] mean / p50 / p90 / p99 latency (s) of one run, and its failed requests."""
    df = pd.read_csv(csv)
    ok = df["status"] == 200
    lat = df.loc[ok, "latency_ms"].to_numpy() / 1000
    return {
        "mean": lat.mean(),
        "p50": np.percentile(lat, 50),
        "p90": np.percentile(lat, 90),
        "p99": np.percentile(lat, 99),
        "fail": int((~ok).sum()),
    }


def load(run_dir: Path, features=None, policy=None) -> pd.DataFrame:
    """[Dishonest I/O] One row per run: policy, run, the four statistics, fail.
    features None = no-feature runs only; [] = also every feature run; [labels] = also those.
    A feature run's 'policy' is '<policy>+<label>'. policy keeps only that policy's runs."""
    rows, skipped = [], set()
    pattern = RUN_FILE if features is None else FEATURE_FILE
    for csv in sorted(run_dir.glob("*.csv")):
        m = pattern.match(csv.name)
        if not m or (policy and m["policy"] != policy):
            continue
        feat = m.groupdict().get("features", "none")
        if feat != "none" and features and feat not in features:
            continue
        if feat != "none" and not single(feat):  # pair / multi-feature run: not in these plots
            skipped.add(feat)
            continue
        name = m["policy"] if feat == "none" else f"{m['policy']}+{feat}"
        rows.append({"policy": name, "run": int(m["run"]), **run_stats(csv)})
    if skipped:
        print(f"skipped non-single feature runs: {', '.join(sorted(skipped))}")
    return pd.DataFrame(rows)


def style(name: str):
    """[Honest] (colour, hatched) of a bar: a policy, or '<policy>+<feature label>'."""
    if "+" in name:
        return FEATURE_STYLE.get(name.split("+", 1)[1], OTHER)
    return STYLE.get(name, OTHER)


def summarise(runs: pd.DataFrame) -> pd.DataFrame:
    """[Honest] Per policy: n runs, failed requests, mean and std over runs of each statistic."""
    g = runs.groupby("policy")
    out = pd.DataFrame({"n": g.size(), "fail": g["fail"].sum()})
    for s in STATS:
        out[s] = g[s].mean()
        out[f"{s}_sd"] = g[s].std(ddof=1).fillna(0.0)
    order = [p for p in STYLE if p in out.index]
    for p in STYLE:  # feature bars of a policy follow, in FEATURE_STYLE order, then unknown labels
        feats = [f"{p}+{f}" for f in FEATURE_STYLE if f"{p}+{f}" in out.index]
        order += feats + sorted(i for i in out.index if i.startswith(f"{p}+") and i not in feats)
    order += sorted(set(out.index) - set(order))
    return out.loc[order]


def print_table(summary: pd.DataFrame, title: str):
    print(f"End-to-end latency (s), mean ± std over runs: {title}")
    header = f"{'policy':16} {'n':>2} {'fail':>4}" + "".join(f"{s:>15}" for s in STATS)
    print(header)
    print("-" * len(header))
    for policy, r in summary.iterrows():
        cells = "".join(f"{f'{r[s]:.1f} ± {r[s + "_sd"]:.1f}':>15}" for s in STATS)
        print(f"{policy:16} {int(r['n']):>2} {int(r['fail']):>4}{cells}")


def plot(
    summary: pd.DataFrame, title: str, out: Path, heading: str = "Latency per policy", cut: bool = True
):
    """One bar chart per statistic, each with its own y axis. The axis stops at the second highest
    bar (+ std) + 10%, so one very bad policy doesn't flatten the rest: its bar is cut and its value
    written inside. Every bar is labelled with its value, the TOP lowest with a rank badge."""
    policies = list(summary.index)
    x = np.arange(len(policies))
    fig, axes = plt.subplots(1, len(STATS), figsize=(24, 8))
    for ax, s in zip(axes, STATS):
        val = summary[s].to_numpy()
        sd = summary[f"{s}_sd"].to_numpy()
        tops = np.sort(val + sd)
        # cut=False (feature plots): the no-feature baseline is the point of comparison, never cut it
        ymax = (tops[-2] if cut and len(tops) > 1 else tops[-1]) * (1.1 if cut else 1.18)
        for i, policy in enumerate(policies):
            colour, hatched = style(policy)
            ax.bar(
                x[i],
                val[i],
                0.86,  # small gap between neighbouring bars
                yerr=sd[i] if val[i] <= ymax else None,
                color=colour,
                hatch="//" if hatched else None,
                edgecolor="#3b3a37" if hatched else "#fcfcfb",
                linewidth=0.6,
                error_kw={"elinewidth": 1.2, "capsize": 3, "ecolor": "#3b3a37"},
                zorder=2,
            )
            text = f"{val[i]:.0f}" if val[i] >= 100 else f"{val[i]:.1f}"
            if val[i] > ymax:  # cut bar: value inside, under the top edge
                ax.text(
                    x[i],
                    ymax * 0.98,
                    f"{text}\n↑",
                    ha="center",
                    va="top",
                    fontsize=FS_VALUE,
                    color="#fcfcfb",
                    fontweight="bold",
                    zorder=3,
                )
            else:
                ax.text(
                    x[i],
                    val[i] + sd[i] + ymax * 0.01,
                    text,
                    ha="center",
                    va="bottom",
                    fontsize=FS_VALUE,
                    color="#3b3a37",
                    zorder=3,
                )
        # rank badges above the value of the TOP lowest bars
        for rank, i in enumerate(np.argsort(val, kind="stable")[:TOP], start=1):
            ax.text(
                x[i],
                val[i] + sd[i] + ymax * 0.07,
                str(rank),
                ha="center",
                va="bottom",
                fontsize=FS_VALUE,
                fontweight="bold",
                color="#fcfcfb",
                bbox={"boxstyle": "circle,pad=0.25", "fc": "#1b1b1a", "ec": "none"},
                zorder=4,
            )
        ax.set_ylim(0, ymax)
        ax.set_xlim(-0.6, len(policies) - 0.4)
        ax.set_xticks(
            x,
            policies,
            rotation=60,
            ha="right",
            rotation_mode="anchor",
            fontsize=FS_TICK,
        )
        ax.tick_params(axis="y", labelsize=FS_TICK)
        ax.set_title(s, fontsize=FS_LABEL)
        ax.grid(axis="y", color="#e4e3df", zorder=0)
        ax.set_axisbelow(True)
        for side in ("top", "right"):
            ax.spines[side].set_visible(False)
    axes[0].set_ylabel("End-to-end latency (s)", fontsize=FS_LABEL)
    fig.suptitle(f"{heading}: {title}", fontsize=FS_TITLE)
    fig.text(
        0.99,
        0.01,
        "bar = mean over runs, error bar = std; hatched = no cost model; ↑ = bar cut, value inside;"
        f" numbered = {TOP} lowest",
        ha="right",
        fontsize=FS_TICK - 2,
        color="#52514e",
    )
    # fixed margins: tight_layout gives up when a rank badge sits above the axes
    fig.subplots_adjust(left=0.05, right=0.99, top=0.84, bottom=0.22, wspace=0.18)
    out.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(out, dpi=150)
    print(f"plot -> {out}")


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("run_dir", type=Path)
    ap.add_argument(
        "--out", type=Path, help="PNG path (default stat/latency_<run dir name>.png)"
    )
    ap.add_argument(
        "--exclude", nargs="+", default=[], help="policies left out of the plot"
    )
    ap.add_argument(
        "--features",
        nargs="*",
        metavar="LABEL",
        help="also read feature runs <policy>_<LABEL>_<N>.csv (no LABEL = all of them)",
    )
    ap.add_argument("--policy", help="keep only this policy (with its feature runs)")
    args = ap.parse_args()
    runs = load(args.run_dir, args.features, args.policy)
    if runs.empty:
        sys.exit(f"no matching run csv files in {args.run_dir}")
    summary = summarise(runs)
    # stat/pre-sitefix/seed2042/run_a0.5_c1 -> seed2042_run_a0.5_c1 (every seed in its own folder, 2037 too)
    d = args.run_dir.resolve()
    title = d.name if d.parent.name == "stat" else f"{d.parent.name}_{d.name}"
    print_table(summary, title)
    shown = summary.drop(index=args.exclude, errors="ignore")
    if args.features is None:
        plot(shown, title, args.out or Path(__file__).parent / f"latency_{title}.png")
    else:
        who = args.policy or "policy"
        plot(
            shown,
            title,
            args.out or Path(__file__).parent / "feature_plot" / f"latency_features_{title}.png",
            heading=f"{who} with no feature vs {who} + one feature",
            cut=False,
        )


if __name__ == "__main__":
    main()
