#!/bin/bash
set -uo pipefail
ROOT=/Users/fanjia/Desktop/code/codex
EV=$ROOT/superagent/evidence/p0-6
export https_proxy=http://127.0.0.1:7890 http_proxy=http://127.0.0.1:7890
mkdir -p "$EV"
python3 "$ROOT/superagent/tests/p0_6.py" "$EV"
echo "P0-6 exit=$?"