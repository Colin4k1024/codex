#!/bin/bash
# Independent grok review gate for one issue closure.
# Usage: grok-review.sh <issue-num> <base-sha> <head-sha> <acceptance-file> <out-json>
set -uo pipefail
ISSUE=$1; BASE=$2; HEAD=$3; ACCEPT=$4; OUT=$5
PATHSPEC=${6:-}
ROOT=/Users/fanjia/Desktop/code/codex
TMP=$(mktemp -d "$ROOT/.grok-review-XXXXXX")
git -C "$ROOT" diff "$BASE..$HEAD" -- $PATHSPEC > "$TMP/diff.patch"
DIFFSIZE=$(wc -c < "$TMP/diff.patch")
git -C "$ROOT" diff --stat "$BASE..$HEAD" -- $PATHSPEC > "$TMP/diffstat.txt"

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
  if [ "$DIFFSIZE" -lt 400000 ]; then
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
  echo "- Do NOT modify any files. Read-only review. Prefer concluding from the inlined diff and acceptance text; only read local files if strictly necessary."
  echo "- Do NOT run any Bash/shell commands at all (compound commands are blocked and will abort this review). If part of this prompt was offloaded to a file, use the read_file tool to read it. The inlined diff IS the complete authoritative diff for the reviewed paths; no git history verification is needed."
  echo "- Code citations are pre-extracted in the appendix below from the reviewed HEAD commit (git show); verify claims against it instead of running commands."
  echo "- The working tree may be checked out at a DIFFERENT branch than the reviewed HEAD. Never judge file presence/content from the working tree; the inlined diff and the appendix are the authoritative state at the reviewed HEAD."
  echo "- Output ONLY JSON matching the schema: verdict APPROVE only if all criteria proven and no blocker/major findings; otherwise CHANGES_REQUESTED with actionable required_fix per finding."
} > "$TMP/prompt.txt"

python3 - "$TMP/prompt.txt" "$ROOT" "$HEAD" <<'PY'
import re, sys, os, subprocess
prompt_path, root, head = sys.argv[1], sys.argv[2], sys.argv[3]
text = open(prompt_path).read()
cites = {}
for m in re.finditer(r'([A-Za-z0-9_./-]+\.(?:rs|py|toml|md|sh)):(\d+)(?:-(\d+))?', text):
    path, a, b = m.group(1), int(m.group(2)), m.group(3)
    b = int(b) if b else a
    if b - a > 60:
        b = a + 60
    cites.setdefault(path, set()).add((max(1, a - 2), b + 2))
appendix = ["", f"## Code citation appendix (extracted via git show from reviewed HEAD {head}; ranges include 2 context lines)"]
budget = 30000
used = 0
for path, ranges in sorted(cites.items()):
    content = None
    for cand in (path, os.path.join("codex-rs", path)):
        r = subprocess.run(["git", "-C", root, "show", f"{head}:{cand}"],
                           capture_output=True, text=True)
        if r.returncode == 0:
            content = r.stdout
            break
    if content is None:
        continue
    lines = content.splitlines()
    merged = []
    for a, b in sorted(ranges):
        if merged and a <= merged[-1][1] + 1:
            merged[-1] = (merged[-1][0], max(merged[-1][1], b))
        else:
            merged.append((a, b))
    for a, b in merged:
        chunk = f"### {path}:{a}-{b}\n" + "\n".join(f"{i}: {lines[i-1]}" for i in range(a, min(b, len(lines)) + 1)) + "\n"
        if used + len(chunk) > budget:
            break
        appendix.append(chunk)
        used += len(chunk)
with open(prompt_path, "a") as fh:
    fh.write("\n".join(appendix))
PY

grok --prompt-file "$TMP/prompt.txt" --output-format json --json-schema "$SCHEMA" \
  --cwd "$ROOT" --max-turns 25 --no-subagents --disable-web-search --permission-mode plan \
  --allow 'Bash(git diff:*)' --allow 'Bash(git log:*)' --allow 'Bash(git show:*)' \
  --allow 'Bash(git status:*)' --allow 'Bash(git rev-parse:*)' --allow 'Bash(ls:*)' \
  --allow 'Bash(cat:*)' --allow 'Bash(head:*)' --allow 'Bash(tail:*)' --allow 'Bash(grep:*)' \
  --allow 'Bash(find:*)' --allow 'Bash(wc:*)' --allow 'Bash(shasum:*)' --allow 'Bash(stat:*)' \
  --allow "Bash(git -C $ROOT diff:*)" --allow "Bash(git -C $ROOT show:*)" \
  --allow "Bash(git -C $ROOT log:*)" --allow "Bash(git -C $ROOT status:*)" \
  --allow "Bash(git -C $ROOT rev-parse:*)" \
  > "$TMP/raw.json" 2>"$TMP/grok-stderr.log"
GROK_EXIT=$?
cp "$TMP/grok-stderr.log" "$OUT.stderr.log" 2>/dev/null

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
    so = raw.get("structuredOutput")
    if isinstance(so, dict) and "verdict" in so:
        candidate = so
    elif isinstance(so, str):
        try:
            v = json.loads(so)
            if isinstance(v, dict) and "verdict" in v:
                candidate = v
        except Exception:
            pass
    if candidate is None:
        for key in ("text", "result", "response", "output", "message", "content"):
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
    candidate = {"verdict": "REVIEW_ERROR", "summary": "no structured verdict in grok output",
                 "findings": [], "raw_keys": list(raw.keys()) if isinstance(raw, dict) else str(type(raw)),
                 "structuredOutputError": raw.get("structuredOutputError") if isinstance(raw, dict) else None}
    json.dump(raw, open(out_path + ".raw.json", "w"), ensure_ascii=False, indent=2, default=str)
json.dump(candidate, open(out_path, "w"), ensure_ascii=False, indent=2)
print(f"verdict={candidate.get('verdict')} findings={len(candidate.get('findings', []))}")
PY
echo "grok exit=$GROK_EXIT; verdict -> $OUT"
rm -rf "$TMP"