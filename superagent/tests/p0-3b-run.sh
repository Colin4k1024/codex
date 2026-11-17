#!/bin/bash
set -uo pipefail
ROOT=/Users/fanjia/Desktop/code/codex
EV=$ROOT/superagent/evidence/p0-3
HOME_DIR="$HOME/Library/Application Support/SuperAgent/codex-home"
export https_proxy=http://127.0.0.1:7890 http_proxy=http://127.0.0.1:7890
mkdir -p "$EV"

echo "== rerun board turn (now includes subscribe) =="
bash "$ROOT/superagent/tests/p0-3-run.sh" > "$EV/run.log" 2>&1
echo "p0-3 rerun exit=$?"

MEMBER=$(python3 -c "import json;print(json.load(open('$EV/board-turn-transcript.json'))['results'][0]['response']['result']['thread']['id'])")
FOREIGN=$(python3 -c "import json;print(json.load(open('$ROOT/superagent/evidence/p0-2/team-turn-transcript.json'))['results'][0]['response']['result']['thread']['id'])")
echo "member=$MEMBER foreign=$FOREIGN"

MCPBIN="$ROOT/codex-rs/target/release/codex-message-board-mcp"
CODEX_MESSAGE_BOARD_TOKEN=spike-token-p03 "$MCPBIN" --sqlite-home "$HOME_DIR" --listen 127.0.0.1:7799 > "$EV/mcp-server.log" 2>&1 &
MCP_PID=$!
sleep 3
python3 "$ROOT/superagent/tests/p0_3b_mcp.py" --url http://127.0.0.1:7799/mcp \
  --token spike-token-p03 --member-thread "$MEMBER" --foreign-thread "$FOREIGN" \
  --out "$EV/mcp-caller-bound.json"
echo "caller-bound probe exit=$?"
kill $MCP_PID 2>/dev/null
echo "== P0-3b run done =="