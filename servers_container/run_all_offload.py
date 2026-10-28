#!/usr/bin/env python3
"""
OBM Offload Cluster Orchestrator
Validates endpoints (incl. cpu/memory caps) from an endpoint CSV (default offload_endpoint.csv),
pre-compiles the Rust offload_server,
pre-builds the offload Docker image, and launches all offload worker nodes into
separate tabs within GNOME Terminal
(or as detached containers with --detach, or when gnome-terminal is not installed).

Usage: ./run_all_offload.py [ENDPOINT_CSV] [--detach|-d] [--new-window|-w]
  e.g. ./run_all_offload.py offload_rs_c1.csv   (sites of one machine for one cluster case)
  MAIN_SERVER_ADDR (env): main server the sites fetch assets from, e.g. 10.10.0.1:7000 on rs
"""

import csv
import os
import shutil
import subprocess
import sys
import time
from collections.abc import Iterable
from pathlib import Path

# -----------------------------------------------------------------------------
# 1. Pure Domain Logic (Honest)
# -----------------------------------------------------------------------------


Endpoint = tuple[
    str, int, str, str, str, str, str, str, str
]  # id, port, dec, comp, enc, cpu, memory, delay_ms, jitter_ms


def parse_limit(value: str) -> float | None:
    """[Honest / Pure Domain Logic]
    Parses a resource limit: -1 (no cap) or a positive number. Returns None if invalid."""
    try:
        limit = float(value)
    except ValueError:
        return None
    return limit if limit == -1 or limit > 0 else None


def validate_endpoints(
    rows: Iterable[list[str]],
) -> tuple[list[Endpoint], list[str]]:
    """[Honest / Pure Domain Logic]
    Validates CSV rows for schema
    (id, port, decoding, composition, encoding, cpu, memory, delay_ms, jitter_ms),
    port integer ranges, valid values:
      - decoding: 'cpu', 'gpu'
      - composition: 'cpu', 'llvmpipe', 'gpu'
      - encoding: 'cpu', 'gpu'
      - cpu: CPU cores (docker --cpus), -1 = no cap
      - memory: GB (docker --memory), -1 = no cap
      - delay_ms / jitter_ms: netem delay/jitter on all site traffic, non-negative integers
    and duplicate IDs/ports.
    Returns a tuple of (valid_endpoints, errors).
    """
    seen_ids: dict[str, int] = {}
    seen_ports: dict[int, int] = {}
    endpoints: list[Endpoint] = []
    errors: list[str] = []

    for line_num, row in enumerate(rows, start=1):
        if not row:
            continue

        fields = [col.strip() for col in row]
        if not fields or not fields[0] or fields[0].startswith("#"):
            continue

        # Skip header line (e.g. "id", "port", ...)
        if (
            fields[0].lower() == "id"
            and len(fields) > 1
            and fields[1].lower() == "port"
        ):
            continue

        if len(fields) < 9 or not all(fields[:9]):
            errors.append(
                f"Line {line_num}: Invalid row format. Expected 'id, port, decoding, composition, encoding, cpu, memory, delay_ms, jitter_ms', got: {row}"
            )
            continue

        (inst_id, port_str, decoding_str, composition_str, encoding_str,
         cpu_str, memory_str, delay_str, jitter_str) = fields[:9]

        # Validate port integer range
        if not port_str.isdigit() or not (1 <= int(port_str) <= 65535):
            errors.append(
                f"Line {line_num}: Invalid port '{port_str}'. Must be an integer (1-65535)."
            )
            continue

        port = int(port_str)

        # Validate decoding ('cpu', 'gpu')
        decoding = decoding_str.lower()
        if decoding not in ("cpu", "gpu"):
            errors.append(
                f"Line {line_num}: Invalid decoding '{decoding_str}'. Must be 'cpu' or 'gpu'."
            )
            continue

        # Validate composition ('cpu', 'llvmpipe', 'gpu')
        composition = composition_str.lower()
        if composition not in ("cpu", "llvmpipe", "gpu"):
            errors.append(
                f"Line {line_num}: Invalid composition '{composition_str}'. Must be 'cpu', 'llvmpipe', or 'gpu'."
            )
            continue

        # Validate encoding ('cpu' only; 'gpu' disabled)
        encoding = encoding_str.lower()
        if encoding == "gpu":
            errors.append(
                f"Line {line_num}: GPU encoding is currently unsupported/disabled. Please set encoding to 'cpu'."
            )
            continue
        if encoding != "cpu":
            errors.append(
                f"Line {line_num}: Invalid encoding '{encoding_str}'. Only 'cpu' is supported."
            )
            continue

        # Validate resource caps (-1 = no cap)
        if parse_limit(cpu_str) is None:
            errors.append(
                f"Line {line_num}: Invalid cpu '{cpu_str}'. Must be -1 (no cap) or a positive number."
            )
            continue
        if parse_limit(memory_str) is None:
            errors.append(
                f"Line {line_num}: Invalid memory '{memory_str}'. Must be -1 (no cap) or a positive number of GB."
            )
            continue

        # Validate network emulation (ms, non-negative integers; jitter needs a delay)
        if not delay_str.isdigit() or not jitter_str.isdigit():
            errors.append(
                f"Line {line_num}: Invalid delay_ms '{delay_str}' / jitter_ms '{jitter_str}'. Must be non-negative integers."
            )
            continue
        if int(delay_str) == 0 and int(jitter_str) != 0:
            errors.append(f"Line {line_num}: jitter_ms requires delay_ms > 0.")
            continue

        # Check duplicate ID
        if inst_id in seen_ids:
            errors.append(
                f"Line {line_num}: Duplicate ID '{inst_id}' detected (already on line {seen_ids[inst_id]})."
            )
        else:
            seen_ids[inst_id] = line_num

        # Check duplicate Port
        if port in seen_ports:
            errors.append(
                f"Line {line_num}: Duplicate Port '{port}' detected (already on line {seen_ports[port]})."
            )
        else:
            seen_ports[port] = line_num

        endpoints.append(
            (inst_id, port, decoding, composition, encoding, cpu_str, memory_str,
             delay_str, jitter_str)
        )

    return endpoints, errors


def format_tab_command(title: str, run_cmd: str) -> str:
    """[Honest / Pure Domain Logic]
    Constructs a bash command string setting the ANSI window/tab title and pausing on exit.
    """
    return (
        f"printf '\\033]0;{title}\\007'; "
        f"{run_cmd}; "
        f"echo; echo 'Process ended. Press Enter to close tab.'; read"
    )


# -----------------------------------------------------------------------------
# 2. I/O Boundary Adapters & Step Functions (Dishonest)
# -----------------------------------------------------------------------------


def load_and_validate_endpoints(
    csv_file: Path,
) -> list[Endpoint]:
    """[Dishonest / I/O Boundary Adapter]
    Reads endpoint definitions from CSV file, executes validation, and halts on error.
    """
    with open(csv_file, "r", encoding="utf-8") as f:
        endpoints, errors = validate_endpoints(csv.reader(f))

    if errors:
        print(
            "===========================================================",
            file=sys.stderr,
        )
        print(
            "Validation failed: duplicate or invalid endpoints detected:",
            file=sys.stderr,
        )
        for err in errors:
            print(f"  • {err}", file=sys.stderr)
        print(f"Please fix {csv_file} before continuing.", file=sys.stderr)
        print(
            "===========================================================",
            file=sys.stderr,
        )
        sys.exit(1)

    if not endpoints:
        sys.exit(f"Error: No valid endpoints found in {csv_file}")

    print(f"✓ Validated {len(endpoints)} offload endpoint(s) with no collisions.")
    return endpoints


def build_rust_offload(project_root: Path) -> None:
    """[Dishonest / Build Driver]
    Compiles the optimized Rust offload server binary.
    """
    print("\n=== [1/3] Pre-compiling Rust Offload Server (--release) ===")
    subprocess.run(
        ["cargo", "build", "--release", "--bin", "offload_server"],
        cwd=project_root,
        check=True,
    )


def build_offload_image(
    dockerfile: Path, project_root: Path, image_tag: str = "obm-offload-server"
) -> None:
    """[Dishonest / Build Driver]
    Builds the Docker image for the offload server nodes.
    """
    print(f"\n=== [2/3] Pre-building Docker Image: {image_tag} ===")
    subprocess.run(
        ["docker", "build", "-f", str(dockerfile), "-t", image_tag, "."],
        cwd=project_root,
        check=True,
    )


def setup_docker_environment(endpoints: list[Endpoint]) -> None:
    """[Dishonest / Container Driver]
    Ensures the shared Docker bridge network exists and removes stale offload containers.
    """
    subprocess.run(
        ["docker", "network", "create", "obm-net"], capture_output=True, check=False
    )

    stale_containers = [f"obm-offload-{inst_id}" for inst_id, *_ in endpoints]
    subprocess.run(
        ["docker", "rm", "-f", *stale_containers], capture_output=True, check=False
    )


def spawn_detached(endpoints: list[Endpoint], offload_run_sh: Path) -> None:
    """[Dishonest / Container Runner]
    Starts every offload node as a detached container (no terminal needed, e.g. over SSH).
    """
    print("\n=== [3/3] Starting Offload Server Nodes (detached) ===")
    env = dict(os.environ, DETACH="1")
    for inst_id, port, decoding, composition, encoding, cpu, memory, delay, jitter in endpoints:
        subprocess.run(
            [str(offload_run_sh), inst_id, str(port), decoding, composition, encoding,
             cpu, memory, delay, jitter, "--skip-build"],
            env=env,
            check=True,
        )
    print("\nAll offload server nodes started in the background.")
    print("Follow logs with:  docker logs -f obm-offload-<id>")
    print("Stop all with:     docker rm -f $(docker ps -q --filter name=obm-offload-)")


def spawn_terminal_tabs(
    endpoints: list[Endpoint],
    offload_run_sh: Path,
    new_window: bool,
) -> None:
    """[Dishonest / Terminal Runner]
    Spawns interactive tabs for each offload worker node inside GNOME Terminal.
    """
    print("\n=== [3/3] Spawning Offload Server Nodes in GNOME Terminal ===")
    if not shutil.which("gnome-terminal"):
        sys.exit("Error: 'gnome-terminal' command not found in PATH.")

    tabs = []
    for inst_id, port, decoding, composition, encoding, cpu, memory, delay, jitter in endpoints:
        cpu_label = "nocap" if cpu == "-1" else f"{cpu}cpu"
        mem_label = "nocap" if memory == "-1" else f"{memory}G"
        net_label = f" {delay}±{jitter}ms" if delay != "0" else ""
        title = f"Offload-{inst_id} ({port}/{decoding.upper()}-{composition.upper()}-{encoding.upper()} {cpu_label}/{mem_label}{net_label})"
        # gnome-terminal tabs don't inherit this process's environment
        main_env = f'MAIN_SERVER_ADDR="{os.environ["MAIN_SERVER_ADDR"]}" ' if "MAIN_SERVER_ADDR" in os.environ else ""
        run_cmd = f'{main_env}"{offload_run_sh}" "{inst_id}" "{port}" "{decoding}" "{composition}" "{encoding}" "{cpu}" "{memory}" "{delay}" "{jitter}" --skip-build'
        tabs.append({"title": title, "cmd": format_tab_command(title, run_cmd)})

    if new_window:
        print(" -> Launching all nodes in a new window with tabs...")
        cmd = [
            "gnome-terminal",
            "--window",
            "--title",
            tabs[0]["title"],
            "--",
            "bash",
            "-c",
            tabs[0]["cmd"],
        ]
        for tab in tabs[1:]:
            cmd.extend(
                ["--tab", "--title", tab["title"], "--", "bash", "-c", tab["cmd"]]
            )
        subprocess.run(cmd, check=True)
    else:
        for tab in tabs:
            print(f" -> Adding tab for [{tab['title']}]...")
            subprocess.run(
                [
                    "gnome-terminal",
                    "--tab",
                    "--title",
                    tab["title"],
                    "--",
                    "bash",
                    "-c",
                    tab["cmd"],
                ],
                check=True,
            )
            time.sleep(0.3)

    print("\nAll offload server nodes successfully launched!")
    for tab in tabs:
        print(f"   • {tab['title']}")
    print("\nTo stop all offload containers when done, run:")
    print("  docker rm -f $(docker ps -q --filter name=obm-offload-)")


# -----------------------------------------------------------------------------
# 3. Main Orchestrator CLI
# -----------------------------------------------------------------------------


def main(argv: list[str] | None = None) -> None:
    """[Dishonest / CLI Entrypoint]
    Top-level orchestrator: verifies files, validates endpoints, builds binaries/images,
    and launches offload nodes.
    """
    args = sys.argv[1:] if argv is None else argv
    positional = [a for a in args if not a.startswith("-")]
    if len(positional) > 1:
        sys.exit("Usage: run_all_offload.py [ENDPOINT_CSV] [--detach|-d] [--new-window|-w]")

    script_dir = Path(__file__).resolve().parent
    project_root = script_dir.parent
    csv_file = (
        Path(positional[0]).resolve() if positional else script_dir / "offload_endpoint.csv"
    )
    offload_run_sh = script_dir / "offload_server" / "run_sh.sh"
    dockerfile = script_dir / "offload_server" / "dockerfile"
    # Validate prerequisite files
    for path, desc in [
        (csv_file, "Endpoint CSV"),
        (offload_run_sh, "Offload run script"),
        (dockerfile, "Offload Dockerfile"),
    ]:
        if not path.is_file():
            sys.exit(f"Error: {desc} not found: {path}")

    offload_run_sh.chmod(0o755)

    # 1. Parse & validate endpoints
    endpoints = load_and_validate_endpoints(csv_file)

    # 2. Pre-compile Rust offload binary
    build_rust_offload(project_root)

    # 3. Pre-build Docker image
    build_offload_image(dockerfile, project_root)

    # 4. Prepare network and clean up stale instances
    setup_docker_environment(endpoints)

    # 5. Launch nodes: terminal tabs, or detached containers (--detach / no gnome-terminal)
    if "--detach" in args or "-d" in args or not shutil.which("gnome-terminal"):
        spawn_detached(endpoints, offload_run_sh)
    else:
        new_window = "--new-window" in args or "-w" in args
        spawn_terminal_tabs(endpoints, offload_run_sh, new_window)


if __name__ == "__main__":
    main()
