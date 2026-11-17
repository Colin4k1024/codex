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
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p0-4"
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
    # Code-mode goal invocations: the model calls tools.create_goal / tools.get_goal
    # / tools.update_goal inside exec custom_tool_call input strings; outputs arrive
    # as custom_tool_call_output (goal JSON) and, for create_goal, a
    # function_call_output named goal_objective. Detect these alongside top-level
    # function_call entries (the engine exposes goal tools via the code-mode bridge).
    exec_inputs, exec_outputs, goal_objective_outs = {}, {}, []
    for fp in files:
        try:
            fh = open(fp)
        except OSError:
            continue
        with fh:
            for line in fh:
                try:
                    p = json.loads(line).get("payload", {})
                except ValueError:
                    continue
                t = p.get("type")
                if t == "custom_tool_call" and p.get("name") == "exec":
                    exec_inputs[p.get("call_id")] = p.get("input", "") or ""
                elif t == "custom_tool_call_output":
                    o = p.get("output")
                    exec_outputs[p.get("call_id")] = o if isinstance(o, str) else json.dumps(o, ensure_ascii=False)
                elif t == "function_call_output" and p.get("name") == "goal_objective":
                    goal_objective_outs.append(p.get("output"))
    code_mode = {tool: [] for tool in GOAL_TOOLS}
    for cid, inp in exec_inputs.items():
        for tool in GOAL_TOOLS:
            if f"tools.{tool}" in inp:
                code_mode[tool].append({"call_id": cid, "input": inp,
                                        "output": exec_outputs.get(cid, "")})
    per_tool = {}
    for tool in GOAL_TOOLS:
        matched = [e for e in goal_calls if e["call"]["name"] == tool]
        cm = code_mode[tool]
        cm_with_out = [c for c in cm if c["output"]]
        if tool == "create_goal" and goal_objective_outs:
            cm_with_out = cm  # goal_objective output confirms the create_goal result
        per_tool[tool] = {
            "callCount": len(matched) + len(cm),
            "withOutput": sum(1 for e in matched if e.get("output")) + len(cm_with_out),
            "samples": matched[:3],
            "codeModeCalls": cm[:3],
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