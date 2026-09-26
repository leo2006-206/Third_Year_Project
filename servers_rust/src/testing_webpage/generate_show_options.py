#!/usr/bin/env python3
"""
generate_show_options.py

Scans a directory containing OBM show definition files (*.json)
and generates / updates show_options.json in place for the evaluation testing webpage.
"""

import json
import sys
from pathlib import Path

SHOW_TITLE_MAP = {
    "f1_full.json": "Formula 1 Race (Full Experience)",
    "f1.json": "Formula 1 Race (Single Variant)",
    "forest720_leaves.json": "Forest 720p (Multi-Layer Leaves & Creature)",
    "forest720.json": "Forest 720p (Standard)",
    "forecast.json": "Weather Forecast",
    "spiders.json": "Spiders Animation",
}


def get_show_title(filename: str) -> str:
    """[Honest / Pure Domain Logic]
    Resolves human-readable titles for shows, including dynamic offload variants.
    """
    if filename in SHOW_TITLE_MAP:
        return SHOW_TITLE_MAP[filename]
    if filename.endswith("_offload.json"):
        base_name = filename.replace("_offload.json", ".json")
        base_title = SHOW_TITLE_MAP.get(base_name, base_name)
        return f"{base_title} (Offload)"
    return filename


def transform_show(filename: str, raw: dict) -> dict | None:
    """[Honest / Pure Domain Logic]
    Deterministically transforms raw show JSON structure into catalog schema.
    Returns None if no variants are defined.
    """
    raw_variants = raw.get("variants", [])
    if not raw_variants:
        return None

    first_var = raw_variants[0]
    catalog_variants = []
    for v in raw_variants:
        catalog_layers = []
        for layer in v.get("layers", []):
            raw_options = layer.get("options", [])
            option_names = [opt["name"] for opt in raw_options if opt.get("name")]
            default_opt = next(
                (opt["name"] for opt in raw_options if opt.get("default")),
                option_names[0] if option_names else "",
            )
            catalog_layers.append(
                {
                    "name": layer.get("name", "layer"),
                    "options": option_names,
                    "default": default_opt,
                }
            )

        variant_dict = {
            "name": v.get("name", "core"),
            "style": v.get("style", "landscape"),
            "default": v.get("default", False),
            "total_layers": len(catalog_layers),
            "layers": catalog_layers,
        }
        if "offloadLayers" in v:
            variant_dict["offload_layers"] = v["offloadLayers"]

        catalog_variants.append(variant_dict)

    # Ensure at least one variant is marked default
    if not any(v["default"] for v in catalog_variants) and catalog_variants:
        catalog_variants[0]["default"] = True

    return {
        "id": filename,
        "title": get_show_title(filename),
        "width": first_var.get("width", 1280),
        "height": first_var.get("height", 720),
        "fps": 25,
        "length_frames": first_var.get("length_frames", 600),
        "segment_length_sec": 10,
        "variants": catalog_variants,
    }


def generate_show_options(shows_dir, output_file=None):
    """[Dishonest / I/O Boundary Adapter]
    Reads show JSON files from `shows_dir` and writes a unified `show_options.json`.
    If `output_file` is None, writes in-place to 'show_options.json' in this script's directory.
    """
    shows_path = Path(shows_dir).resolve()
    if not shows_path.is_dir():
        raise FileNotFoundError(f"Shows directory not found: {shows_path}")

    output_path = (
        Path(__file__).parent / "show_options.json"
        if output_file is None
        else Path(output_file).resolve()
    )

    catalog = {"shows": []}

    for json_path in sorted(shows_path.glob("*.json")):
        try:
            raw = json.loads(json_path.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, OSError) as e:
            print(f"Skipping {json_path.name} due to parse error: {e}")
            continue

        show = transform_show(json_path.name, raw)
        if show:
            catalog["shows"].append(show)

    output_path.write_text(json.dumps(catalog, indent=2), encoding="utf-8")
    print(
        f"Successfully generated {output_path} from {shows_path} ({len(catalog['shows'])} shows)."
    )
    return catalog


if __name__ == "__main__":
    # Default to the obm/shows directory in this repository
    default_shows_dir = Path(__file__).resolve().parents[3] / "obm" / "shows"

    target_dir = sys.argv[1] if len(sys.argv) > 1 else default_shows_dir
    target_out = sys.argv[2] if len(sys.argv) > 2 else None

    generate_show_options(target_dir, target_out)
