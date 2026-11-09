#!/usr/bin/env python3
"""
What each feature (cache) changes, for one sequence directory: mct with no feature vs mct + one
feature (runs <policy>_<label>_<N>.csv; labels Slru / Slfu = single segment cache lru / lfu,
B6lru / B8lru / B8lfu = 2 cost bins with threshold 6 / 8 s (lru / lfu) at the main server, Alru / Alfu = asset cache at
every site, C = coalescing, P = prefetch).

Usage:
  stat/feature_plot/feature_detail_plot.py <run dir> [--policy mct] [--out PNG]
  e.g. stat/feature_plot/feature_detail_plot.py stat/pre-sitefix/seed2042/run_a1.2_c2

Inputs per run: <base>.csv (one row per request; a segment cache hit has worker 'cache' and
queue / work / asset / net = 0), <base>_main.log ('[CACHE] added ... to lower|upper' and
'[OFFLOAD] -> cache' -> segment hits per bin) and <base>_site_<site>.log ('[ASSET] hit' /
'added' / 'add failed' / 'skip' -> asset file hits and misses per site; a miss = a fetch from the
main server). Status 200 requests only. Every number is the mean over the runs of that bar.

Figure (default stat/feature_plot/feature_detail_<run dir name>.png): the common comparison, one
stacked bar per run type (mct, mct+Slru, mct+B6lru, mct+Alru, mct+Alfu, ...) = mean time per request over all
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
    # coalescing / prefetch follower: no render of its own, it waits for an in-flight render
    # (its whole latency); 0 for every other request. Without it C / P bars hide that wait.
    ("follow", "follower wait (C / P)", "#a8a6a0", ".."),
]
HITS = [("seg_req", "segment\nrequests"), ("seg_byte", "segment\nbytes"),
        ("asset_file", "asset\nfiles"), ("asset_byte", "asset\nbytes")]
INK, MUTED, GRID, SURF = "#3b3a37", "#52514e", "#e4e3df", "#fcfcfb"
FS_TITLE, FS_LABEL, FS_TICK, FS_VALUE = 20, 14, 12, 10
TOP = 5  # lowest totals that get a rank badge (as stat/latency_detail_plot.py)


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
    df["follow"] = np.where(df["site"].isin(["coalesce", "prefetch"]), df["latency_ms"] / 1000, 0.0)
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
    # coalescing / prefetch: followers (no render of their own), prefetch renders issued / read
    r["n_coal"] = int((df["site"] == "coalesce").sum())
    r["n_pref"] = int((df["site"] == "prefetch").sum())
    main_log = Path(f"{base}_main.log")
    log = main_log.read_text(errors="replace") if main_log.exists() else ""
    r["pf_issue"] = log.count("[PREFETCH] issue")
    r["pf_read"] = len(re.findall(r"\[PREFETCH\] expire \| [1-9]\d* readers", log))
    r["renders"] = len(sent) + r["pf_issue"]  # renders the sites did (real + prefetch)
    # prefetch renders have no csv row: their work / asset from '[PREFETCH] done | site | Tms |
    # work W asset A' (queue + net only known combined -> left out); per client request, as all_*
    done = re.findall(r"\[PREFETCH\] done \| \S+ \| \d+ms \| work (\d+) asset (\d+)", log)
    r["pf_done"] = len(done)
    r["pf_work_s"] = sum(int(w) for w, _ in done) / 1000
    r["pf_asset_s"] = sum(int(a) for _, a in done) / 1000
    r["pf_work"], r["pf_asset"] = r["pf_work_s"] / len(df), r["pf_asset_s"] / len(df)
    if not (main_log.exists() and "Segment cache ON" in log):
        r["seg_req"] = r["seg_byte"] = np.nan  # off: no bar, not 0 %
    if "PSC ON" in log:
        r |= psc_metrics(csv, df, log)
    return r


PSC_DECISION = re.compile(r"\[PSC\] (create k=\d+|rebuild k=\d+|rebuild failed|plain \| [^|]+?) \|.*?(/offload/\S+)\s*$")
PSC_ROLES = ["create", "rebuild", "plain", "nocut"]


def seg_kind(path: str) -> str:
    """[Honest] Path without its time window: '/offload/show/f1.json/10/20/1280/...' -> 'f1.json/1280/...'."""
    p = path.split("/")
    return "/".join(p[3:4] + p[6:])


def psc_roles(df: pd.DataFrame, log: str) -> tuple[pd.Series, Counter]:
    """[Honest] Per csv row (status 200 rows of df): its PSC decision from the main log ('create' /
    'rebuild' / 'plain' / 'nocut' = kind without a cut, never logged) + counts of every decision
    incl. 'rebuild failed' and each plain reason. Mapping: the k-th decision for a path = the k-th
    request of that path by send time (checked 11-08 on all 24 sitefix PSC runs: counts match)."""
    from urllib.parse import unquote
    dec, n = defaultdict(list), Counter()
    for line in log.splitlines():
        m = PSC_DECISION.search(line)
        if not m:
            continue
        d = m[1].split(" k=")[0]
        n["plain: " + d.split("| ", 1)[1].strip() if d.startswith("plain") else d] += 1
        dec[unquote(m[2])].append("plain" if d.startswith("plain") else d)
    role = pd.Series("nocut", index=df.index)
    for path, rows in df.sort_values("send_epoch_ms").groupby("path", sort=False):
        ds = [d for d in dec.get(path, []) if d != "rebuild failed"]  # failed = 502, not in df
        for i, d in zip(rows.index, ds):
            role[i] = d
    return role, n


def psc_metrics(csv: Path, df: pd.DataFrame, log: str) -> dict:
    """[Dishonest I/O] Partial segment cache numbers of one run: decisions (create / stored /
    rebuild / rebuild failed / plain reasons), hit = rebuild / eligible (= logged decisions; kinds
    without a cut never reach PSC), and the MEASURED site service (work + asset + net, s) of
    rebuild and create rows vs the mean of the same kind (path without time window) in this
    directory's <policy>_none_*.csv runs -> saved per rebuild, extra per pair create, net site s."""
    role, n = psc_roles(df, log)
    r = {"psc_" + k.replace(" ", "_").replace(":", ""): v for k, v in n.items()}
    r["psc_stored"] = log.count("[PSC] stored ")
    r["psc_elig"] = sum(v for k, v in n.items())
    r["psc_hit"] = n["rebuild"] / r["psc_elig"] if r["psc_elig"] else np.nan
    svc = (df["work_ms"] + df["asset_ms"] + df["net_ms"]) / 1000
    policy = csv.name.split("_", 1)[0]
    base = [pd.read_csv(f) for f in sorted(csv.parent.glob(f"{policy}_none_*.csv"))]
    base = pd.concat(base) if base else pd.DataFrame(columns=df.columns)
    base = base[base["status"] == 200]
    full = ((base["work_ms"] + base["asset_ms"] + base["net_ms"]) / 1000).groupby(base["path"].map(seg_kind)).mean()
    kind = df["path"].map(seg_kind)
    for k in ("rebuild", "create"):
        on = (role == k) & kind.isin(full.index)
        r[f"psc_{k}_svc"] = svc[on].mean() if on.any() else np.nan  # measured, this run
        r[f"psc_{k}_full"] = kind[on].map(full).mean() if on.any() else np.nan  # same kinds in none
        r[f"psc_{k}_matched"] = int(on.sum())
        r[f"psc_{k}_delta_s"] = (svc[on] - kind[on].map(full)).sum()  # site s vs none, all rows
    r["psc_net_s"] = r["psc_rebuild_delta_s"] + r["psc_create_delta_s"]  # < 0 = PSC saved site time
    model = [float(x) for x in re.findall(r"\[PSC\] rebuild k=\d+ \| full \S+ s rebuild \S+ s save (\S+) s", log)]
    r["psc_model_save"] = np.mean(model) if model else np.nan
    used = re.findall(r"\[PSC\] stored \d+ B \(used (\d+) B\)", log)
    r["psc_used_mb"] = max(map(int, used)) / 2**20 if used else 0.0
    for k in PSC_ROLES:
        r[f"psc_share_{k}"] = (role == k).mean()
    return r


def load(run_dir: Path, policy: str) -> pd.DataFrame:
    """[Dishonest I/O] Per bar ('mct', 'mct+Slru', ...): mean over its runs of every metric."""
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
    df["all_total"] = df[[f"all_{c}" for c, *_ in PARTS]].sum(axis=1)
    out = df.groupby("bar").mean(numeric_only=True)
    out.insert(0, "runs", df.groupby("bar").size())
    out["sd"] = df.groupby("bar")["all_total"].std()  # NaN with 1 run -> no error bar
    order = [policy] + [f"{policy}+{f}" for f in FEATURE_STYLE]
    order = [b for b in order if b in out.index] + sorted(set(out.index) - set(order))
    return out.loc[order]


def print_tables(s: pd.DataFrame, title: str):
    pd.set_option("display.width", 200)
    print(f"== {title}")
    cols = [f"{w}_{c}" for w in ("all", "sent") for c, *_ in PARTS]
    t = s[["runs", "n", "n_hit"] + cols].copy()
    k = len(PARTS)
    t["all_total"] = s[cols[:k]].sum(axis=1)
    t["sent_total"] = s[cols[k:]].sum(axis=1)
    t["all_sd"] = s["sd"]  # std over runs of all_total (error bar), NaN = 1 run
    # rank of all_total, 1 = lowest (badges 1..TOP on the plot)
    t["rank"] = t["all_total"].rank(method="first").astype(int)
    print("mean time per request (s): all = every request (segment hit = 0, C/P follower = its "
          "wait), sent = sent to a site\n"
          + t.round(2).to_string())
    h = s[[c for c, _ in HITS] + [c for c in s.columns if c.startswith("bin_")] + ["asset_fetches"]]
    print("hit ratio (segment requests / bytes, asset files / bytes), B hits per bin, asset fetches\n"
          + h.round(3).to_string())
    for k, what in [("share", "share of requests"), ("queue", "mean queue (s)"),
                    ("asset", "mean asset + net (s)"), ("asset_file", "asset file hit ratio")]:
        cs = [f"{k}_{x}" for x in SITES + (NON_SITE if k == "share" else [])]
        print(f"per site: {what}\n" + s[cs].rename(columns=lambda c: c.split("_")[-1]).round(3).to_string())


def stacked(ax, s, prefix, ymax, sd=None):
    """[Honest] Panels (a) / (b): one stacked bar per row of s, segment values + bold total;
    sd (per bar, NaN = 1 run) -> error bar = std over runs, total written above it."""
    x = np.arange(len(s))
    bottom = np.zeros(len(s))
    for col, label, colour, hatch in PARTS:
        h = s[f"{prefix}_{col}"].fillna(0).to_numpy()
        if not h.any():  # part absent in every bar (e.g. follower wait without C / P): no legend
            continue
        ax.bar(x, h, 0.72, bottom=bottom, color=colour, hatch=hatch, edgecolor=SURF,
               linewidth=1.2, label=label, zorder=2)
        for i, v in enumerate(h):
            if v > ymax * 0.05:
                ax.text(x[i], bottom[i] + v / 2, f"{v:.1f}", ha="center", va="center",
                        fontsize=FS_VALUE, color="#1b1b1a" if col in ("wait", "follow") else SURF, zorder=3)
        bottom += h
    err = np.zeros(len(s)) if sd is None else np.nan_to_num(np.asarray(sd, dtype=float))
    for i, v in enumerate(bottom):
        if err[i] > 0:
            ax.errorbar(x[i], v, yerr=err[i], color=INK, capsize=4, elinewidth=1.2, zorder=3)
        ax.text(x[i], v + err[i] + ymax * 0.01, f"{v:.1f}", ha="center", va="bottom",
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


def short(bar: str) -> str:
    """[Honest] Tick / legend label: the policy is in the title -> 'mct' -> 'none', 'mct+B8lru' -> 'B8lru'."""
    return "none" if "+" not in bar else bar.split("+", 1)[1]


def rank_badges(ax, s, ymax):
    """[Honest] Rank badges 1..TOP above the lowest stacked totals (+ error bar), same badge as
    stat/latency_detail_plot.py (white number in a dark circle)."""
    total = s[[f"all_{c}" for c, *_ in PARTS]].fillna(0).sum(axis=1).to_numpy()
    err = np.nan_to_num(s["sd"].to_numpy(dtype=float))
    for rank, i in enumerate(np.argsort(total, kind="stable")[:TOP], start=1):
        ax.text(i, total[i] + err[i] + ymax * 0.065, str(rank), ha="center", va="bottom",
                fontsize=FS_VALUE, fontweight="bold", color=SURF, zorder=4,
                bbox={"boxstyle": "circle,pad=0.25", "fc": "#1b1b1a", "ec": "none"})


def plot(s: pd.DataFrame, title: str, out: Path):
    """[Honest] One stacked bar per run type: mean time per request over all requests (hit = 0)."""
    fig, ax = plt.subplots(figsize=(max(12, 1.25 * len(s) + 2), 7.5), facecolor=SURF)  # 15+ bars: widen (11-09)
    ymax = s[[f"all_{c}" for c, *_ in PARTS]].sum(axis=1).max() * 1.22  # headroom: legend row
    stacked(ax, s, "all", ymax, s["sd"])
    ticks = [short(b) for b in s.index]
    ax.set_xticks(np.arange(len(s)), ticks, fontsize=FS_TICK,
                  rotation=25 if len(s) > 10 else 0, ha="right" if len(s) > 10 else "center")
    rank_badges(ax, s, ymax)
    finish(ax, "mean time per request, all requests (segment cache hit = 0 s, C / P follower = "
           "its wait)", "s per request")
    h, l = ax.get_legend_handles_labels()
    ax.legend(h, l, fontsize=FS_TICK - 1, frameon=False, loc="upper center", ncol=len(h))
    fig.suptitle(f"{s.index[0]} with no feature vs + one feature: {title}", fontsize=FS_TITLE - 2,
                 color=INK)
    fig.text(0.01, 0.01, "mean over runs ("
             + ", ".join(f"{t} {int(n)}" for t, n in zip(ticks, s["runs"]))
             + "), whisker = std over runs (2+ runs only); status 200, body left out\nSlru/Slfu/B6lru/B8lru/B8lfu = segment cache, Alru/Alfu = asset "
             "cache, C = coalescing, P = prefetch,\nPSC<heavy><small|save> = partial segment cache; "
             "pairs SlruP, SlfuAlfu, CP = both features on",
             ha="left", fontsize=FS_TICK - 3, color=MUTED)
    fig.subplots_adjust(left=0.08 * 12 / fig.get_figwidth(), right=0.98, top=0.88,
                        bottom=0.19 if len(s) > 10 else 0.11)
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
