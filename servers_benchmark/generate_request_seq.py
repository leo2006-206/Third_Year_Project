#!/usr/bin/env python3
"""
OBM Request Sequence Generator

Builds one stateless request timeline per Zipf exponent alpha, then time-scales it
for each offload cluster so the offered load is a fixed fraction (rho) of that
cluster's measured capacity. Every cluster replays the same request order; only
the arrival rate differs.

Pipeline:
  1. Arrival times: Poisson process in normalised units (mean inter-arrival = 1).
  2. Session assignment: each arrival goes to a user whose previous segment request
     is at least MIN_GAP_UNITS old (FIFO), otherwise to a new user. After each
     request the user continues to the next segment, switches video, or leaves.
  3. Popularity: video first (Zipf over shows), then combination (Zipf over that show's
     request kinds); both Zipf with exponential cutoff, ranked by seeded permutations.
     A flat mode (one Zipf over all kinds) is kept for comparison.
  4. Capacity: per-kind busy time (asset + work) per site from stat/*.csv,
     C = sum_j 1 / E_mix[S_j] over the sequence's own kind mix.
  5. Scaling: lambda_c = rho * C_c, t_sec = t_unit / lambda_c, truncated at DURATION_S.

Arrivals, popularity ranking and user behaviour use separate seeded random streams,
so changing alpha leaves arrival times and user decisions unchanged.

Outputs (servers_benchmark/sequences/):
  seq_a<alpha>.csv            full normalised sequence (t_unit)
  seq_a<alpha>_<cluster>.csv  replay timeline for a cluster (t_ms, truncated)
  meta.csv                    capacity, rate and request count per (alpha, cluster)
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
class SessionParams:
    p_like: float = 0.70  # request the next segment of the same kind
    p_switch: float = 0.15  # request segment 0 of another kind (else: leave)
    p_seek: float = 0.10  # a new video starts at a random segment instead of 0
    min_gap_units: float = (
        5.0  # min time between one user's requests (mean inter-arrivals)
    )


@dataclass
class User:
    uid: int
    kind: int
    next_seg: int
    ready_at: float


@dataclass
class Request:
    t_unit: float
    uid: int
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


def kind_popularity(n_kinds: int, alpha: float, cutoff: float, rank_rng) -> np.ndarray:
    """[Honest] Flat per-kind request probability; kinds are assigned to ranks by a seeded permutation."""
    probs = np.empty(n_kinds)
    probs[rank_rng.permutation(n_kinds)] = zipf_cutoff_probs(n_kinds, alpha, cutoff)
    return probs


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


def poisson_arrivals(n: int, arrival_rng) -> np.ndarray:
    """[Honest] n arrival times of a unit-rate Poisson process."""
    return np.cumsum(arrival_rng.exponential(1.0, n))


def assign_sessions(
    arrivals: np.ndarray,
    seg_counts: list[int],
    popularity: np.ndarray,
    params: SessionParams,
    user_rng,
) -> list[Request]:
    """[Honest] Assigns each arrival to a waiting user (FIFO by ready time) or a new user,
    then draws that user's next action: continue, switch video, or leave."""
    n_kinds = len(seg_counts)
    pending: list[User] = []
    requests: list[Request] = []
    next_uid = 0

    def new_video() -> tuple[int, int]:
        kind = int(user_rng.choice(n_kinds, p=popularity))
        seg = (
            int(user_rng.integers(seg_counts[kind]))
            if user_rng.random() < params.p_seek
            else 0
        )
        return kind, seg

    for t in arrivals:
        ready = [u for u in pending if u.ready_at <= t]
        if ready:
            user = min(ready, key=lambda u: u.ready_at)
            pending.remove(user)
        else:
            kind, seg = new_video()
            user = User(next_uid, kind, seg, t)
            next_uid += 1

        requests.append(Request(float(t), user.uid, user.kind, user.next_seg))

        # Decide this user's next request (a finished video turns "continue" into "switch")
        r = user_rng.random()
        last_seg = user.next_seg + 1 >= seg_counts[user.kind]
        if r < params.p_like and not last_seg:
            user.next_seg += 1
        elif r < params.p_like + params.p_switch or (r < params.p_like and last_seg):
            user.kind, user.next_seg = new_video()
        else:
            continue  # user leaves
        user.ready_at = t + params.min_gap_units
        pending.append(user)

    return requests


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


def window_rate(
    requests: list[Request],
    sites: list[tuple[str, str]],
    tables: dict[str, pd.DataFrame],
    n_kinds: int,
    rho: float,
    duration_s: float,
    max_iter: int = 20,
) -> tuple[float, dict[str, float]]:
    """[Honest] Finds lambda = rho * C(mix of the requests inside the window).
    The window depends on lambda, so iterate to a fixed point (usually 2-4 steps)."""
    t_units = np.array([r.t_unit for r in requests])
    kinds = np.array([r.kind for r in requests])
    in_window = np.ones(len(requests), dtype=bool)
    for _ in range(max_iter):
        mix = np.bincount(kinds[in_window], minlength=n_kinds) / in_window.sum()
        caps = cluster_capacity(sites, tables, mix)
        lam = rho * caps["total"]
        new_window = t_units / lam < duration_s
        if np.array_equal(new_window, in_window):
            break
        in_window = new_window
    return lam, caps


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
    alpha: float, requests: list[Request], kinds: list[str], seg_counts: list[int]
):
    """[Dishonest I/O] Prints sequence statistics."""
    uids = pd.Series([r.uid for r in requests])
    per_user = uids.value_counts()
    kind_counts = pd.Series([r.kind for r in requests]).value_counts()
    shows = pd.Series(
        [re.search(r"/show/([^/]+)/", kinds[r.kind]).group(1) for r in requests]
    )
    print(f"\n=== alpha={alpha}: {len(requests)} requests, {uids.nunique()} users ===")
    print(
        f"requests/user: mean {per_user.mean():.2f}, median {per_user.median():.0f}, max {per_user.max()}"
    )
    print(f"segment-0 share: {np.mean([r.seg == 0 for r in requests]):.2f}")
    print(
        f"distinct kinds: {len(kind_counts)}/{len(kinds)}, top-1 {kind_counts.iloc[0] / len(requests):.2f}, "
        f"top-10 {kind_counts.iloc[:10].sum() / len(requests):.2f}"
    )
    print(
        "show share: "
        + ", ".join(
            f"{s} {v:.2f}" for s, v in shows.value_counts(normalize=True).items()
        )
    )


def main(popularity_mode: str = "two_level", out_dir: Path | None = None):
    # =========================================================================
    # Configuration Variables (Edit directly here)
    # =========================================================================

    # 1. Sequence shape
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
    # "two_level" = Zipf over videos, then Zipf over combinations
    # "flat"      = one Zipf over all request kinds
    n_requests = (
        1000  # full sequence length (only the prefix inside DURATION_S is replayed)
    )
    seed = 2026
    session = SessionParams()

    # 2. Replay window and target load
    duration_s = 300
    rho = 0.7  # offered load as a fraction of cluster capacity

    # 3. Clusters (machine, tier); service times come from stat/<machine>_<port>_<tier>_timing.csv
    clusters = [
        Cluster(
            "case1",
            [
                ("lap", "gpu"),
                ("lap", "llvm"),
                ("lap", "cpu"),
                ("lab", "gpu"),
                ("lab", "llvm"),
                ("lab", "cpu"),
            ],
        ),
        Cluster(
            "case2",
            [("lap", "gpu")] * 2 + [("lab", "gpu")] * 2,
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
    durations = pd.read_csv(script_dir / "video_duration.csv").set_index("video")[
        "duration"
    ]
    kind_shows = [re.search(r"/show/([^/]+)/", k).group(1) for k in kinds]
    seg_counts = [
        int(durations[re.search(r"/show/([^/]+)/", k).group(1)]) // SEG_LEN_S
        for k in kinds
    ]
    tables = {}
    for machine in ["lap", "lab"]:
        busy = {
            tier: load_busy_times(stat_dir / f"{machine}_{port}_{tier}_timing.csv")
            for tier, port in [("gpu", 7010), ("llvm", 7020), ("cpu", 7030)]
        }
        tables[machine] = tier_service_table(
            kinds, busy["gpu"], busy["llvm"], busy["cpu"]
        )

    out_dir.mkdir(exist_ok=True)
    arrival_ss, rank_ss, user_ss = np.random.SeedSequence(seed).spawn(3)
    arrivals = poisson_arrivals(n_requests, np.random.default_rng(arrival_ss))

    # =========================================================================
    # Generate one sequence per alpha, then scale per cluster
    # =========================================================================
    meta = []
    for alpha in alphas:
        rank_rng = np.random.default_rng(rank_ss)
        if popularity_mode == "two_level":
            popularity, used_rank = two_level_popularity(
                kind_shows, alpha, zipf_cutoff, rank_rng, show_rank
            )
            print(f"\nshow popularity ranking: {', '.join(used_rank)}")
        else:
            popularity = kind_popularity(len(kinds), alpha, zipf_cutoff, rank_rng)
        requests = assign_sessions(
            arrivals, seg_counts, popularity, session, np.random.default_rng(user_ss)
        )
        print_summary(alpha, requests, kinds, seg_counts)

        def show_of(k):
            return re.search(r"/show/([^/]+)/", kinds[k]).group(1)

        write_rows(
            out_dir / f"seq_a{alpha}.csv",
            [
                "req_idx",
                "t_unit",
                "user_id",
                "kind_idx",
                "show",
                "seg_start",
                "seg_end",
                "path",
            ],
            [
                [
                    i,
                    f"{r.t_unit:.4f}",
                    r.uid,
                    r.kind,
                    show_of(r.kind),
                    r.seg * SEG_LEN_S,
                    (r.seg + 1) * SEG_LEN_S,
                    segment_path(kinds[r.kind], r.seg),
                ]
                for i, r in enumerate(requests)
            ],
        )

        for c in clusters:
            lam, caps = window_rate(
                requests, c.sites, tables, len(kinds), rho, duration_s
            )
            rows = [
                [
                    i,
                    round(r.t_unit / lam * 1000),
                    r.uid,
                    r.kind,
                    show_of(r.kind),
                    r.seg * SEG_LEN_S,
                    (r.seg + 1) * SEG_LEN_S,
                    segment_path(kinds[r.kind], r.seg),
                ]
                for i, r in enumerate(requests)
                if r.t_unit / lam < duration_s
            ]
            if len(rows) == len(requests):
                print(
                    f"[Warning] {c.name}: all {len(requests)} requests fit in {duration_s}s, increase n_requests",
                    file=sys.stderr,
                )
            write_rows(
                out_dir / f"seq_a{alpha}_{c.name}.csv",
                [
                    "req_idx",
                    "t_ms",
                    "user_id",
                    "kind_idx",
                    "show",
                    "seg_start",
                    "seg_end",
                    "path",
                ],
                rows,
            )
            site_caps = " ".join(
                f"{k.split(':')[1]}={v * duration_s:.0f}"
                for k, v in caps.items()
                if k != "total"
            )
            print(
                f"  {c.name}: capacity {caps['total'] * duration_s:.0f} req/{duration_s}s "
                f"[{site_caps}] -> lambda {lam:.3f} req/s, {len(rows)} requests in window"
                + (f"  ({c.note})" if c.note else "")
            )
            meta.append(
                [
                    alpha,
                    c.name,
                    rho,
                    f"{caps['total']:.4f}",
                    f"{lam:.4f}",
                    len(rows),
                    f"{1 / lam:.3f}",
                    c.note,
                ]
            )

    write_rows(
        out_dir / "meta.csv",
        [
            "alpha",
            "cluster",
            "rho",
            "capacity_rps",
            "lambda_rps",
            "n_in_window",
            "sec_per_t_unit",
            "note",
        ],
        meta,
    )
    print(f"\n[Done] sequences written to {out_dir}")


if __name__ == "__main__":
    main()
