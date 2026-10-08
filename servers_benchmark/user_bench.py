#!/usr/bin/env python3
"""
Simple OBM Request Benchmark Harness
Models request patterns over time (duration & total_requests) using `requests`.
"""

import concurrent.futures
import json
import math
import re
import subprocess
import threading
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

import requests
import urllib3

# Suppress SSL verification warnings when testing domains with underscores
urllib3.disable_warnings(urllib3.exceptions.InsecureRequestWarning)

# Default public OBM base URL
DEFAULT_BASE_URL = "https://obm_main.leowong.space"

# Representative segment paths from the F1 show (fallback)
DEFAULT_REQUESTS = [
    "/offload/show/f1_full.json/0/10/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam",
    "/offload/show/f1_full.json/10/20/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam",
    "/offload/show/f1_full.json/20/30/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam",
    "/offload/show/f1_full.json/30/40/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam",
]


def load_requests_file(file_path: Path | str | None = None) -> list[str]:
    """[Dishonest / File Loader]
    Loads request URLs from a newline-delimited requests.txt file.
    Searches provided path, local directory, or testing_webpage directory.
    Falls back to DEFAULT_REQUESTS if not found.
    """
    candidates = [
        Path(file_path) if file_path else None,
        Path(__file__).resolve().parent / "requests.txt",
        Path(__file__).resolve().parents[1]
        / "servers_rust"
        / "src"
        / "testing_webpage"
        / "requests.txt",
    ]
    for cand in candidates:
        if cand and cand.is_file():
            lines = [
                line.strip()
                for line in cand.read_text(encoding="utf-8").splitlines()
                if line.strip() and not line.startswith("#")
            ]
            if lines:
                return lines
    return DEFAULT_REQUESTS


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


class MeanStd:
    """[Honest / Pure Domain Logic]
    Statistical accumulator tracking count, sum, sum_sq, min, and max.
    Computes mean, variance, and standard deviation.
    """

    def __init__(self):
        self.count: int = 0
        self.sum: float = 0.0
        self.sum_sq: float = 0.0
        self.min_val: float = float("inf")
        self.max_val: float = float("-inf")

    def add(self, x: float | int):
        val = float(x)
        self.count += 1
        self.sum += val
        self.sum_sq += val * val
        if val < self.min_val:
            self.min_val = val
        if val > self.max_val:
            self.max_val = val

    def reset(self):
        self.count = 0
        self.sum = 0.0
        self.sum_sq = 0.0
        self.min_val = float("inf")
        self.max_val = float("-inf")

    def get_mean(self) -> float:
        return (self.sum / self.count) if self.count > 0 else 0.0

    def get_var(self) -> float:
        if self.count == 0:
            return 0.0
        mean = self.sum / self.count
        mean_of_squares = self.sum_sq / self.count
        variance = mean_of_squares - (mean * mean)
        return max(0.0, variance)

    def get_std(self) -> float:
        return math.sqrt(self.get_var())

    def get_min(self) -> float:
        return self.min_val if self.count > 0 else 0.0

    def get_max(self) -> float:
        return self.max_val if self.count > 0 else 0.0

    def as_dict(self) -> dict[str, float]:
        return {
            "avg": round(self.get_mean(), 1),
            "min": round(self.get_min(), 1),
            "max": round(self.get_max(), 1),
            "std": round(self.get_std(), 1),
        }


def parse_mem_str_to_mb(mem_str: str) -> float:
    """[Honest] Parses memory string (e.g. '432.6MiB', '1.2GiB') into megabytes."""
    used = mem_str.split("/")[0].strip().lower()
    num = float(re.sub(r"[^\d.]", "", used) or 0)
    if "gib" in used or "gb" in used:
        return num * 1024.0
    elif "kib" in used or "kb" in used:
        return num / 1024.0
    return num


METRICS = [
    ("duration", "Latency (total)", "duration_ms"),
    ("work", "Render (work)", "work_ms"),
    ("asset", "Asset (asset)", "asset_ms"),
    ("queue", "Queue (queue)", "queue_ms"),
    ("net", "Server Net (net)", "net_ms"),
    ("client_net", "Client Net (c_net)", "client_net_ms"),
]


def compute_summary(results: list[dict[str, Any]]) -> dict[str, Any]:
    """[Honest] Computes statistical summary across all completed request results."""
    if not results:
        return {}

    successful = [r for r in results if r["status"] == 200]
    stats_map = {key: MeanStd() for key, _, _ in METRICS}

    for r in successful:
        for key, _, field in METRICS:
            stats_map[key].add(r[field])

    summary = {
        "total": len(results),
        "successful": len(successful),
        "failed": len(results) - len(successful),
    }
    summary.update({k: v.as_dict() for k, v in stats_map.items()})
    return summary


# ==============================================================================
# 2. Dishonest Functions (I/O Drivers & Boundary Adapters)
# ==============================================================================


class ContainerMonitor:
    """[Dishonest I/O] Background thread streaming docker stats for offload containers."""

    def __init__(
        self,
        containers: list[str],
    ):
        self.containers = containers
        self._proc: subprocess.Popen | None = None
        self._thread: threading.Thread | None = None
        self.snapshots: list[dict[str, Any]] = []
        self.start_time: float | None = None

    def start(self):
        self.start_time = time.time()
        cmd = ["docker", "stats", "--format", "{{json .}}"] + self.containers
        try:
            self._proc = subprocess.Popen(
                ["stdbuf", "-oL"] + cmd,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
                bufsize=1,
            )
        except FileNotFoundError:
            self._proc = subprocess.Popen(
                cmd,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
                bufsize=1,
            )

        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def stop(self):
        if self._proc:
            try:
                self._proc.terminate()
                self._proc.wait(timeout=2.0)
            except Exception:
                try:
                    self._proc.kill()
                except Exception:
                    pass
        if self._thread:
            self._thread.join(timeout=1.0)

    def _run(self):
        if not self._proc or not self._proc.stdout:
            return
        sample = {}
        ansi_re = re.compile(r"\x1b\[[0-9;]*[a-zA-Z]")
        while True:
            line = self._proc.stdout.readline()
            if not line:
                break
            clean = ansi_re.sub("", line).strip()
            if not clean or not clean.startswith("{") or not clean.endswith("}"):
                continue
            try:
                data = json.loads(clean)
                name = data.get("Name", "")
                if name:
                    sample[name] = {
                        "cpu": data.get("CPUPerc", "0%"),
                        "mem": data.get("MemUsage", "0"),
                    }
                    if len(sample) >= len(self.containers):
                        now = time.time()
                        elapsed = (now - self.start_time) if self.start_time else 0.0
                        self.snapshots.append(
                            {
                                "elapsed": elapsed,
                                "time": now,
                                "containers": sample.copy(),
                            }
                        )
                        sample.clear()
            except Exception:
                pass


def send_single_request(req_id: int, url: str) -> dict[str, Any]:
    """[Dishonest I/O] Sends a single HTTP GET request using `requests` and records timings."""
    t0 = time.perf_counter()
    try:
        resp = requests.get(url, timeout=30.0, verify=False)
        duration_ms = round((time.perf_counter() - t0) * 1000)
        timings = parse_timing_headers(dict(resp.headers))

        all_offload_t = (
            timings["work"] + timings["asset"] + timings["queue"] + timings["net"]
        )
        client_net = max(0, duration_ms - all_offload_t)

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
            "client_net_ms": client_net,
        }
        print(
            f"[Req #{req_id:02d}] {resp.status_code} | {duration_ms / 1000:.2f}s | "
            f"{len(resp.content) / (1024 * 1024):.2f}MB | "
            f"work={timings['work']}ms asset={timings['asset']}ms queue={timings['queue']}ms "
            f"net={timings['net']}ms c_net={client_net}ms"
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
            "client_net_ms": 0,
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
    print(f"Request Pool:    {len(requests_list)} unique URL(s) loaded")
    print("-" * 75)

    monitor = None
    if monitor_containers:
        monitor = ContainerMonitor(monitor_containers)
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
        f"  Requests Total:   {summary.get('total', 0)} (Success: {summary.get('successful', 0)}, Failed: {summary.get('failed', 0)})\n"
    )
    metric_labels = [(label, key) for key, label, _ in METRICS]
    print(f"  {'Metric':<20} {'Avg':<12} {'Min':<12} {'Max':<12} {'Std':<12}")
    print("  " + "-" * 68)
    for label, key in metric_labels:
        st = summary.get(key, {})
        print(
            f"  {label:<20} {st.get('avg', 0.0):.1f} ms   "
            f"{st.get('min', 0.0):.1f} ms   "
            f"{st.get('max', 0.0):.1f} ms   "
            f"{st.get('std', 0.0):.1f} ms"
        )

    if monitor and monitor.snapshots:
        all_cnames = sorted(
            {c for snap in monitor.snapshots for c in snap["containers"]}
        )

        # 1. Timeline Log (Sampled ~0.5s buffered in memory, formatted as SS.MS)
        print("\nOffload Container Utilization Timeline:")
        hdr = f"  {'Time':<8} " + " ".join(
            f"{c.replace('obm-', ''):<24}" for c in all_cnames
        )
        print(hdr)
        print("  " + "-" * max(len(hdr) - 2, 57))
        for snap in monitor.snapshots:
            elapsed = snap.get("elapsed", 0.0)
            t_str = f"{elapsed:04.1f}s"
            row_items = []
            for c in all_cnames:
                st = snap["containers"].get(c, {})
                cpu = st.get("cpu", "0%")
                mem = st.get("mem", "0").split("/")[0].strip()
                row_items.append(f"{cpu} ({mem})")
            print(f"  {t_str:<8} " + " ".join(f"{item:<24}" for item in row_items))

        # 2. Utilization Aggregate Summary using MeanStd
        print("\nOffload Container Utilization Summary:")
        print(
            f"  {'Container':<18} {'Avg CPU':<12} {'Min CPU':<12} {'Peak CPU':<12} {'Std CPU':<12} {'Peak Memory':<15}"
        )
        print("  " + "-" * 81)
        container_stats: dict[str, MeanStd] = {}
        container_peak_mems: dict[str, float] = {}

        for snap in monitor.snapshots:
            for name, stats in snap["containers"].items():
                try:
                    cpu_val = float(stats["cpu"].rstrip("%"))
                    container_stats.setdefault(name, MeanStd()).add(cpu_val)
                    mem_mb = parse_mem_str_to_mb(stats["mem"])
                    if mem_mb > container_peak_mems.get(name, 0.0):
                        container_peak_mems[name] = mem_mb
                except ValueError:
                    pass

        for name in sorted(container_stats.keys()):
            cpu_acc = container_stats[name]
            avg_cpu = f"{cpu_acc.get_mean():.1f}%"
            min_cpu = f"{cpu_acc.get_min():.1f}%"
            peak_cpu = f"{cpu_acc.get_max():.1f}%"
            std_cpu = f"{cpu_acc.get_std():.1f}%"
            peak_mb = container_peak_mems.get(name, 0.0)
            peak_mem = (
                f"{peak_mb / 1024:.2f}GiB" if peak_mb >= 1024 else f"{peak_mb:.1f}MiB"
            )
            print(
                f"  {name:<18} {avg_cpu:<12} {min_cpu:<12} {peak_cpu:<12} {std_cpu:<12} {peak_mem:<15}"
            )

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
    duration = 10
    total_requests = 3

    # 3. List of request paths to benchmark (defaults to all URLs from requests.txt)
    requests_list = load_requests_file()

    # 4. Request pattern callback
    #    - make_loop_pattern: cycles through all URLs sequentially (0, 1, 2, 3, 0...)
    #    - make_repeat_pattern: repeatedly requests the same URL index (e.g. 0)
    pattern_fn = make_loop_pattern(len(requests_list))
    # pattern_fn = make_repeat_pattern(0)

    # 5. Containers to monitor CPU/memory for during run (or set to [] to disable)
    containers = ["obm-offload-1", "obm-offload-2"]

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
