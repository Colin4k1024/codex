#!/bin/bash
# One-command SuperAgent install (P1-4 / #76): verify+install pack, then doctor.
# Usage: bash superagent/install.sh [--pack DIR_OR_TAR] [--codex-home DIR]
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
python3 "$HERE/tools/install_pack.py" "$@"
DOCTOR_ARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --codex-home|--pack) DOCTOR_ARGS+=("$1" "$2"); shift 2 ;;
    *) shift ;;
  esac
done
python3 "$HERE/tools/doctor.py" ${DOCTOR_ARGS[@]+"${DOCTOR_ARGS[@]}"}
echo "Install complete. Launch the meta-agent TUI with: $HERE/bin/superagent"