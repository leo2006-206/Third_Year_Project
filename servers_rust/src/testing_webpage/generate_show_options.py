#!/usr/bin/env python3
"""
generate_show_options.py

Scans a directory containing OBM show definition files (*.json)
and generates / updates show_options.json in place for the evaluation testing webpage.
"""

import itertools
import json
import sys
from pathlib import Path


def compute_dag_root_count(raw_layers: list[dict]) -> int:
    """[Honest / Pure Domain Logic]
    Computes the exact number of DAG root nodes (renderable screen nodes)
    built by Dana's Experience.dn.
    Source layers create root nodes; transform layers wrap existing child nodes
    (children == 1 wraps the preceding node; children == 0 wraps all nodes).
    """
    root_count = 0
    for layer in raw_layers:
        ltype = layer.get("type", "source")
        if ltype in ("source", "source-multi"):
            root_count += 1
        elif ltype == "transform" and layer.get("children", 1) == 0:
            root_count = 1
    return root_count


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
        raw_layers = v.get("layers", [])
        catalog_layers = []
        for layer in raw_layers:
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
            "total_layers": compute_dag_root_count(raw_layers),
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
        "title": filename,
        "width": first_var.get("width", 1280),
        "height": first_var.get("height", 720),
        "fps": 25,
        "length_frames": first_var.get("length_frames", 600),
        "segment_length_sec": 10,
        "variants": catalog_variants,
    }


def generate_full_offload_urls(catalog: dict) -> list[str]:
    """[Honest / Pure Domain Logic]
    Generates all valid full-offload request URLs across all shows in the catalog.
    Strictly iterates over valid 10-second segments (within total show duration)
    and all layer option permutations.
    """
    urls: list[str] = []

    for show in catalog.get("shows", []):
        show_id = show["id"]
        width = show.get("width", 1280)
        height = show.get("height", 720)
        fps = show.get("fps", 25)
        length_frames = show.get("length_frames", 600)
        seg_sec = show.get("segment_length_sec", 10)

        # Total 10-second segments covering valid show duration
        total_segs = (length_frames + (seg_sec * fps) - 1) // (seg_sec * fps)

        for variant in show.get("variants", []):
            var_name = variant.get("name", "core")
            var_style = variant.get("style", "landscape")
            total_layers = variant.get("total_layers", len(variant.get("layers", [])))
            layers = variant.get("layers", [])

            # Cartesian product of layer options: [('race|race', 'driver|Sam', ...), ...]
            layer_combos = (
                list(
                    itertools.product(
                        *[
                            [f"{l['name']}|{opt}" for opt in l.get("options", [])]
                            for l in layers
                        ]
                    )
                )
                if layers
                else [()]
            )

            for combo in layer_combos:
                tokens_str = "/".join(combo)
                for seg_idx in range(total_segs):
                    t_from = seg_idx * seg_sec
                    t_to = (seg_idx + 1) * seg_sec
                    base = f"/offload/show/{show_id}/{t_from}/{t_to}/{width}/{height}/{var_name}/{var_style}/{total_layers}"
                    url = f"{base}/{tokens_str}" if tokens_str else base
                    urls.append(url)

    return urls


def generate_show_options(shows_dir, output_file=None, requests_file=None):
    """[Dishonest / I/O Boundary Adapter]
    Reads show JSON files from `shows_dir`, writes 'show_options.json', and writes
    all valid full-offload request URLs to 'requests.txt'.
    """
    shows_path = Path(shows_dir).resolve()
    if not shows_path.is_dir():
        raise FileNotFoundError(f"Shows directory not found: {shows_path}")

    output_path = (
        Path(__file__).parent / "show_options.json"
        if output_file is None
        else Path(output_file).resolve()
    )
    requests_path = (
        output_path.parent / "requests.txt"
        if requests_file is None
        else Path(requests_file).resolve()
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

    urls = generate_full_offload_urls(catalog)
    requests_path.write_text("\n".join(urls) + "\n", encoding="utf-8")
    print(
        f"Successfully generated {requests_path} ({len(urls)} full offload requests)."
    )

    return catalog


if __name__ == "__main__":
    # Default to the obm/shows directory in this repository
    default_shows_dir = Path(__file__).resolve().parents[3] / "obm" / "shows"

    target_dir = sys.argv[1] if len(sys.argv) > 1 else default_shows_dir
    target_out = sys.argv[2] if len(sys.argv) > 2 else None
    target_req = sys.argv[3] if len(sys.argv) > 3 else None

    generate_show_options(target_dir, target_out, target_req)
