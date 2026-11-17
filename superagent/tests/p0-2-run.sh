#!/bin/bash
set -uo pipefail
ROOT=/Users/fanjia/Desktop/code/codex
EV=$ROOT/superagent/evidence/p0-2
HOME_DIR="$HOME/Library/Application Support/SuperAgent/codex-home"
ENGINE=$ROOT/superagent/packaging/superagent-engine
FIX=$ROOT/superagent/tests/fixtures/p0-2-task
export https_proxy=http://127.0.0.1:7890 http_proxy=http://127.0.0.1:7890
mkdir -p "$EV" "$HOME_DIR/agents"

echo "== install P0-2 config overlay + role files =="
printf '# SuperAgent 产品专属 CODEX_HOME\n# 认证：显式复用模式（2026-11-17 goal 决策）\n' > "$HOME_DIR/config.toml"
cat "$ROOT/superagent/tests/p0-2/config-overlay.toml" >> "$HOME_DIR/config.toml"
cp "$ROOT/superagent/tests/p0-2/agents/"*.toml "$HOME_DIR/agents/"
ls "$HOME_DIR/agents/"

echo "== fresh fixture workspace =="
WORK=$(mktemp -d)/p0-2-work
cp -R "$FIX" "$WORK"
git -C "$WORK" init -q
git -C "$WORK" add -A
git -C "$WORK" -c user.email=spike@local -c user.name=spike commit -qm baseline
echo "$WORK" > "$EV/workspace-path.txt"
(cd "$WORK" && python3 -m unittest test_calc -v > "$EV/baseline-test-fail.txt" 2>&1)
echo "baseline test exit=$? (expect non-zero)"

echo "== leader team turn via app-server stdio =="
python3 - "$WORK" "$EV/calls.json" <<'PY'
import json, sys
work, out = sys.argv[1], sys.argv[2]
prompt = (
    "You are the SuperAgent leader. Task: in this repository the tests in test_calc.py fail "
    "because calc.add is wrong. Delegate with the multi-agent tools: "
    "1) spawn_agent with agent_type=planner to produce a one-step plan whose acceptance command is "
    "'python3 -m unittest test_calc -v'; wait_agent for it. "
    "2) spawn_agent with agent_type=implementer to fix calc.py per the plan; wait_agent. "
    "3) spawn_agent with agent_type=verifier to run the acceptance command and report the real exit code; wait_agent. "
    "Then report the final result with the verifier's evidence. Keep each handoff summary under 200 tokens. "
    "Do not fix the code yourself; delegate."
)
calls = [
    {"method": "thread/start", "params": {"cwd": work}, "timeout": 120},
    {"method": "turn/start", "params": {"threadId": "$ref:0:result.thread.id",
     "input": [{"type": "text", "text": prompt}]}, "timeout": 800},
]
json.dump(calls, open(out, "w"))
PY
python3 "$ROOT/superagent/tests/spike_client.py" --engine "$ENGINE" --home "$HOME_DIR" \
  --calls "$(cat "$EV/calls.json")" --out "$EV/team-turn-transcript.json" --deadline 900 --wait-turn-completed
echo "spike exit=$?"

echo "== post-turn evidence =="
(cd "$WORK" && python3 -m unittest test_calc -v > "$EV/post-test.txt" 2>&1)
echo "post test exit=$?"
cp "$WORK/calc.py" "$EV/calc-after.py"
git -C "$WORK" diff > "$EV/workspace-diff.patch"
echo "== P0-2 run done =="