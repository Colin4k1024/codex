#!/usr/bin/env python3
"""P0-8 (issue #71): subagent observability verdict experiment.

Analyzes the P0-2 team turn transcript for SubAgentActivity / CollabAgentToolCall items,
then probes public RPCs (thread/list, thread/read, thread/items/list) against observed
agent thread ids. Outputs verdict.json + event samples; decides whether fork change F-1
is triggered. If the P0-2 transcript is missing or has no subagent items, runs a fresh
team turn first.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spike_client import Client

ROOT = "/Users/fanjia/Desktop/code/codex"
ENGINE = f"{ROOT}/superagent/packaging/superagent-engine"
HOME_A = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p0-8"
os.makedirs(EV, exist_ok=True)
P02_TRANSCRIPT = f"{ROOT}/superagent/evidence/p0-2/team-turn-transcript.json"


def save(name, obj):
    with open(f"{EV}/{name}", "w") as fh:
        json.dump(obj, fh, ensure_ascii=False, indent=2, default=str)


def item_of(note):
    return note.get("params", {}).get("item", {})


def is_subagent_item(item):
    t = str(item.get("type", "")).lower()
    return "subagent" in t or "collab" in t


def collect(notifications):
    items, agent_threads = [], set()
    for note in notifications:
        if note.get("method") not in ("item/started", "item/completed"):
            continue
        item = item_of(note)
        if is_subagent_item(item):
            items.append({"notification": note["method"], "item": item})
            for key in ("agentThreadId", "agent_thread_id"):
                if item.get(key):
                    agent_threads.add(item[key])
            for state in (item.get("agentsStates") or {}).values():
                if isinstance(state, dict) and state.get("threadId"):
                    agent_threads.add(state["threadId"])
    return items, sorted(agent_threads)


def run_fresh_team_turn():
    fix = f"{ROOT}/superagent/tests/fixtures/p0-2-task"
    work = tempfile.mkdtemp(prefix="p08-work-") + "/task"
    shutil.copytree(fix, work)
    subprocess.run(["git", "init", "-q"], cwd=work, check=True)
    subprocess.run(["git", "add", "-A"], cwd=work, check=True)
    subprocess.run(["git", "-c", "user.email=spike@local", "-c", "user.name=spike",
                    "commit", "-qm", "baseline"], cwd=work, check=True)
    prompt = (
        "You are the SuperAgent leader. The tests in test_calc.py fail because calc.add is wrong. "
        "Delegate with the multi-agent tools: 1) spawn_agent with agent_type=planner to produce a "
        "one-step plan whose acceptance command is 'python3 -m unittest test_calc -v'; wait_agent. "
        "2) spawn_agent with agent_type=implementer to fix calc.py per the plan; wait_agent. "
        "3) spawn_agent with agent_type=verifier to run the acceptance command and report the real "
        "exit code; wait_agent. Then report the final result. Do not fix the code yourself; delegate."
    )
    client = Client(ENGINE, HOME_A, deadline=900)
    try:
        ts = client.request("thread/start", {"cwd": work}, timeout=60)
        thread_id = ts["result"]["thread"]["id"]
        rid = client.send_request("turn/start", {"threadId": thread_id,
                                                 "input": [{"type": "text", "text": prompt}]})
        client.wait_notification(lambda n: n.get("method") == "turn/completed", timeout=840)
        resp = client.wait_response(rid, timeout=30)
        save("fresh-team-turn.json", {"threadId": thread_id, "results": [ts, resp], **client.transcript()})
        return client.notifications, thread_id
    finally:
        client.close()


notifications = []
leader_thread = None
source = "p0-2-transcript"
if os.path.exists(P02_TRANSCRIPT):
    data = json.load(open(P02_TRANSCRIPT))
    notifications = data.get("notifications", [])
    results = data.get("results", [])
    if results:
        leader_thread = results[0].get("response", {}).get("result", {}).get("thread", {}).get("id")
items, agent_threads = collect(notifications)
if not items:
    source = "fresh-team-turn"
    notifications, leader_thread = run_fresh_team_turn()
    items, agent_threads = collect(notifications)

kinds = {}
for entry in items:
    item = entry["item"]
    kind = item.get("kind") or item.get("tool") or item.get("type")
    kinds[str(kind)] = kinds.get(str(kind), 0) + 1

lifecycle = {
    "spawnObserved": any(str(i["item"].get("kind", "")).lower() == "started"
                         or "spawn" in str(i["item"].get("tool", "")).lower() for i in items),
    "activityObserved": any(str(i["item"].get("kind", "")).lower() == "interacted" for i in items)
                        or len(items) > 1,
    "terminalObserved": any(str(i["item"].get("kind", "")).lower() in ("completed", "interrupted")
                            for i in items),
}

# Probe public control/observation surface against observed agent threads.
probes = {}
client = Client(ENGINE, HOME_A, deadline=240)
try:
    tl = client.request("thread/list", {}, timeout=60)
    listed = tl.get("result", {}).get("threads", []) or tl.get("result", {}).get("items", [])
    listed_ids = [t.get("id") for t in listed if isinstance(t, dict)]
    probes["thread/list"] = {"response": tl, "agentThreadsListed": [a for a in agent_threads if a in listed_ids]}
    for agent in agent_threads[:4]:
        read = client.request("thread/read", {"threadId": agent, "includeTurns": True}, timeout=60)
        probes[f"thread/read:{agent}"] = {
            "readable": "error" not in read,
            "response": read,
        }
    if leader_thread:
        il = client.request("thread/items/list", {"threadId": leader_thread}, timeout=60)
        probes["thread/items/list:leader"] = {"response": il}
finally:
    client.close()

child_history_readable = any(v.get("readable") for k, v in probes.items() if k.startswith("thread/read:"))
sufficient = all(lifecycle.values()) and child_history_readable

verdict = {
    "issue": 71,
    "source": source,
    "leaderThreadId": leader_thread,
    "subAgentItemCount": len(items),
    "itemKinds": kinds,
    "agentThreadIds": agent_threads,
    "lifecycle": lifecycle,
    "childHistoryReadableViaPublicRpc": child_history_readable,
    "publicAgentControlRpcs": [],
    "verdict": "SUFFICIENT_NO_FORK" if sufficient else "GAP_TRIGGERS_F1",
    "controlPlaneNotes": [
        "No public agent/* RPC exists in the generated ClientRequest schema (verified 2026-11-17).",
        "Per-subagent interrupt/resume from an external client is not directly exposed; outer orchestrator cancels the whole leader turn via turn/interrupt (P0-6 trace C) or asks the leader to use interrupt_agent.",
        "Subagent lifecycle is observable via item/started + item/completed SubAgentActivity items (kind started/interacted/interrupted/completed) and CollabAgentToolCall items with agentsStates.",
        "Child thread history readable via thread/read when agentThreadId is known (see probes).",
    ],
    "minimalRpcProposalIfGap": [
        "agent/list {sessionId} -> [{agentThreadId, role, status}]",
        "agent/interrupt {agentThreadId}",
        "agent/read {agentThreadId} (already covered by thread/read if ids are exposed)",
    ] if not sufficient else [],
}
save("event-samples.json", {"items": items[:60], "probes": probes})
save("verdict.json", verdict)
print(json.dumps({k: verdict[k] for k in ("source", "subAgentItemCount", "itemKinds", "lifecycle",
                                           "childHistoryReadableViaPublicRpc", "verdict")}, indent=2))
sys.exit(0 if sufficient else 2)