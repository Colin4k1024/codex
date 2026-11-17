#!/usr/bin/env python3
"""P1-2 / #74 runtime acceptance probe (quota-gated).

Proves, against the real engine + production pack overlay:
  - leader persona resident intro is injected into the leader session;
  - leader spawns all five declared roles (planner/implementer/verifier/
    reviewer/reporter) per the persona delegation pattern;
  - per-role config layer applied: model matches the overlay role model;
  - role isolation: each child rollout carries ONLY its own role instructions;
  - budget: per-child token usage measured from rollouts.

Runtime teaming is driven via app-server stdio (the same engine + config path
the TUI uses); a TUI render of the persona is a separate lighter claim. This
probe does NOT edit code; content-layer structure is proven by test_roles.py.
"""
import json
import os
import re
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spike_client import Client, rollout_function_calls

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ENGINE = os.environ.get("SUPERAGENT_ENGINE", f"{ROOT}/superagent/packaging/superagent-engine")
HOME_A = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p1-2"
os.makedirs(EV, exist_ok=True)
HOME = f"{EV}/home-p1-2"
os.makedirs(f"{HOME}/agents", exist_ok=True)

ROLES = ["planner", "implementer", "verifier", "reviewer", "reporter"]
ROLE_MODEL = {"planner": "gpt-6.1-sol", "implementer": "gpt-6-astra",
              "verifier": "gpt-5.6-luna", "reviewer": "gpt-6.1-terra",
              "reporter": "gpt-5.6-luna"}
MARKER = {r: f"{r.upper()} of a SuperAgent team" for r in ROLES}
PERSONA_MARKER = "SuperAgent meta-agent (leader), a coordinator not an executor"

with open(f"{HOME}/config.toml", "w") as fh:
    fh.write("# P1-2 dedicated home: production pack overlay\n")
    fh.write(open(f"{ROOT}/superagent/pack/config/agents-overlay.toml").read())
for r in ROLES:
    shutil.copyfile(f"{ROOT}/superagent/pack/roles/{r}.toml", f"{HOME}/agents/{r}.toml")
if os.path.exists(f"{HOME_A}/auth.json"):
    shutil.copyfile(f"{HOME_A}/auth.json", f"{HOME}/auth.json")

WORK = tempfile.mkdtemp(prefix="p12-")


def save(name, obj):
    with open(f"{EV}/{name}", "w") as fh:
        json.dump(obj, fh, ensure_ascii=False, indent=2, default=str)


def is_note(method):
    return lambda n: n.get("method") == method


PROMPT = (
    "You are the SuperAgent leader. Use the multi-agent tools to spawn ALL FIVE role agents, "
    "one spawn_agent call at a time, in this order, then wait_agent for each:\n"
    "1) spawn_agent agent_type=planner: 'Reply with a one-step plan to create hello.txt; do not create it.'\n"
    "2) spawn_agent agent_type=implementer: 'Reply with the exact diff you would apply to create hello.txt; do not write any file.'\n"
    "3) spawn_agent agent_type=verifier: 'Reply with a one-line acceptance checklist; do not run or modify anything.'\n"
    "4) spawn_agent agent_type=reviewer: 'Reply with a one-line review verdict on the plan; do not write code.'\n"
    "5) spawn_agent agent_type=reporter: 'Reply with a one-line evidence summary; introduce no new claims.'\n"
    "After all five are spawned and waited, reply with a numbered list of the five nicknames and models you observed. "
    "Do not do the work yourself; delegate to the five roles."
)

summary = {}
client = Client(ENGINE, HOME, deadline=1200)
try:
    ts = client.request("thread/start", {"cwd": WORK}, timeout=60)
    leader_id = ts["result"]["thread"]["id"]
    rid = client.send_request("turn/start", {"threadId": leader_id,
                                             "input": [{"type": "text", "text": PROMPT}]})
    client.wait_notification(is_note("turn/started"), timeout=120)

    def turn_done(n):
        return (n.get("method") == "turn/completed"
                and n.get("params", {}).get("threadId") == leader_id)
    client.wait_notification(turn_done, timeout=1100)
    resp = client.wait_response(rid, timeout=60)
    save("p1-2-transcript.json", {"results": [ts, resp], **client.transcript()})
finally:
    client.close()

notes = client.notifications
child_ids = {}
child_meta = {}
activities = []
final_text = []
for n in notes:
    if n.get("method") != "item/completed":
        continue
    it = n.get("params", {}).get("item", {})
    if it.get("type") == "subAgentActivity":
        activities.append({"path": it.get("agentPath"), "kind": it.get("kind"),
                           "threadId": it.get("agentThreadId"), "model": it.get("model")})
        if it.get("kind") == "started":
            child_ids[it.get("agentPath")] = it.get("agentThreadId")
            child_meta[it.get("agentThreadId")] = {"model": it.get("model"), "path": it.get("agentPath")}
    if it.get("type") == "agentMessage":
        final_text.append(it.get("text") or "")
leader_summary = "\n".join(final_text)

role_proof = {}
recon = Client(ENGINE, HOME, deadline=240)
try:
    for path, cid in child_ids.items():
        read = recon.request("thread/read", {"threadId": cid, "includeTurns": True}, timeout=60)
        th = read.get("result", {}).get("thread", {})
        role_proof[path] = {"threadId": cid, "sessionId": th.get("sessionId"),
                            "agentRole": th.get("agentRole"), "agentNickname": th.get("agentNickname"),
                            "model": th.get("model"), "readable": "error" not in read}
        save(f"p1-2-child-read-{path.strip('/').replace('/', '_')}.json", read)
finally:
    recon.close()

# developer_instructions + token usage live in rollout JSONL, keyed by sessionId
session_tokens = {}
session_markers = {v["sessionId"]: set() for v in role_proof.values() if v.get("sessionId")}
leader_marker_found = False
for root, _dirs, files in os.walk(HOME):
    for fn in files:
        if not fn.endswith(".jsonl"):
            continue
        try:
            content = open(os.path.join(root, fn), encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        if PERSONA_MARKER in content:
            leader_marker_found = True
        for sid, markers in session_markers.items():
            if sid and sid in content:
                for r in ROLES:
                    if MARKER[r] in content:
                        markers.add(r)
                totals = [int(x) for x in re.findall(r'"total_tokens"\s*:\s*(\d+)', content)]
                if totals:
                    session_tokens[sid] = max(totals)

for path, v in role_proof.items():
    sid = v.get("sessionId")
    markers = session_markers.get(sid, set())
    v["ownInstructionsInRollout"] = v.get("agentRole") in markers
    v["foreignInstructionsInRollout"] = sorted(markers - {v.get("agentRole")})
    v["totalTokens"] = session_tokens.get(sid)

by_role = {}
for v in role_proof.values():
    by_role.setdefault(v.get("agentRole"), []).append(v)

spawned_roles = sorted(by_role.keys())
spawn_ok = all(r in by_role for r in ROLES)
model_ok = all(any(v.get("model") == ROLE_MODEL[r] for v in by_role.get(r, [])) for r in ROLES)
isolation_ok = all(v["ownInstructionsInRollout"] and not v["foreignInstructionsInRollout"]
                   for v in role_proof.values()) and len(role_proof) >= 5
persona_ok = leader_marker_found
budget_measured = sum(1 for v in role_proof.values() if isinstance(v.get("totalTokens"), int))
budget_ok = budget_measured >= 5

summary = {
    "issue": 74,
    "leaderThreadId": leader_id,
    "spawnedRoles": spawned_roles,
    "childCount": len(role_proof),
    "personaInjected": persona_ok,
    "allFiveRolesSpawned": spawn_ok,
    "perRoleModelApplied": model_ok,
    "roleIsolation": isolation_ok,
    "budgetMeasuredChildren": budget_measured,
    "budgetMeasured": budget_ok,
    "roleProof": role_proof,
    "leaderSummary": leader_summary,
    "allPass": bool(spawn_ok and model_ok and isolation_ok and persona_ok and budget_ok),
    "note": "Runtime teaming via app-server stdio (same engine+config path as TUI). "
            "TUI persona render is a separate lighter claim; not asserted here.",
}
save("p1-2-runtime-conclusion.json", summary)
print(json.dumps({k: summary[k] for k in
                  ["allPass", "spawnedRoles", "personaInjected", "perRoleModelApplied",
                   "roleIsolation", "budgetMeasured", "childCount"]}, ensure_ascii=False))
sys.exit(0 if summary["allPass"] else 1)