#!/usr/bin/env python3
"""
What each feature (cache) changes, for one sequence directory: mct with no feature vs mct + one
feature (runs <policy>_<label>_<N>.csv, labels S / B = segment cache single / 2 bins at the main
server, Alru / Alfu = asset cache at every site).

Usage:
  stat/feature_plot/feature_detail_plot.py <run dir> [--policy mct] [--out PNG]
  e.g. stat/feature_plot/feature_detail_plot.py stat/seed2042/run_a1.2_c2

Inputs per run: <base>.csv (one row per request; a segment cache hit has worker 'cache' and
queue / work / asset / net = 0), <base>_main.log ('[CACHE] added ... to lower|upper' and
'[OFFLOAD] -> cache' -> segment hits per bin) and <base>_site_<site>.log ('[ASSET] hit' /
'added' / 'add failed' / 'skip' -> asset file hits and misses per site; a miss = a fetch from the
main server). Status 200 requests only. Every number is the mean over the runs of that bar.

Figure (default stat/feature_plot/feature_detail_<run dir name>.png): the common comparison, one
stacked bar per run type (mct, mct+S, mct+B, mct+Alru, mct+Alfu) = mean time per request over all
requests (a hit counts 0): site queue / central wait / work / asset + net. Per-feature detail
(% of no feature, hit / miss) -> segment_plot.py, asset_plot.py.
Prints tables: time per request (all / sent to a site), hit ratios, per-site share / queue / asset.
"""

import argparse
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
import pandas as pd
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))  # stat/ for latency_plot
from latency_plot import FEATURE_FILE, FEATURE_STYLE, OTHER, style

SITES = ["om1", "om2", "om3", "rs1", "rs2", "rs3"]
# answered by the main server without a render of its own: segment cache hit, follower of a real
# render (coalescing), follower of a prefetch (merged prefetch)
NON_SITE = ["cache", "coalesce", "prefetch"]
PARTS = [  # same as stat/latency_detail_plot.py
    ("site_queue", "site queue", "#2a78d6", None),
    ("wait", "central wait", "#86b6ef", "//"),
    ("work", "work", "#eb6834", None),
    ("asset_net", "asset + net", "#1baf7a", None),
]
HITS = [("seg_req", "segment\nrequests"), ("seg_byte", "segment\nbytes"),
        ("asset_file", "asset\nfiles"), ("asset_byte", "asset\nbytes")]
INK, MUTED, GRID, SURF = "#3b3a37", "#52514e", "#e4e3df", "#fcfcfb"
FS_TITLE, FS_LABEL, FS_TICK, FS_VALUE = 20, 14, 12, 10


def site_of(worker: str) -> str:
    """[Honest] 'obm-offload-om2:7020' -> om2, '10.10.0.2:7030' -> rs3, 'cache' / 'coalesce' /
    'prefetch' -> unchanged."""
    if worker in NON_SITE:
        return worker
    m = re.search(r"(om\d)", worker)
    if m:
        return m[1]
    port = int(worker.rsplit(":", 1)[1])
    return f"rs{(port - 7000) // 10}"


def segment_bins(main_log: Path) -> Counter:
    """[Dishonest I/O] Segment hits per bin from the main log (bin of the key's last add)."""
    if not main_log.exists():
        return Counter()
    where, hits = {}, Counter()
    for line in main_log.read_text(errors="replace").splitlines():
        a = re.search(r"\[CACHE\] added \d+ B to (\w+) .*\| (\S+)", line)
        if a:
            where[a[2]] = a[1]
            continue
        h = re.search(r"\[OFFLOAD\] -> cache \| \d+ms \| (\S+)", line)
        if h and h[1] in where:  # single cache (S): adds name no bin -> nothing counted
            hits[where[h[1]]] += 1
    return hits


def asset_counts(site_log: Path) -> dict:
    """[Dishonest I/O] Asset cache file / byte hits and misses of one site (all 0 when off)."""
    c = dict(hit=0, miss=0, hit_b=0, miss_b=0)
    if not site_log.exists():
        return c
    for line in site_log.read_text(errors="replace").splitlines():
        if "[ASSET]" not in line:
            continue
        m = re.search(r"\[ASSET\] (hit|added|add failed \(|skip)\D*(\d+)? B", line)
        if not m:
            continue
        kind = "hit" if m[1] == "hit" else "miss"
        c[kind] += 1
        c[kind + "_b"] += int(m[2] or 0)
    return c


def run_metrics(csv: Path) -> dict:
    """[Dishonest I/O] Every number of one run (see module doc)."""
    base = csv.with_suffix("")
    df = pd.read_csv(csv)
    df = df[df["status"] == 200].copy()
    df["site"] = df["worker"].map(site_of)
    wait = df["wait_ms"].fillna(0)
    df["site_queue"], df["wait"] = (df["queue_ms"] - wait) / 1000, wait / 1000
    df["work"], df["asset_net"] = df["work_ms"] / 1000, (df["asset_ms"] + df["net_ms"]) / 1000
    hit = df["site"] == "cache"
    sent = df[~df["site"].isin(NON_SITE)]
    r = {f"all_{c}": df[c].mean() for c, *_ in PARTS}
    r |= {f"sent_{c}": sent[c].mean() for c, *_ in PARTS}
    r["n"], r["n_hit"] = len(df), int(hit.sum())
    r["seg_req"] = hit.mean()
    r["seg_byte"] = df.loc[hit, "size_bytes"].sum() / df["size_bytes"].sum()
    for b, n in segment_bins(Path(f"{base}_main.log")).items():
        r[f"bin_{b}"] = n
    tot = Counter()
    for s in SITES:
        a = asset_counts(Path(f"{base}_site_{s}.log"))
        tot.update(a)
        r[f"asset_file_{s}"] = a["hit"] / (a["hit"] + a["miss"]) if a["hit"] + a["miss"] else np.nan
    has_asset = tot["hit"] + tot["miss"] > 0
    r["asset_file"] = tot["hit"] / (tot["hit"] + tot["miss"]) if has_asset else np.nan
    r["asset_byte"] = tot["hit_b"] / (tot["hit_b"] + tot["miss_b"]) if has_asset else np.nan
    r["asset_fetches"] = r["asset_miss"] = tot["miss"]
    r["asset_hit"] = tot["hit"]
    for s in SITES + NON_SITE:
        on = df[df["site"] == s]
        r[f"share_{s}"] = len(on) / len(df)
        r[f"queue_{s}"] = on["site_queue"].mean() + on["wait"].mean() if s in SITES else np.nan
        r[f"asset_{s}"] = on["asset_net"].mean() if s in SITES else np.nan
    main_log = Path(f"{base}_main.log")
    if not (main_log.exists() and "Segment cache ON" in main_log.read_text(errors="replace")):
        r["seg_req"] = r["seg_byte"] = np.nan  # off: no bar, not 0 %
    return r


def load(run_dir: Path, policy: str) -> pd.DataFrame:
    """[Dishonest I/O] Per bar ('mct', 'mct+S', ...): mean over its runs of every metric."""
    rows = []
    for csv in sorted(run_dir.glob(f"{policy}_*.csv")):
        m = FEATURE_FILE.match(csv.name)
        if not m or m["policy"] != policy:
            continue
        name = policy if m["features"] == "none" else f"{policy}+{m['features']}"
        rows.append({"bar": name, **run_metrics(csv)})
    if not rows:
        return pd.DataFrame()
    df = pd.DataFrame(rows)
    out = df.groupby("bar").mean(numeric_only=True)
    out.insert(0, "runs", df.groupby("bar").size())
    order = [policy] + [f"{policy}+{f}" for f in FEATURE_STYLE]
    order = [b for b in order if b in out.index] + sorted(set(out.index) - set(order))
    return out.loc[order]


def print_tables(s: pd.DataFrame, title: str):
    pd.set_option("display.width", 200)
    print(f"== {title}")
    cols = [f"{w}_{c}" for w in ("all", "sent") for c, *_ in PARTS]
    t = s[["runs", "n", "n_hit"] + cols].copy()
    t["all_total"] = s[cols[:4]].sum(axis=1)
    t["sent_total"] = s[cols[4:]].sum(axis=1)
    print("mean time per request (s): all = every request (hit = 0), sent = sent to a site\n"
          + t.round(2).to_string())
    h = s[[c for c, _ in HITS] + [c for c in s.columns if c.startswith("bin_")] + ["asset_fetches"]]
    print("hit ratio (segment requests / bytes, asset files / bytes), B hits per bin, asset fetches\n"
          + h.round(3).to_string())
    for k, what in [("share", "share of requests"), ("queue", "mean queue (s)"),
                    ("asset", "mean asset + net (s)"), ("asset_file", "asset file hit ratio")]:
        cs = [f"{k}_{x}" for x in SITES + (NON_SITE if k == "share" else [])]
        print(f"per site: {what}\n" + s[cs].rename(columns=lambda c: c.split("_")[-1]).round(3).to_string())


def stacked(ax, s, prefix, ymax):
    """[Honest] Panels (a) / (b): one stacked bar per row of s, segment values + bold total."""
    x = np.arange(len(s))
    bottom = np.zeros(len(s))
    for col, label, colour, hatch in PARTS:
        h = s[f"{prefix}_{col}"].fillna(0).to_numpy()
        ax.bar(x, h, 0.72, bottom=bottom, color=colour, hatch=hatch, edgecolor=SURF,
               linewidth=1.2, label=label, zorder=2)
        for i, v in enumerate(h):
            if v > ymax * 0.05:
                ax.text(x[i], bottom[i] + v / 2, f"{v:.1f}", ha="center", va="center",
                        fontsize=FS_VALUE, color="#1b1b1a" if col == "wait" else SURF, zorder=3)
        bottom += h
    for i, v in enumerate(bottom):
        ax.text(x[i], v + ymax * 0.01, f"{v:.1f}", ha="center", va="bottom",
                fontsize=FS_VALUE + 1, fontweight="bold", color="#1b1b1a")
    ax.set_ylim(0, ymax)


def finish(ax, title, ylabel=None):
    ax.set_title(title, fontsize=FS_LABEL, color=INK, loc="left")
    if ylabel:
        ax.set_ylabel(ylabel, fontsize=FS_TICK, color=INK)
    ax.set_facecolor(SURF)
    ax.grid(axis="y", color=GRID, zorder=0)
    ax.set_axisbelow(True)
    for side in ("top", "right"):
        ax.spines[side].set_visible(False)
    for side in ("left", "bottom"):
        ax.spines[side].set_color(MUTED)
    ax.tick_params(colors=MUTED, labelsize=FS_TICK)


def bar_colour(bar):
    return ("#104281", False) if "+" not in bar else style(bar)


def plot(s: pd.DataFrame, title: str, out: Path):
    """[Honest] One stacked bar per run type: mean time per request over all requests (hit = 0)."""
    fig, ax = plt.subplots(figsize=(12, 7.5), facecolor=SURF)
    ymax = s[[f"all_{c}" for c, *_ in PARTS]].sum(axis=1).max() * 1.1
    stacked(ax, s, "all", ymax)
    ax.set_xticks(np.arange(len(s)), list(s.index), fontsize=FS_TICK)
    finish(ax, "mean time per request, all requests (segment cache hit = 0 s)", "s per request")
    h, l = ax.get_legend_handles_labels()
    ax.legend(h[::-1], l[::-1], fontsize=FS_TICK - 1, frameon=False, loc="upper right")
    fig.suptitle(f"{s.index[0]} with no feature vs + one feature: {title}", fontsize=FS_TITLE - 2,
                 color=INK)
    fig.text(0.99, 0.01, "mean over runs ("
             + ", ".join(f"{b} {int(n)}" for b, n in s["runs"].items())
             + "); status 200, body left out; S/B = segment cache, Alru/Alfu = asset cache",
             ha="right", fontsize=FS_TICK - 3, color=MUTED)
    fig.subplots_adjust(left=0.08, right=0.98, top=0.88, bottom=0.09)
    out.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(out, dpi=140)
    print(f"plot -> {out}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run_dir", type=Path)
    ap.add_argument("--policy", default="mct")
    ap.add_argument("--out", type=Path)
    args = ap.parse_args()
    s = load(args.run_dir, args.policy)
    if s.empty:
        sys.exit(f"no {args.policy}_<label>_<N>.csv files in {args.run_dir}")
    d = args.run_dir.resolve()
    title = d.name if d.parent.name == "stat" else f"{d.parent.name}_{d.name}"
    print_tables(s, title)
    plot(s, title, args.out or Path(__file__).parent / f"feature_detail_{title}.png")


if __name__ == "__main__":
    main()
