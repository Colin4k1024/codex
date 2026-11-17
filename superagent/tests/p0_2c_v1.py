#!/usr/bin/env python3
"""P0-2c (issue #65): V1 multi-agent close_agent/resume_agent round-trip.

V2 has no close_agent (spec_plan.rs:794 lists it for V1 only). This probe runs a
V1 leader (gpt-5.6-luna, collab feature default-on, multi_agent_v2 off) and captures
the actual close_agent function_call + result and a resume_agent attempt.
"""
import json
import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spike_client import Client, rollout_function_calls

ROOT = "/Users/fanjia/Desktop/code/codex"
ENGINE = f"{ROOT}/superagent/packaging/superagent-engine"
HOME_A = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p0-2"
os.makedirs(EV, exist_ok=True)
HOME_C = f"{EV}/home-c"
os.makedirs(HOME_C, exist_ok=True)
with open(f"{HOME_C}/config.toml", "w") as fh:
    fh.write("# P0-2c V1 leader home\n")
    fh.write(open(f"{ROOT}/superagent/tests/p0-2/config-overlay-v1.toml").read())
if os.path.exists(f"{HOME_A}/auth.json"):
    shutil.copyfile(f"{HOME_A}/auth.json", f"{HOME_C}/auth.json")

WORK = tempfile.mkdtemp(prefix="p02c-")


def save(name, obj):
    with open(f"{EV}/{name}", "w") as fh:
        json.dump(obj, fh, ensure_ascii=False, indent=2, default=str)


PROMPT = (
    "Use the multi-agent tools exactly in this order:\n"
    "1) spawn_agent a sub-agent with task: 'Reply with the single word V1-CHILD-OK. Do nothing else.'\n"
    "2) wait_agent for it and report its reply.\n"
    "3) close_agent on that agent; quote the exact tool result (status).\n"
    "4) resume_agent on the same (closed) agent; quote the exact tool result or error.\n"
    "Reply with a numbered summary including the agent id, the close result and the resume result."
)

client = Client(ENGINE, HOME_C, deadline=600)
try:
    ts = client.request("thread/start", {"cwd": WORK}, timeout=60)
    leader_id = ts["result"]["thread"]["id"]
    rid = client.send_request("turn/start", {"threadId": leader_id,
                                             "input": [{"type": "text", "text": PROMPT}]})
    client.wait_notification(lambda n: n.get("method") == "turn/started", timeout=120)

    def leader_completed(n):
        p = n.get("params", {})
        return n.get("method") == "turn/completed" and p.get("threadId") == leader_id
    completed = client.wait_notification(leader_completed, timeout=480)
    resp = client.wait_response(rid, timeout=60)
    save("p0-2c-transcript.json", {"results": [ts, resp], **client.transcript()})
finally:
    client.close()

collab_items = []
child_ids = {}
final_text = []
for n in client.notifications:
    if n.get("method") != "item/completed":
        continue
    it = n.get("params", {}).get("item", {})
    if it.get("type") == "collabAgentToolCall":
        collab_items.append({"tool": it.get("tool"), "status": it.get("status"),
                             "receiverThreadIds": it.get("receiverThreadIds"),
                             "agentsStates": it.get("agentsStates")})
    if it.get("type") == "subAgentActivity" and it.get("kind") == "started":
        child_ids[it.get("agentPath")] = it.get("agentThreadId")
    if it.get("type") == "agentMessage":
        final_text.append(it.get("text") or "")

leader_files, leader_calls = rollout_function_calls(HOME_C, leader_id)
save("p0-2c-leader-function-calls.json", {"leaderThreadId": leader_id, "rolloutFiles": leader_files,
                                          "calls": leader_calls})


def find_call(substr):
    return [c for c in leader_calls if substr in str(c.get("call", {}).get("name", ""))]


spawn_calls = find_call("spawn_agent")
close_calls = find_call("close_agent")
resume_calls = find_call("resume_agent")
wait_calls = find_call("wait_agent")
leader_summary = "\n".join(final_text)

summary = {
    "leaderThreadId": leader_id,
    "leaderModelExpected": "gpt-5.6-luna",
    "multiAgentVersionExpected": "v1",
    "childThreads": child_ids,
    "collabItems": collab_items,
    "spawnCallFound": bool(spawn_calls),
    "waitCallFound": bool(wait_calls),
    "closeCallFound": bool(close_calls),
    "closeCalls": close_calls,
    "resumeCallFound": bool(resume_calls),
    "resumeCalls": resume_calls,
    "leaderFinalMessage": leader_summary[-3000:],
    "turnCompleted": completed,
}
summary["allPass"] = bool(spawn_calls and close_calls and resume_calls
                          and all(c.get("output") for c in close_calls))
save("p0-2c-v1-summary.json", summary)
print(json.dumps({k: v for k, v in summary.items()
                  if k in ("allPass", "spawnCallFound", "waitCallFound", "closeCallFound",
                           "resumeCallFound", "childThreads")}, indent=2))
sys.exit(0 if summary["allPass"] else 1)