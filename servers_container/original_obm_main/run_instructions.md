# Original OBM Main Server (Dana Standalone on Port 7000)

This container runs the **baseline original Dana Web Server** (`dana ws.core -p 7000`) without any Rust proxy or custom middleware.

---

## Quick Start

Run the automated build and startup script:

```bash
./servers_container/original_obm_main/run_sh.sh
```

---

## Ports & Endpoints

| Service | Container Port | Host Port | URL |
| :--- | :--- | :--- | :--- |
| **Dana Web Server** | `7000` | `7000` | `http://localhost:7000/` |

---

## What This Container Does

1. Compiles `ws/Web.dn` to `ws/Web.o` if not already up to date.
2. Ensures the Dana runtime engine is staged in `servers_container/dana_runtime_copy`.
3. Builds the Docker image `original-obm-main` with:
   - Cached media assets (`/app/obm/assets/`).
   - Core Dana runtime (`/opt/dana/`).
   - OBM application files (`/app/obm/`).
4. Runs `dana ws.core -p 7000` serving the original OBM player, show catalog, and WASM runtime directly.

