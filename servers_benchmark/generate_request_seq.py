#!/usr/bin/env python3
"""
OBM Request Sequence Generator (viewer model)

Builds one set of viewer paths per Zipf exponent alpha, then places the viewers in time
for each offload cluster so the offered load is a fixed fraction (rho) of that cluster's
measured capacity. Every cluster sees the same viewers watching the same things; only how
densely they arrive differs.

Pipeline:
  1. Popularity: video first (Zipf over shows, fixed ranking), then layer combination
     (Zipf over that show's request kinds, seeded ranking); both Zipf with exponential cutoff.
  2. Viewer paths (no times yet): a viewer starts on a show + combination at segment 0.
     After each segment it leaves, switches video (new show + combination, segment 0), or
     continues to the next segment; when continuing it may change combination (redrawn from
     the same show's Zipf). Abandonment is higher after the first segment of a video.
  3. Capacity: per-kind busy time (asset + work) per site from stat/*.csv,
     C = sum_j 1 / E_mix[S_j] over the requests inside the window.
  4. Rates: target requests in the window N = rho * C * DURATION_S; lambda_viewers is
     fitted (bisection) so the window holds exactly N requests.
  5. Timing: viewer v starts at T_v = -PRE_ROLL + sum(Exp(1)) / lambda_viewers and requests
     its k-th segment at T_v + 10 s * k (own playback clock). Requests in [0, DURATION_S)
     are kept; the pre-roll makes the window start at steady load.
  Steps 3-5 repeat until the window's mix stops changing (a few iterations).

Separate seeded random streams: viewer arrival gaps, combination ranking, content choices
(show/combination), and viewer decisions (leave/switch/continue/change). Changing alpha only
changes which shows/combinations are drawn.

Outputs (servers_benchmark/sequences/):
  seq_a<alpha>_viewers.csv    every viewer's path (user_id, step, kind, segment), no times
  seq_a<alpha>_<cluster>.csv  replay timeline for a cluster (t_ms, truncated to the window)
  meta.csv                    capacity, rates and request counts per (alpha, cluster)
"""

import csv
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
import pandas as pd

SEG_LEN_S = 10
SEGMENT_PLACEHOLDER = "/t_start/t_end/"


@dataclass
class ViewerParams:
    # Decision after the FIRST segment of a video (high abandonment, "video surfing")
    p_first_continue: float = 0.40
    p_first_switch: float = 0.45  # remainder (0.15) leaves
    # Decision after any later segment
    p_continue: float = 0.80
    p_switch: float = 0.10  # remainder (0.10) leaves
    # When continuing: chance to change layer combination (same show, redrawn from Zipf)
    p_combo: float = 0.10
    max_segments: int = 30  # cap on one viewer's session (5 min of video)
    segment_gap_s: float = 10.0  # one request per segment, at playback pace


@dataclass
class Request:
    t_s: float
    uid: int
    step: int
    kind: int
    seg: int


@dataclass
class Cluster:
    name: str
    sites: list[tuple[str, str]]  # (machine, tier)
    note: str = ""
    caps: dict = field(default_factory=dict)


# ==============================================================================
# 1. Honest Functions (Pure Domain Logic)
# ==============================================================================


def zipf_cutoff_probs(n: int, alpha: float, cutoff: float) -> np.ndarray:
    """[Honest] Zipf with exponential cutoff over ranks 1..n: p(r) ~ r^-alpha * exp(-r/cutoff)."""
    r = np.arange(1, n + 1)
    p = r**-alpha * np.exp(-r / cutoff)
    return p / p.sum()


def two_level_popularity(
    kind_shows: list[str],
    alpha: float,
    cutoff: float,
    rank_rng,
    show_rank: list[str] | None = None,
) -> tuple[np.ndarray, list[str]]:
    """[Honest] Video first, then combination: p(kind) = p(show) * p(kind | show).
    Both levels are Zipf with exponential cutoff. Shows follow `show_rank` (most popular
    first) or a seeded permutation if None; combinations within a show use a seeded permutation.
    Returns per-kind probabilities and the show popularity ranking used."""
    shows = sorted(set(kind_shows))
    random_rank = [shows[i] for i in rank_rng.permutation(len(shows))]
    if show_rank is None:
        show_rank = random_rank
    elif sorted(show_rank) != shows:
        raise ValueError(f"show_rank must list each show once: {shows}")
    show_p = dict(zip(show_rank, zipf_cutoff_probs(len(shows), alpha, cutoff)))

    probs = np.zeros(len(kind_shows))
    for show in shows:
        idx = np.array([i for i, s in enumerate(kind_shows) if s == show])
        probs[idx[rank_rng.permutation(len(idx))]] = show_p[show] * zipf_cutoff_probs(
            len(idx), alpha, cutoff
        )
    return probs, show_rank


def show_tables(kind_shows: list[str], probs: np.ndarray) -> dict[str, tuple[np.ndarray, np.ndarray]]:
    """[Honest] Per show: (kind indices, P(combination | show)) from the per-kind probabilities."""
    tables = {}
    for show in sorted(set(kind_shows)):
        idx = np.array([i for i, s in enumerate(kind_shows) if s == show])
        tables[show] = (idx, probs[idx] / probs[idx].sum())
    return tables


def viewer_path(
    shows: list[str],
    show_p: np.ndarray,
    combos: dict[str, tuple[np.ndarray, np.ndarray]],
    seg_counts: dict[str, int],
    params: ViewerParams,
    content_rng,
    decision_rng,
) -> list[tuple[int, int]]:
    """[Honest] One viewer's sequence of (kind, segment), following the decision tree:
    leave / switch video / continue (+ maybe change combination)."""

    def pick_show() -> str:
        return shows[content_rng.choice(len(shows), p=show_p)]

    def pick_combo(show: str, exclude: int | None = None) -> int:
        idx, p = combos[show]
        if exclude is not None and len(idx) > 1:
            keep = idx != exclude
            idx, p = idx[keep], p[keep] / p[keep].sum()
        return int(content_rng.choice(idx, p=p))

    show = pick_show()
    kind, seg, first = pick_combo(show), 0, True
    path = []
    while True:
        path.append((kind, seg))
        if len(path) >= params.max_segments:
            return path
        p_cont, p_sw = (
            (params.p_first_continue, params.p_first_switch)
            if first
            else (params.p_continue, params.p_switch)
        )
        r = decision_rng.random()
        change_combo = decision_rng.random() < params.p_combo
        last_seg = seg + 1 >= seg_counts[show]
        if r < p_cont and not last_seg:  # continue
            seg, first = seg + 1, False
            if change_combo:
                kind = pick_combo(show, exclude=kind)
        elif r < p_cont + p_sw or (r < p_cont and last_seg):  # switch video
            show = pick_show()
            kind, seg, first = pick_combo(show), 0, True
        else:  # leave
            return path


def place_viewers(
    paths: list[list[tuple[int, int]]],
    unit_gaps: np.ndarray,
    lam_viewers: float,
    pre_roll_s: float,
    duration_s: float,
    gap_s: float,
) -> list[Request]:
    """[Honest] Viewer v starts at -pre_roll + cumsum(Exp(1))/lambda and requests step k at
    start + k * gap_s; returns the requests inside [0, duration_s), sorted by time."""
    starts = -pre_roll_s + np.cumsum(unit_gaps) / lam_viewers
    requests = []
    for uid, (start, path) in enumerate(zip(starts, paths)):
        if start >= duration_s:
            break
        for step, (kind, seg) in enumerate(path):
            t = start + step * gap_s
            if 0 <= t < duration_s:
                requests.append(Request(float(t), uid, step, kind, seg))
    return sorted(requests, key=lambda r: r.t_s)


def kind_class(kind: str) -> str:
    """[Honest] Maps a request kind to one of the 8 cpu-tier representative classes."""
    if "forecast" in kind:
        return "forecast"
    if "spiders" in kind:
        return "spiders"
    if "forest720_leaves" in kind:
        return "leaves_alpha" if kind.endswith("leaves_alpha|alpha") else "leaves_off"
    if "forest720" in kind:
        return "leaves_off"
    if "/track/landscape" in kind:
        return "track"
    if "/portrait/" in kind:
        return "portrait"
    if "driver|Sam" in kind:
        return "race_full_sam"
    return "race_off"


def tier_service_table(
    kinds: list[str], gpu: pd.Series, llvm: pd.Series, cpu8: pd.Series
) -> pd.DataFrame:
    """[Honest] Per-kind busy time (s) per tier for one machine.
    The cpu tier only has 8 measured kinds, so every kind uses its class's cpu/gpu ratio."""
    class_ratio = {kind_class(k): cpu8[k] / gpu[k] for k in cpu8.index}
    return pd.DataFrame(
        {
            "gpu": gpu.loc[kinds].values,
            "llvm": llvm.loc[kinds].values,
            "cpu": [gpu[k] * class_ratio[kind_class(k)] for k in kinds],
        },
        index=kinds,
    )


def cluster_capacity(
    sites: list[tuple[str, str]], tables: dict[str, pd.DataFrame], mix: np.ndarray
) -> dict[str, float]:
    """[Honest] Per-site and total capacity (req/s) under kind mix `mix` (indexed like request_kind.txt)."""
    caps = {}
    for i, (machine, tier) in enumerate(sites):
        mean_s = float(np.dot(tables[machine][tier].values, mix))
        caps[f"{i}:{machine}_{tier}"] = 1.0 / mean_s
    caps["total"] = sum(caps.values())
    return caps


def fit_viewer_rate(
    paths: list[list[tuple[int, int]]],
    unit_gaps: np.ndarray,
    target_n: int,
    lam_guess: float,
    pre_roll_s: float,
    duration_s: float,
    gap_s: float,
) -> tuple[float, list[Request]]:
    """[Honest] Bisection on lambda_viewers so the window holds exactly target_n requests
    (sessions make the realised count vary a lot around lambda_req * duration)."""
    lo, hi = lam_guess / 4, lam_guess * 4
    best = None
    for _ in range(60):
        lam = (lo + hi) / 2
        requests = place_viewers(paths, unit_gaps, lam, pre_roll_s, duration_s, gap_s)
        if best is None or abs(len(requests) - target_n) < abs(len(best[1]) - target_n):
            best = (lam, requests)
        if len(requests) == target_n:
            break
        lo, hi = (lam, hi) if len(requests) < target_n else (lo, lam)
    return best


def window_rate(
    paths: list[list[tuple[int, int]]],
    unit_gaps: np.ndarray,
    sites: list[tuple[str, str]],
    tables: dict[str, pd.DataFrame],
    n_kinds: int,
    rho: float,
    params: ViewerParams,
    pre_roll_s: float,
    duration_s: float,
    max_iter: int = 20,
) -> tuple[float, float, dict[str, float], list[Request]]:
    """[Honest] Offered load in the window = rho * C(mix of the requests inside the window):
    the target request count rho * C * duration is met exactly by fitting lambda_viewers.
    The window's mix depends on the rate, so iterate to a fixed point.
    Returns (lambda_req, lambda_viewers, capacities, requests)."""
    mean_len = float(np.mean([len(p) for p in paths]))
    mix = np.bincount([k for p in paths for k, _ in p], minlength=n_kinds).astype(float)
    mix /= mix.sum()
    prev = None
    for _ in range(max_iter):
        caps = cluster_capacity(sites, tables, mix)
        target_n = round(rho * caps["total"] * duration_s)
        lam_v, requests = fit_viewer_rate(
            paths, unit_gaps, target_n, target_n / duration_s / mean_len,
            pre_roll_s, duration_s, params.segment_gap_s,
        )
        key = [(r.uid, r.step) for r in requests]
        if key == prev:
            break
        prev = key
        mix = np.bincount([r.kind for r in requests], minlength=n_kinds) / len(requests)
    return len(requests) / duration_s, lam_v, caps, requests


def segment_path(kind: str, seg: int) -> str:
    """[Honest] Fills the segment placeholder of a request kind."""
    start = seg * SEG_LEN_S
    return kind.replace(SEGMENT_PLACEHOLDER, f"/{start}/{start + SEG_LEN_S}/", 1)


# ==============================================================================
# 2. Dishonest Functions (I/O Drivers & Boundary Adapters)
# ==============================================================================


def load_kinds(path: Path) -> list[str]:
    """[Dishonest I/O] Reads request kind templates."""
    return [
        l.strip() for l in path.read_text(encoding="utf-8").splitlines() if l.strip()
    ]


def load_busy_times(path: Path) -> pd.Series:
    """[Dishonest I/O] Reads a stat_collect CSV as busy time (s) per request kind."""
    df = pd.read_csv(path)
    df["kind"] = df.request.str.replace(
        r"^(/offload/show/[^/]+)/\d+/\d+/", r"\1" + SEGMENT_PLACEHOLDER, regex=True
    )
    df["busy_s"] = (df.asset_ms + df.work_ms) / 1000
    return df.groupby("kind").busy_s.mean()


def write_rows(path: Path, header: list[str], rows: list[list]):
    """[Dishonest I/O] Writes a CSV file."""
    with path.open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(header)
        w.writerows(rows)


def print_summary(
    alpha: float, paths: list[list[tuple[int, int]]], kinds: list[str], kind_shows: list[str]
):
    """[Dishonest I/O] Prints viewer and request statistics (over all generated viewers)."""
    lens = np.array([len(p) for p in paths])
    videos = [sum(seg == 0 for _, seg in p) for p in paths]  # every video starts at segment 0
    combo_changes = [sum(k1 != k2 and s2 == s1 + 1 for (k1, s1), (k2, s2) in zip(p, p[1:])) for p in paths]
    flat = [(k, s) for p in paths for k, s in p]
    segs = pd.Series([s for _, s in flat]).value_counts(normalize=True).sort_index()
    shows = pd.Series([kind_shows[k] for k, _ in flat]).value_counts(normalize=True)
    kind_counts = pd.Series([k for k, _ in flat]).value_counts()
    print(f"\n=== alpha={alpha}: {len(paths)} viewers, {len(flat)} segment requests ===")
    print(f"segments/viewer: mean {lens.mean():.2f}, median {np.median(lens):.0f}, p90 {np.percentile(lens, 90):.0f}, "
          f"max {lens.max()} | videos/viewer {np.mean(videos):.2f} | combination changes/viewer {np.mean(combo_changes):.2f}")
    print("segment share: " + ", ".join(f"{k * SEG_LEN_S}s {v:.2f}" for k, v in segs.items()))
    print(f"distinct kinds: {len(kind_counts)}/{len(kinds)}, top-10 {kind_counts.iloc[:10].sum() / len(flat):.2f}")
    print("show share: " + ", ".join(f"{s} {v:.2f}" for s, v in shows.items()))


def main(out_dir: Path | None = None):
    # =========================================================================
    # Configuration Variables (Edit directly here)
    # =========================================================================

    # 1. Content and viewers
    alphas = [0.5, 1.2]  # Zipf exponents (uniform-ish vs concentrated), per proposal
    zipf_cutoff = 50.0  # exponential cutoff rank
    show_rank = [  # video popularity, most popular first (None = seeded random order)
        "f1_full.json",
        "forecast.json",
        "f1.json",
        "forest720_leaves.json",
        "spiders.json",
        "forest720.json",
    ]
    viewer = ViewerParams()
    n_viewers = 3000  # generated viewers (only those overlapping the window are replayed)
    seed = 2026

    # 2. Replay window and target load
    duration_s = 300
    pre_roll_s = 600  # viewers start this long before t=0, so the window opens at steady load
    rho = 0.7  # offered load as a fraction of cluster capacity

    # 3. Clusters (machine, tier); service times come from stat/<machine>_<port>_<tier>_timing.csv
    clusters = [
        Cluster(
            "case1",
            [
                ("om", "gpu"),
                ("om", "llvm"),
                ("om", "cpu"),
                ("rs", "gpu"),
                ("rs", "llvm"),
                ("rs", "cpu"),
            ],
        ),
        Cluster(
            "case2",
            [("om", "gpu")] * 2 + [("rs", "gpu")] * 2,
            note="provisional: uses case-1 co-located gpu timings, not a measured 4-gpu run",
        ),
    ]

    # 4. Paths
    script_dir = Path(__file__).resolve().parent
    stat_dir = script_dir.parent / "stat"
    out_dir = out_dir or script_dir / "sequences"

    # =========================================================================
    # Load inputs
    # =========================================================================
    kinds = load_kinds(script_dir / "request_kind.txt")
    durations = pd.read_csv(script_dir / "video_duration.csv").set_index("video")["duration"]
    kind_shows = [re.search(r"/show/([^/]+)/", k).group(1) for k in kinds]
    show_segments = {s: int(durations[s]) // SEG_LEN_S for s in set(kind_shows)}
    # Load calibration only for the machines the clusters use (stat/<machine>_<port>_<tier>_timing.csv)
    tables = {}
    for machine in sorted({m for c in clusters for m, _ in c.sites}):
        busy = {
            tier: load_busy_times(stat_dir / f"{machine}_{port}_{tier}_timing.csv")
            for tier, port in [("gpu", 7010), ("llvm", 7020), ("cpu", 7030)]
        }
        tables[machine] = tier_service_table(kinds, busy["gpu"], busy["llvm"], busy["cpu"])

    out_dir.mkdir(exist_ok=True)
    arrival_ss, rank_ss, content_ss, decision_ss = np.random.SeedSequence(seed).spawn(4)
    unit_gaps = np.random.default_rng(arrival_ss).exponential(1.0, n_viewers)

    # =========================================================================
    # Generate viewer paths per alpha, then place viewers in time per cluster
    # =========================================================================
    meta = []
    for alpha in alphas:
        probs, used_rank = two_level_popularity(
            kind_shows, alpha, zipf_cutoff, np.random.default_rng(rank_ss), show_rank
        )
        combos = show_tables(kind_shows, probs)
        shows = list(combos)
        show_p = np.array([probs[combos[s][0]].sum() for s in shows])
        content_rng = np.random.default_rng(content_ss)
        decision_rng = np.random.default_rng(decision_ss)
        paths = [
            viewer_path(shows, show_p, combos, show_segments, viewer, content_rng, decision_rng)
            for _ in range(n_viewers)
        ]
        print(f"\nshow popularity ranking: {', '.join(used_rank)}")
        print_summary(alpha, paths, kinds, kind_shows)

        write_rows(
            out_dir / f"seq_a{alpha}_viewers.csv",
            ["user_id", "step", "kind_idx", "show", "seg_start", "seg_end"],
            [[uid, step, k, kind_shows[k], s * SEG_LEN_S, (s + 1) * SEG_LEN_S]
             for uid, p in enumerate(paths) for step, (k, s) in enumerate(p)],
        )

        for c in clusters:
            lam_req, lam_v, caps, requests = window_rate(
                paths, unit_gaps, c.sites, tables, len(kinds), rho, viewer, pre_roll_s, duration_s
            )
            last_start = -pre_roll_s + unit_gaps.sum() / lam_v
            if last_start < duration_s:
                print(f"[Warning] {c.name}: viewers run out at {last_start:.0f}s, increase n_viewers",
                      file=sys.stderr)
            write_rows(
                out_dir / f"seq_a{alpha}_{c.name}.csv",
                ["req_idx", "t_ms", "user_id", "step", "kind_idx", "show", "seg_start", "seg_end", "path"],
                [[i, round(r.t_s * 1000), r.uid, r.step, r.kind, kind_shows[r.kind], r.seg * SEG_LEN_S,
                  (r.seg + 1) * SEG_LEN_S, segment_path(kinds[r.kind], r.seg)]
                 for i, r in enumerate(requests)],
            )
            n_users = len({r.uid for r in requests})
            site_caps = " ".join(
                f"{k.split(':')[1]}={v * duration_s:.0f}" for k, v in caps.items() if k != "total"
            )
            print(
                f"  {c.name}: capacity {caps['total'] * duration_s:.0f} req/{duration_s}s [{site_caps}] "
                f"-> lambda_req {lam_req:.3f}/s, lambda_viewers {lam_v:.4f}/s, "
                f"{len(requests)} requests from {n_users} viewers in window"
                + (f"  ({c.note})" if c.note else "")
            )
            meta.append([alpha, c.name, rho, f"{caps['total']:.4f}", f"{lam_req:.4f}", f"{lam_v:.5f}",
                         f"{np.mean([len(p) for p in paths]):.3f}", len(requests), n_users, c.note])

    write_rows(
        out_dir / "meta.csv",
        ["alpha", "cluster", "rho", "capacity_rps", "lambda_req_rps", "lambda_viewers_rps",
         "mean_segments_per_viewer", "n_in_window", "viewers_in_window", "note"],
        meta,
    )
    print(f"\n[Done] sequences written to {out_dir}")


if __name__ == "__main__":
    main()
