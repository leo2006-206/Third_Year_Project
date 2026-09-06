#!/usr/bin/env python3
"""
OBM Multi-Server Orchestrator
Validates endpoints from offload_endpoint.csv, pre-compiles Rust binaries,
pre-builds Docker images, and launches all servers (offload instances + main server)
into separate tabs within a single GNOME Terminal window.
"""

import csv
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path


def main():
    script_dir = Path(__file__).resolve().parent
    project_root = script_dir.parent
    csv_file = script_dir / "offload_endpoint.csv"
    offload_run_sh = script_dir / "offload_server" / "run_sh.sh"
    main_run_sh = script_dir / "main_server" / "run_sh.sh"
    servers_rust_dir = project_root / "servers_rust"

    # -------------------------------------------------------------------------
    # 1. Validate Prerequisite Files
    # -------------------------------------------------------------------------
    if not csv_file.is_file():
        sys.exit(f"Error: Endpoint CSV file not found: {csv_file}")
    if not offload_run_sh.is_file():
        sys.exit(f"Error: Offload run script not found: {offload_run_sh}")
    if not main_run_sh.is_file():
        sys.exit(f"Error: Main server run script not found: {main_run_sh}")

    offload_run_sh.chmod(0o755)
    main_run_sh.chmod(0o755)

    # -------------------------------------------------------------------------
    # 2. Parse CSV & Validate Duplicates
    # -------------------------------------------------------------------------
    seen_ids = {}
    seen_ports = {}
    endpoints = []
    errors = []

    with open(csv_file, "r", encoding="utf-8") as f:
        reader = csv.reader(f)
        for line_num, row in enumerate(reader, start=1):
            if not row:
                continue

            # Strip whitespace
            fields = [col.strip() for col in row]
            if not fields or not fields[0]:
                continue

            # Skip comments
            if fields[0].startswith("#"):
                continue

            # Skip header line (e.g. "id", "port")
            if fields[0].lower() == "id" and len(fields) > 1 and fields[1].lower() == "port":
                continue

            if len(fields) < 2 or not fields[0] or not fields[1]:
                errors.append(f"Line {line_num}: Invalid row format. Expected 'id, port', got: {row}")
                continue

            inst_id, port_str = fields[0], fields[1]

            # Validate port integer range
            if not port_str.isdigit() or not (1 <= int(port_str) <= 65535):
                errors.append(f"Line {line_num}: Invalid port '{port_str}'. Must be an integer between 1 and 65535.")
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

    if errors:
        print("===========================================================", file=sys.stderr)
        print("Validation failed: duplicate or invalid endpoints detected:", file=sys.stderr)
        for err in errors:
            print(f"  • {err}", file=sys.stderr)
        print(f"Please fix {csv_file} before continuing.", file=sys.stderr)
        print("===========================================================", file=sys.stderr)
        sys.exit(1)

    if not endpoints:
        sys.exit(f"Error: No valid endpoints found in {csv_file}")

    print(f"✓ Validated {len(endpoints)} offload endpoint(s) successfully with no duplicates.")

    # -------------------------------------------------------------------------
    # 3. Pre-Compile Rust Binaries Once
    # -------------------------------------------------------------------------
    print("\n=== [1/3] Pre-compiling Rust Servers (--release) ===")
    subprocess.run(
        ["cargo", "build", "--release", "--bin", "offload_server", "--bin", "main_server"],
        cwd=servers_rust_dir,
        check=True,
    )

    # -------------------------------------------------------------------------
    # 4. Pre-Build Docker Images Once
    # -------------------------------------------------------------------------
    print("\n=== [2/3] Pre-building Docker Images ===")
    print(" -> Building obm-offload-server image...")
    subprocess.run(
        [
            "docker", "build",
            "-f", str(script_dir / "offload_server" / "dockerfile"),
            "-t", "obm-offload-server",
            "."
        ],
        cwd=project_root,
        check=True,
    )

    print(" -> Building obm-main-server image...")
    subprocess.run(
        [
            "docker", "build",
            "-f", str(script_dir / "main_server" / "dockerfile"),
            "-t", "obm-main-server",
            "."
        ],
        cwd=project_root,
        check=True,
    )

    # Ensure shared network exists
    subprocess.run(["docker", "network", "create", "obm-net"], capture_output=True)

    # Clean up any existing stale containers to prevent name collisions
    for inst_id, _ in endpoints:
        subprocess.run(["docker", "rm", "-f", f"obm-offload-{inst_id}"], capture_output=True)
    subprocess.run(["docker", "rm", "-f", "obm-main-server"], capture_output=True)

    # -------------------------------------------------------------------------
    # 5. Launch Terminals with Tabs
    # -------------------------------------------------------------------------
    print("\n=== [3/3] Spawning Servers in GNOME Terminal ===")
    if not shutil.which("gnome-terminal"):
        sys.exit("Error: 'gnome-terminal' command not found in PATH.")

    tabs = []
    for inst_id, port in endpoints:
        title = f"Offload-{inst_id} ({port})"
        # Set terminal title via ANSI sequence + run script + keep alive
        cmd = (
            f"printf '\\033]0;{title}\\007'; "
            f"\"{offload_run_sh}\" \"{inst_id}\" \"{port}\" --skip-build; "
            f"echo; echo 'Process ended. Press Enter to close tab.'; read"
        )
        tabs.append({"title": title, "cmd": cmd})

    # # Add Main Server tab
    # main_title = "Main Server (7000)"
    # main_cmd = (
    #     f"printf '\\033]0;{main_title}\\007'; "
    #     f"\"{main_run_sh}\" --skip-build; "
    #     f"echo; echo 'Process ended. Press Enter to close tab.'; read"
    # )
    # tabs.append({"title": main_title, "cmd": main_cmd})

    new_window = "--new-window" in sys.argv or "-w" in sys.argv

    if new_window:
        # Launch all tabs in a single new window
        print(" -> Launching all servers in a new window with tabs...")
        cmd = [
            "gnome-terminal",
            "--window",
            "--title", tabs[0]["title"],
            "--", "bash", "-c", tabs[0]["cmd"]
        ]
        for tab in tabs[1:]:
            cmd.extend([
                "--tab",
                "--title", tab["title"],
                "--", "bash", "-c", tab["cmd"]
            ])
        subprocess.run(cmd, check=True)
    else:
        # Add all tabs to the current terminal window
        for tab in tabs:
            print(f" -> Adding tab for [{tab['title']}]...")
            subprocess.run([
                "gnome-terminal",
                "--tab",
                "--title", tab["title"],
                "--", "bash", "-c", tab["cmd"]
            ], check=True)
            time.sleep(0.3)

    print("\n✓ All servers successfully launched in a single GNOME Terminal window!")
    print("  Tabs opened:")
    for tab in tabs:
        print(f"   • {tab['title']}")
    print("\nTip: To stop all containers when done, you can run:")
    print("  docker rm -f obm-main-server $(docker ps -q --filter name=obm-offload-)")


if __name__ == "__main__":
    main()

