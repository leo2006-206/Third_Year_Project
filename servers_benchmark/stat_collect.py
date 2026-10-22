#!/usr/bin/env python3
"""
OBM Request Statistics Collector (per offload site calibration)
Sends the first segment (0/10) of each request kind in request_kind.txt
directly to one offload site at http://localhost:<port>, one at a time (no queueing).
For the slow 'cpu' composition tier, only a predefined set of 8 representative kinds is sent.
Extracts fine-grained timing breakdowns (decode, render, encode, etc.),
sorts the dataset by request URL, and writes to <port>_<config>_timing.csv.

Usage: ./stat_collect.py <port> <cpu|llvm|gpu>
"""

import argparse
import concurrent.futures
import csv
import sys
import time
from pathlib import Path
from typing import Any

import requests
import urllib3

# Suppress TLS verification warnings for local/custom domains
urllib3.disable_warnings(urllib3.exceptions.InsecureRequestWarning)

CSV_HEADERS = [
    "request",
    "run_idx",
    "duration_ms",
    "net_ms",
    "queue_ms",
    "asset_ms",
    "work_ms",
    "decode_ms",
    "render_ms",
    "encode_ms",
    "client_net_ms",
]

FIRST_SEGMENT = "/0/10/"
SEGMENT_PLACEHOLDER = "/t_start/t_end/"
REQUEST_TIMEOUT_S = 300.0

# Representative kinds for the 'cpu' composition tier (~20s+ per request),
# one per distinct composition workload (cpu composition scales each type differently):
CPU_REQUEST_KINDS = [
    # 1. animation only, no video composite (lightest f1, ~2.1s on gpu)
    "/offload/show/f1.json/t_start/t_end/1280/720/track/landscape/3/track|race/drivers|Sam/driver|Off",
    # 2. single 1080p video + driver sprites (typical f1, ~3.8s)
    "/offload/show/f1.json/t_start/t_end/1280/720/race/landscape/4/race|race/driver|Off/track|off/drivers|Sam",
    # 3. video + PIP video + track overlay (heaviest f1_full, ~4.9s)
    "/offload/show/f1_full.json/t_start/t_end/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam",
    # 4. portrait reframe transforms + PIP + track (~3.8s)
    "/offload/show/f1.json/t_start/t_end/720/1280/race/portrait/3/race|race/race-frame|frame-default/driver|Sam/driver-frame|frame-default/track|track/drivers|Sam",
    # 5. video + full-frame animated overlay, different show (~2.3s)
    "/offload/show/spiders.json/t_start/t_end/1280/720/main/landscape/2/main|race/spider_blocker|nospiders",
    # 6. 4 videos + 3 alpha masks (~5.3s)
    "/offload/show/forest720_leaves.json/t_start/t_end/1280/720/core/landscape/5/background|background_default/creature|creature_default/creature_alpha|alpha/bird|bird_default/bird_alpha|alpha/near_forest|near_default/near_alpha|alpha/leaves|leaves_default/leaves_alpha|Off",
    # 7. 4 videos + 4 alpha masks (~6.1s)
    "/offload/show/forest720_leaves.json/t_start/t_end/1280/720/core/landscape/5/background|background_default/creature|creature_default/creature_alpha|alpha/bird|bird_default/bird_alpha|alpha/near_forest|near_default/near_alpha|alpha/leaves|leaves_default/leaves_alpha|alpha",
    # 8. full-frame chroma key (heaviest overall, ~9.0s)
    "/offload/show/forecast.json/t_start/t_end/1280/720/core/landscape/4/map|map_colour/weather|Meteorological/presenter|BSL/presenter_chroma|chroma/time|time",
]


# ==============================================================================
# 1. Honest Functions (Pure Domain Logic)
# ==============================================================================


def parse_timing_headers(headers: dict[str, str]) -> dict[str, int]:
    """[Honest] Extracts timing metrics (ms) from HTTP response headers."""
    timings = {
        "queue": 0,
        "asset": 0,
        "work": 0,
        "decode": 0,
        "render": 0,
        "encode": 0,
        "net": 0,
    }

    # Parse X-Dana-Timings: queue=X;asset=Y;work=Z;decode=A;render=B;encode=C
    dana_header = headers.get("x-dana-timings") or headers.get("X-Dana-Timings", "")
    if dana_header:
        for part in dana_header.split(";"):
            if "=" in part:
                k, v = part.split("=", 1)
                k_clean = k.strip().lower()
                if k_clean in timings and v.strip().isdigit():
                    timings[k_clean] = int(v.strip())

    # Parse X-OBM-Net: N
    obm_net = headers.get("x-obm-net") or headers.get("X-OBM-Net", "")
    if obm_net.isdigit():
        timings["net"] = int(obm_net)

    return timings


def is_valid_timing_row(row: dict[str, Any]) -> bool:
    """[Honest] Validates that a CSV row has non-empty request and valid positive timings."""
    req = str(row.get("request", "")).strip()
    if not req:
        return False
    for key in ("duration_ms", "work_ms"):
        val = str(row.get(key, "")).strip()
        if not val.isdigit() or int(val) <= 0:
            return False
    return True


def filter_pending_tasks(
    requests_list: list[str],
    repetitions: int,
    existing_rows: list[dict[str, Any]],
) -> tuple[list[tuple[str, int]], set[tuple[str, int]]]:
    """[Honest] Determines which (request, run_idx) tasks still need to be executed."""
    completed = {
        (str(row["request"]).strip(), int(row["run_idx"]))
        for row in existing_rows
        if str(row.get("run_idx", "")).strip().isdigit()
    }
    pending = [
        (path, run_idx)
        for path in requests_list
        for run_idx in range(1, repetitions + 1)
        if (path, run_idx) not in completed
    ]
    return pending, completed


def select_request_kinds(all_kinds: list[str], config: str) -> list[str]:
    """[Honest] Picks the kinds to benchmark: all kinds, or the 8 representatives for 'cpu'."""
    if config != "cpu":
        return all_kinds
    missing = [k for k in CPU_REQUEST_KINDS if k not in all_kinds]
    if missing:
        raise ValueError(f"CPU_REQUEST_KINDS not found in request_kind.txt: {missing}")
    return CPU_REQUEST_KINDS


def first_segment_paths(kinds: list[str]) -> list[str]:
    """[Honest] Replaces the segment placeholder with the first segment (0/10)."""
    return [k.replace(SEGMENT_PLACEHOLDER, FIRST_SEGMENT, 1) for k in kinds]


# ==============================================================================
# 2. Dishonest Functions (I/O Drivers & Boundary Adapters)
# ==============================================================================


def load_requests_file(file_path: Path) -> list[str]:
    """[Dishonest I/O] Reads request kind templates from request_kind.txt."""
    if not file_path.is_file():
        print(f"[Error] requests file not found: {file_path}", file=sys.stderr)
        sys.exit(1)

    lines = [
        line.strip()
        for line in file_path.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.startswith("#")
    ]
    if not lines:
        print(f"[Error] No request lines found in {file_path}", file=sys.stderr)
        sys.exit(1)
    return lines


def fetch_request(
    session: requests.Session,
    base_url: str,
    path: str,
    run_idx: int,
    task_id: int,
    total_tasks: int,
) -> dict[str, Any]:
    """[Dishonest I/O] Sends single HTTP GET and extracts timing metrics.
    Fails fast if request encounters error or non-200 status.
    """
    url = base_url.rstrip("/") + path
    t0 = time.perf_counter()
    try:
        resp = session.get(url, timeout=REQUEST_TIMEOUT_S, verify=False)
        duration_ms = round((time.perf_counter() - t0) * 1000)

        if resp.status_code != 200:
            print(
                f"\n[FATAL ERROR] Request {path} (Run #{run_idx}) failed with status {resp.status_code}!",
                file=sys.stderr,
            )
            sys.exit(1)

        timings = parse_timing_headers(dict(resp.headers))
        all_server_t = (
            timings["work"] + timings["asset"] + timings["queue"] + timings["net"]
        )
        client_net = max(0, duration_ms - all_server_t)

        print(
            f"[{task_id:04d}/{total_tasks:04d}] Run #{run_idx} | {duration_ms}ms | "
            f"work={timings['work']}ms (dec={timings['decode']} rnd={timings['render']} enc={timings['encode']}) "
            f"queue={timings['queue']}ms | {path[:60]}..."
        )

        return {
            "request": path,
            "run_idx": run_idx,
            "duration_ms": duration_ms,
            "net_ms": timings["net"],
            "queue_ms": timings["queue"],
            "asset_ms": timings["asset"],
            "work_ms": timings["work"],
            "decode_ms": timings["decode"],
            "render_ms": timings["render"],
            "encode_ms": timings["encode"],
            "client_net_ms": client_net,
        }
    except (requests.RequestException, KeyError, ValueError) as e:
        print(
            f"\n[FATAL ERROR] Request {path} (Run #{run_idx}) failed with exception: {e}",
            file=sys.stderr,
        )
        sys.exit(1)


def load_existing_results(csv_path: Path) -> list[dict[str, Any]]:
    """[Dishonest I/O] Reads existing CSV and returns rows with valid timing data."""
    if not csv_path.is_file() or csv_path.stat().st_size == 0:
        return []
    valid_rows: list[dict[str, Any]] = []
    try:
        with csv_path.open("r", newline="", encoding="utf-8") as f:
            reader = csv.DictReader(f)
            for row in reader:
                if is_valid_timing_row(row):
                    valid_rows.append(row)
    except (OSError, csv.Error) as e:
        print(f"[Warning] Failed reading existing CSV {csv_path}: {e}", file=sys.stderr)
        return []
    return valid_rows


def collect_statistics(
    base_url: str,
    requests_list: list[str],
    repetitions: int,
    num_offloads: int,
    output_csv_path: Path,
):
    """[Dishonest Orchestrator] Manages worker-limited request execution,
    sorts rows by request string, and writes output to CSV.
    """
    total_tasks = len(requests_list) * repetitions
    existing_rows = load_existing_results(output_csv_path)
    tasks, _ = filter_pending_tasks(requests_list, repetitions, existing_rows)

    print("=" * 75)
    print("Starting OBM Statistics Collection")
    print(f"Target Base URL:     {base_url}")
    print(f"Unique URLs:         {len(requests_list)}")
    print(f"Repetitions Per URL: {repetitions}")
    print(f"Total Requests:      {total_tasks}")
    if existing_rows:
        print(f"Resumed from CSV:    {len(existing_rows)} valid tasks already recorded")
        print(f"Remaining to Run:    {len(tasks)}")
    print(f"Worker Concurrency:  {num_offloads} (limited by active offloads)")
    print(f"Output CSV File:     {output_csv_path}")
    print("=" * 75)

    if not tasks:
        print(
            f"All {total_tasks} requests already completed in {output_csv_path}. Nothing to do!"
        )
        print("=" * 75)
        return

    results: list[dict[str, Any]] = []
    start_time = time.perf_counter()

    # Initialize CSV file with headers only if the file does not exist or is empty
    if not output_csv_path.exists() or output_csv_path.stat().st_size == 0:
        with output_csv_path.open("w", newline="", encoding="utf-8") as f:
            writer = csv.DictWriter(f, fieldnames=CSV_HEADERS)
            writer.writeheader()

    with (
        requests.Session() as session,
        concurrent.futures.ThreadPoolExecutor(max_workers=num_offloads) as executor,
    ):
        future_to_task = {}
        for i, (path, run_idx) in enumerate(tasks, start=1):
            f = executor.submit(
                fetch_request,
                session,
                base_url,
                path,
                run_idx,
                i,
                len(tasks),
            )
            future_to_task[f] = (path, run_idx)

        batch_buffer: list[dict[str, Any]] = []
        BATCH_SIZE = 10

        for future in concurrent.futures.as_completed(future_to_task):
            row = future.result()
            results.append(row)
            batch_buffer.append(row)

            if len(batch_buffer) >= BATCH_SIZE:
                with output_csv_path.open("a", newline="", encoding="utf-8") as f:
                    writer = csv.DictWriter(f, fieldnames=CSV_HEADERS)
                    writer.writerows(batch_buffer)
                batch_buffer.clear()

        # Flush any remaining rows in the buffer
        if batch_buffer:
            with output_csv_path.open("a", newline="", encoding="utf-8") as f:
                writer = csv.DictWriter(f, fieldnames=CSV_HEADERS)
                writer.writerows(batch_buffer)
            batch_buffer.clear()

    total_time = time.perf_counter() - start_time
    print("-" * 75)
    print(
        f"Completed {len(results)} remaining requests successfully in {total_time:.2f}s."
    )
    print("Sorting collected data by request URL...")

    # Combine existing valid rows + newly collected rows
    all_results = existing_rows + results
    # Sort dataset by request string, then by run_idx
    all_results.sort(key=lambda r: (r["request"], int(r["run_idx"])))

    print(f"Writing {len(all_results)} sorted rows to {output_csv_path}...")
    with output_csv_path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=CSV_HEADERS)
        writer.writeheader()
        writer.writerows(all_results)

    print(f"[Done] CSV saved successfully to: {output_csv_path.resolve()}")
    print("=" * 75)


# ==============================================================================
# 3. CLI Entry Point
# ==============================================================================


def parse_args() -> argparse.Namespace:
    """[Dishonest I/O] Parses CLI arguments."""
    parser = argparse.ArgumentParser(
        description="Benchmark the first segment of each request kind on one offload site."
    )
    parser.add_argument(
        "port", type=int, help="offload site port on localhost (e.g. 7010)"
    )
    parser.add_argument(
        "config",
        choices=["cpu", "llvm", "gpu"],
        help="composition tier of the offload site ('cpu' runs only 8 representative kinds)",
    )
    args = parser.parse_args()
    if not 1 <= args.port <= 65535:
        parser.error(f"port must be 1-65535, got {args.port}")
    return args


def main():
    args = parse_args()

    # =========================================================================
    # Configuration Variables (Edit directly here)
    # =========================================================================

    # 1. Target offload site, bypassing the main server load balancer
    base_url = f"http://localhost:{args.port}"

    # 2. Number of times each request URL is executed
    repetitions = 1

    # 3. One in-flight request at a time, so the site never queues
    num_offloads = 1

    # 4. Input file path next to this script
    script_dir = Path(__file__).resolve().parent
    requests_file = script_dir / "request_kind.txt"

    # 5. Output CSV file path, separate per port/config (resume logic is per file)
    output_csv = script_dir / f"{args.port}_{args.config}_timing.csv"

    # =========================================================================
    # Run Statistics Collection
    # =========================================================================
    try:
        kinds = select_request_kinds(load_requests_file(requests_file), args.config)
    except ValueError as e:
        print(f"[Error] {e}", file=sys.stderr)
        sys.exit(1)
    requests_list = first_segment_paths(kinds)
    collect_statistics(
        base_url=base_url,
        requests_list=requests_list,
        repetitions=repetitions,
        num_offloads=num_offloads,
        output_csv_path=output_csv,
    )


if __name__ == "__main__":
    main()
