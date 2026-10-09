#!/usr/bin/env python3
"""
OBM Request Statistics Collector
Dispatches each URL from requests.txt 3 times with worker-limited concurrency
(matching the number of active offload servers to avoid queueing).
Extracts fine-grained timing breakdowns (decode, render, encode, etc.),
sorts the dataset by request URL, and writes directly to CSV.
"""

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


# ==============================================================================
# 2. Dishonest Functions (I/O Drivers & Boundary Adapters)
# ==============================================================================


def load_requests_file(file_path: Path) -> list[str]:
    """[Dishonest I/O] Reads request URLs from requests.txt."""
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
        resp = session.get(url, timeout=30.0, verify=False)
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
    except Exception as e:
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
    except Exception as e:
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

    with requests.Session() as session:
        with concurrent.futures.ThreadPoolExecutor(
            max_workers=num_offloads
        ) as executor:
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


def main():
    # =========================================================================
    # Configuration Variables (Edit directly here)
    # =========================================================================

    # 1. Target server base URL (Main Server with Round-Robin Load Balancer)
    base_url = "https://obm_main.leowong.space"

    # 2. Number of times each request URL is executed
    repetitions = 1

    # 3. Number of active offload servers (limits concurrent in-flight requests)
    num_offloads = 2

    # 4. Input file path next to this script
    script_dir = Path(__file__).resolve().parent
    requests_file = script_dir / "requests.txt"

    # 5. Output CSV file path
    output_csv = script_dir / "request_timings.csv"

    # =========================================================================
    # Run Statistics Collection
    # =========================================================================
    requests_list = load_requests_file(requests_file)
    collect_statistics(
        base_url=base_url,
        requests_list=requests_list,
        repetitions=repetitions,
        num_offloads=num_offloads,
        output_csv_path=output_csv,
    )


if __name__ == "__main__":
    main()
