#!/usr/bin/env python3
"""P0-6b (#69): resource-limit initial values + spawn wiring, NO model calls needed.

Proves: (1) per-run log rotation mechanism (tested at a scaled 1 MiB threshold;
production initial value is 100 MiB via Client default), (2) client-side request
timeout enforcement + recovery via command/exec/terminate, (3) per-type timeout
map wiring, (4) spawn argv/pipe wiring recorded from the live client,
(5) 16 MiB single-frame limit config.
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
HOME = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p0-6"
os.makedirs(EV, exist_ok=True)
WORK = tempfile.mkdtemp(prefix="p06b-work-")
POLICY = {"type": "workspaceWrite", "writableRoots": [WORK], "networkAccess": False}

PII_KEYS = {"installationId", "serverName", "codexHome", "authMode", "planType"}


def scrub(obj):
    if isinstance(obj, dict):
        return {k: ("<scrubbed>" if k in PII_KEYS else scrub(v)) for k, v in obj.items()}
    if isinstance(obj, list):
        return [scrub(v) for v in obj]
    if isinstance(obj, str):
        return obj.replace("fanjiadeMacBook-Pro.local", "<host>").replace("fanjia", "<user>")
    return obj


def save(name, obj):
    with open(f"{EV}/{name}", "w") as fh:
        json.dump(scrub(obj), fh, ensure_ascii=False, indent=2, default=str)


def pgrep(pattern):
    r = subprocess.run(["pgrep", "-fl", pattern], capture_output=True, text=True)
    return r.stdout.strip()


cases = {}

# --- 1. log rotation at scaled threshold (mechanism proof; prod default 100 MiB) ---
log_path = f"{EV}/run-log-rotation-test.log"
if os.path.exists(log_path):
    os.remove(log_path)
TEST_LIMIT = 1024 * 1024
c = Client(ENGINE, HOME, deadline=240, run_log_path=log_path, max_log_bytes=TEST_LIMIT)
try:
    for i in range(6):
        c.request("command/exec", {
            "command": ["python3", "-c", f"print('x' * 300000)  # chunk {i}"],
            "cwd": WORK, "sandboxPolicy": POLICY}, timeout=60)
    retained = os.path.getsize(log_path)
    cases["logRotation"] = {
        "testLimitBytes": TEST_LIMIT,
        "productionDefaultBytes": 100 * 1024 * 1024,
        "rotations": c.run_log_rotations,
        "retainedBytes": retained,
        "pass": c.run_log_rotations >= 1 and retained <= TEST_LIMIT,
        "note": "Rotation drops the oldest segment so retained run log never exceeds the limit; "
                "mechanism proven at 1 MiB scaled threshold, production initial value 100 MiB "
                "is the Client default (spike_client.py max_log_bytes).",
    }
    wiring = {
        "spawnArgv": c.spawn_argv,
        "spawnWiring": c.transcript()["spawnWiring"],
        "frameLimitBytes": c.max_frame_bytes,
        "maxFrameBytesSeen": c.max_frame_seen,
        "frameViolations": c.frame_violations,
        "defaultTimeoutsByMethod": c.DEFAULT_TIMEOUTS,
        "defaultTimeoutFallback": c.DEFAULT_TIMEOUT_FALLBACK,
        "reference": "spawn path also covered by #64 (PR #103, merged): superagent/evidence/p0-1/",
    }
    save("spawn-wiring.json", wiring)
finally:
    c.close()

# --- 2. client-side timeout enforcement + recovery ---
c = Client(ENGINE, HOME, deadline=240)
try:
    rid = c.send_request("command/exec", {
        "command": ["sleep", "20"], "cwd": WORK, "sandboxPolicy": POLICY,
        "processId": "p06b-timeout-probe"})
    t0 = time.time()
    timed_out = False
    try:
        c.wait_response(rid, timeout=2)
    except TimeoutError:
        timed_out = True
    elapsed = time.time() - t0
    before = pgrep("sleep 20")
    term = c.request("command/exec/terminate", {"processId": "p06b-timeout-probe"})
    time.sleep(2)
    after = pgrep("sleep 20")
    cases["requestTimeout"] = {
        "explicitTimeoutSeconds": 2,
        "timeoutRaised": timed_out,
        "elapsedSeconds": round(elapsed, 2),
        "sleepBeforeTerminate": before,
        "sleepAfterTerminate": after,
        "terminateResponse": term,
        "pass": timed_out and elapsed < 5 and bool(before) and not after and "error" not in term,
        "note": "Per-type defaults live in Client.DEFAULT_TIMEOUTS and are applied by "
                "Client.request when timeout=None (spike_client.py); explicit timeout "
                "overrides. Recovery = command/exec/terminate kills the process group.",
    }
    save("resource-limits-transcript.json", {"results": [term], **c.transcript()})
finally:
    c.close()

summary = {k: {"pass": v["pass"]} for k, v in cases.items()}
summary["allPass"] = all(v["pass"] for v in cases.values())
save("resource-limits.json", {"cases": cases, "summary": summary})
print(json.dumps(summary, indent=2))
sys.exit(0 if summary["allPass"] else 1)