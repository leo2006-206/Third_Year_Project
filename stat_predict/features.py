"""
Generic feature vector of an offload request, from metadata only: the show spec's layer graph,
the options the request chooses, each asset's meta file and the segment file size. Show and layer
names are never features, so the model applies to shows it has not seen.

Partial offload: the number after the orientation in a request is how many root nodes of the layer
graph the site renders (the player renders the rest). Features cover only the layers in those roots,
so full and partial requests use the same model.

seg_frac (frames in this segment / 250) is not a model input: the model predicts a full segment and
scales by it (the last segment of a show is often shorter).
"""
import json
from functools import lru_cache
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ASSET_ROOTS = [ROOT / "obm", ROOT / "syn_video"]  # where "assets/..." and "shows/..." live
SEG_S = 10

# Features per pipeline stage (the model uses these; see model.py)
DECODE = ["n_video", "dec_mpix", "dec_mbps"]
RENDER = ["n_layers", "disp_frac", "anim_frac", "scaled_frac", "chroma_frac", "alpha_frac", "n_reframe"]
ENCODE = ["video_frac", "anim_frac"]
FPS = 25
ALL = list(dict.fromkeys(DECODE + RENDER + ENCODE + ["n_anim", "out_mpix", "seg_frac"]))


# ==============================================================================
# 1. Honest Functions (Pure Domain Logic)
# ==============================================================================


def parse_request(path: str) -> dict:
    """[Honest] /offload/show/<show>/<t0>/<t1>/<w>/<h>/<variant>/<orientation>/<roots>/<layer|opt,opt>/..."""
    p = path.split("/")
    opts = {}
    for tok in p[11:]:
        name, val = tok.split("|", 1)
        opts[name] = val.split(",")
    return {"show": p[3], "t0": int(p[4]), "w": int(p[6]), "h": int(p[7]), "variant": p[8],
            "roots": int(p[10]), "opts": opts}


def display_size(layer: dict, cw: int, ch: int) -> tuple[int, int]:
    """[Honest] Displayed (w, h) of a layer, following Experience.dn loadTrack."""
    d = layer.get("display", "full")
    if d == "window_absolute":
        return layer["display_width"], layer["display_height"]
    if d == "window_relative":
        pw, ph = layer.get("display_width", 0), layer.get("display_height", 0)
        if pw == 0 and ph == 0:
            return cw, ch
        if pw == 0:
            h = ph * ch // 100
            return h * 1280 // 720, h
        if ph == 0:
            w = pw * cw // 100
            return w, w * 720 // 1280
        return pw * ch // 100, ph * ch // 100
    return cw, ch


def rendered_layers(layers: list, roots: int) -> set[int]:
    """[Honest] Indices of the layers inside the first `roots` root nodes of the layer graph
    (a source adds a root, a children-1 transform joins the last root, a children-0 transform
    merges all roots), i.e. what the site renders for a full or partial offload request."""
    graph: list[list[int]] = []
    for i, layer in enumerate(layers):
        if layer["type"] == "transform":
            if layer.get("children", 0) == 1 and graph:
                graph[-1].append(i)
            else:
                graph = [[j for node in graph for j in node] + [i]]
        else:
            graph.append([i])
    return {i for node in graph[:roots] for i in node}


# ==============================================================================
# 2. Dishonest Functions (I/O: show specs, asset meta, segment sizes)
# ==============================================================================


def find(rel: str) -> Path:
    """[Dishonest I/O] Resolves an "assets/..." or "shows/..." path in obm/ or syn_video/."""
    for r in ASSET_ROOTS:
        if (r / rel).exists():
            return r / rel
    raise FileNotFoundError(rel)


@lru_cache(maxsize=None)
def load_json(rel: str) -> dict:
    """[Dishonest I/O] Show spec or asset meta, cached."""
    return json.loads(find(rel).read_text())


@lru_cache(maxsize=None)
def segment_mbps(seg_base: str, ext: str, t0: int) -> float:
    """[Dishonest I/O] Source bitrate of one segment from its file size."""
    return find(f"{seg_base}_{t0}_{t0 + SEG_S}.{ext}").stat().st_size * 8 / SEG_S / 1e6


def features(path: str) -> dict[str, float]:
    """[Dishonest I/O] Feature vector of one request (see module docstring)."""
    r = parse_request(path)
    spec = load_json(f"shows/{r['show']}")
    v = next(x for x in spec["variants"] if x["name"] == r["variant"] and x.get("width", r["w"]) == r["w"])
    cw, ch = r["w"], r["h"]
    canvas = cw * ch
    f = dict.fromkeys(ALL, 0.0)
    f["out_mpix"] = canvas / 1e6
    # frames in this segment / a full segment: the last segment of a show is often shorter
    seg_frames = SEG_S * FPS
    f["seg_frac"] = max(0, min(seg_frames, v.get("length_frames", seg_frames * 1000) - r["t0"] * FPS)) / seg_frames
    keep = rendered_layers(v["layers"], r["roots"])
    roots: list[float] = []  # displayed area under each root, for transforms
    for i, layer in enumerate(v["layers"]):
        chosen = r["opts"].get(layer["name"], [])
        options = [o for o in layer["options"] if o["name"] in chosen]
        dw, dh = display_size(layer, cw, ch)
        area = dw * dh / canvas
        if layer["type"] == "transform":
            child = roots[-1] if layer.get("children", 0) == 1 and roots else sum(roots)
            if i in keep:
                for o in options:
                    if o["type"] == "chromakey":
                        f["chroma_frac"] += child
                    elif o["type"] == "alphamask":
                        f["alpha_frac"] += child
                    elif o["type"] == "reframe":
                        f["n_reframe"] += 1
            roots = roots[:-1] + [child] if layer.get("children", 0) == 1 and roots else [child]
            continue
        active = [o for o in options if o["type"] != "none"]
        roots.append(area if active else 0.0)
        if i not in keep:
            continue
        for o in active:
            f["n_layers"] += 1
            f["disp_frac"] += area
            ent = load_json(o["spec"])["entities"][0]
            if o["type"] == "video":
                f["n_video"] += 1
                f["video_frac"] += area
                f["dec_mpix"] += ent["w"] * ent["h"] / 1e6
                f["dec_mbps"] += segment_mbps(ent["seg_base"], ent["seg_ext"], r["t0"])
                if (ent["w"], ent["h"]) != (dw, dh):
                    f["scaled_frac"] += area
            elif o["type"] == "anim_img":
                f["n_anim"] += 1
                f["anim_frac"] += area
    f["video_frac"] = min(f["video_frac"], 1.0)
    return f
