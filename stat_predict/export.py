#!/usr/bin/env python3
"""
Exports the cost model for the Rust load balancer (rust_obm/src/cost_predict.rs):

  stat_predict/coef.csv      stage,backend,a,<feature...>     one row per (stage, backend) stage model
  stat_predict/features.csv  <feature...>,path               one row per request path (segment included)

PSC (partial segment cache) rows, for every valid cut of every kind x segment (cuts: stat_predict/psc_cuts.py):
  '<k>'   sub segment = roots 0..k-1 only (features of the partial request, as before)
  '<k>+<n>' rebuild = roots k..n-1 drawn over the cached sub (rebuild_features: full minus sub + one full-frame video)
  '<k>*<n>' pair has NO row: its cost = decode + render + encode of the full + encode of the sub, computed in Rust
            (CostPredictor::predict_pair) from the full row and the '<k>' row.

In a real deployment features are extracted when an (unseen) request arrives; for the experiments
they are precomputed here for every BBC and synthetic request kind, every segment of the show, plus
every path that appears in the sequences and measurements. `path` is the last column so Rust can split
each line a fixed number of times and keep commas / spaces in the path without quoting.

Usage: stat_predict/export.py [MODEL_JSON]   (default stat_predict/model_all_runs.json)
       stat_predict/export.py [MODEL_JSON] --kinds FILE [--kinds FILE ..] --out-dir DIR [--asset-root DIR ..]
         = features.csv for ANOTHER kind list (kinds x segments + PSC rows only; no sequence / measured paths, no coef.csv:
           the model is the same) written to DIR/features.csv. The default stat_predict/coef.csv + features.csv are not
           touched. Example (ideal-world fake PSC show): --kinds syn_video/PSC_ideal/request_kind_psc.txt
           --out-dir stat_predict/psc_ideal --asset-root syn_video/PSC_ideal
"""
import argparse
import csv
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import features as features_mod  # noqa: E402
from features import ALL, FPS, SEG_S, features, load_json, parse_request  # noqa: E402
from model import STAGES, CostModel  # noqa: E402
from psc_cuts import cut_rows  # noqa: E402

ROOT = HERE.parent
KIND_FILES = [ROOT / "servers_benchmark/request_kind.txt", ROOT / "syn_video/request_kind_syn.txt"]


def segment_paths(kind: str) -> list[str]:
    """[Dishonest I/O] One path per segment of the show variant (t_start/t_end placeholder filled)."""
    r = parse_request(kind.replace("t_start/t_end", "0/10", 1))
    spec = load_json(f"shows/{r['show']}")
    v = next(x for x in spec["variants"] if x["name"] == r["variant"] and x.get("width", r["w"]) == r["w"])
    n_seg = -(-v["length_frames"] // (SEG_S * FPS))
    return [kind.replace("t_start/t_end", f"{i * SEG_S}/{(i + 1) * SEG_S}", 1) for i in range(n_seg)]


def all_paths(kind_files: list[Path] = KIND_FILES, extra: bool = True) -> list[str]:
    """[Dishonest I/O] Request kinds x segments, plus (extra) every path in sequences and measurements."""
    paths: set[str] = set()
    for kinds in kind_files:
        for k in filter(None, (line.strip() for line in kinds.read_text().splitlines())):
            paths.update(segment_paths(k))
    if not extra:
        return sorted(paths)
    for f in list((ROOT / "servers_benchmark/sequences").glob("seq_a*_case*.csv")) + list((ROOT / "stat" / "pre-sitefix" / "seed2037").glob("run_*/*.csv")) \
            + [ROOT / "stat/timings/syn_timings.csv"]:
        with f.open() as fh:
            paths.update(row["path"] for row in csv.DictReader(fh))
    return sorted(paths)


# bitrate of a cached sub segment as a decode input of the rebuild (approx., same as temp/psc_study)
CACHED_MBPS = 2.048


def with_count(path: str, token: str) -> str:
    """[Honest] the same request with another count token (path part 10), e.g. '3' or '3+4'."""
    p = path.split("/")
    p[10] = token
    return "/".join(p)


def rebuild_features(full: dict, sub: dict) -> dict:
    """[Honest] features of a '<k>+<n>' rebuild: the full request minus the sub's layers, plus the cached sub as one
    full-frame video layer (decoded at CACHED_MBPS); segment length and output size stay the full's."""
    rec = {f: full[f] - sub[f] for f in full}
    for f in ("seg_frac", "out_mpix"):
        rec[f] = full[f]
    rec["n_video"] += 1
    rec["dec_mpix"] += full["out_mpix"]
    rec["dec_mbps"] += CACHED_MBPS
    rec["n_layers"] += 1
    rec["disp_frac"] += 1.0
    rec["video_frac"] = min(1.0, rec["video_frac"] + 1.0)
    return rec


def psc_rows(kind_files: list[Path] = KIND_FILES) -> dict[str, dict]:
    """[Dishonest I/O] path -> features of every sub '<k>' and rebuild '<k>+<n>' path (kinds x segments x valid cuts)."""
    rows: dict[str, dict] = {}
    for kinds in kind_files:
        for kind in filter(None, (line.strip() for line in kinds.read_text().splitlines())):
            cuts = cut_rows(kind)
            for seg in segment_paths(kind) if cuts else []:
                full = features(seg)
                for c in cuts:
                    sub_path = with_count(seg, str(c["k"]))
                    sub = features(sub_path)
                    rows[sub_path] = sub
                    rows[with_count(seg, f"{c['k']}+{c['n']}")] = rebuild_features(full, sub)
    return rows


def main():
    ap = argparse.ArgumentParser(description="export coef.csv + features.csv (default: BBC + syn kinds, in stat_predict/)")
    ap.add_argument("model", nargs="?", type=Path, default=HERE / "model_all_runs.json")
    ap.add_argument("--kinds", action="append", type=Path, help="request kind file(s) instead of the default lists")
    ap.add_argument("--out-dir", type=Path, help="write features.csv here (required with --kinds); no coef.csv")
    ap.add_argument("--asset-root", action="append", type=Path, default=[], help="extra dir with assets/ + shows/")
    a = ap.parse_args()
    if a.kinds and not a.out_dir:
        ap.error("--kinds needs --out-dir (never overwrite the default features.csv with another kind list)")
    features_mod.ASSET_ROOTS.extend(r.resolve() for r in a.asset_root)
    model = CostModel.load(a.model)
    kind_files = [k.resolve() for k in a.kinds] if a.kinds else KIND_FILES
    out_dir = a.out_dir.resolve() if a.out_dir else HERE
    out_dir.mkdir(parents=True, exist_ok=True)
    feats = list(dict.fromkeys(f for fs in STAGES.values() for f in fs))
    if not a.out_dir:
        with (HERE / "coef.csv").open("w") as fh:
            fh.write(",".join(["stage", "backend", "a"] + feats) + "\n")
            for key, c in sorted(model.coef.items()):
                stage, backend = key.split("|")
                fh.write(",".join([stage, backend, f"{c['a']:.6g}"] + [f"{c.get(f, 0.0):.6g}" for f in feats]) + "\n")
    rows, skipped = 0, 0
    done: set[str] = set()
    with (out_dir / "features.csv").open("w") as fh:
        fh.write(",".join(ALL + ["path"]) + "\n")
        for p in all_paths(kind_files, extra=not a.kinds):
            try:
                f = features(p)
            except (FileNotFoundError, StopIteration, KeyError, ValueError):
                skipped += 1  # segment file missing / malformed path
                continue
            fh.write(",".join(f"{f[k]:.6g}" for k in ALL) + "," + p + "\n")
            rows += 1
            done.add(p)
        psc = psc_rows(kind_files)
        for p, f in psc.items():
            if p not in done:  # a sub path may also be a measured / sequence path
                fh.write(",".join(f"{f[k]:.6g}" for k in ALL) + "," + p + "\n")
                rows += 1
    n_sub = sum(1 for p in psc if "+" not in p.split("/")[10])
    print(f"{'coef.csv: ' + str(len(model.coef)) + ' stage models; ' if not a.out_dir else ''}"
          f"{(out_dir / 'features.csv').relative_to(ROOT)}: {rows} paths ({skipped} skipped; "
          f"PSC {n_sub} sub + {len(psc) - n_sub} rebuild paths); "
          f"machine factor (for the load balancer, not exported): {model.machine_factor}")


if __name__ == "__main__":
    main()
