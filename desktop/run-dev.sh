#!/usr/bin/env bash
# Development wrap: run the real ClaudeCodeUI frontend inside the YourAgent Tauri shell.
#
# This is a DEV-mode wrap. It runs the upstream ClaudeCodeUI Express/WebSocket
# backend (server/index.js) plus its Vite dev server, and points the Tauri WebView
# at the Vite dev URL (which proxies /api, /ws, /shell to the backend). The native
# window therefore shows the real, fully functional ClaudeCodeUI interface.
#
# Production boundary (see HARNESS_CLAUDECODEUI_MIGRATION.md): a distributable build
# must NOT bundle this Express backend. The self-contained production path replaces
# fetch/WebSocket with a typed DesktopClient over Tauri commands backed by the Rust
# app-server adapter (migration steps U02-U06). This script is the dev baseline only.
#
# Source baseline: claudecodeui @ 388134c7a5f1a2741931d47c4a1f89aae31d4292 (AGPL-3.0-or-later).
set -euo pipefail

CLAUDECODEUI_SRC="${CLAUDECODEUI_SRC:-/Users/fanjia/Desktop/code/claudecodeui}"
DESKTOP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export SERVER_PORT="${SERVER_PORT:-3001}"
export VITE_PORT="${VITE_PORT:-5173}"
export HOST="${HOST:-127.0.0.1}"

if [ ! -d "$CLAUDECODEUI_SRC/node_modules" ]; then
  echo "error: $CLAUDECODEUI_SRC/node_modules missing." >&2
  echo "Install deps first (public registry, skip native scripts then rebuild):" >&2
  echo "  cd '$CLAUDECODEUI_SRC' && npm install --registry https://registry.npmjs.org --ignore-scripts" >&2
  echo "  npm rebuild better-sqlite3 sqlite3 node-pty bcrypt sharp --registry https://registry.npmjs.org" >&2
  exit 1
fi

echo "[run-dev] starting ClaudeCodeUI backend + Vite from $CLAUDECODEUI_SRC"
( cd "$CLAUDECODEUI_SRC" && npm run dev ) &
BACKEND_PID=$!
cleanup() { kill "$BACKEND_PID" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

echo "[run-dev] waiting for Vite on http://localhost:$VITE_PORT ..."
for _ in $(seq 1 60); do
  if curl -sf "http://localhost:$VITE_PORT/" >/dev/null 2>&1; then break; fi
  sleep 1
done

echo "[run-dev] launching Tauri shell (cargo tauri dev)"
( cd "$DESKTOP_DIR" && cargo tauri dev )