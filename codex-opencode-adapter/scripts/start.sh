#!/usr/bin/env bash
# codex-opencode-adapter launcher
# Starts the adapter service that translates codex app-server JSON-RPC to OpenCode HTTP/SSE API.
#
# Usage: ./start.sh [port] [hostname]
#
# Environment variables:
#   PORT                    — listen port (default: 0 = random available port)
#   HOSTNAME                — listen address (default: 127.0.0.1)
#   CODEX_APP_SERVER_COMMAND — path to codex binary (default: codex)
#   CODEX_APP_SERVER_ARGS    — arguments to codex (default: app-server)
#   OPENCODE_SERVICE_VERSION — version reported to OpenCode desktop
#   OPENCODE_SERVICE_PASSWORD — auth password for OpenCode desktop connections
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"

export PORT="${1:-${PORT:-0}}"
export HOSTNAME="${2:-${HOSTNAME:-127.0.0.1}}"

exec "${DIR}/codex-opencode-adapter"