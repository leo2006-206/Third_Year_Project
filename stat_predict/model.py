"""
Lightweight, explainable cost model for offload requests.

    work(request, site) = machine_factor[machine]
                          x ( decode[decode_backend](x)
                            + render[render_backend](x)
                            + encode[(render_backend, encode_backend)](x) )

Each stage model is   t = a + sum(beta_i * x_i),   a, beta >= 0   (non-negative least squares, fitted
on relative error so cheap and expensive requests count alike). One small model per (stage, backend):
a new backend is profiled and fitted on its own, the others stay as they are. The intercept a is the
fixed cost per task on that backend (setup, copies), beta the cost per unit of each feature.

Encode is keyed by the (render, encode) backend pair: Dana's encode timer includes the GPU readback of
the frame, so it depends on where the frame was rendered.

The machine factor is NOT part of the model: it is one number per machine (here estimated from the
training data, in the load balancer learned from feedback). Network, asset and queue time are the load
balancer's job, not predicted here. Predictions are per full 10 s segment, scaled by the
request's segment fraction (frames in the segment / 250).
"""
import json
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
from scipy.optimize import nnls

from features import DECODE, ENCODE, RENDER

STAGES = {"decode": DECODE, "render": RENDER, "encode": ENCODE}


def backend_key(stage: str, decode: str, render: str, encode: str) -> str:
    """[Honest] Which backend(s) a stage model is keyed by."""
    return {"decode": decode, "render": render, "encode": f"{render}+{encode}"}[stage]


@dataclass
class CostModel:
    coef: dict = field(default_factory=dict)            # "stage|backend" -> {"a": a, feature: beta, ...}
    machine_factor: dict = field(default_factory=dict)  # machine -> factor (reference machine = 1.0)

    # ---- fitting -------------------------------------------------------------------------------
    def fit(self, rows: list[dict], ref_machine: str = "om", min_rows: int = 3):
        """[Honest] rows: dicts with kind, machine, backends (decode, render, encode), stage times in s
        (t_decode, t_render, t_encode), work, and the features."""
        self.machine_factor = estimate_machine_factor(rows, ref_machine)
        self.coef = {}
        for stage, feats in STAGES.items():
            groups: dict[str, list[dict]] = {}
            for r in rows:
                groups.setdefault(backend_key(stage, r["decode"], r["render"], r["encode"]), []).append(r)
            for key, grp in groups.items():
                if len(grp) < min_rows:  # too few samples for this backend
                    continue
                X = np.array([[1.0] + [g[f] for f in feats] for g in grp])
                # targets per full segment and reference machine
                y = np.array([g[f"t_{stage}"] / self.machine_factor.get(g["machine"], 1.0) / max(g.get("seg_frac", 1.0), 0.04)
                              for g in grp])
                w = 1.0 / np.maximum(y, 0.05)  # relative error
                beta, _ = nnls(X * w[:, None], y * w)
                self.coef[f"{stage}|{key}"] = {"a": beta[0], **dict(zip(feats, beta[1:]))}
        return self

    # ---- prediction ----------------------------------------------------------------------------
    def stage_parts(self, x: dict, stage: str, key: str) -> dict[str, float]:
        """[Honest] Additive contributions (s) of the intercept and each feature to one stage."""
        c = self.coef.get(f"{stage}|{key}")
        if c is None:
            raise KeyError(f"no model for {stage} on backend {key}")
        return {"a": c["a"], **{f: c[f] * x[f] for f in STAGES[stage] if c[f] * x[f] != 0}}

    def predict(self, x: dict, machine: str, decode: str, render: str, encode: str) -> dict[str, float]:
        """[Honest] Predicted stage times and total work (s) of a request (features x) on a site."""
        m = self.machine_factor.get(machine, 1.0) * x.get("seg_frac", 1.0)
        out = {s: m * sum(self.stage_parts(x, s, backend_key(s, decode, render, encode)).values()) for s in STAGES}
        out["work"] = out["decode"] + out["render"] + out["encode"]
        return out

    def explain(self, x: dict, machine: str, decode: str, render: str, encode: str) -> str:
        """[Honest] Human-readable breakdown of a prediction."""
        m = self.machine_factor.get(machine, 1.0)
        lines, total = [], 0.0
        for s in STAGES:
            parts = self.stage_parts(x, s, backend_key(s, decode, render, encode))
            t = sum(parts.values())
            total += t
            lines.append(f"{s:7} {t:6.2f} s = " + " + ".join(f"{k} {v:.2f}" for k, v in parts.items()))
        sf = x.get("seg_frac", 1.0)
        lines.append(f"work    {total:6.2f} s x machine factor {machine} {m:.2f}"
                     + (f" x segment fraction {sf:.2f}" if sf != 1.0 else "") + f" = {total * m * sf:.2f} s")
        return "\n".join(lines)

    # ---- persistence ---------------------------------------------------------------------------
    def save(self, path: Path):
        path.write_text(json.dumps({"machine_factor": self.machine_factor, "coef": self.coef}, indent=1))

    @classmethod
    def load(cls, path: Path) -> "CostModel":
        d = json.loads(path.read_text())
        return cls(coef=d["coef"], machine_factor=d["machine_factor"])


def estimate_machine_factor(rows: list[dict], ref: str) -> dict[str, float]:
    """[Honest] Per machine: median work ratio to the reference machine over the same request on the
    same backends. Stands in for the hardcoded / feedback-learned factor of the load balancer."""
    ref_work = {(r["kind"], r["decode"], r["render"], r["encode"]): r["work"] for r in rows if r["machine"] == ref}
    ratios: dict[str, list[float]] = {}
    for r in rows:
        k = (r["kind"], r["decode"], r["render"], r["encode"])
        if r["machine"] != ref and k in ref_work and ref_work[k] > 0:
            ratios.setdefault(r["machine"], []).append(r["work"] / ref_work[k])
    return {ref: 1.0, **{m: float(np.median(v)) for m, v in ratios.items()}}
