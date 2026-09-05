#!/usr/bin/env python3
"""
generate_show_options.py

Scans a directory containing OBM show definition files (*.json)
and generates / updates show_options.json in place for the evaluation testing webpage.
"""

import os
import sys
import json
from pathlib import Path


SHOW_TITLE_MAP = {
    "f1_full.json": "Formula 1 Race (Full Experience)",
    "f1.json": "Formula 1 Race (Single Variant)",
    "forest720_leaves.json": "Forest 720p (Multi-Layer Leaves & Creature)",
    "forest720.json": "Forest 720p (Standard)",
    "forecast.json": "Weather Forecast",
    "spiders.json": "Spiders Animation"
}


def generate_show_options(shows_dir, output_file=None):
    """
    Reads all show JSON files in `shows_dir` and writes a unified `show_options.json`.
    If `output_file` is None, writes in-place to 'show_options.json' in this script's directory.
    """
    shows_path = Path(shows_dir).resolve()
    if not shows_path.is_dir():
        raise FileNotFoundError(f"Shows directory not found: {shows_path}")

    if output_file is None:
        output_file = Path(__file__).parent / "show_options.json"
    else:
        output_file = Path(output_file).resolve()

    catalog = {"shows": []}

    # Find all JSON show files
    json_files = sorted(shows_path.glob("*.json"))

    for json_path in json_files:
        filename = json_path.name

        try:
            with open(json_path, "r", encoding="utf-8") as f:
                raw = json.load(f)
        except Exception as e:
            print(f"Skipping {filename} due to parse error: {e}")
            continue

        raw_variants = raw.get("variants", [])
        if not raw_variants:
            continue

        first_var = raw_variants[0]
        width = first_var.get("width", 1280)
        height = first_var.get("height", 720)
        length_frames = first_var.get("length_frames", 600)
        title = filename

        catalog_variants = []
        for v in raw_variants:
            v_name = v.get("name", "core")
            v_style = v.get("style", "landscape")
            v_default = v.get("default", False)
            raw_layers = v.get("layers", [])

            catalog_layers = []
            for layer in raw_layers:
                layer_name = layer.get("name", "layer")
                raw_options = layer.get("options", [])

                option_names = []
                default_opt = None

                for opt in raw_options:
                    opt_name = opt.get("name", "")
                    if opt_name:
                        option_names.append(opt_name)
                    if opt.get("default", False) and default_opt is None:
                        default_opt = opt_name

                # Fallback to the first option if none is explicitly marked default
                if default_opt is None and option_names:
                    default_opt = option_names[0]

                catalog_layers.append({
                    "name": layer_name,
                    "options": option_names,
                    "default": default_opt or ""
                })

            catalog_variants.append({
                "name": v_name,
                "style": v_style,
                "default": v_default,
                "total_layers": len(catalog_layers),
                "layers": catalog_layers
            })

        # Ensure at least one variant is marked default
        if not any(v["default"] for v in catalog_variants) and catalog_variants:
            catalog_variants[0]["default"] = True

        catalog["shows"].append({
            "id": filename,
            "title": title,
            "width": width,
            "height": height,
            "fps": 25,
            "length_frames": length_frames,
            "segment_length_sec": 10,
            "variants": catalog_variants
        })

    # Write out the resulting show_options.json in place
    with open(output_file, "w", encoding="utf-8") as f:
        json.dump(catalog, f, indent=2)

    print(f"Successfully generated {output_file} from {shows_path} ({len(catalog['shows'])} shows).")
    return catalog


if __name__ == "__main__":
    # Default to the obm/shows directory in this repository
    default_shows_dir = Path(__file__).resolve().parent.parent.parent.parent / "obm" / "shows"

    target_dir = sys.argv[1] if len(sys.argv) > 1 else default_shows_dir
    target_out = sys.argv[2] if len(sys.argv) > 2 else None

    generate_show_options(target_dir, target_out)

