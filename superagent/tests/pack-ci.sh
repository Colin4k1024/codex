#!/bin/bash
# P1-1 (#73) CI gate: deterministic digest re-export comparison + full tests.
# Usage: bash superagent/tests/pack-ci.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PACK="$ROOT/superagent/pack"
TOOL="$ROOT/superagent/tools/pack.py"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# clean-environment copy (no git metadata, fresh dir)
mkdir -p "$TMP/pack"
(cd "$PACK" && tar cf - .) | (cd "$TMP/pack" && tar xf -)
rm -f "$TMP/pack/manifest.json"

python3 "$TOOL" export --pack "$TMP/pack" > "$TMP/reexport.json"
COMMITTED_DIGEST=$(python3 -c "import json;print(json.load(open('$PACK/manifest.json'))['digest'])")
REEXPORT_DIGEST=$(python3 -c "import json;print(json.load(open('$TMP/reexport.json'))['digest'])")
if [ "$COMMITTED_DIGEST" != "$REEXPORT_DIGEST" ]; then
  echo "FAIL: re-export digest $REEXPORT_DIGEST != committed $COMMITTED_DIGEST"
  exit 1
fi
echo "re-export comparison: PASS ($COMMITTED_DIGEST)"

python3 "$TOOL" verify --pack "$PACK"
python3 "$TOOL" verify --pack "$TMP/pack"
python3 "$ROOT/superagent/tests/test_pack.py"
echo "pack-ci: ALL PASS"