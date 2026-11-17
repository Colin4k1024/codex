#!/bin/bash
set -uo pipefail
ROOT=/Users/fanjia/Desktop/code/codex
EV=$ROOT/superagent/evidence/p0-4
HOME_DIR="$HOME/Library/Application Support/SuperAgent/codex-home"
ENGINE=$ROOT/superagent/packaging/superagent-engine
export https_proxy=http://127.0.0.1:7890 http_proxy=http://127.0.0.1:7890
mkdir -p "$EV" "$HOME_DIR/agents"
echo "== install config overlay + goal budget feature (self-contained) =="
printf '# SuperAgent 产品专属 CODEX_HOME\n# 认证：显式复用模式（2026-11-17 goal 决策）\n' > "$HOME_DIR/config.toml"
cat "$ROOT/superagent/tests/p0-2/config-overlay.toml" >> "$HOME_DIR/config.toml"
printf '\n[features]\ngoals = true\ntoken_budget = true\n' >> "$HOME_DIR/config.toml"
cp "$ROOT/superagent/tests/p0-2/agents/"*.toml "$HOME_DIR/agents/"
WORK=$(mktemp -d)/p0-4-work
cp -R "$ROOT/superagent/tests/fixtures/p0-2-task" "$WORK"
git -C "$WORK" init -q && git -C "$WORK" add -A && git -C "$WORK" -c user.email=s@l -c user.name=s commit -qm base

python3 - "$WORK" "$EV/calls.json" <<'PY'
import json, sys
work, out = sys.argv[1], sys.argv[2]
prompt = (
    "Check your current goal with the get_goal tool and report its objective and remaining token budget. "
    "Then fix calc.py so that 'python3 -m unittest test_calc -v' passes (the add function subtracts instead of adds). "
    "Run the test to verify, then update_goal to record progress. Report the final test exit code."
)
calls = [
    {"method": "thread/start", "params": {"cwd": work}, "timeout": 120},
    {"method": "thread/goal/set", "params": {"threadId": "$ref:0:result.thread.id",
     "objective": "Fix calc.add so test_calc passes, with real test evidence.", "tokenBudget": 60000}, "timeout": 60},
    {"method": "thread/goal/get", "params": {"threadId": "$ref:0:result.thread.id"}, "timeout": 60},
    {"method": "turn/start", "params": {"threadId": "$ref:0:result.thread.id",
     "input": [{"type": "text", "text": prompt}]}, "timeout": 800},
    {"method": "thread/goal/get", "params": {"threadId": "$ref:0:result.thread.id"}, "timeout": 60},
    {"method": "thread/goal/clear", "params": {"threadId": "$ref:0:result.thread.id"}, "timeout": 60, "allowError": True},
]
json.dump(calls, open(out, "w"))
PY
python3 "$ROOT/superagent/tests/spike_client.py" --engine "$ENGINE" --home "$HOME_DIR" \
  --calls "$(cat "$EV/calls.json")" --out "$EV/goal-transcript.json" --deadline 1000 --wait-turn-completed
echo "goal spike exit=$?"
(cd "$WORK" && python3 -m unittest test_calc -v > "$EV/post-test.txt" 2>&1)
echo "post test exit=$?"
echo "== P0-4 run done =="