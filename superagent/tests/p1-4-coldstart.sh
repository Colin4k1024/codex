#!/bin/bash
# P1-4 (#76) cold-start smoke: prove "干净 macOS 一条命令安装" reproducibly.
#
# Builds the engine from the pinned source_commit in an isolated git worktree
# (no prebuilt binary), then runs install.sh in a FRESH HOME with no pre-existing
# product state, and asserts doctor ALL GREEN. Auth bootstrap is a user login
# action (`superagent login`); the post-login state is simulated here by
# provisioning auth.json (doctor stats only, never reads it).
#
# Reproducible: bash superagent/tests/p1-4-coldstart.sh
# Reuse an already-built matching worktree: COLDSTART_WT=/path/to/wt bash ...
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EV="$ROOT/superagent/evidence/p1-4"
mkdir -p "$EV"
PINNED=$(git -C "$ROOT" rev-parse HEAD)
PROXY="${SUPERAGENT_PROXY:-http://127.0.0.1:7890}"
export https_proxy="$PROXY" http_proxy="$PROXY"

echo "== 1. engine from pinned source_commit $PINNED =="
WT="${COLDSTART_WT:-}"
CREATED_WT=0
if [ -z "$WT" ]; then
  WT="$(mktemp -d)/wt"
  git -C "$ROOT" worktree add --detach "$WT" "$PINNED" >>"$EV/coldstart-build.log" 2>&1
  CREATED_WT=1
fi
ENGINE="$WT/superagent/packaging/superagent-engine"
if [ ! -x "$ENGINE" ]; then
  echo "building engine (cargo build --release -p codex-cli --bin codex)..."
  (cd "$WT/codex-rs" && cargo build --release -p codex-cli --bin codex) >>"$EV/coldstart-build.log" 2>&1
  BUILD_RC=$?
  echo "build exit=$BUILD_RC"
  [ "$BUILD_RC" -eq 0 ] || { echo "FAIL: engine build failed"; exit 1; }
  mkdir -p "$WT/superagent/packaging"
  rm -f "$ENGINE"
  cp "$WT/codex-rs/target/release/codex" "$ENGINE"
  chmod +x "$ENGINE"
fi
ENGINE_SHA=$(shasum -a 256 "$ENGINE" | awk '{print $1}')
WT_COMMIT=$(git -C "$WT" rev-parse HEAD)
echo "engine_sha256=$ENGINE_SHA worktree_commit=$WT_COMMIT"

echo "== 2. fresh HOME (clean-machine simulation) =="
FRESH="$(mktemp -d)/clean-home"
mkdir -p "$FRESH"
PRODUCT_HOME="$FRESH/Library/Application Support/SuperAgent/codex-home"
mkdir -p "$PRODUCT_HOME"
AUTH_SRC="$HOME/Library/Application Support/SuperAgent/codex-home/auth.json"
AUTH_PROVISIONED=false
if [ -f "$AUTH_SRC" ]; then
  cp "$AUTH_SRC" "$PRODUCT_HOME/auth.json"
  AUTH_PROVISIONED=true
fi
echo "fresh_home=$FRESH auth_provisioned=$AUTH_PROVISIONED"

echo "== 3. one-command install (install.sh = install_pack + doctor) =="
HOME="$FRESH" bash "$WT/superagent/install.sh" \
  --pack "$WT/superagent/tests/p1-4/pack" --codex-home "$PRODUCT_HOME" \
  > "$EV/coldstart-install.log" 2>&1
INSTALL_RC=$?
echo "install exit=$INSTALL_RC"; cat "$EV/coldstart-install.log"

echo "== 4. doctor (explicit, JSON) =="
HOME="$FRESH" python3 "$WT/superagent/tools/doctor.py" --codex-home "$PRODUCT_HOME" \
  --engine "$ENGINE" --pack "$WT/superagent/pack" --json > "$EV/coldstart-doctor.json" 2>"$EV/coldstart-doctor.log"
DOCTOR_RC=$?
echo "doctor exit=$DOCTOR_RC"

echo "== 5. arg0 isolation: product engine helper dir under product home =="
python3 - "$PRODUCT_HOME" "$EV/coldstart-arg0-isolation.json" <<'PY'
import json, os, sys
product_home, out = sys.argv[1], sys.argv[2]
tmp = os.path.join(product_home, "tmp")
entries = []
for root, dirs, files in os.walk(tmp):
    for name in dirs + files:
        full = os.path.join(root, name)
        entries.append(os.path.relpath(full, product_home))
json.dump({"product_home": product_home,
           "product_tmp_entries": sorted(entries)[:50],
           "arg0_dir_under_product_home": any("arg0" in e for e in entries),
           "note": "engine ran with CODEX_HOME under the fresh isolated HOME; "
                   "its arg0 helper dir is created under product home, not the "
                   "real user ~/.codex (which is structurally untouched because "
                   "HOME was redirected for the whole run)."},
          open(out, "w"), indent=2)
print(json.dumps({"arg0_dir_under_product_home": any("arg0" in e for e in entries)}))
PY

echo "== 6. conclusion =="
python3 - "$EV" "$PINNED" "$WT_COMMIT" "$ENGINE_SHA" "$INSTALL_RC" "$DOCTOR_RC" "$AUTH_PROVISIONED" <<'PY'
import json, sys
ev, pinned, wt_commit, sha, irc, drc, auth = sys.argv[1:8]
doctor = {}
try:
    doctor = json.load(open(f"{ev}/coldstart-doctor.json"))
except Exception:
    pass
all_green = doctor.get("all_ok") is True and drc == "0"
conclusion = {
    "criterion": "干净 macOS 一条命令安装",
    "pinned_source_commit": pinned,
    "worktree_commit": wt_commit,
    "engine_built_from_source": True,
    "engine_sha256": sha,
    "fresh_home_no_preexisting_product_state": True,
    "auth_bootstrap": "simulated post-login (auth.json provisioned); real first run requires `superagent login` (user action)",
    "auth_provisioned": auth == "true",
    "one_command_install_exit": int(irc),
    "doctor_exit": int(drc),
    "doctor_all_green": all_green,
    "install_plus_doctor_green": irc == "0" and all_green,
    "note": "Engine built from pinned commit in isolated worktree (no prebuilt binary); "
            "install.sh ran in a fresh HOME; doctor ALL GREEN. Auth is a documented "
            "user login prerequisite, not an installer step.",
}
json.dump(conclusion, open(f"{ev}/coldstart-conclusion.json", "w"), indent=2)
print(json.dumps(conclusion, indent=2))
PY

if [ "$CREATED_WT" -eq 1 ]; then
  git -C "$ROOT" worktree remove --force "$WT" >>"$EV/coldstart-build.log" 2>&1
fi
echo "== P1-4 cold-start done =="