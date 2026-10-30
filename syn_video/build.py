#!/usr/bin/env python3
"""
Synthetic OBM shows (round 1): unseen video classes for the cost-prediction study.

Builds, in the same layout as obm/ (see docs.md):
  assets/syn/video/<name>_seg_<t0>_<t1>.h264   10 s H.264 segments in OBM's H2i framing
                                                (ffmpeg + obm/tools/SegmentH264, gitignored)
  assets/syn/meta/<name>.json                   video spec per asset, chroma spec
  shows/<show>.json, shows/<show>.png           show specs + thumbnail
  request_kind_syn.txt                          every valid request path (t_start/t_end placeholder)

Usage: ./build.py [--force]      (--force re-encodes videos that already exist)
"""
import json
from concurrent.futures import ThreadPoolExecutor
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
ASSET_DIR = "assets/syn"  # path as seen by the player / offload site (relative to the web root)
VIDEO_DIR = ROOT / ASSET_DIR / "video"
META_DIR = ROOT / ASSET_DIR / "meta"
SHOW_DIR = ROOT / "shows"
OBM = ROOT.parent / "obm"  # OBM code: tools/SegmentH264.dn writes the H2i segments

FPS = 25
SEG_S = 10
N_SEG = 3  # 30 s per video
FRAMES = FPS * SEG_S * N_SEG
CANVAS = (1280, 720)
KEY_RGB = (36, 73, 111)  # same key colour as the forecast presenter
SEGMENT_PLACEHOLDER = "t_start/t_end"

# ==============================================================================
# 1. Video assets: name -> (lavfi graph, width, height, bitrate, level)
#    All match the existing OBM assets: Constrained Baseline, yuv420p, no B-frames,
#    1 reference frame, 25 fps, 10 s segments that each start with SPS/PPS + IDR, stored in
#    OBM's H2i framing (5-byte length/type header before every NAL), which Track.video expects
# ==============================================================================
KEY_HEX = "0x%02X%02X%02X" % KEY_RGB


def content(kind: str, w: int, h: int) -> str:
    """[Honest] lavfi source graph for a content class at w x h."""
    s = f"s={w}x{h}:r={FPS}"
    return {
        # smooth slowly drifting colour gradient: very little detail or motion
        "calm": f"gradients={s}:speed=0.003:c0=0x203050:c1=0x506080:c2=0x305060:nb_colors=3",
        # moving test pattern: sharp edges, moderate motion
        "graphics": f"testsrc2={s}",
        # zooming Mandelbrot fractal, constant motion; detail grows over the clip (segment 0 shows
        # the whole set, segment 2 dense boundary detail). end_pts counts frames, not seconds
        "texture": f"mandelbrot={s}:end_pts={FRAMES}:end_scale=0.0005:maxiter=600",
        # temporal noise: worst case for prediction (every block changes every frame)
        "noise": f"color=c=gray:{s},noise=alls=90:allf=t+u",
        # chroma foreground: key-colour background, moving test pattern in front (forecast-like)
        "keyfg": (
            f"color=c={KEY_HEX}:{s}[bg];testsrc2=s={w * 3 // 8}x{h * 3 // 8}:r={FPS}[fg];"
            f"[bg][fg]overlay=x='(W-w)/2+(W/4)*sin(t*0.9)':y='(H-h)/2+(H/5)*cos(t*0.6)':shortest=1"
        ),
    }[kind]


VIDEOS = {
    # content classes at 720p, same bitrate (2 Mbit/s, like the existing 720p assets)
    "calm": ("calm", 1280, 720, "2M"),
    "graphics": ("graphics", 1280, 720, "2M"),
    "texture": ("texture", 1280, 720, "2M"),
    "noise": ("noise", 1280, 720, "2M"),
    "keyfg": ("keyfg", 1280, 720, "2M"),
    # keyed foreground at window size (chroma area: the engine keys a layer only unscaled)
    "keyfg_320": ("keyfg", 320, 180, "350k"),
    "keyfg_640": ("keyfg", 640, 360, "700k"),
    # same content, different source bitrate (decode cost vs bits)
    "texture_br1": ("texture", 1280, 720, "1M"),
    "texture_br6": ("texture", 1280, 720, "6M"),
    # same content, different decoded resolution
    "texture_1080": ("texture", 1920, 1080, "4M"),
    "texture_480": ("texture", 854, 480, "1M"),
    # grid tiles
    "tile360_calm": ("calm", 640, 360, "700k"),
    "tile360_graphics": ("graphics", 640, 360, "700k"),
    "tile360_texture": ("texture", 640, 360, "700k"),
    "tile360_noise": ("noise", 640, 360, "700k"),
    "tile240_graphics": ("graphics", 426, 240, "350k"),
    "tile240_texture": ("texture", 426, 240, "350k"),
    "tile240_noise": ("noise", 426, 240, "350k"),
}


def level_for(w: int, h: int) -> str:
    return "4.0" if w * h > 1280 * 720 else "3.1"


def segments(name: str) -> list[Path]:
    return [VIDEO_DIR / f"{name}_seg_{i * SEG_S}_{(i + 1) * SEG_S}.h264" for i in range(N_SEG)]


def is_h2i(path: Path) -> bool:
    """[Dishonest I/O] OBM's H2i framing: every NAL is preceded by a 5-byte header (int4 length,
    byte type), so the file starts with the header, then the 00 00 00 01 start code of the SPS.
    A plain Annex B file starts with the start code itself."""
    head = path.read_bytes()[:9]
    start = b"\x00\x00\x00\x01"
    return head[5:9] == start and head[:4] != start and head[:3] != b"\x00\x00\x01"


def ensure_segment_tool():
    """[Dishonest I/O] Compiles obm/tools/SegmentH264.dn (OBM's own H2i segmenter) if needed."""
    if not (OBM / "tools" / "SegmentH264.o").exists():
        subprocess.run(["dnc", "tools/SegmentH264.dn"], cwd=OBM, check=True)


def to_h2i(name: str, annexb: Path):
    """[Dishonest I/O] Splits a full Annex B stream into 10 s H2i segments with OBM's tool.
    x264's leading SEI NAL is removed first: the tool requires the stream to start with an SPS."""
    work = VIDEO_DIR / ".src"
    work.mkdir(exist_ok=True)
    src = work / f"{name}.h264"
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(annexb), "-c:v", "copy",
                    "-bsf:v", "filter_units=remove_types=6", "-f", "h264", str(src)], check=True)
    subprocess.run(["dana", "tools.SegmentH264", str(src), "frames", str(FPS * SEG_S)],
                   cwd=OBM, check=True, capture_output=True)
    for i, o in enumerate(segments(name)):
        (work / f"{name}_seg_{i * SEG_S}_{(i + 1) * SEG_S}.h264").rename(o)
    src.unlink()
    if annexb.parent == work:
        annexb.unlink()


def encode_video(name: str, force: bool):
    """[Dishonest I/O] Encodes one asset with ffmpeg (Annex B, SPS/PPS before every IDR), then
    segments it into N_SEG H2i files like the existing OBM assets."""
    kind, w, h, br = VIDEOS[name]
    outs = segments(name)
    if not force and all(o.exists() for o in outs):
        if all(is_h2i(o) for o in outs):
            print(f"  {name}: exists")
            return
        # plain Annex B segments from an older build: join them and re-segment, no re-encode
        joined = VIDEO_DIR / ".src" / f"{name}_annexb.h264"
        joined.parent.mkdir(exist_ok=True)
        joined.write_bytes(b"".join(o.read_bytes() for o in outs))
        to_h2i(name, joined)
        print(f"  {name}: converted to H2i")
        return
    full = VIDEO_DIR / ".src" / f"{name}_annexb.h264"
    full.parent.mkdir(exist_ok=True)
    cmd = [
        "ffmpeg", "-v", "error", "-y",
        "-f", "lavfi", "-i", content(kind, w, h),
        "-frames:v", str(FRAMES),
        "-c:v", "libx264", "-profile:v", "baseline", "-level", level_for(w, h), "-pix_fmt", "yuv420p",
        "-bf", "0", "-refs", "1",
        # fixed GOP = one segment, no scene-cut keyframes, SPS/PPS before every IDR
        "-g", str(FPS * SEG_S), "-keyint_min", str(FPS * SEG_S), "-sc_threshold", "0",
        "-x264-params", "repeat-headers=1",
        "-b:v", br, "-maxrate", br, "-bufsize", br,
        "-f", "h264", str(full),
    ]
    subprocess.run(cmd, check=True)
    to_h2i(name, full)
    print(f"  {name}: {w}x{h} {br} -> {len(outs)} H2i segments")


def video_meta(name: str) -> dict:
    _, w, h, _ = VIDEOS[name]
    return {"speed": FPS, "xpan": 0, "ypan": 0, "entities": [
        {"seg_base": f"{ASSET_DIR}/video/{name}_seg", "seg_ext": "h264", "type": "h264",
         "w": w, "h": h, "frames": FRAMES, "segment_length_sec": SEG_S}]}


# ==============================================================================
# 2. Shows: layer graphs built from the assets above
# ==============================================================================
def vopt(name: str, option: str | None = None, default: bool = False) -> dict:
    o = {"type": "video", "name": option or name, "spec": f"{ASSET_DIR}/meta/{name}.json"}
    if default:
        o["default"] = True
    return o


NONE = {"type": "none", "name": "off"}
CHROMA = {"type": "chromakey", "name": "chroma", "spec": f"{ASSET_DIR}/meta/key_chroma.json"}


def window(x: int, y: int, w: int, h: int) -> dict:
    return {"display": "window_absolute", "display_align": "top_left",
            "display_x": x, "display_y": y, "display_width": w, "display_height": h}


def source(name: str, options: list, win: dict | None = None) -> dict:
    return {"name": name, "type": "source", **(win or {}), "options": options}


def chroma_layer(name: str, win: dict | None = None, with_off: bool = True) -> dict:
    return {"name": name, "type": "transform", "children": 1, **(win or {}),
            "options": ([NONE] if with_off else []) + [CHROMA]}


def variant(name: str, layers: list, default: bool = False) -> dict:
    v = {"name": name, "length_frames": FRAMES, "width": CANVAS[0], "height": CANVAS[1], "layers": layers}
    if default:
        v["default"] = True
    return v


SHOWS = {
    # Class 7: same structure (one full-screen video), content / bitrate only
    "syn_content": [variant("main", [source("main", [
        vopt("calm", default=True), vopt("graphics"), vopt("texture"), vopt("noise"),
        vopt("texture_br1"), vopt("texture_br6")])], default=True)],

    # Class 1: one show whose combinations differ a lot in cost
    # (cheap background / heavy-motion background / full-screen chroma foreground)
    "syn_combo": [variant("main", [
        source("background", [vopt("calm", default=True), vopt("texture"), vopt("noise")]),
        source("foreground", [NONE, vopt("keyfg")]),
        chroma_layer("foreground_chroma"),
    ], default=True)],

    # Class 2: chroma cost vs the area of the keyed layer (25 / 50 / 100 % of the width).
    # The keyed video is made at its window size: Transform.chromakey draws its output at (0, 0)
    # and a scaled (windowed) child breaks the frame, so windows are unscaled and at the top-left
    "syn_chroma_area": [
        variant(f"w{pct}", [
            source("background", [vopt("calm", default=True)]),
            source("foreground", [vopt(name, "keyfg", default=True)], win),
            chroma_layer("foreground_chroma", win),
        ], default=(pct == 25))
        for pct, name, win in [(25, "keyfg_320", window(0, 0, 320, 180)),
                               (50, "keyfg_640", window(0, 0, 640, 360)),
                               (100, "keyfg", None)]],

    # Class 4: decoded resolution vs displayed area
    "syn_res": [
        variant("d720_full", [source("main", [vopt("texture", default=True)])], default=True),
        variant("d1080_full", [source("main", [vopt("texture_1080", default=True)])]),
        variant("d480_full", [source("main", [vopt("texture_480", default=True)])]),
        variant("d1080_small", [
            source("background", [vopt("calm", default=True)]),
            source("pip", [vopt("texture_1080", default=True)], window(32, 32, 320, 180)),
        ]),
    ],

    # Class 5: number of simultaneous video streams, no transforms
    "syn_grid": [
        variant("grid4", [source(f"tile{i}", [vopt(n, default=True)], window((i % 2) * 640, (i // 2) * 360, 640, 360))
                          for i, n in enumerate(["tile360_calm", "tile360_graphics", "tile360_texture", "tile360_noise"])],
                default=True),
        variant("grid9", [source(f"tile{i}", [vopt(n, default=True)], window((i % 3) * 426, (i // 3) * 240, 426, 240))
                          for i, n in enumerate(["tile240_graphics", "tile240_texture", "tile240_noise"] * 3)]),
    ],
}


def root_nodes(layers: list) -> int:
    """[Honest] Number of root nodes in Dana's layer graph (Experience.dn), which is the render count
    the offload request carries after the orientation: a source adds a root, a transform with
    children 1 replaces the last root, a transform with children 0 merges all roots into one."""
    roots = 0
    for layer in layers:
        if layer["type"] == "transform":
            roots = roots if layer.get("children", 0) == 1 else 1
        else:
            roots += 1
    return roots


def request_paths(show: str, variants: list) -> list[str]:
    """[Honest] Every option combination of every variant as an offload request path (landscape).
    A chroma transform over an 'off' foreground has nothing to key, so that combination is left out."""
    paths = []
    for v in variants:
        combos = [[]]
        for layer in v["layers"]:
            combos = [c + [(layer, o)] for c in combos for o in layer["options"]]
        for combo in combos:
            chosen = {l["name"]: o for l, o in combo}
            if chosen.get("foreground_chroma", NONE)["type"] == "chromakey" and chosen.get("foreground", {}).get("type") == "none":
                continue
            layers = "/".join(f"{l['name']}|{o['name']}" for l, o in combo)
            paths.append(f"/offload/show/{show}.json/{SEGMENT_PLACEHOLDER}/{CANVAS[0]}/{CANVAS[1]}/"
                         f"{v['name']}/landscape/{root_nodes(v['layers'])}/{layers}")
    return paths


def main(force: bool):
    for d in (VIDEO_DIR, META_DIR, SHOW_DIR):
        d.mkdir(parents=True, exist_ok=True)

    print("encoding videos")
    ensure_segment_tool()
    # Videos are independent and the lavfi generators (mandelbrot) are single-threaded,
    # so run one ffmpeg per video in parallel; x264 threads share the remaining cores
    with ThreadPoolExecutor(max_workers=len(VIDEOS)) as pool:
        for f in [pool.submit(encode_video, name, force) for name in VIDEOS]:
            f.result()
    for name in VIDEOS:
        (META_DIR / f"{name}.json").write_text(json.dumps(video_meta(name), indent=1) + "\n")
    (META_DIR / "key_chroma.json").write_text(
        json.dumps({"r": KEY_RGB[0], "g": KEY_RGB[1], "b": KEY_RGB[2], "variance": 22}) + "\n")

    print("writing shows")
    kinds = []
    for show, variants in SHOWS.items():
        (SHOW_DIR / f"{show}.json").write_text(json.dumps({"variants": variants}, indent=1) + "\n")
        first = variants[0]["layers"][0]["options"][0]["spec"].split("/")[-1][:-5]
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(VIDEO_DIR / f"{first}_seg_0_10.h264"),
                        "-frames:v", "1", "-vf", "scale=320:-2", str(SHOW_DIR / f"{show}.png")], check=True)
        paths = request_paths(show, variants)
        kinds += paths
        print(f"  {show}: {len(variants)} variant(s), {len(paths)} request kinds")
    (ROOT / "request_kind_syn.txt").write_text("\n".join(kinds) + "\n")
    print(f"{len(kinds)} request kinds -> request_kind_syn.txt")


if __name__ == "__main__":
    main("--force" in sys.argv)
