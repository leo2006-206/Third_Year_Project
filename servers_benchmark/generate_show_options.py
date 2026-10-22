#!/usr/bin/env python3
"""
generate_show_options.py

Scans a directory containing OBM show definition files (*.json)
and generates:
- show_options.json: show catalog for the testing webpage and main_server
- video_duration.csv: max duration (in seconds) for each video
- request_kind.txt: unique offload request templates using placeholder '/x/y/'
"""

import itertools
import json
import sys
from pathlib import Path


def compute_dag_root_count(raw_layers: list[dict]) -> int:
    """[Honest / Pure Domain Logic]
    Computes the exact number of DAG root nodes (renderable screen nodes)
    built by Dana's Experience.dn.
    In Dana, absent 'children' in JSON defaults to 0 (default integer in Dana).
    - source / source-multi: appends 1 node to graph.
    - transform with children == 1: wraps the preceding node (graph length unchanged).
    - transform with children == 0: wraps all preceding nodes (graph length becomes 1).
    """
    root_count = 0
    for layer in raw_layers:
        sense = layer.get("sense", "visual")
        if sense == "audio":
            continue
        ltype = layer.get("type", "source")
        if ltype in ("source", "source-multi"):
            root_count += 1
        elif ltype == "transform":
            children = layer.get("children", 0)  # In Dana, absent int defaults to 0
            if children == 0:
                root_count = 1
            # if children == 1: wraps preceding node, root_count unchanged
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
            "width": v.get("width", first_var.get("width", 1280)),
            "height": v.get("height", first_var.get("height", 720)),
            "length_frames": v.get(
                "length_frames", first_var.get("length_frames", 600)
            ),
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
        fps = show.get("fps", 25)
        seg_sec = show.get("segment_length_sec", 10)

        for variant in show.get("variants", []):
            var_name = variant.get("name", "core")
            var_style = variant.get("style", "landscape")
            width = variant.get("width", show.get("width", 1280))
            height = variant.get("height", show.get("height", 720))
            length_frames = variant.get("length_frames", show.get("length_frames", 600))

            # Total 10-second segments covering valid show duration
            total_segs = (length_frames + (seg_sec * fps) - 1) // (seg_sec * fps)

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


def generate_request_kinds(catalog: dict) -> list[str]:
    """[Honest / Pure Domain Logic]
    Generates unique offload request templates across all shows in the catalog.
    Replaces the time segment part with placeholder
    to eliminate duplicate segment copies while preserving all layer combinations.
    """
    kinds: list[str] = []

    for show in catalog.get("shows", []):
        show_id = show["id"]

        for variant in show.get("variants", []):
            var_name = variant.get("name", "core")
            var_style = variant.get("style", "landscape")
            width = variant.get("width", show.get("width", 1280))
            height = variant.get("height", show.get("height", 720))
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
                base = f"/offload/show/{show_id}/t_start/t_end/{width}/{height}/{var_name}/{var_style}/{total_layers}"
                url = f"{base}/{tokens_str}" if tokens_str else base
                kinds.append(url)

    return kinds


def compute_video_durations(catalog: dict) -> list[tuple[str, int]]:
    """[Honest / Pure Domain Logic]
    Computes the maximum duration in seconds for each video in the catalog
    based on its valid segment ranges (e.g. if the last segment is 80/90, duration is 90).
    """
    durations: list[tuple[str, int]] = []

    for show in catalog.get("shows", []):
        show_id = show["id"]
        fps = show.get("fps", 25)
        seg_sec = show.get("segment_length_sec", 10)
        max_duration = 0

        for variant in show.get("variants", []):
            length_frames = variant.get("length_frames", show.get("length_frames", 600))
            total_segs = (length_frames + (seg_sec * fps) - 1) // (seg_sec * fps)
            max_duration = max(max_duration, total_segs * seg_sec)

        durations.append((show_id, max_duration))

    return sorted(durations, key=lambda x: x[0])


def generate_show_options(
    shows_dir,
    output_file=None,
    request_kind_file=None,
    duration_file=None,
):
    """[Dishonest / I/O Boundary Adapter]
    Reads show JSON files from `shows_dir`, writes 'show_options.json',
    writes 'video_duration.csv', and writes template request URLs to
    'request_kind.txt' (leaving original 'requests.txt' untouched).
    """
    shows_path = Path(shows_dir).resolve()
    if not shows_path.is_dir():
        raise FileNotFoundError(f"Shows directory not found: {shows_path}")

    output_path = (
        Path(__file__).parent / "show_options.json"
        if output_file is None
        else Path(output_file).resolve()
    )
    request_kind_path = (
        output_path.parent / "request_kind.txt"
        if request_kind_file is None
        else Path(request_kind_file).resolve()
    )
    duration_path = (
        output_path.parent / "video_duration.csv"
        if duration_file is None
        else Path(duration_file).resolve()
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

    # 1. Generate video_duration.csv (2 columns: video,duration)
    durations = compute_video_durations(catalog)
    duration_lines = ["video,duration"] + [f"{vid},{dur}" for vid, dur in durations]
    duration_path.write_text("\n".join(duration_lines) + "\n", encoding="utf-8")
    print(f"Successfully generated {duration_path} ({len(durations)} video durations).")

    # 2. Generate request_kind.txt (/x/y/ template URLs)
    kinds = generate_request_kinds(catalog)
    kinds_content = "\n".join(kinds) + "\n"
    request_kind_path.write_text(kinds_content, encoding="utf-8")
    print(
        f"Successfully generated {request_kind_path} ({len(kinds)} unique request templates)."
    )

    # If using default path, also write reqeust_kind.txt so both spellings exist
    if request_kind_file is None:
        alt_path = output_path.parent / "reqeust_kind.txt"
        alt_path.write_text(kinds_content, encoding="utf-8")

    return catalog


if __name__ == "__main__":
    # Default to the obm/shows directory in this repository
    default_shows_dir = Path(__file__).resolve().parent.parent / "obm" / "shows"

    target_dir = sys.argv[1] if len(sys.argv) > 1 else default_shows_dir
    target_out = sys.argv[2] if len(sys.argv) > 2 else None
    target_kind = sys.argv[3] if len(sys.argv) > 3 else None
    target_dur = sys.argv[4] if len(sys.argv) > 4 else None

    generate_show_options(target_dir, target_out, target_kind, target_dur)
