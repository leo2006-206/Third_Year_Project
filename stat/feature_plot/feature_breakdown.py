"""Shared code of segment_plot.py / asset_plot.py / coalesce_plot.py: mct with no feature vs mct + the feature's labels.

Left panel: each latency part as % of the no-feature run (no feature = 100 %): queue (site queue +
central wait), work, asset + net, and the total (without body). Below 100 % = the feature removed
that much, above = it added. Right panel: mean time per request stacked (site queue, central wait,
work, asset + net), as stat/latency_detail_plot.py. Bottom line: per feature, cache hits / misses
and hit ratio (segment: requests, B also hits per bin; asset: file fetches, all sites summed;
coalesce: followers = requests served by an in-flight render, prefetches issued / read, site renders). All requests (status 200) count; a segment
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

from feature_detail_plot import (PARTS, bar_colour, finish, load, rank_badges, short, stacked,
                                 INK, MUTED, SURF)

FS_TITLE, FS_LABEL, FS_TICK, FS_VALUE = 18, 14, 12, 11
# left panel groups: (label, part columns summed)
GROUPS = [
    ("queue\n(site queue + central wait)", ["site_queue", "wait"]),
    ("work", ["work"]),
    ("asset + net", ["asset_net"]),
    ("total", [c for c, *_ in PARTS]),
]
# coalescing / prefetch: a follower's whole latency is 'follower wait' -> its own group
GROUPS_COAL = GROUPS[:3] + [("follower wait (C / P)\n% of none's total", ["follow"])] + GROUPS[3:]


# coalesce: site cost of the prefetch renders (no csv row, from the main log) stacked on top of
# the client part of these groups, in its own colour
EXTRA = {"work": "pf_work", "asset + net": "pf_asset"}
PF_COLOUR = "#5c1a3c"  # prefetch render part (log), dark rose: not a feature colour


def groups(kind):
    return GROUPS_COAL if kind == "coalesce" else GROUPS


def relative(s, base, kind):
    """[Honest] Per bar and group: sum of the group's parts / the no-feature bar's (1.0 = same).
    A group that is 0 in the no-feature bar (follower wait) is / the no-feature TOTAL instead."""
    out, total = {}, [c for c, *_ in PARTS]
    for bar in s.index:
        out[bar] = []
        for _, cols in groups(kind):
            b = s.loc[base, [f"all_{c}" for c in cols]].sum()
            b = b if b > 0 else s.loc[base, [f"all_{c}" for c in total]].sum()
            out[bar].append(s.loc[bar, [f"all_{c}" for c in cols]].sum() / b)
    return out


def extra(s, base, kind):
    """[Honest] Per bar and group: prefetch render work / asset (from the main log) / the
    no-feature bar's group (0 where there is none)."""
    out = {}
    for bar in s.index:
        out[bar] = []
        for g, cols in groups(kind):
            col = EXTRA.get(g) if kind == "coalesce" else None
            if not (col and col in s.columns):
                out[bar].append(0.0)  # group without a prefetch part (queue, follower, total)
                continue
            out[bar].append(np.nan_to_num(s.loc[bar, col])
                            / s.loc[base, [f"all_{c}" for c in cols]].sum())
    return out


def hit_entries(s, kind):
    """[Honest] Per feature bar: (bar, 'bar: (hit H / miss M req) = X% hit (Y% of bytes)').
    hit ratio = hit / (hit + miss); kind 'segment': requests (B: + hits per bin), 'asset': files,
    all sites summed."""
    out = []
    for bar in s.index[1:]:
        r = s.loc[bar]
        if kind == "coalesce":
            f = r["n_coal"] + r["n_pref"]
            txt = (f"{short(bar)}: followers {f:.0f} / {r['n']:.0f} req = {100 * f / r['n']:.1f}% no "
                   f"render of their own; site renders {r['renders']:.0f} (none "
                   f"{s.loc[s.index[0], 'renders']:.0f})")
            if r["pf_issue"] > 0:
                txt += (f"; prefetch issued {r['pf_issue']:.0f}, read {r['pf_read']:.0f} "
                        f"({100 * r['pf_read'] / r['pf_issue']:.0f}%)")
            out.append((bar, txt))
            continue
        if kind == "segment":
            hit, miss, unit, ratio, byte = r["n_hit"], r["n"] - r["n_hit"], "req", r["seg_req"], r["seg_byte"]
        else:
            hit, miss, unit, ratio, byte = (r["asset_hit"], r["asset_miss"], "files",
                                            r["asset_file"], r["asset_byte"])
        txt = f"{short(bar)}: (hit {hit:.0f} / miss {miss:.0f} {unit}) = {100 * ratio:.0f}% hit ({100 * byte:.0f}% of bytes)"
        bins = [f"{c[4:]} {r[c]:.0f}" for c in s.columns if c.startswith("bin_") and not np.isnan(r[c])]
        if bins:
            txt += " [" + ", ".join(bins) + "]"
        out.append((bar, txt))
    return out


HIT_COLS = 2  # hit entries per row (4 segment caches = 2 rows; 11-07: one row clipped B6 / B8)


HIT_DY = 0.04  # row height of the hit entries, figure height fraction


def hit_cols(kind):
    return 1 if kind == "coalesce" else HIT_COLS  # coalesce entries are long: one per row


def hit_rows(s, kind):
    return -(-(len(s) - 1) // hit_cols(kind))


def draw_hit_line(fig, s, kind, x0=0.05, y_last=None):
    """[Honest] The hit entries at the figure bottom, HIT_COLS per row (last row at y_last, rows
    stacked upwards so they never reach the tick labels), each after a square of its bar colour."""
    y_last = y_last or (0.11 if kind == "coalesce" else 0.075)  # footer 4 / 2 lines
    sq = 0.009  # square side, figure width fraction
    h = sq * fig.bbox.width / fig.bbox.height
    rows, cols = hit_rows(s, kind), hit_cols(kind)
    for k, (bar, txt) in enumerate(hit_entries(s, kind)):
        x, y = x0 + (k % cols) * 0.47, y_last + (rows - 1 - k // cols) * HIT_DY
        colour, _ = bar_colour(bar)
        fig.patches.append(matplotlib.patches.Rectangle((x, y - h / 2), sq, h, color=colour,
                                                       transform=fig.transFigure, figure=fig))
        fig.text(x + sq * 1.6, y, txt, ha="left", va="center", fontsize=FS_TICK + 1,
                 color=INK, fontweight="bold")


def plot(s, title, out, feature_name, kind, names):
    base = s.index[0]
    rel, ext = relative(s, base, kind), extra(s, base, kind)
    G = groups(kind)
    tick = lambda b: names.get(short(b), short(b))
    fig, (left, right) = plt.subplots(1, 2, figsize=(20, 7.5), facecolor=SURF,
                                      gridspec_kw={"width_ratios": [1.35, 1]})
    # left: % of no feature
    x = np.arange(len(G))
    w = 0.8 / len(s)
    top = max(max(np.add(rel[b], ext[b])) for b in s.index)
    pf_label = True
    for j, bar in enumerate(s.index):
        colour, _ = bar_colour(bar)
        xs = x - 0.4 + w * (j + 0.5)
        v, e = np.array(rel[bar]), np.array(ext[bar])
        left.bar(xs, v, w * 0.9, color=colour, label=tick(bar).replace("\n", " "), edgecolor=SURF, linewidth=0.6, zorder=2)
        if e.any():
            left.bar(xs, e, w * 0.9, bottom=v, color=PF_COLOUR, hatch="//", edgecolor=SURF,
                     linewidth=0.6, zorder=2,
                     label="prefetch renders (main log)" if pf_label else None)
            pf_label = False
            for xi, vi, ei in zip(xs, v, e):
                if ei > top * 0.06:
                    left.text(xi, vi + ei / 2, f"+{100 * ei:.0f}%", ha="center", va="center",
                              fontsize=FS_VALUE - 1, color=SURF, fontweight="bold", zorder=3)
        v = v + e
        for k, (xi, vi) in enumerate(zip(xs, v)):
            if G[k][1] == ["follow"]:  # / none's total: no '(-M%)', it is 0 without C / P
                txt = f"{100 * vi:.0f}%"
            else:
                txt = (f"{100 * vi:.0f}%" if bar == base
                       else f"{100 * vi:.0f}%\n({100 * (vi - 1):+.0f}%)")
            left.text(xi, vi + top * 0.01, txt, ha="center", va="bottom", fontsize=FS_VALUE - 1,
                      color=INK)
    left.axhline(1.0, color=INK, linewidth=1, linestyle="--", zorder=3)
    left.set_xticks(x, [g for g, _ in G], fontsize=FS_TICK)
    left.set_ylim(0, max(1.0, top) * 1.22)
    left.yaxis.set_major_formatter(matplotlib.ticker.PercentFormatter(1.0))
    finish(left, f"Each latency part vs {base} with no feature (= 100 %; dashed line)",
           "% of no feature")
    h, l = left.get_legend_handles_labels()
    left.legend(h, l, fontsize=FS_TICK, frameon=False, loc="upper center", ncol=len(h))
    # right: stacked seconds
    ymax = s[[f"all_{c}" for c, *_ in PARTS]].sum(axis=1).max() * 1.25  # headroom: legend row
    stacked(right, s, "all", ymax, s["sd"])
    right.set_xticks(np.arange(len(s)), [tick(b) for b in s.index], fontsize=FS_TICK)
    rank_badges(right, s, ymax)
    finish(right, "Mean time per request (s)", "s per request")
    h, l = right.get_legend_handles_labels()
    right.legend(h, l, fontsize=FS_TICK - 1, frameon=False, loc="upper center", ncol=len(h))
    fig.suptitle(f"{base} + {feature_name.lower()} vs {base} alone: {title}", fontsize=FS_TITLE,
                 color=INK)
    draw_hit_line(fig, s, kind)
    unit = {"segment": "hit ratio = hit / (hit + miss) over requests (a segment cache hit = 0 s)",
            "asset": "hit ratio = hit / (hit + miss) over asset files, all sites summed",
            "coalesce": "follower = request answered by an identical in-flight render (C) or a "
                        "prefetch (P), its latency = follower wait; prefetch read = 1+ follower\n"
                        "left work / asset: + prefetch renders' site work / asset per client request "
                        "(main log '[PREFETCH] done'; their queue + net not split, left out)\n"
                        "right = client latency only (a prefetch's render time is inside its "
                        "followers' wait)"}[kind]
    fig.text(0.01, 0.01, f"{unit}\nstatus 200 requests, body "
             "left out; mean over runs (" + ", ".join(f"{short(b)} {int(n)}" for b, n in s["runs"].items())
             + "), whisker = std over runs (2+ runs only)", ha="left", fontsize=FS_TICK - 2, color=MUTED)
    fig.subplots_adjust(left=0.05, right=0.99, top=0.88,
                        bottom=0.12 + (0.05 if kind == "coalesce" else 0.02) + HIT_DY * hit_rows(s, kind),
                        wspace=0.14)
    out.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(out, dpi=140)
    print(f"plot -> {out}")


def print_table(s, title, kind):
    base = s.index[0]
    rel = relative(s, base, kind)
    G = groups(kind)
    total = s[[f"all_{c}" for c, *_ in PARTS]].sum(axis=1)
    print(f"== {title}: s per request, and % of {base} with no feature ('none')")
    print("\n".join(t for _, t in hit_entries(s, kind)))
    names = [g.split("\n")[0].split(" (")[0].replace(" ", "") for g, _ in G]
    rank = total.rank(method="first").astype(int)
    print(f"{'bar':8}" + "".join(f"{n:>13}" for n in names) + f"{'sd':>8}{'rank':>6}"
          + "".join(f"{n + ' %':>15}" for n in names))
    for bar in s.index:
        sec = [s.loc[bar, [f"all_{c}" for c in cols]].sum() for _, cols in G]
        print(f"{short(bar):8}" + "".join(f"{v:>13.2f}" for v in sec) + f"{s.loc[bar, 'sd']:>8.2f}"
              + f"{rank[bar]:>6}" + "".join(f"{100 * v:>14.0f}%" for v in rel[bar]))
    if kind == "coalesce":
        ext = extra(s, base, kind)
        print("prefetch renders (main log '[PREFETCH] done'; not in the csv): s per client request "
              "and % of none's group; site total s")
        for bar in s.index:
            if s.loc[bar, "pf_done"] > 0:
                r = s.loc[bar]
                wi, ai = [g for g, _ in G].index("work"), [g for g, _ in G].index("asset + net")
                print(f"{short(bar):8} done {r['pf_done']:.0f}: work {r['pf_work']:.2f} s/req "
                      f"(+{100 * ext[bar][wi]:.0f}%, client+prefetch {100 * (rel[bar][wi] + ext[bar][wi]):.0f}%"
                      f" of none) asset {r['pf_asset']:.2f} s/req (+{100 * ext[bar][ai]:.0f}%, "
                      f"{100 * (rel[bar][ai] + ext[bar][ai]):.0f}%); site work total "
                      f"{r['all_work'] * r['n'] + r['pf_work_s']:.0f} s (none "
                      f"{s.loc[base, 'all_work'] * s.loc[base, 'n']:.0f} s)")
    print("sd = std over runs of the total (nan = 1 run); rank 1 = lowest total (badges on the plot)"
          + ("; followerwait % = of none's total" if kind == "coalesce" else ""))


def main(labels, feature_name, prefix, doc, names=None):
    """prefix = 'segment' | 'asset' | 'coalesce' (also the hit-line kind); names = tick label per
    feature label (default the label)."""
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
    print_table(s, title, prefix)
    plot(s, title, args.out or Path(__file__).parent / f"{prefix}_{title}.png", feature_name, prefix,
         names or {})
