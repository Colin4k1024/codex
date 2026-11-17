#!/bin/bash
set -uo pipefail
ROOT=/Users/fanjia/Desktop/code/codex
EV=$ROOT/superagent/evidence/p0-3
HOME_DIR="$HOME/Library/Application Support/SuperAgent/codex-home"
ENGINE=$ROOT/superagent/packaging/superagent-engine
export https_proxy=http://127.0.0.1:7890 http_proxy=http://127.0.0.1:7890
mkdir -p "$EV" "$HOME_DIR/agents"
echo "== install config overlay + board feature (self-contained) =="
printf '# SuperAgent 产品专属 CODEX_HOME\n# 认证：显式复用模式（2026-11-17 goal 决策）\n' > "$HOME_DIR/config.toml"
cat "$ROOT/superagent/tests/p0-2/config-overlay.toml" >> "$HOME_DIR/config.toml"
printf '\n[features]\nagent_message_board = true\n' >> "$HOME_DIR/config.toml"
cp "$ROOT/superagent/tests/p0-2/agents/"*.toml "$HOME_DIR/agents/"
WORK=$(mktemp -d)/p0-3-work
cp -R "$ROOT/superagent/tests/fixtures/p0-2-task" "$WORK"
git -C "$WORK" init -q && git -C "$WORK" add -A && git -C "$WORK" -c user.email=s@l -c user.name=s commit -qm base

echo "== in-session board turn =="
python3 - "$WORK" "$EV/calls.json" <<'PY'
import json, sys
work, out = sys.argv[1], sys.argv[2]
prompt = (
    "You are the SuperAgent leader verifying the message board. Do exactly this: "
    "1) use the message-board create_channel tool to create channel 'run-spike-p03'. "
    "2) spawn_agent agent_type=planner asking it to post a one-line plan summary to channel 'run-spike-p03' via the board post tool; wait_agent. "
    "3) spawn_agent agent_type=verifier asking it to read_thread on 'run-spike-p03' and post an acknowledgment reply; wait_agent. "
    "4) read_thread yourself and report the posts you see. "
    "5) use the subscribe tool on channel 'run-spike-p03' and confirm the subscription. Keep summaries under 100 tokens."
)
calls = [
    {"method": "thread/start", "params": {"cwd": work}, "timeout": 120},
    {"method": "turn/start", "params": {"threadId": "$ref:0:result.thread.id",
     "input": [{"type": "text", "text": prompt}]}, "timeout": 800},
]
json.dump(calls, open(out, "w"))
PY
python3 "$ROOT/superagent/tests/spike_client.py" --engine "$ENGINE" --home "$HOME_DIR" \
  --calls "$(cat "$EV/calls.json")" --out "$EV/board-turn-transcript.json" --deadline 900 --wait-turn-completed
echo "board turn exit=$?"

echo "== build + probe external message-board MCP (HTTP, caller-bound) =="
cd "$ROOT/codex-rs"
cargo build --release -p codex-message-board-mcp > "$EV/mcp-build.log" 2>&1
echo "mcp build exit=$?"
MCPBIN="$ROOT/codex-rs/target/release/codex-message-board-mcp"
if [ -x "$MCPBIN" ]; then
  CODEX_MESSAGE_BOARD_TOKEN=spike-token-p03 "$MCPBIN" --sqlite-home "$HOME_DIR" --listen 127.0.0.1:7799 > "$EV/mcp-server.log" 2>&1 &
  MCP_PID=$!
  sleep 3
  echo "-- no token (expect 401):" > "$EV/mcp-probe.txt"
  curl -s -o "$EV/mcp-noauth-body.txt" -w 'HTTP %{http_code}\n' -X POST http://127.0.0.1:7799/mcp \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"spike","version":"0"}}}' >> "$EV/mcp-probe.txt" 2>&1
  echo "-- with token:" >> "$EV/mcp-probe.txt"
  curl -s -o "$EV/mcp-auth-body.txt" -w 'HTTP %{http_code}\n' -X POST http://127.0.0.1:7799/mcp \
    -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
    -H 'Authorization: Bearer spike-token-p03' \
    -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"spike","version":"0"}}}' >> "$EV/mcp-probe.txt" 2>&1
  head -c 600 "$EV/mcp-auth-body.txt" >> "$EV/mcp-probe.txt"
  kill $MCP_PID 2>/dev/null
  cat "$EV/mcp-probe.txt"
fi
echo "== P0-3 run done =="