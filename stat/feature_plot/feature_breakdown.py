"""Shared code of segment_plot.py / asset_plot.py: mct with no feature vs mct + the feature's labels.

Left panel: each latency part as % of the no-feature run (no feature = 100 %): queue (site queue +
central wait), work, asset + net, and the total (without body). Below 100 % = the feature removed
that much, above = it added. Right panel: mean time per request stacked (site queue, central wait,
work, asset + net), as stat/latency_detail_plot.py. Bottom line: per feature, cache hits / misses
and hit ratio (segment: requests, B also hits per bin; asset: file fetches, all sites summed). All requests (status 200) count; a segment
cache hit counts 0 s in every part. Numbers = mean over the runs of each bar (feature_detail_plot).
"""

import argparse
import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.patches
import matplotlib.pyplot as plt
import numpy as np

from feature_detail_plot import PARTS, bar_colour, finish, load, stacked, INK, MUTED, SURF

FS_TITLE, FS_LABEL, FS_TICK, FS_VALUE = 18, 14, 12, 11
# left panel groups: (label, part columns summed)
GROUPS = [
    ("queue\n(site queue + central wait)", ["site_queue", "wait"]),
    ("work", ["work"]),
    ("asset + net", ["asset_net"]),
    ("total", [c for c, *_ in PARTS]),
]


def relative(s, base):
    """[Honest] Per bar and group: sum of the group's parts / the no-feature bar's (1.0 = same)."""
    out = {}
    for bar in s.index:
        out[bar] = [s.loc[bar, [f"all_{c}" for c in cols]].sum()
                    / s.loc[base, [f"all_{c}" for c in cols]].sum() for _, cols in GROUPS]
    return out


def hit_entries(s, kind):
    """[Honest] Per feature bar: (bar, 'bar: (hit H / miss M req) = X% hit (Y% of bytes)').
    hit ratio = hit / (hit + miss); kind 'segment': requests (B: + hits per bin), 'asset': files,
    all sites summed."""
    out = []
    for bar in s.index[1:]:
        r = s.loc[bar]
        if kind == "segment":
            hit, miss, unit, ratio, byte = r["n_hit"], r["n"] - r["n_hit"], "req", r["seg_req"], r["seg_byte"]
        else:
            hit, miss, unit, ratio, byte = (r["asset_hit"], r["asset_miss"], "files",
                                            r["asset_file"], r["asset_byte"])
        txt = f"{bar}: (hit {hit:.0f} / miss {miss:.0f} {unit}) = {100 * ratio:.0f}% hit ({100 * byte:.0f}% of bytes)"
        bins = [f"{c[4:]} {r[c]:.0f}" for c in s.columns if c.startswith("bin_") and not np.isnan(r[c])]
        if bins:
            txt += " [" + ", ".join(bins) + "]"
        out.append((bar, txt))
    return out


def draw_hit_line(fig, s, kind, x=0.05, y=0.045):
    """[Honest] The hit entries in one row at the figure bottom, each after a square of its bar colour."""
    fig.canvas.draw()  # renderer for text widths
    renderer = fig.canvas.get_renderer()
    fw = fig.bbox.width
    sq = 0.009  # square side, figure width fraction
    for bar, txt in hit_entries(s, kind):
        colour, _ = bar_colour(bar)
        h = sq * fw / fig.bbox.height
        fig.patches.append(matplotlib.patches.Rectangle((x, y - h / 2), sq, h, color=colour,
                                                       transform=fig.transFigure, figure=fig))
        t = fig.text(x + sq * 1.6, y, txt, ha="left", va="center", fontsize=FS_TICK + 1,
                     color=INK, fontweight="bold")
        x += sq * 1.6 + t.get_window_extent(renderer).width / fw + 0.03


def plot(s, title, out, feature_name, kind):
    base = s.index[0]
    rel = relative(s, base)
    fig, (left, right) = plt.subplots(1, 2, figsize=(20, 7.5), facecolor=SURF,
                                      gridspec_kw={"width_ratios": [1.35, 1]})
    # left: % of no feature
    x = np.arange(len(GROUPS))
    w = 0.8 / len(s)
    top = max(max(v) for v in rel.values())
    for j, bar in enumerate(s.index):
        colour, _ = bar_colour(bar)
        xs = x - 0.4 + w * (j + 0.5)
        v = np.array(rel[bar])
        left.bar(xs, v, w * 0.9, color=colour, label=bar, edgecolor=SURF, linewidth=0.6, zorder=2)
        for xi, vi in zip(xs, v):
            txt = f"{100 * vi:.0f}%" if bar == base else f"{100 * vi:.0f}%\n({100 * (vi - 1):+.0f}%)"
            left.text(xi, vi + top * 0.01, txt, ha="center", va="bottom", fontsize=FS_VALUE - 1,
                      color=INK)
    left.axhline(1.0, color=INK, linewidth=1, linestyle="--", zorder=3)
    left.set_xticks(x, [g for g, _ in GROUPS], fontsize=FS_TICK)
    left.set_ylim(0, max(1.0, top) * 1.22)
    left.yaxis.set_major_formatter(matplotlib.ticker.PercentFormatter(1.0))
    finish(left, f"Each latency part vs {base} with no feature (= 100 %; dashed line)",
           "% of no feature")
    left.legend(fontsize=FS_TICK, frameon=False, loc="upper right", ncol=len(s))
    # right: stacked seconds
    ymax = s[[f"all_{c}" for c, *_ in PARTS]].sum(axis=1).max() * 1.1
    stacked(right, s, "all", ymax)
    right.set_xticks(np.arange(len(s)), list(s.index), fontsize=FS_TICK)
    finish(right, "Mean time per request (s)", "s per request")
    h, l = right.get_legend_handles_labels()
    right.legend(h[::-1], l[::-1], fontsize=FS_TICK - 1, frameon=False, loc="upper right")
    fig.suptitle(f"{feature_name}: latency parts with vs without, {title}", fontsize=FS_TITLE,
                 color=INK)
    draw_hit_line(fig, s, kind)
    fig.text(0.99, 0.01, "hit ratio = hit / (hit + miss), all sites summed for the asset cache; all status 200 requests (a segment cache hit = 0 s), body left out; "
             "mean over runs (" + ", ".join(f"{b} {int(n)}" for b, n in s["runs"].items()) + ")",
             ha="right", fontsize=FS_TICK - 2, color=MUTED)
    fig.subplots_adjust(left=0.05, right=0.99, top=0.88, bottom=0.17, wspace=0.14)
    out.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(out, dpi=140)
    print(f"plot -> {out}")


def print_table(s, title):
    base = s.index[0]
    rel = relative(s, base)
    print(f"== {title}: s per request, and % of {base}")
    kind = "segment" if any(c in s.index[-1] for c in ("+S", "+B")) else "asset"
    print("\n".join(t for _, t in hit_entries(s, kind)))
    names = ["queue", "work", "asset+net", "total"]
    print(f"{'bar':12}" + "".join(f"{n:>11}" for n in names) + "".join(f"{n + ' %':>13}" for n in names))
    for bar in s.index:
        sec = [s.loc[bar, [f"all_{c}" for c in cols]].sum() for _, cols in GROUPS]
        print(f"{bar:12}" + "".join(f"{v:>11.2f}" for v in sec)
              + "".join(f"{100 * v:>12.0f}%" for v in rel[bar]))


def main(labels, feature_name, prefix, doc):  # prefix = 'segment' | 'asset' (also the hit-line kind)
    ap = argparse.ArgumentParser(description=doc, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run_dir", type=Path)
    ap.add_argument("--policy", default="mct")
    ap.add_argument("--out", type=Path)
    args = ap.parse_args()
    s = load(args.run_dir, args.policy)
    keep = [args.policy] + [f"{args.policy}+{l}" for l in labels]
    s = s.loc[[b for b in keep if b in s.index]]
    if args.policy not in s.index or len(s) < 2:
        sys.exit(f"need {args.policy}_none_N and one of {labels} runs in {args.run_dir}")
    d = args.run_dir.resolve()
    title = d.name if d.parent.name == "stat" else f"{d.parent.name}_{d.name}"
    print_table(s, title)
    plot(s, title, args.out or Path(__file__).parent / f"{prefix}_{title}.png", feature_name, prefix)
