#!/bin/bash
set -uo pipefail
ROOT=/Users/fanjia/Desktop/code/codex
EV=$ROOT/superagent/evidence/p0-5
HOME_DIR="$HOME/Library/Application Support/SuperAgent/codex-home"
export https_proxy=http://127.0.0.1:7890 http_proxy=http://127.0.0.1:7890
mkdir -p "$EV" "$HOME_DIR/agents"
printf '# SuperAgent 产品专属 CODEX_HOME\n' > "$HOME_DIR/config.toml"
cat "$ROOT/superagent/tests/p0-2/config-overlay.toml" >> "$HOME_DIR/config.toml"
cp "$ROOT/superagent/tests/p0-2/agents/"*.toml "$HOME_DIR/agents/"
WORK=$(mktemp -d)/p0-5b-work
cp -R "$ROOT/superagent/tests/fixtures/p0-2-task" "$WORK"
git -C "$WORK" init -q && git -C "$WORK" add -A && git -C "$WORK" -c user.email=s@l -c user.name=s commit -qm base

echo "== phase 1: \$mention + implicit trigger (default skills budget) =="
python3 "$ROOT/superagent/tests/p0_5b.py" "$WORK" --phase mention
echo "phase1 exit=$?"

echo "== phase 2: max_context_tokens=1 cap =="
printf '\n[skills]\nmax_context_tokens = 1\n' >> "$HOME_DIR/config.toml"
python3 "$ROOT/superagent/tests/p0_5b.py" "$WORK" --phase cap
echo "phase2 exit=$?"

# restore config without the cap
printf '# SuperAgent 产品专属 CODEX_HOME\n' > "$HOME_DIR/config.toml"
cat "$ROOT/superagent/tests/p0-2/config-overlay.toml" >> "$HOME_DIR/config.toml"
echo "== P0-5b run done =="