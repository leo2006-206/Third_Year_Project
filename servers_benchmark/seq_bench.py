#!/usr/bin/env python3
"""
OBM Sequence Replayer (open-loop benchmark)
Replays a generated request sequence (sequences/seq_a<alpha>_<case>.csv) against the
main server: every request is sent at its scheduled t_ms on its own thread, regardless of
whether earlier requests have finished. Latency is measured from the scheduled send time,
so client-side lag can never hide server queueing (no coordinated omission).
Failed requests are recorded (status / error columns), not fatal.

Usage: ./seq_bench.py [SEQ_CSV] [NUM_OFFLOAD] [--base-url URL] [--timeout S] [--limit N]
Output: stat/run_<alpha>_<num_offload>_<run_idx>.csv (run_idx = 1 + highest existing index)
"""

import argparse
import concurrent.futures
import csv
import re
import sys
import threading
import time
from pathlib import Path
from typing import Any

import requests

sys.path.insert(0, str(Path(__file__).resolve().parent))
from stat_collect import parse_timing_headers  # noqa: E402

CSV_HEADERS = [
    "req_idx",
    "user_id",  # viewer in the generated sequence
    "step",  # viewer's k-th request
    "kind_idx",
    "show",
    "seg_start",
    "seg_end",
    "sched_ms",  # scheduled send time, relative to run start
    "lag_ms",  # actual send - scheduled (client-side lateness, should be ~0)
    "ttfb_ms",  # scheduled send -> response headers
    "latency_ms",  # scheduled send -> full response body received (client perceived)
    "status",
    "size_bytes",
    "worker",  # X-OBM-Worker: offload site chosen by the load balancer
    "net_ms",
    "queue_ms",
    "asset_ms",
    "work_ms",
    "decode_ms",
    "render_ms",
    "encode_ms",
    "client_net_ms",
    "send_epoch_ms",  # wall clock at scheduled send, to align with server logs
    "error",
    "path",
]


# ==============================================================================
# 1. Honest Functions (Pure Domain Logic)
# ==============================================================================


def parse_alpha(seq_path: Path) -> str:
    """[Honest] Extracts the Zipf alpha from a sequence file name (seq_a<alpha>_...)."""
    m = re.search(r"seq_a([0-9.]+?)(?:_|\.csv$)", seq_path.name)
    return m.group(1) if m else "unknown"


def next_run_index(existing_names: list[str], alpha: str, num_offload: int) -> int:
    """[Honest] 1 + highest run index among run_<alpha>_<num_offload>_<idx>.csv, or 1 if none."""
    pattern = re.compile(rf"^run_{re.escape(alpha)}_{num_offload}_(\d+)\.csv$")
    indices = [int(m.group(1)) for n in existing_names if (m := pattern.match(n))]
    return max(indices, default=0) + 1


def result_row(
    req: dict[str, str], sched_ms: int, start_epoch: float
) -> dict[str, Any]:
    """[Honest] Empty result row for a sequence request (filled in by the sender)."""
    row: dict[str, Any] = {h: "" for h in CSV_HEADERS}
    row.update(
        req_idx=req["req_idx"],
        user_id=req.get("user_id", ""),
        step=req.get("step", ""),
        kind_idx=req["kind_idx"],
        show=req["show"],
        seg_start=req["seg_start"],
        seg_end=req["seg_end"],
        sched_ms=sched_ms,
        send_epoch_ms=round((start_epoch + sched_ms / 1000) * 1000),
        path=req["path"],
    )
    return row


# ==============================================================================
# 2. Dishonest Functions (I/O Drivers & Boundary Adapters)
# ==============================================================================


def load_sequence(seq_path: Path, limit: int | None) -> list[dict[str, str]]:
    """[Dishonest I/O] Reads a replay sequence CSV (needs t_ms and path columns)."""
    with seq_path.open(newline="", encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    if not rows or "t_ms" not in rows[0] or "path" not in rows[0]:
        sys.exit(
            f"[Error] {seq_path} is not a replay sequence (needs t_ms and path columns)"
        )
    rows.sort(key=lambda r: int(r["t_ms"]))
    return rows[:limit] if limit else rows


def send_request(
    base_url: str, row: dict[str, Any], sched_abs: float, timeout: float
) -> dict[str, Any]:
    """[Dishonest I/O] Sends one GET, streams the body, fills timing/result fields of `row`."""
    row["lag_ms"] = round((time.perf_counter() - sched_abs) * 1000)
    try:
        with requests.get(
            base_url.rstrip("/") + row["path"],
            timeout=timeout,
            stream=True,
            verify=False,
        ) as resp:
            row["ttfb_ms"] = round((time.perf_counter() - sched_abs) * 1000)
            size = sum(len(chunk) for chunk in resp.iter_content(chunk_size=65536))
            row["latency_ms"] = round((time.perf_counter() - sched_abs) * 1000)
            row["status"] = resp.status_code
            row["size_bytes"] = size
            row["worker"] = resp.headers.get("X-OBM-Worker", "")
            t = parse_timing_headers(dict(resp.headers))
            for k in ("net", "queue", "asset", "work", "decode", "render", "encode"):
                row[f"{k}_ms"] = t[k]
            server_ms = t["work"] + t["asset"] + t["queue"] + t["net"]
            row["client_net_ms"] = (
                max(0, row["latency_ms"] - server_ms) if server_ms else ""
            )
            if resp.status_code != 200:
                row["error"] = f"HTTP {resp.status_code}"
    except requests.RequestException as e:
        row["latency_ms"] = round((time.perf_counter() - sched_abs) * 1000)
        row["error"] = type(e).__name__
    return row


def replay(
    base_url: str, seq: list[dict[str, str]], timeout: float
) -> list[dict[str, Any]]:
    """[Dishonest Orchestrator] Open-loop replay: submits each request at its t_ms."""
    results: list[dict[str, Any]] = []
    lock = threading.Lock()
    done = 0

    def on_done(fut: concurrent.futures.Future):
        nonlocal done
        row = fut.result()
        with lock:
            results.append(row)
            done += 1
            status = row["error"] or row["status"]
            print(
                f"[{done:4d}/{len(seq)}] #{row['req_idx']:>4} {status!s:>6} "
                f"{row['latency_ms']:>7}ms worker={row['worker'] or '-'} {row['path'][14:70]}",
                flush=True,
            )

    # One thread per request so the client never queues requests itself
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, len(seq))) as pool:
        start_epoch = time.time()
        start = time.perf_counter()
        for req in seq:
            sched_ms = int(req["t_ms"])
            sched_abs = start + sched_ms / 1000
            delay = sched_abs - time.perf_counter()
            if delay > 0:
                time.sleep(delay)
            row = result_row(req, sched_ms, start_epoch)
            pool.submit(
                send_request, base_url, row, sched_abs, timeout
            ).add_done_callback(on_done)
        print(
            f"--- all {len(seq)} requests sent after {time.perf_counter() - start:.1f}s, draining ---",
            flush=True,
        )
    return sorted(results, key=lambda r: int(r["req_idx"]))


def write_results(path: Path, rows: list[dict[str, Any]]):
    """[Dishonest I/O] Writes the run CSV."""
    with path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=CSV_HEADERS)
        writer.writeheader()
        writer.writerows(rows)


def print_summary(rows: list[dict[str, Any]], wall_s: float):
    """[Dishonest I/O] Prints a short latency summary."""
    ok = sorted(int(r["latency_ms"]) for r in rows if not r["error"])
    failed = len(rows) - len(ok)
    pct = lambda p: ok[min(len(ok) - 1, int(p / 100 * len(ok)))] if ok else 0  # noqa: E731
    max_lag = max((int(r["lag_ms"]) for r in rows if r["lag_ms"] != ""), default=0)
    print("=" * 75)
    print(
        f"Requests: {len(rows)}  ok: {len(ok)}  failed: {failed}  wall: {wall_s:.1f}s  max send lag: {max_lag}ms"
    )
    if ok:
        print(
            f"Latency (ms): mean {sum(ok) / len(ok):.0f}  p50 {pct(50)}  p90 {pct(90)}  p99 {pct(99)}  max {ok[-1]}"
        )
    print("=" * 75)


# ==============================================================================
# 3. CLI Entry Point
# ==============================================================================


def main():
    script_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(
        description="Open-loop replay of an OBM request sequence."
    )
    parser.add_argument(
        "seq",
        nargs="?",
        type=Path,
        default=script_dir / "sequences" / "seq_a0.5_case1.csv",
        help="replay sequence CSV (default: sequences/seq_a0.5_case1.csv)",
    )
    parser.add_argument(
        "num_offload",
        nargs="?",
        type=int,
        default=6,
        help="number of offload sites in the cluster, used in the output name (default: 6)",
    )
    parser.add_argument(
        "--base-url",
        default="https://ct-obm-main.leowong.space/",
        help="main server URL (default: https://ct-obm-main.leowong.space/)",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=600.0,
        help="per-request timeout in seconds (default: 600)",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=None,
        help="replay only the first N requests (smoke tests)",
    )
    args = parser.parse_args()

    requests.packages.urllib3.disable_warnings()  # type: ignore[attr-defined]
    stat_dir = script_dir.parent / "stat"
    stat_dir.mkdir(exist_ok=True)

    seq = load_sequence(args.seq, args.limit)
    alpha = parse_alpha(args.seq)
    run_idx = next_run_index(
        [p.name for p in stat_dir.glob("run_*.csv")], alpha, args.num_offload
    )
    out_path = stat_dir / f"run_{alpha}_{args.num_offload}_{run_idx}.csv"

    print("=" * 75)
    print(
        f"Sequence:     {args.seq} ({len(seq)} requests, last at {int(seq[-1]['t_ms']) / 1000:.1f}s)"
    )
    print(f"Target:       {args.base_url}")
    print(f"Num offload:  {args.num_offload}   alpha: {alpha}   run: {run_idx}")
    print(f"Output:       {out_path}")
    print("=" * 75)

    t0 = time.perf_counter()
    try:
        rows = replay(args.base_url, seq, args.timeout)
    except KeyboardInterrupt:
        sys.exit("\n[Aborted] no output written")
    write_results(out_path, rows)
    print_summary(rows, time.perf_counter() - t0)
    print(f"[Done] {out_path}")


if __name__ == "__main__":
    main()
