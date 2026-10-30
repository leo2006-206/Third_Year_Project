#!/usr/bin/env python3
"""
Exports the cost model for the Rust load balancer (rust_obm/src/cost_predict.rs):

  stat_predict/coef.csv      stage,backend,a,<feature...>     one row per (stage, backend) stage model
  stat_predict/features.csv  <feature...>,path               one row per request path (segment included)

In a real deployment features are extracted when an (unseen) request arrives; for the experiments
they are precomputed here for every BBC and synthetic request kind, every segment of the show, plus
every path that appears in the sequences and measurements. `path` is the last column so Rust can split
each line a fixed number of times and keep commas / spaces in the path without quoting.

Usage: stat_predict/export.py [MODEL_JSON]   (default stat_predict/model_all_runs.json)
"""
import csv
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from features import ALL, FPS, SEG_S, features, load_json, parse_request  # noqa: E402
from model import STAGES, CostModel  # noqa: E402

ROOT = HERE.parent


def segment_paths(kind: str) -> list[str]:
    """[Dishonest I/O] One path per segment of the show variant (t_start/t_end placeholder filled)."""
    r = parse_request(kind.replace("t_start/t_end", "0/10", 1))
    spec = load_json(f"shows/{r['show']}")
    v = next(x for x in spec["variants"] if x["name"] == r["variant"] and x.get("width", r["w"]) == r["w"])
    n_seg = -(-v["length_frames"] // (SEG_S * FPS))
    return [kind.replace("t_start/t_end", f"{i * SEG_S}/{(i + 1) * SEG_S}", 1) for i in range(n_seg)]


def all_paths() -> list[str]:
    """[Dishonest I/O] Request kinds x segments, plus every path in sequences and measurements."""
    paths: set[str] = set()
    for kinds in [ROOT / "servers_benchmark/request_kind.txt", ROOT / "syn_video/request_kind_syn.txt"]:
        for k in filter(None, (line.strip() for line in kinds.read_text().splitlines())):
            paths.update(segment_paths(k))
    for f in list((ROOT / "servers_benchmark/sequences").glob("seq_a*_case*.csv")) + list((ROOT / "stat").glob("run_*/*.csv")) \
            + [ROOT / "stat/syn_timings.csv"]:
        with f.open() as fh:
            paths.update(row["path"] for row in csv.DictReader(fh))
    return sorted(paths)


def main():
    model = CostModel.load(Path(sys.argv[1]) if len(sys.argv) > 1 else HERE / "model_all_runs.json")
    feats = list(dict.fromkeys(f for fs in STAGES.values() for f in fs))
    with (HERE / "coef.csv").open("w") as fh:
        fh.write(",".join(["stage", "backend", "a"] + feats) + "\n")
        for key, c in sorted(model.coef.items()):
            stage, backend = key.split("|")
            fh.write(",".join([stage, backend, f"{c['a']:.6g}"] + [f"{c.get(f, 0.0):.6g}" for f in feats]) + "\n")
    rows, skipped = 0, 0
    with (HERE / "features.csv").open("w") as fh:
        fh.write(",".join(ALL + ["path"]) + "\n")
        for p in all_paths():
            try:
                f = features(p)
            except (FileNotFoundError, StopIteration, KeyError, ValueError):
                skipped += 1  # segment file missing / malformed path
                continue
            fh.write(",".join(f"{f[k]:.6g}" for k in ALL) + "," + p + "\n")
            rows += 1
    print(f"coef.csv: {len(model.coef)} stage models; features.csv: {rows} paths ({skipped} skipped); "
          f"machine factor (for the load balancer, not exported): {model.machine_factor}")


if __name__ == "__main__":
    main()
