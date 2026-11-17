#!/bin/bash
# P1-4 (#76) runtime acceptance: install -> restart -> skills still effective.
# Also serves as the #102 closure-gate trace (f2-decision.md §6).
# Model-free: skills/list RPC only; no turn is started.
set -uo pipefail
ROOT=/Users/fanjia/Desktop/code/codex
EV=$ROOT/superagent/evidence/p1-4
HOME_DIR="$HOME/Library/Application Support/SuperAgent/codex-home"
ENGINE=$ROOT/superagent/packaging/superagent-engine
FIXTURE=$ROOT/superagent/tests/p1-4/pack
export https_proxy=http://127.0.0.1:7890 http_proxy=http://127.0.0.1:7890
mkdir -p "$EV"

echo "== 0. user ~/.codex snapshot (before) =="
python3 "$ROOT/superagent/tests/p1_4_snapshot.py" "$HOME" "$EV/user-codex-before"

echo "== 1. one-command install (fixture pack with real SKILL.md) =="
python3 "$ROOT/superagent/tools/install_pack.py" --pack "$FIXTURE" --codex-home "$HOME_DIR" \
  > "$EV/install.log" 2>&1
echo "install exit=$?"; cat "$EV/install.log"

echo "== 2. session 1 (fresh engine process): skills/list, NO extraRoots set =="
python3 "$ROOT/superagent/tests/spike_client.py" --engine "$ENGINE" --home "$HOME_DIR" \
  --calls '[{"method":"skills/list","params":{}}]' \
  --out "$EV/skills-after-install-session1.json" --deadline 90
echo "session1 exit=$?"

echo "== 3. session 2 (RESTART, another fresh process): skills/list again =="
python3 "$ROOT/superagent/tests/spike_client.py" --engine "$ENGINE" --home "$HOME_DIR" \
  --calls '[{"method":"skills/list","params":{}}]' \
  --out "$EV/skills-after-install-session2.json" --deadline 90
echo "session2 exit=$?"

echo "== 4. doctor =="
python3 "$ROOT/superagent/tools/doctor.py" --codex-home "$HOME_DIR" --engine "$ENGINE" \
  --pack "$ROOT/superagent/pack" > "$EV/doctor.log" 2>&1
echo "doctor exit=$?"; cat "$EV/doctor.log"

echo "== 5. TUI launch via bin/superagent =="
python3 "$ROOT/superagent/tests/p1_4_tui.py" "$EV/tui-screen.log" > "$EV/tui-probe.log" 2>&1
echo "tui exit=$?"; cat "$EV/tui-probe.log"

echo "== 6. user ~/.codex snapshot (after) + config diff =="
python3 "$ROOT/superagent/tests/p1_4_snapshot.py" "$HOME" "$EV/user-codex-after"
if diff -u "$EV/user-codex-before.config.json" "$EV/user-codex-after.config.json" > "$EV/user-codex-config.diff"; then
  echo "user-config-unchanged=true"
else
  echo "user-config-unchanged=false"; head -20 "$EV/user-codex-config.diff"
fi

echo "== 7. persistence conclusion =="
python3 - "$EV" <<'PY'
import json, sys
ev = sys.argv[1]
def has_skill(path):
    with open(path) as fh:
        d = json.load(fh)
    return "p1-4-greet" in json.dumps(d.get("results", []))
s1 = has_skill(f"{ev}/skills-after-install-session1.json")
s2 = has_skill(f"{ev}/skills-after-install-session2.json")
with open(f"{ev}/user-codex-before.config.json") as fh:
    before_cfg = json.load(fh)
with open(f"{ev}/user-codex-after.config.json") as fh:
    after_cfg = json.load(fh)
with open(f"{ev}/user-codex-before.bulk.json") as fh:
    before_bulk = json.load(fh)
with open(f"{ev}/user-codex-after.bulk.json") as fh:
    after_bulk = json.load(fh)
config_unchanged = before_cfg == after_cfg
meta_changed = sorted(k for k in set(before_bulk) | set(after_bulk)
                      if before_bulk.get(k) != after_bulk.get(k))
with open(f"{ev}/user-codex-bulk-changes.json", "w") as fh:
    json.dump({"changed_count": len(meta_changed), "changed_keys": meta_changed[:100]}, fh, indent=2)
conclusion = {
    "skill_visible_after_install_session1": s1,
    "skill_visible_after_restart_session2": s2,
    "restart_persistence_via_install": "PERSISTENT" if (s1 and s2) else "NOT_PERSISTENT",
    "mechanism": "pack skills installed into product CODEX_HOME/skills (engine default discovery root, codex-rs/exec-server/src/discoverV2/capability_locations.rs:73); loaded from disk every session — no extraRoots re-injection needed",
    "user_config_unchanged": config_unchanged,
    "user_bulk_state_metadata_changed": meta_changed[:20],
    "note": "config surface = top-level files + skills/agents/prompts hashed; bulk sessions/history tracked as size+mtime metadata (may change from unrelated user activity)",
    "issue_102_closure_gate": "SATISFIED" if (s1 and s2 and config_unchanged) else "NOT_SATISFIED",
}
with open(f"{ev}/restart-persistence-conclusion.json", "w") as fh:
    json.dump(conclusion, fh, indent=2)
print(json.dumps(conclusion, indent=2))
PY
echo "== P1-4 run done =="