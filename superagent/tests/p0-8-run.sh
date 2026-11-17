#!/bin/bash
set -uo pipefail
ROOT=/Users/fanjia/Desktop/code/codex
EV=$ROOT/superagent/evidence/p0-8
export https_proxy=http://127.0.0.1:7890 http_proxy=http://127.0.0.1:7890
mkdir -p "$EV"

echo "== static evidence: SubagentStart/SubagentStop hook events =="
grep -n 'SubagentStart\|SubagentStop' "$ROOT/codex-rs/hooks/src/lib.rs" > "$EV/hooks-events.txt" 2>&1
cat "$EV/hooks-events.txt"

echo "== static evidence: no public agent/* control RPC in generated schema =="
python3 - "$EV/no-agent-rpc.txt" <<'PY'
import json, sys
d = json.load(open("/Users/fanjia/Desktop/code/codex/codex-rs/app-server-protocol/schema/json/ClientRequest.json"))
acc = set()
def m(o):
    if isinstance(o, dict):
        if o.get("type") == "string" and isinstance(o.get("enum"), list) and len(o["enum"]) == 1:
            acc.add(o["enum"][0])
        for v in o.values(): m(v)
    elif isinstance(o, list):
        for v in o: m(v)
m(d)
methods = sorted(x for x in acc if "/" in x)
agent = [x for x in methods if x.startswith("agent/")]
with open(sys.argv[1], "w") as fh:
    fh.write(f"agent/* methods in ClientRequest schema: {agent}\n")
    fh.write("thread observation methods: " + ", ".join(x for x in methods if x.startswith("thread/")) + "\n")
print(open(sys.argv[1]).read())
PY

python3 "$ROOT/superagent/tests/p0_8.py" "$EV"
echo "P0-8 exit=$?"