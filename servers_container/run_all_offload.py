#!/usr/bin/env python3
"""
OBM Offload Cluster Orchestrator
Validates endpoints from offload_endpoint.csv, pre-compiles the Rust offload_server,
pre-builds the offload Docker image, and launches all offload worker nodes into
separate tabs within GNOME Terminal.
"""

import csv
import shutil
import subprocess
import sys
import time
from collections.abc import Iterable
from pathlib import Path

# -----------------------------------------------------------------------------
# 1. Pure Domain Logic (Honest)
# -----------------------------------------------------------------------------


def validate_endpoints(
    rows: Iterable[list[str]],
) -> tuple[list[tuple[str, int, str]], list[str]]:
    """[Honest / Pure Domain Logic]
    Validates CSV rows for schema (id, port, device), port integer ranges,
    valid device types ('cpu' or 'gpu'), and duplicate IDs/ports.
    Returns a tuple of (valid_endpoints, errors).
    """
    seen_ids: dict[str, int] = {}
    seen_ports: dict[int, int] = {}
    endpoints: list[tuple[str, int, str]] = []
    errors: list[str] = []

    for line_num, row in enumerate(rows, start=1):
        if not row:
            continue

        fields = [col.strip() for col in row]
        if not fields or not fields[0] or fields[0].startswith("#"):
            continue

        # Skip header line (e.g. "id", "port", "device")
        if (
            fields[0].lower() == "id"
            and len(fields) > 1
            and fields[1].lower() == "port"
        ):
            continue

        if len(fields) < 3 or not fields[0] or not fields[1] or not fields[2]:
            errors.append(
                f"Line {line_num}: Invalid row format. Expected 'id, port, device', got: {row}"
            )
            continue

        inst_id, port_str, device_str = fields[0], fields[1], fields[2]

        # Validate port integer range
        if not port_str.isdigit() or not (1 <= int(port_str) <= 65535):
            errors.append(
                f"Line {line_num}: Invalid port '{port_str}'. Must be an integer (1-65535)."
            )
            continue

        port = int(port_str)

        # Validate device type ('cpu' or 'gpu')
        device = device_str.lower()
        if device not in ("cpu", "gpu"):
            errors.append(
                f"Line {line_num}: Invalid device '{device_str}'. Must be 'cpu' or 'gpu'."
            )
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

        endpoints.append((inst_id, port, device))

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


def load_and_validate_endpoints(csv_file: Path) -> list[tuple[str, int, str]]:
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


def build_rust_offload(servers_rust_dir: Path) -> None:
    """[Dishonest / Build Driver]
    Compiles the optimized Rust offload server binary.
    """
    print("\n=== [1/3] Pre-compiling Rust Offload Server (--release) ===")
    subprocess.run(
        ["cargo", "build", "--release", "--bin", "offload_server"],
        cwd=servers_rust_dir,
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


def setup_docker_environment(endpoints: list[tuple[str, int, str]]) -> None:
    """[Dishonest / Container Driver]
    Ensures the shared Docker bridge network exists and removes stale offload containers.
    """
    subprocess.run(
        ["docker", "network", "create", "obm-net"], capture_output=True, check=False
    )

    stale_containers = [f"obm-offload-{inst_id}" for inst_id, _, _ in endpoints]
    subprocess.run(
        ["docker", "rm", "-f", *stale_containers], capture_output=True, check=False
    )


def spawn_terminal_tabs(
    endpoints: list[tuple[str, int, str]],
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
    for inst_id, port, device in endpoints:
        title = f"Offload-{inst_id} ({port}/{device.upper()})"
        run_cmd = f'"{offload_run_sh}" "{inst_id}" "{port}" "{device}" --skip-build'
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

    print("\n✓ All offload server nodes successfully launched!")
    for tab in tabs:
        print(f"   • {tab['title']}")
    print("\nTip: To stop all offload containers when done, run:")
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

    script_dir = Path(__file__).resolve().parent
    project_root = script_dir.parent
    csv_file = script_dir / "offload_endpoint.csv"
    offload_run_sh = script_dir / "offload_server" / "run_sh.sh"
    dockerfile = script_dir / "offload_server" / "dockerfile"
    servers_rust_dir = project_root / "servers_rust"

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
    build_rust_offload(servers_rust_dir)

    # 3. Pre-build Docker image
    build_offload_image(dockerfile, project_root)

    # 4. Prepare network and clean up stale instances
    setup_docker_environment(endpoints)

    # 5. Launch interactive terminal tabs
    new_window = "--new-window" in args or "-w" in args
    spawn_terminal_tabs(endpoints, offload_run_sh, new_window)


if __name__ == "__main__":
    main()
