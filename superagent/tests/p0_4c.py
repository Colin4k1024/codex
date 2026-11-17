#!/usr/bin/env python3
"""P0-4c (#67 F1): prove the MODEL invokes create_goal/get_goal/update_goal tools.

Runs a real turn instructing the model to call the goal tools, then scans the leader
rollout JSONL for function_call/function_call_output pairs (rollout_function_calls).
Pass requires >=1 call of each tool with a recorded output. No fabricated success.
"""
import json
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spike_client import Client, rollout_function_calls

ROOT = "/Users/fanjia/Desktop/code/codex"
EV = f"{ROOT}/superagent/evidence/p0-4"
HOME = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
ENGINE = f"{ROOT}/superagent/packaging/superagent-engine"
WORK = tempfile.mkdtemp(prefix="p04c-work-")
os.makedirs(EV, exist_ok=True)

PII_KEYS = {"installationId", "serverName", "codexHome", "authMode", "planType"}
GOAL_TOOLS = ("create_goal", "get_goal", "update_goal")


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


PROMPT = (
    "You have goal tools available: create_goal, get_goal, update_goal. "
    "Call them in this exact order, actually invoking the tools (do not just describe them): "
    "1) create_goal with objective 'P04C-TOOL-PROBE'; "
    "2) get_goal and note the returned status; "
    "3) update_goal to record progress 'halfway done'; "
    "4) get_goal again. "
    "Then reply with exactly GOAL-TOOLS-OK."
)

client = Client(ENGINE, HOME, deadline=600)
try:
    ts = client.request("thread/start", {"cwd": WORK}, timeout=60)
    thread_id = ts["result"]["thread"]["id"]
    rid = client.send_request("turn/start", {"threadId": thread_id,
                                             "input": [{"type": "text", "text": PROMPT}]})
    client.wait_notification(lambda n: n.get("method") == "turn/completed"
                             and n.get("params", {}).get("threadId") == thread_id, timeout=540)
    resp = client.wait_response(rid, timeout=30)
    files, calls = rollout_function_calls(HOME, thread_id)
    goal_calls = [e for e in calls if e.get("call", {}).get("name") in GOAL_TOOLS]
    per_tool = {}
    for tool in GOAL_TOOLS:
        matched = [e for e in goal_calls if e["call"]["name"] == tool]
        per_tool[tool] = {
            "callCount": len(matched),
            "withOutput": sum(1 for e in matched if e.get("output")),
            "samples": matched[:3],
        }
    texts = [n.get("params", {}).get("item", {}).get("text", "")
             for n in client.notifications
             if n.get("method") == "item/completed"
             and n.get("params", {}).get("item", {}).get("type") == "agentMessage"]
    summary = {
        "threadId": thread_id,
        "rolloutFiles": files,
        "perTool": per_tool,
        "goalToolsRoundTrip": all(v["callCount"] >= 1 and v["withOutput"] >= 1
                                  for v in per_tool.values()),
        "replyOk": any("GOAL-TOOLS-OK" in t for t in texts),
        "turnStartResponse": resp,
    }
    summary["allPass"] = summary["goalToolsRoundTrip"] and summary["replyOk"]
    save("goal-tool-calls.json", {"prompt": PROMPT, "goalCalls": goal_calls,
                                  "allFunctionCallCount": len(calls)})
    save("p0-4c-summary.json", summary)
    save("p0-4c-transcript.json", {"results": [ts, resp], **client.transcript()})
    print(json.dumps({k: summary[k] for k in ("goalToolsRoundTrip", "replyOk", "allPass")}, indent=2))
finally:
    client.close()
sys.exit(0 if summary["allPass"] else 1)