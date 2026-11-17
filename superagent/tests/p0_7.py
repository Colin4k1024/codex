#!/usr/bin/env python3
"""P0-7 (issue #70): command/exec verification gates with real exit codes.

Cases: success / failure / timeout / cancel(+cleanup) / out-of-workspace write blocked /
network blocked. Explicit workspaceWrite sandbox policy on every call; no host-shell fallback.
"""
import json
import os
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spike_client import Client

ROOT = "/Users/fanjia/Desktop/code/codex"
ENGINE = f"{ROOT}/superagent/packaging/superagent-engine"
HOME_A = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p0-7"
os.makedirs(EV, exist_ok=True)

WORK = tempfile.mkdtemp(prefix="p07-work-")
POLICY = {"type": "workspaceWrite", "writableRoots": [WORK], "networkAccess": False}
ESCAPE_TARGET = os.path.expanduser("~/p0-7-escape-probe.txt")
if os.path.exists(ESCAPE_TARGET):
    os.remove(ESCAPE_TARGET)


def save(name, obj):
    with open(f"{EV}/{name}", "w") as fh:
        json.dump(obj, fh, ensure_ascii=False, indent=2, default=str)


def pgrep(pattern):
    r = subprocess.run(["pgrep", "-fl", pattern], capture_output=True, text=True)
    return r.stdout.strip()


client = Client(ENGINE, HOME_A, deadline=300)
cases = {}
try:
    def exec_call(name, command, extra=None, timeout=90):
        params = {"command": command, "cwd": WORK, "sandboxPolicy": POLICY}
        if extra:
            params.update(extra)
        resp = client.request("command/exec", params, timeout=timeout)
        cases[name] = {"params": params, "response": resp}
        return resp

    # 1. success
    r = exec_call("success", ["python3", "-c", "print('gate-ok')"])
    cases["success"]["pass"] = r.get("result", {}).get("exitCode") == 0

    # 2. failure with distinct exit code
    r = exec_call("failure", ["python3", "-c", "import sys; sys.exit(3)"])
    cases["failure"]["pass"] = r.get("result", {}).get("exitCode") == 3

    # 3. timeout: sleep 10 with 1500ms cap; engine reports conventional exit code 124
    r = exec_call("timeout", ["sleep", "10"], {"timeoutMs": 1500}, timeout=60)
    res = r.get("result", {})
    cases["timeout"]["pass"] = res.get("exitCode") == 124
    cases["timeout"]["observed"] = res if "error" not in r else r["error"]

    # 4. cancel: long sleep with client processId, terminate, confirm cleanup
    params = {"command": ["sleep", "30"], "cwd": WORK, "sandboxPolicy": POLICY,
              "processId": "p07-cancel-probe"}
    rid = client.send_request("command/exec", params)
    time.sleep(2)
    before = pgrep("sleep 30")
    term = client.request("command/exec/terminate", {"processId": "p07-cancel-probe"}, timeout=30)
    try:
        r = client.wait_response(rid, timeout=60)
    except TimeoutError:
        r = {"error": {"message": "exec response missing after terminate"}}
    time.sleep(2)
    after = pgrep("sleep 30")
    cases["cancel"] = {"params": params, "terminateResponse": term, "execResponse": r,
                       "sleepBeforeTerminate": before, "sleepAfterTerminate": after,
                       "pass": bool(before) and not after and "error" not in term
                               and r.get("result", {}).get("exitCode") not in (None, 124)}

    # 5. out-of-workspace write must be blocked by sandbox
    r = exec_call("escapeWrite", ["python3", "-c",
                  f"open({ESCAPE_TARGET!r}, 'w').write('escaped')"])
    cases["escapeWrite"]["escapeFileExists"] = os.path.exists(ESCAPE_TARGET)
    cases["escapeWrite"]["pass"] = (not os.path.exists(ESCAPE_TARGET)) and (
        r.get("result", {}).get("exitCode") not in (0, None) or "error" in r)

    # 6. network must be blocked (networkAccess=false)
    r = exec_call("networkBlocked", ["python3", "-c",
                  "import urllib.request; urllib.request.urlopen('https://example.com', timeout=8); print('NET-OK')"],
                  timeout=60)
    out = json.dumps(r)
    cases["networkBlocked"]["pass"] = "NET-OK" not in out and (
        r.get("result", {}).get("exitCode") not in (0, None) or "error" in r)

    deltas = [n for n in client.notifications if n.get("method") == "command/exec/outputDelta"]
    summary = {
        "workspace": WORK,
        "sandboxPolicy": POLICY,
        "cases": {k: {"pass": v.get("pass")} for k, v in cases.items()},
        "outputDeltaCount": len(deltas),
        "allPass": all(v.get("pass") for v in cases.values()),
    }
    save("cases.json", cases)
    save("summary.json", summary)
    save("transcript.json", client.transcript())
    print(json.dumps(summary, indent=2))
finally:
    client.close()
    if os.path.exists(ESCAPE_TARGET):
        os.remove(ESCAPE_TARGET)
sys.exit(0 if summary["allPass"] else 1)