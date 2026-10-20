#!/usr/bin/env python3
"""
servers_benchmark/stage_meta_regression.py

Prototype evaluating stage-specific linear regression trained on generic
metadata extracted from show JSON specifications and asset metadata:

  x_decode = [1, N_video_streams, Sum_Decoded_Megapixels, Input_Segment_MB]
  x_render = [1, N_chroma, N_alpha, N_anim_img, N_reframe, N_total_layers]
  x_encode = [1, Output_Megapixels, FPS, N_video * N_anim (overlay interaction)]

Enables zero-shot prediction on newly added shows by deriving structural features
directly from the show's JSON specification.
"""

import csv
import glob
import json
import os
from pathlib import Path

os.environ.setdefault("MPLCONFIGDIR", "/tmp/matplotlib")
import matplotlib.pyplot as plt
import numpy as np


def build_meta_extractor(repo_root: Path):
    """[Honest / Pure Domain Logic]
    Builds a generic meta-feature extractor from show specs and asset files.
    """
    shows = {}
    for sf in glob.glob(str(repo_root / "obm" / "shows" / "*.json")):
        sname = os.path.basename(sf)
        with open(sf, encoding="utf-8") as f:
            shows[sname] = json.load(f)

    assets = {}
    for af in glob.glob(str(repo_root / "obm" / "assets" / "**" / "meta" / "*.json"), recursive=True):
        rel = af.replace(str(repo_root / "obm") + "/", "")
        try:
            with open(af, encoding="utf-8") as f:
                d = json.load(f)
                ents = d.get("entities", [])
                if ents:
                    assets[rel] = {
                        "w": ents[0].get("w", 1280),
                        "h": ents[0].get("h", 720),
                        "fps": ents[0].get("speed", d.get("speed", 25)),
                        "type": ents[0].get("type", "unknown"),
                        "seg_base": ents[0].get("seg_base", ""),
                        "seg_ext": ents[0].get("seg_ext", "h264"),
                    }
                else:
                    assets[rel] = {
                        "w": 1280,
                        "h": 720,
                        "fps": d.get("speed", 25),
                        "type": "unknown",
                        "seg_base": d.get("segment_base", ""),
                        "seg_ext": d.get("segment_ext", "h264"),
                    }
        except Exception:
            pass

    def extract_features(row: dict) -> dict:
        req = row["request"]
        parts = req.split("/")
        show_name = parts[3]
        t_start = int(parts[4])
        t_end = int(parts[5])
        out_w = int(parts[6])
        out_h = int(parts[7])
        variant_name = parts[8]
        layer_tokens = parts[11:]

        selected_options = {}
        for tok in layer_tokens:
            if "|" in tok:
                k, v = tok.split("|", 1)
                selected_options[k] = v
            else:
                selected_options[tok] = "on"

        show_spec = shows.get(show_name, {})
        variant_spec = None
        for v in show_spec.get("variants", []):
            if v.get("name") == variant_name:
                variant_spec = v
                break
        if not variant_spec and show_spec.get("variants"):
            variant_spec = show_spec["variants"][0]

        n_video = 0
        decoded_mp = 0.0
        input_seg_bytes = 0
        n_chroma = 0
        n_alpha = 0
        n_anim = 0
        n_reframe = 0
        n_total = 0

        if variant_spec:
            for l in variant_spec.get("layers", []):
                lname = l.get("name")
                vis_if = l.get("visible_if")
                vis_val = l.get("visible_if_value")
                if vis_if and selected_options.get(vis_if) != vis_val:
                    continue

                opt_val = selected_options.get(lname)
                matched_opt = None
                for opt in l.get("options", []):
                    if opt.get("name") == opt_val:
                        matched_opt = opt
                        break
                if not matched_opt and l.get("options"):
                    for opt in l.get("options", []):
                        if opt.get("default"):
                            matched_opt = opt
                            break
                    if not matched_opt:
                        matched_opt = l.get("options")[0]

                if not matched_opt:
                    continue

                otype = matched_opt.get("type", "none")
                if otype in ["none", "off"]:
                    continue

                n_total += 1
                spec_path = matched_opt.get("spec", "")
                ameta = assets.get(spec_path, {"w": 1280, "h": 720, "fps": 25, "type": "unknown"})

                if otype == "video":
                    n_video += 1
                    decoded_mp += (ameta["w"] * ameta["h"]) / 1_000_000.0
                    seg_base = ameta.get("seg_base", "")
                    seg_ext = ameta.get("seg_ext", "h264")
                    if seg_base:
                        seg_file = repo_root / "obm" / f"{seg_base}_{t_start}_{t_end}.{seg_ext}"
                        if seg_file.exists():
                            input_seg_bytes += seg_file.stat().st_size
                elif otype == "chromakey":
                    n_chroma += 1
                elif otype == "alphamask":
                    n_alpha += 1
                elif otype == "anim_img":
                    n_anim += 1
                elif otype == "reframe":
                    n_reframe += 1

        out_mp = (out_w * out_h) / 1_000_000.0
        input_mb = input_seg_bytes / (1024.0 * 1024.0)

        return {
            "show": show_name,
            "t_start": t_start,
            "n_video": n_video,
            "decoded_mp": decoded_mp,
            "input_mb": input_mb,
            "n_chroma": n_chroma,
            "n_alpha": n_alpha,
            "n_anim": n_anim,
            "n_reframe": n_reframe,
            "n_total": n_total,
            "out_mp": out_mp,
            "decode_ms": float(row["decode_ms"]),
            "render_ms": float(row["render_ms"]),
            "encode_ms": float(row["encode_ms"]),
            "work_ms": float(row["work_ms"]),
        }

    return extract_features


def evaluate_models(dataset: list[dict]) -> dict:
    """[Honest / Pure Domain Logic]
    Fits stage-specific linear models and returns evaluation metrics.
    """
    # 1. Decode Stage Model: [1, N_video, Decoded_Megapixels, Input_MB]
    x_dec = np.array([[1.0, d["n_video"], d["decoded_mp"], d["input_mb"]] for d in dataset])
    y_dec = np.array([d["decode_ms"] for d in dataset])
    b_dec, _, _, _ = np.linalg.lstsq(x_dec, y_dec, rcond=None)
    pred_dec = x_dec @ b_dec
    r2_dec = 1.0 - np.sum((y_dec - pred_dec) ** 2) / np.sum((y_dec - np.mean(y_dec)) ** 2)
    mae_dec = np.mean(np.abs(y_dec - pred_dec))

    # 2. Render Stage Model: [1, N_chroma, N_alpha, N_anim, N_reframe, N_total]
    x_rnd = np.array([[1.0, d["n_chroma"], d["n_alpha"], d["n_anim"], d["n_reframe"], d["n_total"]] for d in dataset])
    y_rnd = np.array([d["render_ms"] for d in dataset])
    b_rnd, _, _, _ = np.linalg.lstsq(x_rnd, y_rnd, rcond=None)
    pred_rnd = x_rnd @ b_rnd
    r2_rnd = 1.0 - np.sum((y_rnd - pred_rnd) ** 2) / np.sum((y_rnd - np.mean(y_rnd)) ** 2)
    mae_rnd = np.mean(np.abs(y_rnd - pred_rnd))

    # 3. Encode Stage Model: [1, Output_Megapixels, N_total, N_video, N_video * N_anim]
    x_enc = np.array([[1.0, d["out_mp"], d["n_total"], d["n_video"], d["n_video"] * d["n_anim"]] for d in dataset])
    y_enc = np.array([d["encode_ms"] for d in dataset])
    b_enc, _, _, _ = np.linalg.lstsq(x_enc, y_enc, rcond=None)
    pred_enc = x_enc @ b_enc
    r2_enc = 1.0 - np.sum((y_enc - pred_enc) ** 2) / np.sum((y_enc - np.mean(y_enc)) ** 2)
    mae_enc = np.mean(np.abs(y_enc - pred_enc))

    # 4. Total Work Time: Sum of Stage Predictions
    y_work = np.array([d["work_ms"] for d in dataset])
    pred_work = pred_dec + pred_rnd + pred_enc
    r2_work = 1.0 - np.sum((y_work - pred_work) ** 2) / np.sum((y_work - np.mean(y_work)) ** 2)
    mae_work = np.mean(np.abs(y_work - pred_work))
    mape_work = np.mean(np.abs(y_work - pred_work) / np.maximum(y_work, 1.0)) * 100.0

    return {
        "b_dec": b_dec,
        "b_rnd": b_rnd,
        "b_enc": b_enc,
        "r2_dec": r2_dec,
        "mae_dec": mae_dec,
        "r2_rnd": r2_rnd,
        "mae_rnd": mae_rnd,
        "r2_enc": r2_enc,
        "mae_enc": mae_enc,
        "r2_work": r2_work,
        "mae_work": mae_work,
        "mape_work": mape_work,
        "y_dec": y_dec,
        "pred_dec": pred_dec,
        "y_rnd": y_rnd,
        "pred_rnd": pred_rnd,
        "y_enc": y_enc,
        "pred_enc": pred_enc,
        "y_work": y_work,
        "pred_work": pred_work,
        "shows": [d["show"] for d in dataset],
    }


def plot_stage_meta_results(metrics: dict, output_path: Path) -> None:
    """[Dishonest / I/O Boundary]
    Generates a 4-panel diagnostic figure comparing actual vs predicted timings.
    """
    _fig, axes = plt.subplots(2, 2, figsize=(13, 11), dpi=300)
    stages = [
        (axes[0, 0], metrics["y_dec"], metrics["pred_dec"], "A. Decode Stage: x_dec", metrics["r2_dec"], metrics["mae_dec"], "#4caf50"),
        (axes[0, 1], metrics["y_rnd"], metrics["pred_rnd"], "B. Render Stage: x_render", metrics["r2_rnd"], metrics["mae_rnd"], "#e91e63"),
        (axes[1, 0], metrics["y_enc"], metrics["pred_enc"], "C. Encode Stage: x_encode", metrics["r2_enc"], metrics["mae_enc"], "#9c27b0"),
        (axes[1, 1], metrics["y_work"], metrics["pred_work"], "D. Total Work Time: Σ Stages", metrics["r2_work"], metrics["mae_work"], "#1976d2"),
    ]

    for ax, y_act, y_pred, title, r2, mae, color in stages:
        ax.scatter(y_act, y_pred, alpha=0.6, color=color, edgecolor="black", s=35, label="Benchmark Runs")
        min_v = min(np.min(y_act), np.min(y_pred)) * 0.9
        max_v = max(np.max(y_act), np.max(y_pred)) * 1.05
        ax.plot([min_v, max_v], [min_v, max_v], color="red", linestyle="--", linewidth=1.8, label="Ideal (y = x)")
        ax.set_xlim(min_v, max_v)
        ax.set_ylim(min_v, max_v)
        ax.set_xlabel("Actual Time (ms)", fontsize=11, fontweight="bold")
        ax.set_ylabel("Predicted Time (ms)", fontsize=11, fontweight="bold")
        ax.set_title(title, fontsize=12, fontweight="bold")
        ax.text(
            0.05,
            0.82,
            f"R² = {r2:.3f}\nMAE = {mae:.1f} ms",
            transform=ax.transAxes,
            fontsize=10,
            fontweight="bold",
            bbox=dict(boxstyle="round,pad=0.4", facecolor="#ffffff", edgecolor="black", alpha=0.9),
        )
        ax.legend(loc="lower right", frameon=True, edgecolor="black", fontsize=9)
        ax.grid(True, linestyle="--", alpha=0.5)

    plt.tight_layout()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    plt.savefig(output_path)
    plt.close()
    print(f"[Plot Saved] {output_path}")


def main():
    repo_root = Path(__file__).resolve().parent.parent
    csv_path = repo_root / "servers_benchmark" / "request_timings.csv"
    plot_path = repo_root / "servers_benchmark" / "stage_meta_prediction.png"

    extractor = build_meta_extractor(repo_root)
    with open(csv_path, encoding="utf-8") as f:
        records = list(csv.DictReader(f))

    dataset = [extractor(r) for r in records]
    metrics = evaluate_models(dataset)

    print("=== Generic Metadata Stage-Specific Linear Regression ===")
    print(
        f"1. Decode Stage:\n"
        f"   Features: [Intercept, N_video, Decoded_Megapixels, Input_Segment_MB]\n"
        f"   Coefficients (ms): {metrics['b_dec'].round(2)}\n"
        f"   Fit: R² = {metrics['r2_dec']:.3f} | MAE = {metrics['mae_dec']:.1f} ms\n"
    )
    print(
        f"2. Render Stage:\n"
        f"   Features: [Intercept, N_chroma, N_alpha, N_anim, N_reframe, N_total]\n"
        f"   Coefficients (ms): {metrics['b_rnd'].round(2)}\n"
        f"   Fit: R² = {metrics['r2_rnd']:.3f} | MAE = {metrics['mae_rnd']:.1f} ms\n"
    )
    print(
        f"3. Encode Stage:\n"
        f"   Features: [Intercept, Output_Megapixels, N_total, N_video, N_video * N_anim]\n"
        f"   Coefficients (ms): {metrics['b_enc'].round(2)}\n"
        f"   Fit: R² = {metrics['r2_enc']:.3f} | MAE = {metrics['mae_enc']:.1f} ms\n"
    )
    print(
        f"4. Total Work Time (Sum of Stage Predictions):\n"
        f"   Fit: R² = {metrics['r2_work']:.3f} | MAE = {metrics['mae_work']:.1f} ms | MAPE = {metrics['mape_work']:.1f}%\n"
    )

    plot_stage_meta_results(metrics, plot_path)


if __name__ == "__main__":
    main()

