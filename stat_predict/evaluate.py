#!/usr/bin/env python3
"""
Builds the dataset (BBC calibration + synthetic shows, features from metadata only), evaluates the cost
model on several train/test splits, fits a final model on all data and prints its coefficients.

Splits (requests of one kind or one show never appear in both train and test):
  A  random 80/20 split of request kinds (BBC + syn), 20 seeds
  B  leave one BBC show out, train on the other BBC shows only
  C  leave one BBC show out, train on the other BBC shows + all syn shows
  D  leave one syn show out, train on BBC + the other syn shows
  E  train on BBC only, test on all syn shows (fully unseen content classes)
  F  train on syn only, test on all BBC shows
In-situ replay data (stat/run_*/*.csv: requests measured in the busy cluster, every segment):
  G  train on calibration (BBC + syn), test on every replay request (never seen in training)
  H  leave one BBC show out with replay data of the other shows added to training
  I  train on BBC calibration + replay data, test on all syn shows
  J  leave one show out, test = that show's replay requests: calibration only vs + other shows' replays
Baseline: mean work of the same backends in the training set (no features).
Metric: error of predicted total work (decode + render + encode) per request and site.

Usage: stat_predict/evaluate.py   -> prints the report, writes stat_predict/model_all.json (calibration)
                                     and stat_predict/model_all_runs.json (calibration + replay data)
"""
import sys
from pathlib import Path

import numpy as np
import pandas as pd

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from features import ALL, features  # noqa: E402
from model import CostModel  # noqa: E402

ROOT = HERE.parent
TIER = {"gpu": ("gpu", "gpu", "cpu"), "llvm": ("gpu", "llvmpipe", "cpu"), "cpu": ("gpu", "cpu", "cpu")}
PORT_TIER = {"7010": "gpu", "7020": "llvm", "7030": "cpu"}


# ==============================================================================
# 1. Dataset
# ==============================================================================


def row(kind: str, show: str, source: str, machine: str, tier: str, dec: float, ren: float, enc: float,
        run: str = "") -> dict:
    d, r, e = TIER[tier]
    return {"kind": kind, "show": show.removesuffix(".json"), "source": source, "run": run, "machine": machine,
            "decode": d, "render": r, "encode": e, "tier": tier,
            "t_decode": dec, "t_render": ren, "t_encode": enc, "work": dec + ren + enc}


def load_dataset() -> pd.DataFrame:
    """[Dishonest I/O] BBC calibration (stat/<machine>_<port>_<tier>_timing.csv), syn cluster timings and
    replay requests (stat/run_*/*.csv; in case 2 both ports are gpu sites)."""
    rows = []
    for f in sorted((ROOT / "stat").glob("*_70?0_*_timing.csv")):
        machine, _, tier = f.stem.removesuffix("_timing").split("_")
        for _, r in pd.read_csv(f).iterrows():
            rows.append(row(r.request, r.request.split("/")[3], "bbc", machine, tier,
                            r.decode_ms / 1e3, r.render_ms / 1e3, r.encode_ms / 1e3))
    syn = pd.read_csv(ROOT / "stat/syn_timings.csv")
    for _, r in syn[syn.status == 200].iterrows():
        machine = "rs" if r.worker.startswith("10.10") else "om"
        rows.append(row(r.path, r.path.split("/")[3], "syn", machine, PORT_TIER[r.worker[-4:]],
                        r.decode_ms / 1e3, r.render_ms / 1e3, r.encode_ms / 1e3))
    for f in sorted((ROOT / "stat").glob("run_*/*.csv")):
        case2 = "_c2" in f.parent.name
        d = pd.read_csv(f)
        for _, r in d[d.status == 200].iterrows():
            machine = "rs" if r.worker.startswith("10.10") else "om"
            tier = "gpu" if case2 else PORT_TIER[r.worker[-4:]]
            rows.append(row(r.path, r.path.split("/")[3], "run", machine, tier,
                            r.decode_ms / 1e3, r.render_ms / 1e3, r.encode_ms / 1e3, f"{f.parent.name}/{f.stem}"))
    df = pd.DataFrame(rows)
    feats = {k: features(k) for k in df.kind.unique()}
    for name in ALL:
        df[name] = df.kind.map(lambda k: feats[k][name])
    return df


def fit(train: pd.DataFrame) -> CostModel:
    return CostModel().fit(train.to_dict("records"))


def predict(model: CostModel, test: pd.DataFrame) -> np.ndarray:
    return np.array([model.predict(r, r["machine"], r["decode"], r["render"], r["encode"])["work"]
                     for r in test.to_dict("records")])


def baseline(train: pd.DataFrame, test: pd.DataFrame) -> np.ndarray:
    """No-feature baseline: mean work of the same backends (and machine) in the training set."""
    mean = train.groupby(["machine", "tier"]).work.mean()
    return np.array([mean.get((r.machine, r.tier), train.work.mean()) for r in test.itertuples()])


# ==============================================================================
# 2. Model, baseline and evaluation
# ==============================================================================


def errors(y: np.ndarray, p: np.ndarray) -> dict:
    ape = np.abs(p - y) / y
    return {"n": len(y), "MAPE%": 100 * ape.mean(), "medAPE%": 100 * np.median(ape),
            "MAE_s": np.abs(p - y).mean(), "within20%": 100 * (ape <= 0.2).mean()}


def run_split(name: str, train: pd.DataFrame, test: pd.DataFrame, results: list, by_tier: list | None = None):
    model = fit(train)
    p, b = predict(model, test), baseline(train, test)
    e, eb = errors(test.work.values, p), errors(test.work.values, b)
    results.append({"split": name, **e, "baseline MAPE%": eb["MAPE%"]})
    if by_tier is not None:
        for tier in ["gpu", "llvm", "cpu"]:
            m = (test.tier == tier).values
            if m.any():
                by_tier.append({"split": name, "tier": tier, **errors(test.work.values[m], p[m])})


def main():
    pd.set_option("display.width", 200)
    full = load_dataset()
    runs = full[full.source == "run"]
    df = full[full.source != "run"]
    print(f"dataset: {len(df)} rows, {df.kind.nunique()} request kinds, "
          f"{df[df.source == 'bbc'].show.nunique()} BBC + {df[df.source == 'syn'].show.nunique()} syn shows")
    print(df.groupby(["source", "tier"]).size().unstack().to_string(), "\n")

    results, per_show, by_tier = [], [], []
    kinds = df.kind.unique()
    for seed in range(20):
        rng = np.random.default_rng(seed)
        test_k = set(rng.choice(kinds, size=len(kinds) // 5, replace=False))
        run_split("A random 80/20 kinds", df[~df.kind.isin(test_k)], df[df.kind.isin(test_k)],
                  results, by_tier if seed == 0 else None)
    bbc, syn = df[df.source == "bbc"], df[df.source == "syn"]
    for show in sorted(bbc.show.unique()):
        run_split(f"B LOSO {show} (train BBC)", bbc[bbc.show != show], bbc[bbc.show == show], per_show)
        run_split(f"C LOSO {show} (train BBC+syn)", df[df.show != show], df[df.show == show], per_show)
    for show in sorted(syn.show.unique()):
        run_split(f"D LOSO {show} (train BBC+syn)", df[df.show != show], df[df.show == show], per_show)
    run_split("E train BBC -> test syn", bbc, syn, results, by_tier)
    run_split("F train syn -> test BBC", syn, bbc, results, by_tier)

    r = pd.DataFrame(results)
    a = r[r.split.str.startswith("A")].drop(columns="split").mean().to_frame().T
    a.insert(0, "split", "A random 80/20 kinds (mean of 20)")
    summary = pd.concat([a, r[~r.split.str.startswith("A")]])
    print("== Total work error: model vs no-feature baseline")
    print(summary.round(1).to_string(index=False), "\n")
    ps = pd.DataFrame(per_show)
    print("== Leave one show out (each row: that show unseen in training)")
    print(ps.round(1).to_string(index=False), "\n")
    print("== By backend tier (split A seed 0, E, F)")
    print(pd.DataFrame(by_tier).round(1).to_string(index=False), "\n")

    print("== In-situ replay data: " + f"{len(runs)} requests " + str(runs.tier.value_counts().to_dict()))
    calib = df
    g = fit(calib)
    p = predict(g, runs)
    rows_g = [{"split": "G calib -> all replay requests", "tier": "all", **errors(runs.work.values, p)}]
    for tier in ["gpu", "llvm", "cpu"]:
        m = (runs.tier == tier).values
        rows_g.append({"split": "G calib -> replay", "tier": tier, **errors(runs.work.values[m], p[m]),
                       "pred/actual median": float(np.median(p[m] / runs.work.values[m]))})
    load = runs.run.str.contains("_p95").values
    for name, m in [("heavy load (p95 runs)", load), ("lighter load (other runs)", ~load)]:
        rows_g.append({"split": f"G calib -> replay, {name}", "tier": "all", **errors(runs.work.values[m], p[m]),
                       "pred/actual median": float(np.median(p[m] / runs.work.values[m]))})
    print(pd.DataFrame(rows_g).round(2).to_string(index=False), "\n")

    with_runs = []
    for show in sorted(bbc.show.unique()):
        test = df[(df.show == show) & (df.source == "bbc")]
        run_split(f"C LOSO {show} (train BBC+syn)", df[df.show != show], test, with_runs)
        run_split(f"H LOSO {show} (train BBC+syn+runs)", full[full.show != show], test, with_runs)
    run_split("E train BBC -> test syn", bbc, syn, with_runs, by_tier_runs := [])
    run_split("I train BBC+runs -> test syn", full[full.source != "syn"], syn, with_runs, by_tier_runs)
    j_rows = []
    for show in sorted(runs.show.unique()):
        test = runs[runs.show == show]
        for name, train in [("calib only", df[df.show != show]), ("calib + replay", full[full.show != show])]:
            p = predict(fit(train), test)
            for tier in ["gpu", "llvm", "cpu"]:
                m = (test.tier == tier).values
                if m.any():
                    j_rows.append({"show": show, "train": name, "tier": tier, **errors(test.work.values[m], p[m])})
    j = pd.DataFrame(j_rows)
    print("== J: unseen show, test = its replay requests (mean over shows)")
    print(j.groupby(["tier", "train"])[["MAPE%", "medAPE%", "within20%"]].mean().round(1).to_string())
    print(j.groupby("train")[["MAPE%", "medAPE%", "within20%"]].mean().round(1).to_string(), "\n")
    print("== Adding replay data to training (test rows = calibration of the unseen show / syn)")
    print(pd.DataFrame(with_runs).round(1).to_string(index=False))
    print(pd.DataFrame(by_tier_runs).round(1).to_string(index=False), "\n")

    final_runs = fit(full)
    final_runs.save(HERE / "model_all_runs.json")
    print("== Model with replay data: machine factor", {k: round(v, 3) for k, v in final_runs.machine_factor.items()})
    print(pd.DataFrame(final_runs.coef).T.fillna("").map(lambda v: f"{v:.3f}" if isinstance(v, float) else v).to_string(), "\n")

    final = fit(df)
    final.save(HERE / "model_all.json")
    print("== Final model (all data): machine factor", {k: round(v, 3) for k, v in final.machine_factor.items()})
    coef = pd.DataFrame(final.coef).T.fillna("")
    print(coef.map(lambda v: f"{v:.3f}" if isinstance(v, float) else v).to_string(), "\n")

    print("== Example explanations (final model)")
    examples = [
        ("forecast, full offload (4 roots)", "/offload/show/forecast.json/0/10/1280/720/core/landscape/4/map|map_colour/weather|Meteorological/presenter|hearing/presenter_chroma|chroma/time|time"),
        ("forecast, partial offload: first 2 roots (map + weather)", "/offload/show/forecast.json/0/10/1280/720/core/landscape/2/map|map_colour/weather|Meteorological/presenter|hearing/presenter_chroma|chroma/time|time"),
    ]
    for label, path in examples:
        x = features(path)
        for machine, tier in [("om", "gpu"), ("om", "cpu")]:
            print(f"-- {label} on {machine} {tier}")
            print(final.explain(x, machine, *TIER[tier]))
    print("\nmodel written to", HERE / "model_all.json")


if __name__ == "__main__":
    main()
