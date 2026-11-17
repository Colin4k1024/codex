#!/bin/bash
set -uo pipefail
ROOT=/Users/fanjia/Desktop/code/codex
EV=$ROOT/superagent/evidence/p0-5
HOME_DIR="$HOME/Library/Application Support/SuperAgent/codex-home"
ENGINE=$ROOT/superagent/packaging/superagent-engine
SKILLS=$ROOT/superagent/tests/p0-5/skills
export https_proxy=http://127.0.0.1:7890 http_proxy=http://127.0.0.1:7890
mkdir -p "$EV"

echo "== session 1: set extraRoots + list =="
python3 "$ROOT/superagent/tests/spike_client.py" --engine "$ENGINE" --home "$HOME_DIR" \
  --calls "[{\"method\":\"skills/extraRoots/set\",\"params\":{\"extraRoots\":[\"$SKILLS\"]}},{\"method\":\"skills/list\",\"params\":{}}]" \
  --out "$EV/skills-session1.json" --deadline 90
echo "session1 exit=$?"

echo "== session 2 (fresh process): list again -> persistence check =="
python3 "$ROOT/superagent/tests/spike_client.py" --engine "$ENGINE" --home "$HOME_DIR" \
  --calls '[{"method":"skills/list","params":{}}]' \
  --out "$EV/skills-session2.json" --deadline 90
echo "session2 exit=$?"

python3 - "$EV" <<'PY'
import json, sys
ev = sys.argv[1]
def has_skill(path):
    d = json.load(open(path))
    txt = json.dumps(d.get("results", []))
    return "spike-greet" in txt
s1, s2 = has_skill(f"{ev}/skills-session1.json"), has_skill(f"{ev}/skills-session2.json")
conclusion = {
    "visible_after_set": s1,
    "visible_after_restart": s2,
    "persistence": "PERSISTENT" if s2 else "NOT_PERSISTENT",
    "fork_trigger_F2": "not triggered" if s2 else "TRIGGERED: extraRoots lost across restart; evaluate config path or minimal fork fix (#102)",
}
json.dump(conclusion, open(f"{ev}/persistence-conclusion.json", "w"), indent=2)
print(json.dumps(conclusion, indent=2))
PY
echo "== P0-5 run done =="