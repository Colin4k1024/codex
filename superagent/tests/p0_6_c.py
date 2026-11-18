#!/usr/bin/env python3
"""P0-6 trace C isolated rerun: turn/interrupt must kill the exec child group.
exec.rs:1052-1064 kills/terminates the child process group on cancel; this probe
verifies the sleep child disappears after interrupt (retry-tolerant timing)."""
import json, os, subprocess, sys, tempfile, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spike_client import Client
ROOT = "/Users/fanjia/Desktop/code/codex"
ENGINE = f"{ROOT}/superagent/packaging/superagent-engine"
HOME_A = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p0-6"
os.makedirs(EV, exist_ok=True)
def pgrep(p):
    r = subprocess.run(["pgrep","-fl",p],capture_output=True,text=True); return r.stdout.strip()
def is_note(m): return lambda n: n.get("method")==m
work = tempfile.mkdtemp(prefix="p06c-")
client = Client(ENGINE, HOME_A, deadline=300)
try:
    ts = client.request("thread/start", {"cwd": work}, timeout=60)
    tid = ts["result"]["thread"]["id"]
    rid = client.send_request("turn/start", {"threadId": tid, "input":[{"type":"text","text":
        "Use your command execution tool to run `sleep 60` and wait for it to finish. Do nothing else before it finishes."}]})
    started = client.wait_notification(is_note("turn/started"), timeout=120)
    turn_id = started.get("params",{}).get("turn",{}).get("id") or started.get("params",{}).get("turnId")
    before=""
    for _ in range(60):
        before=pgrep("sleep 60")
        if before: break
        time.sleep(1)
    ir = client.request("turn/interrupt", {"threadId": tid, "turnId": turn_id}, timeout=30)
    cn = client.wait_notification(is_note("turn/completed"), timeout=120)
    try: resp = client.wait_response(rid, timeout=60)
    except TimeoutError: resp={"error":{"message":"no turn/start response after interrupt"}}
    after=""; secs=0
    for _ in range(30):
        after=pgrep("sleep 60")
        if not after: break
        time.sleep(1); secs+=1
    status=cn.get("params",{}).get("turn",{}).get("status")
    summary={"pass": bool(before) and not after and "error" not in ir,
             "turnId":turn_id,"sleepBeforeInterrupt":before,"sleepAfterInterrupt":after,
             "cleanupSeconds":secs,"interruptResponse":ir,"turnStatus":status,"turnStartResponse":resp}
    json.dump(summary, open(f"{EV}/trace-c-rerun.json","w"), ensure_ascii=False, indent=2, default=str)
    print(json.dumps({k:summary[k] for k in ("pass","sleepBeforeInterrupt","sleepAfterInterrupt","cleanupSeconds","turnStatus")}, indent=2))
finally:
    client.close()
sys.exit(0 if summary["pass"] else 1)
