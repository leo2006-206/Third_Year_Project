#!/usr/bin/env python3
"""
Lookup-table work predictor for the Rust load balancer (rust_obm/src/cost_lookup.rs):

  stat_predict/lookup.csv   config,time_s,n,path   one row per (site config, request path)

time_s is the measured work (decode + render + encode, the site's work_ms) of that path on that
config, in om-equivalent seconds: rs samples are divided by the rs/om work ratio of their config
(median over the paths measured on both machines), then the median of all samples is taken. The
machine factor of the load balancer turns it back into per-site time, as for the cost model.
config is <decoding>_<composition>_<encoding> as in the cluster CSVs (e.g. gpu_llvmpipe_cpu).

Data: every replay run of set 2 (stat/seed2037/run_a*_c*/) and set 1 (stat/set1_unload/run_a*_c*/), work_ms
of the requests that succeeded; the worker column is mapped to its site through
servers_container/cluster_endpoint_c<case>.csv. Set 1 and set 2 agree on the same keys (median
set1/set2 0.985, p10-p90 0.90-1.08), work_ms is not affected by the Dana asset timer change.

Usage: stat_predict/lookup_table.py
"""

import csv
import statistics
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
RUN_GLOBS = ["stat/seed2037/run_a*_c*/*_none_*.csv", "stat/set1_unload/run_a*_c*/*_none_*.csv"]


def load_sites(case: str) -> dict[str, tuple[str, str]]:
    """[Dishonest I/O] endpoint "ip:port" -> (machine, config) for cluster c<case>."""
    path = ROOT / f"servers_container/cluster_endpoint_c{case}.csv"
    with path.open() as fh:
        rows = csv.DictReader(line for line in fh if not line.startswith("#"))
        return {
            f"{r['ip']}:{r['port']}": (
                r["id"][:2],
                f"{r['decoding']}_{r['composition']}_{r['encoding']}",
            )
            for r in rows
        }


def load_samples() -> list[tuple[str, str, str, float]]:
    """[Dishonest I/O] (machine, config, path, work s) of every successful replay request."""
    samples = []
    for pattern in RUN_GLOBS:
        for f in sorted(ROOT.glob(pattern)):
            case = f.parent.name.rsplit("_c", 1)[1]
            sites = load_sites(case)
            with f.open() as fh:
                for r in csv.DictReader(fh):
                    if r["status"] != "200" or float(r["work_ms"] or 0) <= 0:
                        continue
                    machine, config = sites[r["worker"]]
                    samples.append(
                        (machine, config, r["path"], float(r["work_ms"]) / 1000)
                    )
    return samples


def machine_ratio(samples: list[tuple[str, str, str, float]]) -> dict[str, float]:
    """[Honest] Per config: median over paths measured on both machines of median(rs) / median(om)."""
    by_key = defaultdict(list)
    for machine, config, path, t in samples:
        by_key[(machine, config, path)].append(t)
    ratios = defaultdict(list)
    for (machine, config, path), ts in by_key.items():
        if machine == "rs" and ("om", config, path) in by_key:
            ratios[config].append(
                statistics.median(ts) / statistics.median(by_key[("om", config, path)])
            )
    return {config: statistics.median(r) for config, r in ratios.items()}


def lookup_table(samples, ratio) -> dict[tuple[str, str], list[float]]:
    """[Honest] (config, path) -> om-equivalent work samples (rs divided by its config's ratio)."""
    table = defaultdict(list)
    for machine, config, path, t in samples:
        table[(config, path)].append(t / ratio[config] if machine == "rs" else t)
    return table


def main():
    samples = load_samples()
    ratio = machine_ratio(samples)
    table = lookup_table(samples, ratio)
    out = HERE / "lookup.csv"
    with out.open("w") as fh:
        fh.write("config,time_s,n,path\n")
        for (config, path), ts in sorted(table.items()):
            fh.write(f"{config},{statistics.median(ts):.4f},{len(ts)},{path}\n")
    print(f"{len(samples)} samples -> {len(table)} (config, path) rows -> {out}")
    print(
        "rs/om work ratio: "
        + ", ".join(f"{c} {r:.3f}" for c, r in sorted(ratio.items()))
    )


if __name__ == "__main__":
    main()
