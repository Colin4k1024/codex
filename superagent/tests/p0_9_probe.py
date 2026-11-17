#!/usr/bin/env python3
"""P0-9 (issue #72) probe: actual availability of experimental app-server APIs.

Tests environments / permissions / historyMode / memory / collaborationMode / goal / queue
with experimentalApi on and off. --no-model skips turn/start probes (model quota safe);
all other probes are model-free.
"""
import json
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spike_client import Client

ROOT = "/Users/fanjia/Desktop/code/codex"
ENGINE = f"{ROOT}/superagent/packaging/superagent-engine"
HOME_A = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p0-9"
NO_MODEL = "--no-model" in sys.argv
os.makedirs(EV, exist_ok=True)
WORK = tempfile.mkdtemp(prefix="p09-probe-")

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


def classify(resp):
    if "error" not in resp:
        return "AVAILABLE"
    msg = json.dumps(resp["error"]).lower()
    if "experimental" in msg:
        return "GATED_REQUIRES_EXPERIMENTAL"
    if "not supported yet" in msg:
        return "UNSUPPORTED_SERVER_SIDE"
    if "permissions" in msg and ("requires" in msg or "profile" in msg):
        return "AVAILABLE_REQUIRES_CONFIG"
    return "ERROR_OTHER"


probes = {}

exp = Client(ENGINE, HOME_A, experimental=True, deadline=300)
try:
    for method, params in [
        ("environment/info", {"environmentId": "local"}),
        ("memory/status", {}),
        ("collaborationMode/list", {}),
    ]:
        try:
            r = exp.request(method, params, timeout=30)
        except Exception as exc:
            r = {"error": {"message": str(exc)}}
        probes[f"experimental:{method}"] = {"response": r, "status": classify(r)}

    ts = exp.request("thread/start", {"cwd": WORK, "historyMode": "paginated"}, timeout=60)
    probes["experimental:thread/start.historyMode=paginated"] = {"response": ts, "status": classify(ts)}
    thread_id = ts.get("result", {}).get("thread", {}).get("id")

    if thread_id:
        if not NO_MODEL:
            r = exp.request("turn/start", {"threadId": thread_id,
                                           "input": [{"type": "text", "text": "Reply with the single word OK."}],
                                           "permissions": "p09-nonexistent-profile"}, timeout=180)
            probes["experimental:turn/start.permissions=unknown-profile"] = {"response": r, "status": classify(r)}

            r = exp.request("turn/start", {"threadId": thread_id,
                                           "input": [{"type": "text", "text": "Reply with the single word OK."}],
                                           "environments": []}, timeout=180)
            probes["experimental:turn/start.environments=[]"] = {"response": r, "status": classify(r)}
        else:
            probes["experimental:turn/start.permissions=unknown-profile"] = {"status": "SKIPPED_NO_MODEL"}
            probes["experimental:turn/start.environments=[]"] = {"status": "SKIPPED_NO_MODEL"}

        il = exp.request("thread/items/list", {"threadId": thread_id}, timeout=60)
        probes["experimental:thread/items/list"] = {"response": il, "status": classify(il)}

        g = exp.request("thread/goal/set", {"threadId": thread_id, "objective": "p09 probe goal"}, timeout=30)
        probes["experimental:thread/goal/set"] = {"response": g, "status": classify(g)}
        g = exp.request("thread/goal/get", {"threadId": thread_id}, timeout=30)
        probes["experimental:thread/goal/get"] = {"response": g, "status": classify(g)}
        g = exp.request("thread/goal/clear", {"threadId": thread_id}, timeout=30)
        probes["experimental:thread/goal/clear"] = {"response": g, "status": classify(g)}

        q = exp.request("thread/queue/add", {"threadId": thread_id,
                                             "input": [{"type": "text", "text": "p09 queue probe"}],
                                             "clientUserMessageId": "p09-q1"}, timeout=30)
        probes["experimental:thread/queue/add"] = {"response": q, "status": classify(q)}
        q = exp.request("thread/queue/list", {"threadId": thread_id}, timeout=30)
        probes["experimental:thread/queue/list"] = {"response": q, "status": classify(q)}
        items = (q.get("result", {}).get("data") or [])
        sub_id = (items[0].get("queuedSubmissionId") or items[0].get("id")
                  if items and isinstance(items[0], dict) else None) or "p09-q1"
        q = exp.request("thread/queue/delete", {"threadId": thread_id, "queuedSubmissionId": sub_id}, timeout=30)
        probes["experimental:thread/queue/delete"] = {"response": q, "status": classify(q),
                                                      "queuedSubmissionIdUsed": sub_id}
    save("experimental-transcript.json", exp.transcript())
finally:
    exp.close()

plain = Client(ENGINE, HOME_A, experimental=False, deadline=120)
try:
    for method, params in [("environment/info", {"environmentId": "local"}), ("memory/status", {})]:
        try:
            r = plain.request(method, params, timeout=30)
        except Exception as exc:
            r = {"error": {"message": str(exc)}}
        probes[f"plain:{method}"] = {"response": r, "status": classify(r)}
    ts = plain.request("thread/start", {"cwd": WORK, "historyMode": "paginated"}, timeout=60)
    probes["plain:thread/start.historyMode=paginated"] = {"response": ts, "status": classify(ts)}
    save("plain-transcript.json", plain.transcript())
finally:
    plain.close()

summary = {k: v["status"] for k, v in probes.items()}
save("probe-summary.json", {"workdir": WORK, "probes": probes, "summary": summary})
print(json.dumps(summary, indent=2))