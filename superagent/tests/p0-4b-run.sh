#!/bin/bash
set -uo pipefail
ROOT=/Users/fanjia/Desktop/code/codex
EV=$ROOT/superagent/evidence/p0-4
HOME_DIR="$HOME/Library/Application Support/SuperAgent/codex-home"
export https_proxy=http://127.0.0.1:7890 http_proxy=http://127.0.0.1:7890
mkdir -p "$EV" "$HOME_DIR/agents"
printf '# SuperAgent 产品专属 CODEX_HOME\n' > "$HOME_DIR/config.toml"
cat "$ROOT/superagent/tests/p0-2/config-overlay.toml" >> "$HOME_DIR/config.toml"
printf '\n[features]\ngoals = true\ntoken_budget = true\n' >> "$HOME_DIR/config.toml"
cp "$ROOT/superagent/tests/p0-2/agents/"*.toml "$HOME_DIR/agents/"
WORK=$(mktemp -d)/p0-4b-work
cp -R "$ROOT/superagent/tests/fixtures/p0-2-task" "$WORK"
git -C "$WORK" init -q && git -C "$WORK" add -A && git -C "$WORK" -c user.email=s@l -c user.name=s commit -qm base
python3 "$ROOT/superagent/tests/p0_4b.py" "$WORK"
echo "p0-4b exit=$?"
echo "== P0-4b run done =="