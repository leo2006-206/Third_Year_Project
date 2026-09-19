#!/usr/bin/env python3
"""
OBM Multi-Server Orchestrator
Validates endpoints from offload_endpoint.csv, pre-compiles Rust binaries,
pre-builds Docker images, and launches all servers (offload instances + main server)
into separate tabs within a single GNOME Terminal window.
"""

import csv
import shutil
import subprocess
import sys
import time
from collections.abc import Iterable
from pathlib import Path


def validate_endpoints(
    rows: Iterable[list[str]],
) -> tuple[list[tuple[str, int]], list[str]]:
    """[Honest / Pure Domain Logic]
    Validates CSV rows for schema, port integer ranges, and duplicate IDs/ports.
    Returns a tuple of (endpoints, errors).
    """
    seen_ids: dict[str, int] = {}
    seen_ports: dict[int, int] = {}
    endpoints: list[tuple[str, int]] = []
    errors: list[str] = []

    for line_num, row in enumerate(rows, start=1):
        if not row:
            continue

        fields = [col.strip() for col in row]
        if not fields or not fields[0]:
            continue

        # Skip comments
        if fields[0].startswith("#"):
            continue

        # Skip header line (e.g. "id", "port")
        if (
            fields[0].lower() == "id"
            and len(fields) > 1
            and fields[1].lower() == "port"
        ):
            continue

        if len(fields) < 2 or not fields[0] or not fields[1]:
            errors.append(
                f"Line {line_num}: Invalid row format. Expected 'id, port', got: {row}"
            )
            continue

        inst_id, port_str = fields[0], fields[1]

        # Validate port integer range
        if not port_str.isdigit() or not (1 <= int(port_str) <= 65535):
            errors.append(
                f"Line {line_num}: Invalid port '{port_str}'. Must be an integer between 1 and 65535."
            )
            continue

        port = int(port_str)

        # Check duplicate ID
        if inst_id in seen_ids:
            errors.append(
                f"Line {line_num}: Duplicate ID '{inst_id}' detected (already defined on line {seen_ids[inst_id]})."
            )
        else:
            seen_ids[inst_id] = line_num

        # Check duplicate Port
        if port in seen_ports:
            errors.append(
                f"Line {line_num}: Duplicate Port '{port}' detected (already defined on line {seen_ports[port]})."
            )
        else:
            seen_ports[port] = line_num

        endpoints.append((inst_id, port))

    return endpoints, errors


def format_tab_command(title: str, run_cmd: str) -> str:
    """[Honest / Pure Domain Logic]
    Constructs bash command string setting ANSI title and pausing on exit.
    """
    return (
        f"printf '\\033]0;{title}\\007'; "
        f"{run_cmd}; "
        f"echo; echo 'Process ended. Press Enter to close tab.'; read"
    )


def load_endpoints_from_csv(
    csv_file: Path,
) -> tuple[list[tuple[str, int]], list[str]]:
    """[Dishonest / I/O Boundary Adapter]
    Reads CSV file from disk and delegates to validate_endpoints.
    """
    with open(csv_file, "r", encoding="utf-8") as f:
        return validate_endpoints(csv.reader(f))


def main(argv: list[str] | None = None):
    """[Dishonest / CLI Orchestrator]"""
    args = sys.argv[1:] if argv is None else argv

    script_dir = Path(__file__).resolve().parent
    project_root = script_dir.parent
    csv_file = script_dir / "offload_endpoint.csv"
    offload_run_sh = script_dir / "offload_server" / "run_sh.sh"
    main_run_sh = script_dir / "main_server" / "run_sh.sh"
    servers_rust_dir = project_root / "servers_rust"

    # -------------------------------------------------------------------------
    # 1. Validate Prerequisite Files
    # -------------------------------------------------------------------------
    required_files = [
        (csv_file, "Endpoint CSV file"),
        (offload_run_sh, "Offload run script"),
        (main_run_sh, "Main server run script"),
    ]
    for path, desc in required_files:
        if not path.is_file():
            sys.exit(f"Error: {desc} not found: {path}")

    offload_run_sh.chmod(0o755)
    main_run_sh.chmod(0o755)

    # -------------------------------------------------------------------------
    # 2. Parse CSV & Validate Duplicates
    # -------------------------------------------------------------------------
    endpoints, errors = load_endpoints_from_csv(csv_file)

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

    print(
        f"✓ Validated {len(endpoints)} offload endpoint(s) successfully with no duplicates."
    )

    # -------------------------------------------------------------------------
    # 3. Pre-Compile Rust Binaries Once
    # -------------------------------------------------------------------------
    print("\n=== [1/3] Pre-compiling Rust Servers (--release) ===")
    subprocess.run(
        [
            "cargo",
            "build",
            "--release",
            "--bin",
            "offload_server",
            "--bin",
            "main_server",
        ],
        cwd=servers_rust_dir,
        check=True,
    )

    # -------------------------------------------------------------------------
    # 4. Pre-Build Docker Images Once
    # -------------------------------------------------------------------------
    print("\n=== [2/3] Pre-building Docker Images ===")
    images_to_build = [
        ("obm-offload-server", script_dir / "offload_server" / "dockerfile"),
        ("obm-main-server", script_dir / "main_server" / "dockerfile"),
    ]
    for tag, dockerfile in images_to_build:
        print(f" -> Building {tag} image...")
        subprocess.run(
            [
                "docker",
                "build",
                "-f",
                str(dockerfile),
                "-t",
                tag,
                ".",
            ],
            cwd=project_root,
            check=True,
        )

    # Ensure shared network exists
    subprocess.run(
        ["docker", "network", "create", "obm-net"], capture_output=True, check=False
    )

    # Clean up any existing stale containers to prevent name collisions
    stale_containers = [
        *(f"obm-offload-{inst_id}" for inst_id, _ in endpoints),
        "obm-main-server",
    ]
    subprocess.run(
        ["docker", "rm", "-f", *stale_containers], capture_output=True, check=False
    )

    # -------------------------------------------------------------------------
    # 5. Launch Terminals with Tabs
    # -------------------------------------------------------------------------
    print("\n=== [3/3] Spawning Servers in GNOME Terminal ===")
    if not shutil.which("gnome-terminal"):
        sys.exit("Error: 'gnome-terminal' command not found in PATH.")

    tabs = []
    for inst_id, port in endpoints:
        title = f"Offload-{inst_id} ({port})"
        run_cmd = f'"{offload_run_sh}" "{inst_id}" "{port}" --skip-build'
        tabs.append({"title": title, "cmd": format_tab_command(title, run_cmd)})

    new_window = "--new-window" in args or "-w" in args

    if new_window:
        # Launch all tabs in a single new window
        print(" -> Launching all servers in a new window with tabs...")
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
        # Add all tabs to the current terminal window
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

    print("\n✓ All servers successfully launched in a single GNOME Terminal window!")
    print("  Tabs opened:")
    for tab in tabs:
        print(f"   • {tab['title']}")
    print("\nTip: To stop all containers when done, you can run:")
    print("  docker rm -f obm-main-server $(docker ps -q --filter name=obm-offload-)")


if __name__ == "__main__":
    main()
