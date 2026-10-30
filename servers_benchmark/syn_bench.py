#!/usr/bin/env python3
"""
OBM Synthetic-Show Benchmark (per-site timings of unseen content)

For every request kind in KINDS_FILE, sends REPS rounds of N simultaneous requests through the main
server. With round robin (and N = number of sites) every round puts exactly one request on each
site, and the next round only starts when the whole round is back, so no site ever queues: each
site gets REPS requests per kind. Repetition r uses segment r % 3 (0-10 s, 10-20 s, 20-30 s).
Rows are appended after every round, so an interrupted run keeps what it measured.

Usage: ./syn_bench.py [KINDS_FILE] [--sites N] [--reps R] [--base-url URL] [--timeout S] [--out CSV] [--match TEXT]
  KINDS_FILE  default syn_video/request_kind_syn.txt
  --out       default stat/syn_timings.csv (appended to, header written once)
Needs the main server running with policy rr and the syn shows in obm/ (see README section 5).
"""
import argparse
import csv
import sys
import threading
import time
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
from seq_bench import CSV_HEADERS, send_request  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SEG_S = 10
N_SEGMENTS = 3
EXTRA_HEADERS = ["round", "kind_idx", "show", "variant", "segment", "rep"]


# ==============================================================================
# 1. Honest Functions (Pure Domain Logic)
# ==============================================================================


def request_path(kind: str, segment: int) -> str:
    """[Honest] Fills the t_start/t_end placeholder of a request kind."""
    return kind.replace("t_start/t_end", f"{segment * SEG_S}/{(segment + 1) * SEG_S}", 1)


def kind_labels(kind: str) -> tuple[str, str]:
    """[Honest] (show, variant) of a request kind path /offload/show/<show>/t0/t1/w/h/<variant>/..."""
    parts = kind.split("/")
    return parts[3].removesuffix(".json"), parts[8]


# ==============================================================================
# 2. Dishonest Functions (I/O Drivers & Boundary Adapters)
# ==============================================================================


def send_round(base_url: str, rows: list[dict[str, Any]], timeout: float):
    """[Dishonest I/O] Sends every row's request at once (one thread each) and waits for all."""
    threads = []
    for row in rows:
        row["send_epoch_ms"] = round(time.time() * 1000)
        t = threading.Thread(target=send_request, args=(base_url, row, time.perf_counter(), timeout))
        t.start()
        threads.append(t)
    for t in threads:
        t.join()


def append_rows(path: Path, rows: list[dict[str, Any]]):
    """[Dishonest I/O] Appends rows to the CSV, writing the header if the file is new."""
    new = not path.exists()
    with path.open("a", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=EXTRA_HEADERS + CSV_HEADERS, extrasaction="ignore")
        if new:
            w.writeheader()
        w.writerows(rows)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("kinds", nargs="?", default=str(ROOT / "syn_video/request_kind_syn.txt"))
    ap.add_argument("--sites", type=int, default=6, help="requests per round (= number of sites)")
    ap.add_argument("--reps", type=int, default=6, help="rounds per kind (= requests per site per kind)")
    ap.add_argument("--base-url", default="http://localhost:7000")
    ap.add_argument("--timeout", type=float, default=900)
    ap.add_argument("--out", default=str(ROOT / "stat/syn_timings.csv"))
    ap.add_argument("--match", default="", help="only kinds containing this text")
    args = ap.parse_args()

    import requests
    requests.packages.urllib3.disable_warnings()  # type: ignore[attr-defined]

    kinds = [k.strip() for k in Path(args.kinds).read_text().splitlines() if k.strip() and args.match in k]
    out = Path(args.out)
    total = len(kinds) * args.reps
    print(f"{len(kinds)} kinds x {args.reps} rounds x {args.sites} requests -> {out}", flush=True)
    t_start = time.time()
    rnd = 0
    for ki, kind in enumerate(kinds):
        show, variant = kind_labels(kind)
        for rep in range(args.reps):
            seg = rep % N_SEGMENTS
            path = request_path(kind, seg)
            rows = [{"round": rnd, "kind_idx": ki, "show": show, "variant": variant, "segment": seg,
                     "rep": rep, "req_idx": rnd * args.sites + i, "sched_ms": 0, "path": path}
                    for i in range(args.sites)]
            t0 = time.time()
            send_round(args.base_url, rows, args.timeout)
            append_rows(out, rows)
            workers = {r.get("worker", "") for r in rows}
            ok = sum(r.get("status") == 200 for r in rows)
            warn = "" if len(workers) == args.sites else f"  [warning: {len(workers)} distinct sites]"
            rnd += 1
            print(f"[{rnd:4}/{total}] {show} {variant} seg{seg} rep{rep}: {ok}/{args.sites} ok, "
                  f"slowest {max(r.get('latency_ms', 0) for r in rows) / 1000:.1f}s, "
                  f"round {time.time() - t0:.1f}s, elapsed {(time.time() - t_start) / 60:.0f} min{warn}", flush=True)
    print(f"done in {(time.time() - t_start) / 60:.1f} min -> {out}")


if __name__ == "__main__":
    main()
