#!/usr/bin/env python3
"""P0-7 (issue #70): command/exec verification gates with real exit codes.

Cases: success / failure / timeout / cancel(+cleanup) / cancel process-TREE cleanup /
out-of-workspace write blocked / network blocked / malformed policy fail-closed /
well-formed-but-inexpressible policy semantic rejection / default-permission inheritance.
Explicit workspaceWrite sandbox policy on most calls; no host-shell fallback.
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
        json.dump(scrub(obj), fh, ensure_ascii=False, indent=2, default=str)


PII_KEYS = {"installationId", "serverName", "codexHome", "authMode", "planType"}


def scrub(obj):
    if isinstance(obj, dict):
        return {k: ("<scrubbed>" if k in PII_KEYS else scrub(v)) for k, v in obj.items()}
    if isinstance(obj, list):
        return [scrub(v) for v in obj]
    if isinstance(obj, str):
        return obj.replace("fanjiadeMacBook-Pro.local", "<host>").replace("fanjia", "<user>")
    return obj


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

    # 6. network must be blocked (networkAccess=false); check stdout only —
    # stderr tracebacks echo the source line containing the NET-OK literal
    r = exec_call("networkBlocked", ["python3", "-c",
                  "import urllib.request; urllib.request.urlopen('https://example.com', timeout=8); print('NET-OK')"],
                  timeout=60)
    res = r.get("result", {})
    cases["networkBlocked"]["pass"] = "NET-OK" not in (res.get("stdout") or "") and (
        res.get("exitCode") not in (0, None) or "error" in r)

    # 7. inexpressible policy (relative writableRoot) must fail closed, never run via host shell
    marker = os.path.join(WORK, "p07-invalid-policy-marker.txt")
    bad_policy = {"type": "workspaceWrite", "writableRoots": ["relative/not-absolute"],
                  "networkAccess": False}
    r = client.request("command/exec", {"command": ["python3", "-c",
                      f"open({marker!r}, 'w').write('ran')"],
                      "cwd": WORK, "sandboxPolicy": bad_policy}, timeout=60)
    cases["invalidPolicyBlocked"] = {
        "params": {"sandboxPolicy": bad_policy}, "response": r,
        "markerExists": os.path.exists(marker),
        "pass": "error" in r and not os.path.exists(marker),
    }

    # 8. cancel process TREE: parent python spawns a descendant sleep; both must die
    tree_script = ("import subprocess,time; subprocess.Popen(['sleep','47']); "
                   "print('TREE-PARENT-READY',flush=True); time.sleep(47) # p07-tree-parent")
    params = {"command": ["python3", "-c", tree_script], "cwd": WORK, "sandboxPolicy": POLICY,
              "processId": "p07-cancel-tree"}
    rid = client.send_request("command/exec", params)
    deadline = time.time() + 20
    parent_before = child_before = ""
    while time.time() < deadline:
        parent_before = pgrep("p07-tree-parent")
        child_before = pgrep("sleep 47")
        if parent_before and child_before:
            break
        time.sleep(0.5)
    term = client.request("command/exec/terminate", {"processId": "p07-cancel-tree"}, timeout=30)
    try:
        r = client.wait_response(rid, timeout=60)
    except TimeoutError:
        r = {"error": {"message": "exec response missing after terminate"}}
    time.sleep(3)
    parent_after = pgrep("p07-tree-parent")
    child_after = pgrep("sleep 47")
    cases["cancelTree"] = {
        "params": params, "terminateResponse": term, "execResponse": r,
        "parentBefore": parent_before, "descendantBefore": child_before,
        "parentAfter": parent_after, "descendantAfter": child_after,
        "pass": bool(parent_before) and bool(child_before)
                and not parent_after and not child_after and "error" not in term,
    }

    # 9. well-formed but semantically inexpressible policy: readOnly.access=restricted
    # is a recognized legacy field rejected by SandboxPolicy Deserialize
    # (app-server-protocol/src/protocol/v2/permissions.rs:618-620), not a malformed path.
    marker2 = os.path.join(WORK, "p07-semantic-policy-marker.txt")
    legacy_policy = {"type": "readOnly", "networkAccess": False, "access": {"type": "restricted"}}
    r = client.request("command/exec", {"command": ["python3", "-c",
                      f"open({marker2!r}, 'w').write('ran')"],
                      "cwd": WORK, "sandboxPolicy": legacy_policy}, timeout=60)
    err_msg = json.dumps(r.get("error", {}))
    cases["semanticPolicyBlocked"] = {
        "params": {"sandboxPolicy": legacy_policy}, "response": r,
        "markerExists": os.path.exists(marker2),
        "rejectionIsSemantic": "readOnly.access is no longer supported" in err_msg,
        "pass": "error" in r and not os.path.exists(marker2)
                and "readOnly.access is no longer supported" in err_msg,
    }

    # 10. no sandboxPolicy/permissionProfile: falls back to session config.permissions
    # (command_exec_processor.rs else-branch). Must NOT grant host-level write.
    r = client.request("command/exec", {"command": ["python3", "-c",
                      f"open({ESCAPE_TARGET!r}, 'w').write('escaped-default')"],
                      "cwd": WORK}, timeout=60)
    cases["defaultPolicyNoHostWrite"] = {
        "params": {"cwd": WORK, "note": "no sandboxPolicy, no permissionProfile"},
        "response": r,
        "escapeFileExists": os.path.exists(ESCAPE_TARGET),
        "pass": not os.path.exists(ESCAPE_TARGET),
    }
    if os.path.exists(ESCAPE_TARGET):
        os.remove(ESCAPE_TARGET)
    r = client.request("command/exec", {"command": ["python3", "-c", "print('default-ok')"],
                      "cwd": WORK}, timeout=60)
    cases["defaultPolicyInWorkspace"] = {
        "response": r,
        "pass": r.get("result", {}).get("exitCode") == 0
                and "default-ok" in (r.get("result", {}).get("stdout") or ""),
    }

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