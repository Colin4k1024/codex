#!/usr/bin/env python3
"""P0-4c (#67 F1): prove the MODEL invokes create_goal/get_goal/update_goal tools.

Runs a real turn instructing the model to call the goal tools, then scans the leader
rollout JSONL for function_call/function_call_output pairs (rollout_function_calls).
Pass requires >=1 call of each tool with a recorded output. No fabricated success.
"""
import json
import os
import re
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
    "Invoke each tool in its OWN separate exec call (one tool per exec), actually calling them "
    "(do not just describe them), in this exact order: "
    "1) create_goal with objective 'P04C-TOOL-PROBE'; "
    "2) get_goal and note the returned status; "
    "3) update_goal with status 'complete' (update_goal's only parameter is 'status', one of "
    "complete/blocked/paused; there is NO 'progress' field); "
    "4) get_goal again to show the new status. "
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
    # as custom_tool_call_output. The model may batch several tools into one exec, so
    # a single output can contain multiple goal-JSON snapshots. We extract every goal
    # snapshot and require the real active->complete transition, never counting an
    # error string (e.g. "missing field `status`") as a successful round-trip.
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

    def output_texts(out):
        try:
            parsed = json.loads(out)
        except (ValueError, TypeError):
            return [out] if isinstance(out, str) else []
        if isinstance(parsed, list):
            return [s.get("text", "") for s in parsed if isinstance(s, dict) and isinstance(s.get("text"), str)]
        if isinstance(parsed, dict):
            return [json.dumps(parsed, ensure_ascii=False)]
        return [str(parsed)]

    def goal_snapshots(out):
        snaps = []
        for txt in output_texts(out):
            for m in re.finditer(r'"goal"\s*:\s*\{', txt):
                start = m.end() - 1
                depth = 0
                for i in range(start, len(txt)):
                    if txt[i] == "{":
                        depth += 1
                    elif txt[i] == "}":
                        depth -= 1
                        if depth == 0:
                            try:
                                snaps.append(json.loads(txt[start:i + 1]))
                            except ValueError:
                                pass
                            break
        return snaps

    def is_error(out):
        low = out.lower()
        return ("missing field" in low) or ("error=" in low) or ('"error"' in low)

    code_mode = {tool: [] for tool in GOAL_TOOLS}
    all_snapshots, error_outputs = [], []
    for cid, inp in exec_inputs.items():
        out = exec_outputs.get(cid, "")
        snaps = goal_snapshots(out)
        all_snapshots.extend(snaps)
        if is_error(out):
            error_outputs.append({"call_id": cid, "output": out[:500]})
        for tool in GOAL_TOOLS:
            if f"tools.{tool}" in inp:
                code_mode[tool].append({"call_id": cid, "input": inp, "output": out,
                                        "goalSnapshots": snaps, "hasError": is_error(out)})

    # update_goal must be called with its real 'status' parameter (not 'progress').
    update_inputs = [c["input"] for c in code_mode["update_goal"]]
    update_uses_status = any(("status" in s) and ("progress" not in s) for s in update_inputs)
    statuses = [s.get("status") for s in all_snapshots if isinstance(s, dict)]
    # Corroborating server notifications: thread/goal/updated carrying the new status.
    goal_updated_notifs = [n.get("params", {}) for n in client.notifications
                           if n.get("method") == "thread/goal/updated"]
    notif_statuses = []
    for p in goal_updated_notifs:
        g = p.get("goal") or p
        if isinstance(g, dict) and g.get("status"):
            notif_statuses.append(g["status"])
    complete_observed = ("complete" in statuses) or ("complete" in notif_statuses)
    created_observed = any(isinstance(s, dict) and s.get("objective") == "P04C-TOOL-PROBE" for s in all_snapshots)

    per_tool = {}
    for tool in GOAL_TOOLS:
        matched = [e for e in goal_calls if e["call"]["name"] == tool]
        cm = code_mode[tool]
        cm_valid = [c for c in cm if c["goalSnapshots"] and not c["hasError"]]
        if tool == "create_goal" and goal_objective_outs and not cm_valid:
            cm_valid = cm
        per_tool[tool] = {
            "nativeCallCount": len(matched),
            "codeModeCallCount": len(cm),
            "validOutputCount": len(cm_valid),
            "samples": matched[:3],
            "codeModeCalls": [{k: c[k] for k in ("call_id", "input", "goalSnapshots", "hasError")} for c in cm[:3]],
        }
    create_ok = (per_tool["create_goal"]["nativeCallCount"] + per_tool["create_goal"]["codeModeCallCount"] >= 1) and created_observed
    get_ok = (per_tool["get_goal"]["nativeCallCount"] + per_tool["get_goal"]["codeModeCallCount"] >= 1) and bool(all_snapshots)
    update_ok = (per_tool["update_goal"]["codeModeCallCount"] + per_tool["update_goal"]["nativeCallCount"] >= 1) \
        and update_uses_status and complete_observed and not error_outputs
    texts = [n.get("params", {}).get("item", {}).get("text", "")
             for n in client.notifications
             if n.get("method") == "item/completed"
             and n.get("params", {}).get("item", {}).get("type") == "agentMessage"]
    summary = {
        "threadId": thread_id,
        "rolloutFiles": files,
        "perTool": per_tool,
        "goalStatusesObserved": statuses,
        "notificationStatuses": notif_statuses,
        "updateGoalUsesStatusParam": update_uses_status,
        "createGoalProven": create_ok,
        "getGoalProven": get_ok,
        "updateGoalProven": update_ok,
        "errorOutputs": error_outputs,
        "goalToolsRoundTrip": create_ok and get_ok and update_ok,
        "replyOk": any("GOAL-TOOLS-OK" in t for t in texts),
        "turnStartResponse": resp,
    }
    summary["allPass"] = summary["goalToolsRoundTrip"] and summary["replyOk"]
    save("goal-tool-calls.json", {"prompt": PROMPT, "goalCalls": goal_calls,
                                  "allFunctionCallCount": len(calls),
                                  "goalSnapshots": all_snapshots,
                                  "goalUpdatedNotifications": goal_updated_notifs})
    save("p0-4c-summary.json", summary)
    save("p0-4c-transcript.json", {"results": [ts, resp], **client.transcript()})
    print(json.dumps({k: summary[k] for k in ("createGoalProven", "getGoalProven", "updateGoalProven",
                                              "goalToolsRoundTrip", "replyOk", "allPass")}, indent=2))
finally:
    client.close()
sys.exit(0 if summary["allPass"] else 1)