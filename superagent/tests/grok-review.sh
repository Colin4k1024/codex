#!/bin/bash
# Independent grok review gate for one issue closure.
# Usage: grok-review.sh <issue-num> <base-sha> <head-sha> <acceptance-file> <out-json>
set -uo pipefail
ISSUE=$1; BASE=$2; HEAD=$3; ACCEPT=$4; OUT=$5
ROOT=/Users/fanjia/Desktop/code/codex
TMP=$(mktemp -d)
git -C "$ROOT" diff "$BASE..$HEAD" > "$TMP/diff.patch"
DIFFSIZE=$(wc -c < "$TMP/diff.patch")
git -C "$ROOT" diff --stat "$BASE..$HEAD" > "$TMP/diffstat.txt"

SCHEMA='{"type":"object","properties":{"verdict":{"type":"string","enum":["APPROVE","CHANGES_REQUESTED"]},"summary":{"type":"string"},"findings":{"type":"array","items":{"type":"object","properties":{"severity":{"type":"string","enum":["blocker","major","minor"]},"location":{"type":"string"},"description":{"type":"string"},"required_fix":{"type":"string"}},"required":["severity","description"]}}},"required":["verdict","summary","findings"]}'

{
  echo "You are an INDEPENDENT code/evidence reviewer (not the implementer). Review commit range ${BASE}..${HEAD} in repo Colin4k1024/codex (local checkout $ROOT) against GitHub issue #${ISSUE} acceptance criteria below."
  echo ""
  echo "## Issue #${ISSUE} acceptance criteria"
  cat "$ACCEPT"
  echo ""
  echo "## Diff stat"
  cat "$TMP/diffstat.txt"
  echo ""
  if [ "$DIFFSIZE" -lt 150000 ]; then
    echo "## Full diff"
    cat "$TMP/diff.patch"
  else
    echo "## Diff too large to inline ($DIFFSIZE bytes). Read the changed files directly from $ROOT and inspect git diff ${BASE}..${HEAD} yourself (read-only)."
  fi
  echo ""
  echo "## Review rules"
  echo "- Verify EVERY acceptance criterion is met by concrete evidence in the diff/files; mark unproven items as blocker findings (NOT PROVEN)."
  echo "- Check: no fabricated success, no weakened acceptance, no secrets committed, no out-of-scope changes to codex-rs core, context-budget rules if prompts/skills changed."
  echo "- For spike/evidence issues: traces must be real command outputs, conclusions must follow from evidence."
  echo "- Do NOT modify any files. Read-only review."
  echo "- Output ONLY JSON matching the schema: verdict APPROVE only if all criteria proven and no blocker/major findings; otherwise CHANGES_REQUESTED with actionable required_fix per finding."
} > "$TMP/prompt.txt"

grok --prompt-file "$TMP/prompt.txt" --output-format json --json-schema "$SCHEMA" \
  --max-turns 25 --no-subagents --disable-web-search --permission-mode plan \
  > "$TMP/raw.json" 2>"$TMP/grok-stderr.log"
GROK_EXIT=$?

python3 - "$TMP/raw.json" "$OUT" <<'PY'
import json, sys, re
raw_path, out_path = sys.argv[1], sys.argv[2]
try:
    raw = json.load(open(raw_path))
except Exception as e:
    json.dump({"verdict": "REVIEW_ERROR", "summary": f"cannot parse grok output: {e}", "findings": []}, open(out_path, "w"))
    sys.exit(0)
# grok json envelope: find the structured result
candidate = None
if isinstance(raw, dict):
    for key in ("result", "response", "output", "message", "content"):
        v = raw.get(key)
        if isinstance(v, str):
            try:
                candidate = json.loads(v)
                break
            except Exception:
                m = re.search(r"\{.*\}", v, re.S)
                if m:
                    try:
                        candidate = json.loads(m.group(0))
                        break
                    except Exception:
                        pass
        elif isinstance(v, dict) and "verdict" in v:
            candidate = v
            break
    if candidate is None and "verdict" in raw:
        candidate = raw
if candidate is None:
    candidate = {"verdict": "REVIEW_ERROR", "summary": "no structured verdict in grok output", "findings": [], "raw_keys": list(raw.keys()) if isinstance(raw, dict) else str(type(raw))}
json.dump(candidate, open(out_path, "w"), ensure_ascii=False, indent=2)
print(f"verdict={candidate.get('verdict')} findings={len(candidate.get('findings', []))}")
PY
echo "grok exit=$GROK_EXIT; verdict -> $OUT"
rm -rf "$TMP"