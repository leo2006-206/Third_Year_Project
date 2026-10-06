#!/usr/bin/env python3
"""
Simple OBM Request Benchmark Harness
Models request patterns over time (duration & total_requests) using `requests`.
"""

import concurrent.futures
import json
import re
import subprocess
import threading
import time
from collections.abc import Callable
from typing import Any

import requests
import urllib3

# Suppress SSL verification warnings when testing domains with underscores
urllib3.disable_warnings(urllib3.exceptions.InsecureRequestWarning)

# Default public OBM base URL
DEFAULT_BASE_URL = "https://obm_main.leowong.space"

# Representative segment paths from the F1 show
DEFAULT_REQUESTS = [
    "/offload/show/f1_full.json/0/10/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam",
    "/offload/show/f1_full.json/10/20/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam",
    "/offload/show/f1_full.json/20/30/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam",
    "/offload/show/f1_full.json/30/40/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam",
]


# ==============================================================================
# 1. Honest Functions (Pure Domain Logic)
# ==============================================================================


def make_loop_pattern(list_len: int) -> Callable[[], int]:
    """[Honest] Creates a stateful callback that cycles sequentially: 0, 1, ..., N-1, 0..."""
    counter = 0

    def pattern() -> int:
        nonlocal counter
        idx = counter % list_len
        counter += 1
        return idx

    return pattern


def make_repeat_pattern(fixed_idx: int) -> Callable[[], int]:
    """[Honest] Creates a callback that always returns the same request index."""
    return lambda: fixed_idx


def parse_timing_headers(headers: dict[str, str]) -> dict[str, int]:
    """[Honest] Extracts timing metrics (ms) from HTTP response headers."""
    timings = {"queue": 0, "asset": 0, "work": 0, "net": 0}

    # Parse X-Dana-Timings: queue=X;asset=Y;work=Z
    dana_header = headers.get("x-dana-timings") or headers.get("X-Dana-Timings", "")
    match = re.search(r"queue=(\d+);asset=(\d+);work=(\d+)", dana_header)
    if match:
        timings["queue"] = int(match.group(1))
        timings["asset"] = int(match.group(2))
        timings["work"] = int(match.group(3))

    # Parse X-OBM-Net: N
    obm_net = headers.get("x-obm-net") or headers.get("X-OBM-Net", "")
    if obm_net.isdigit():
        timings["net"] = int(obm_net)

    return timings


def compute_summary(results: list[dict[str, Any]]) -> dict[str, Any]:
    """[Honest] Computes statistical summary across all completed request results."""
    if not results:
        return {}

    successful = [r for r in results if r["status"] == 200]
    latencies = [r["duration_ms"] for r in successful]

    return {
        "total": len(results),
        "successful": len(successful),
        "failed": len(results) - len(successful),
        "avg_duration_ms": round(sum(latencies) / len(latencies), 1)
        if latencies
        else 0,
        "min_duration_ms": min(latencies) if latencies else 0,
        "max_duration_ms": max(latencies) if latencies else 0,
        "avg_work_ms": (
            round(sum(r["work_ms"] for r in successful) / len(successful), 1)
            if successful
            else 0
        ),
        "avg_asset_ms": (
            round(sum(r["asset_ms"] for r in successful) / len(successful), 1)
            if successful
            else 0
        ),
        "avg_queue_ms": (
            round(sum(r["queue_ms"] for r in successful) / len(successful), 1)
            if successful
            else 0
        ),
        "avg_net_ms": (
            round(sum(r["net_ms"] for r in successful) / len(successful), 1)
            if successful
            else 0
        ),
    }


# ==============================================================================
# 2. Dishonest Functions (I/O Drivers & Boundary Adapters)
# ==============================================================================


class ContainerMonitor:
    """[Dishonest I/O] Background thread sampling docker stats for offload containers."""

    def __init__(
        self,
        containers: list[str],
        sample_interval: float = 1.0,
        live_print: bool = True,
    ):
        self.containers = containers
        self.sample_interval = sample_interval
        self.live_print = live_print
        self._stop_event = threading.Event()
        self._thread: threading.Thread | None = None
        self.snapshots: list[dict[str, Any]] = []

    def start(self):
        self._stop_event.clear()
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def stop(self):
        self._stop_event.set()
        if self._thread:
            self._thread.join(timeout=2.0)

    def _run(self):
        while not self._stop_event.is_set():
            try:
                cmd = [
                    "docker",
                    "stats",
                    "--no-stream",
                    "--format",
                    "{{json .}}",
                ] + self.containers
                proc = subprocess.run(cmd, capture_output=True, text=True, timeout=2.0)
                if proc.returncode == 0:
                    entry = {"time": time.time(), "containers": {}}
                    for line in proc.stdout.strip().splitlines():
                        if not line:
                            continue
                        data = json.loads(line)
                        entry["containers"][data.get("Name", "")] = {
                            "cpu": data.get("CPUPerc", "0%"),
                            "mem": data.get("MemUsage", "0"),
                        }
                    self.snapshots.append(entry)

                    if self.live_print and entry["containers"]:
                        status_parts = [
                            f"{name.replace('obm-', '')}: {st['cpu']} ({st['mem'].split('/')[0].strip()})"
                            for name, st in sorted(entry["containers"].items())
                        ]
                        print(f"  [Live Stats] {' | '.join(status_parts)}")
            except Exception:
                pass
            time.sleep(self.sample_interval)


def send_single_request(req_id: int, url: str) -> dict[str, Any]:
    """[Dishonest I/O] Sends a single HTTP GET request using `requests` and records timings."""
    t0 = time.perf_counter()
    try:
        resp = requests.get(url, timeout=30.0, verify=False)
        duration_ms = round((time.perf_counter() - t0) * 1000)
        timings = parse_timing_headers(dict(resp.headers))

        result = {
            "id": req_id,
            "url": url,
            "status": resp.status_code,
            "duration_ms": duration_ms,
            "bytes": len(resp.content),
            "queue_ms": timings["queue"],
            "asset_ms": timings["asset"],
            "work_ms": timings["work"],
            "net_ms": timings["net"],
        }
        print(
            f"[Req #{req_id:02d}] {resp.status_code} | {duration_ms / 1000:.2f}s | "
            f"{len(resp.content) / (1024 * 1024):.2f}MB | "
            f"work={timings['work']}ms asset={timings['asset']}ms queue={timings['queue']}ms net={timings['net']}ms"
        )
        return result
    except Exception as e:
        duration_ms = round((time.perf_counter() - t0) * 1000)
        print(f"[Req #{req_id:02d}] FAILED after {duration_ms}ms: {e}")
        return {
            "id": req_id,
            "url": url,
            "status": 0,
            "duration_ms": duration_ms,
            "bytes": 0,
            "queue_ms": 0,
            "asset_ms": 0,
            "work_ms": 0,
            "net_ms": 0,
            "error": str(e),
        }


def run_benchmark(
    base_url: str,
    requests_list: list[str],
    pattern_fn: Callable[[], int],
    duration: float,
    total_requests: int,
    monitor_containers: list[str] | None = None,
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    """[Dishonest I/O Orchestrator] Fires requests according to the schedule."""
    interval = duration / max(total_requests, 1)
    results: list[dict[str, Any]] = []

    print(
        f"\nStarting benchmark: {total_requests} requests over {duration:.1f}s "
        f"(1 req every {interval:.3f}s / {total_requests / duration:.2f} req/s)"
    )
    print(f"Target Base URL: {base_url}")
    print("-" * 75)

    monitor = None
    if monitor_containers:
        monitor = ContainerMonitor(monitor_containers, sample_interval=1.0)
        monitor.start()

    start_time = time.perf_counter()
    # Use ThreadPoolExecutor so rendering delays do not stall the firing schedule
    with concurrent.futures.ThreadPoolExecutor(
        max_workers=min(32, total_requests)
    ) as executor:
        futures = []
        for i in range(total_requests):
            req_idx = pattern_fn()
            target_url = base_url.rstrip("/") + requests_list[req_idx]

            # Submit request asynchronously
            futures.append(executor.submit(send_single_request, i + 1, target_url))

            # Wait until next scheduled dispatch time
            target_next = start_time + (i + 1) * interval
            sleep_time = target_next - time.perf_counter()
            if sleep_time > 0 and i < total_requests - 1:
                time.sleep(sleep_time)

        # Await completion of all active requests
        for future in concurrent.futures.as_completed(futures):
            results.append(future.result())

    if monitor:
        monitor.stop()

    results.sort(key=lambda r: r["id"])
    summary = compute_summary(results)

    # Print summary
    print("-" * 75)
    print("Benchmark Summary:")
    print(
        f"  Requests Total:   {summary.get('total', 0)} (Success: {summary.get('successful', 0)}, Failed: {summary.get('failed', 0)})"
    )
    print(f"  Avg Latency:      {summary.get('avg_duration_ms', 0)} ms")
    print(
        f"  Min / Max:        {summary.get('min_duration_ms', 0)} ms / {summary.get('max_duration_ms', 0)} ms"
    )
    print(f"  Avg Render(work): {summary.get('avg_work_ms', 0)} ms")
    print(f"  Avg Asset(asset): {summary.get('avg_asset_ms', 0)} ms")
    print(f"  Avg Queue(queue): {summary.get('avg_queue_ms', 0)} ms")
    print(f"  Avg Network(net): {summary.get('avg_net_ms', 0)} ms")

    if monitor and monitor.snapshots:
        print("\nOffload Container Utilization Summary:")
        print(
            f"  {'Container':<18} {'Peak CPU':<12} {'Avg CPU':<12} {'Peak Memory':<15}"
        )
        print("  " + "-" * 57)
        container_stats: dict[str, list[float]] = {}
        container_mems: dict[str, str] = {}
        for snap in monitor.snapshots:
            for name, stats in snap["containers"].items():
                try:
                    cpu_val = float(stats["cpu"].rstrip("%"))
                    container_stats.setdefault(name, []).append(cpu_val)
                    container_mems[name] = stats["mem"].split("/")[0].strip()
                except ValueError:
                    pass

        for name in sorted(container_stats.keys()):
            cpus = container_stats[name]
            peak_cpu = f"{max(cpus):.1f}%"
            avg_cpu = f"{(sum(cpus) / len(cpus)):.1f}%"
            peak_mem = container_mems.get(name, "-")
            print(f"  {name:<18} {peak_cpu:<12} {avg_cpu:<12} {peak_mem:<15}")

    print("-" * 75)
    return results, summary


# ==============================================================================
# 3. CLI Entry Point
# ==============================================================================


def main():
    # =========================================================================
    # Benchmark Parameters (Edit directly here)
    # =========================================================================

    # 1. Target server base URL
    base_url = "https://obm_main.leowong.space"

    # 2. Timing and request volume
    #    e.g. duration = 10.0 and total_requests = 10 -> 1 request every 1.0s
    #    e.g. duration = 10.0 and total_requests = 100 -> 10 requests every 1.0s
    duration = 30.0
    total_requests = 10

    # 3. List of request paths to benchmark
    requests_list = [
        "/offload/show/f1_full.json/0/10/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam",
        "/offload/show/f1_full.json/10/20/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam",
        "/offload/show/f1_full.json/20/30/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam",
        "/offload/show/f1_full.json/30/40/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam",
    ]

    # 4. Request pattern callback
    #    - make_loop_pattern: cycles through all URLs sequentially (0, 1, 2, 3, 0...)
    #    - make_repeat_pattern: repeatedly requests the same URL index (e.g. 0)
    pattern_fn = make_loop_pattern(len(requests_list))
    # pattern_fn = make_repeat_pattern(0)

    # 5. Containers to monitor CPU/memory for during run (or set to [] to disable)
    containers = ["obm-offload-1", "obm-offload-2", "obm-offload-3"]

    # =========================================================================
    # Run Benchmark
    # =========================================================================
    run_benchmark(
        base_url=base_url,
        requests_list=requests_list,
        pattern_fn=pattern_fn,
        duration=duration,
        total_requests=total_requests,
        monitor_containers=containers,
    )


if __name__ == "__main__":
    main()
