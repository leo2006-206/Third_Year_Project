#!/usr/bin/env python3
"""
IDEAL-WORLD (FAKE) PSC SHOW forecast_psc (D95 / D98). NOT a realistic workload: a best-case show built only to show the
regime where the partial segment cache (PSC) wins ("what if there is an ideal world for PSC"). Report it next to the
BBC-set results, never as one of them.

Show = forecast 'core' 1280x720, 2075 frames (9 segments):
  map / weather / presenter fixed to their DEFAULT option (1 option each) + presenter_chroma + time   = 4 roots = heavy prefix
  badge_a: anim_img, 10 options A0..A9, BIG   badge 480x120 at (40, 560)   (lower left)
  badge_b: anim_img, 10 options B0..B9, SMALL badge 160x40  at (40, 40)    (top left)
  -> 6 roots, 100 request kinds; heavy PSC cut k = 4 (all fixed layers), cheap cut k = 5 too.
Each option = one PNG (coloured box + label) shown on every frame. Different sizes per layer (user D98); the cost model
prices anim_img by the LAYER display area (full frame), so both badge layers get identical feature rows.

Builds, in the same layout as obm/ (asset paths are relative to the web root, like syn_video/build.py):
  assets/psc_ideal/png/<opt>.png                    badge pictures
  assets/psc_ideal/meta/<opt>.af + <opt>_seg_*.af   AnimateAF animation + 10 s segments (obm/tools AnimateAF / SegmentAF)
  assets/psc_ideal/meta/<opt>.json                  anim_img meta (format of obm/assets/forecaster/meta/time_osm.json)
  shows/forecast_psc.json                           show spec
  request_kind_psc.txt                              all 100 request kinds (t_start/t_end placeholder)
  contact.png                                       all badges on one sheet (visual check)

Usage: ./build.py [--force]     (--force rebuilds .af files that already exist; needs `dana` on PATH and tools/AnimateAF.o +
       tools/SegmentAF.o: dnc ../../obm/tools/<X>.dn -o tools/<X>.o)
"""
import json
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
OBM = HERE.parent.parent / "obm"
ASSET_DIR = "assets/psc_ideal"
PNG_DIR = HERE / ASSET_DIR / "png"
META_DIR = HERE / ASSET_DIR / "meta"
SHOW = "forecast_psc.json"
FRAMES = 2250  # 9 full segments: SegmentAF drops a partial last segment, show needs t0 0..80 (2075 frames)
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"

# layer name -> (option prefix, x, y, w, h)
BADGES = {
    "badge_a": ("A", 40, 560, 480, 120),
    "badge_b": ("B", 40, 40, 160, 40),
}
M = 10
# 10 well separated colours (option i of every layer)
COLOURS = ["#c0392b", "#d35400", "#f1c40f", "#27ae60", "#16a085", "#2980b9", "#8e44ad", "#2c3e50", "#7f8c8d", "#e84393"]


# ==============================================================================
# 1. Honest Functions
# ==============================================================================


def options() -> list[tuple[str, str, int, int, int, int, int]]:
    """[Honest] (layer, option name, colour index, x, y, w, h) for every badge option."""
    return [(layer, f"{p}{i}", i, x, y, w, h) for layer, (p, x, y, w, h) in BADGES.items() for i in range(M)]


def show_spec(base: dict) -> dict:
    """[Honest] forecast core with map / weather / presenter fixed to the default option + the badge layers."""
    v = json.loads(json.dumps(next(x for x in base["variants"] if x["name"] == "core")))
    for layer in v["layers"]:
        opts = [o for o in layer["options"] if o.get("default")] or layer["options"][:1]
        layer["options"] = opts
        layer.pop("optionSelect", None)
    for layer, (p, *_r) in BADGES.items():
        v["layers"].append({
            "name": layer, "type": "source",
            "options": [{"type": "anim_img", "name": f"{p}{i}", "spec": f"{ASSET_DIR}/meta/{p}{i}.json", **({"default": True} if i == 0 else {})}
                        for i in range(M)],
            "optionSelect": {"type": "track-listNames", "icon": "assets/forecaster/icons/ui/weather_toggle.png"},
        })
    v["default"] = True
    return {"variants": [v]}  # no comment key: Dana's json loader may reject unknown fields; the label is in README.md


def request_kinds(spec: dict) -> list[str]:
    """[Honest] every option combination; count token = roots (source layers)."""
    v = spec["variants"][0]
    fixed = [f"{l['name']}|{l['options'][0]['name']}" for l in v["layers"] if not l["name"].startswith("badge_")]
    roots = sum(1 for l in v["layers"] if l["type"] == "source")
    head = f"/offload/show/{SHOW}/t_start/t_end/{v['width']}/{v['height']}/core/landscape/{roots}/" + "/".join(fixed)
    a, b = ([o["name"] for o in l["options"]] for l in v["layers"] if l["name"].startswith("badge_"))
    return [f"{head}/badge_a|{x}/badge_b|{y}" for x in a for y in b]


# ==============================================================================
# 2. Dishonest Functions (files, dana)
# ==============================================================================


def make_png(name: str, ci: int, w: int, h: int) -> Path:
    """[Dishonest I/O] coloured box + white border + label, RGBA."""
    img = Image.new("RGBA", (w, h), COLOURS[ci])
    d = ImageDraw.Draw(img)
    d.rectangle([0, 0, w - 1, h - 1], outline="white", width=max(2, h // 20))
    font = ImageFont.truetype(FONT, int(h * 0.55))
    label = f"{name} ideal"
    tw, th = d.textbbox((0, 0), label, font=font)[2:]
    d.text(((w - tw) // 2, (h - th) // 2 - h // 12), label, fill="white", font=font)
    out = PNG_DIR / f"{name}.png"
    img.save(out)
    return out


def dana(*args: str):
    """[Dishonest I/O] run an obm tool from HERE (asset paths in the .af are relative to the web root)."""
    tool, *rest = args
    # compiled from obm/tools/*.dn into HERE/tools (obm/ is not touched): dnc obm/tools/X.dn -o syn_video/PSC_ideal/tools/X.o
    subprocess.run(["dana", str(HERE / "tools" / (tool.split(".")[1] + ".o")), *rest], cwd=HERE, check=True, stdout=subprocess.DEVNULL)


def make_af(name: str, x: int, y: int, w: int, h: int, force: bool):
    """[Dishonest I/O] AnimateAF header + frames, SegmentAF 10 s; meta json."""
    af = f"{ASSET_DIR}/meta/{name}.af"
    if force or not (HERE / af).exists():
        for old in META_DIR.glob(f"{name}_seg_*.af"):
            old.unlink()
        dana("tools.AnimateAF", "header", af, "25", "0", "0", "1280", "720", "0", "0", "1", f"{ASSET_DIR}/png/{name}.png")
        dana("tools.AnimateAF", "frames", af, str(FRAMES), "xpan", "0", "0", "ypan", "0", "0",
             "ent:0:x", str(x), str(x), "ent:0:y", str(y), str(y), "ent:0:w", str(w), str(w), "ent:0:h", str(h), str(h))
        dana("tools.SegmentAF", af, "time", "10000")
    meta = {"speed": 25, "xpan": 0, "ypan": 0, "entities": [
        {"seg_base": f"{ASSET_DIR}/meta/{name}_seg", "seg_ext": "af", "w": 1280, "h": 720, "frames": FRAMES, "segment_length_sec": 10}]}
    (META_DIR / f"{name}.json").write_text(json.dumps(meta, indent=1) + "\n")


def contact_sheet(pngs: list[Path]):
    """[Dishonest I/O] all badges on one sheet."""
    # A column left (scaled to half), B column right
    sheet = Image.new("RGB", (760, M * 70 + 20), "#202020")
    for i, p in enumerate(pngs):
        im = Image.open(p)
        col, row = divmod(i, M)
        if im.width > 300:
            im = im.resize((im.width // 2, im.height // 2))
        sheet.paste(im, (20 + col * 300, 10 + row * 70), im)
    sheet.save(HERE / "contact.png")


def main():
    force = "--force" in sys.argv
    PNG_DIR.mkdir(parents=True, exist_ok=True)
    META_DIR.mkdir(parents=True, exist_ok=True)
    opts = options()
    pngs = [make_png(n, ci, w, h) for _l, n, ci, _x, _y, w, h in opts]
    with ThreadPoolExecutor(4) as ex:
        list(ex.map(lambda o: make_af(o[1], *o[3:], force), opts))
    contact_sheet(pngs)
    spec = show_spec(json.loads((OBM / "shows/forecast.json").read_text()))
    (HERE / "shows").mkdir(exist_ok=True)
    (HERE / "shows" / SHOW).write_text(json.dumps(spec, indent=4) + "\n")
    kinds = request_kinds(spec)
    (HERE / "request_kind_psc.txt").write_text("\n".join(kinds) + "\n")
    n_seg = len(list(META_DIR.glob("A0_seg_*.af")))
    print(f"{len(pngs)} badges, {n_seg} segments per badge, {len(kinds)} request kinds -> {HERE.relative_to(HERE.parent.parent)}")


if __name__ == "__main__":
    main()
