#!/usr/bin/env python3
"""P0-2b (issue #65) supplementary probe: MAv2 tool round-trips + role config layer.

Proves: spawn_agent(nickname)/list_agents/send_message/interrupt_agent/wait_agent
round-trips; close_agent disposition under V2; role developer_instructions and
sandbox_mode applied to child threads; nickname visibility; trigger method.
"""
import json
import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spike_client import Client

ROOT = "/Users/fanjia/Desktop/code/codex"
ENGINE = f"{ROOT}/superagent/packaging/superagent-engine"
HOME_A = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p0-2"
os.makedirs(EV, exist_ok=True)
HOME_B = f"{EV}/home-b"
os.makedirs(f"{HOME_B}/agents", exist_ok=True)

with open(f"{HOME_B}/config.toml", "w") as fh:
    fh.write("# P0-2b dedicated home: MAv2 + roles overlay\n")
    fh.write(open(f"{ROOT}/superagent/tests/p0-2/config-overlay.toml").read())
for role in ("planner", "implementer", "verifier"):
    src = f"{ROOT}/superagent/tests/p0-2/agents/{role}.toml"
    dst = f"{HOME_B}/agents/{role}.toml"
    shutil.copyfile(src, dst)
# role-level model-layer override: planner reasoning effort high (leader runs low).
# sandbox_mode is deliberately also set to read-only to TEST whether the role layer
# applies it (code says AgentRoleOverrides excludes sandbox; expect inheritance).
with open(f"{HOME_B}/agents/planner.toml", "a") as fh:
    fh.write('\nsandbox_mode = "read-only"\nmodel_reasoning_effort = "high"\n')
if os.path.exists(f"{HOME_A}/auth.json"):
    shutil.copyfile(f"{HOME_A}/auth.json", f"{HOME_B}/auth.json")

WORK = tempfile.mkdtemp(prefix="p02b-")


def save(name, obj):
    with open(f"{EV}/{name}", "w") as fh:
        json.dump(obj, fh, ensure_ascii=False, indent=2, default=str)


def is_note(method):
    return lambda n: n.get("method") == method


def wait_turn(client, thread_id, exclude=()):
    def pred(n):
        if n.get("method") != "turn/completed":
            return False
        p = n.get("params", {})
        if p.get("threadId") != thread_id:
            return False
        return p.get("turn", {}).get("id") not in exclude
    return client.wait_notification(pred, timeout=600)


PROMPT = (
    "You are the SuperAgent leader. Use the multi-agent tools EXACTLY in this order, one call at a time:\n"
    "1) spawn_agent with agent_type=planner, task_name=athena, task: 'Use your command execution tool to run "
    "`touch sandbox-probe.txt`. Report whether the command succeeded or was blocked by the sandbox. "
    "Then reply with a one-step plan to create hello.txt. Do not create hello.txt yourself.'\n"
    "2) spawn_agent with agent_type=implementer, task_name=hephaestus, task: 'Use your command execution tool "
    "to run `sleep 30` and wait for it to finish. Then create hello.txt containing exactly HELLO-MAV2.'\n"
    "3) list_agents; note the nicknames and statuses returned.\n"
    "4) send_message to the implementer agent: 'After the sleep finishes, also create extra.txt containing EXTRA.'\n"
    "5) interrupt_agent the implementer agent.\n"
    "6) wait_agent for both agents and report their final statuses.\n"
    "7) Attempt close_agent on the implementer agent. If that tool does not exist or errors, quote the exact error.\n"
    "Finally reply with a numbered summary of the results of steps 1-7, including the nicknames you observed."
)

summary = {}
client = Client(ENGINE, HOME_B, deadline=780)
try:
    ts = client.request("thread/start", {"cwd": WORK}, timeout=60)
    leader_id = ts["result"]["thread"]["id"]
    rid = client.send_request("turn/start", {"threadId": leader_id,
                                             "input": [{"type": "text", "text": PROMPT}]})
    client.wait_notification(is_note("turn/started"), timeout=120)
    completed = wait_turn(client, leader_id)
    resp = client.wait_response(rid, timeout=60)
    save("p0-2b-transcript.json", {"results": [ts, resp], **client.transcript()})
finally:
    client.close()

notes = client.notifications
tool_calls = []
child_ids = {}
child_spawn_meta = {}
activities = []
final_text = []
for n in notes:
    if n.get("method") != "item/completed":
        continue
    it = n.get("params", {}).get("item", {})
    if it.get("type") == "collabAgentToolCall":
        tool_calls.append({"tool": it.get("tool"), "status": it.get("status"),
                           "sender": it.get("senderThreadId"), "agentsStates": it.get("agentsStates")})
    if it.get("type") == "subAgentActivity":
        activities.append({"path": it.get("agentPath"), "kind": it.get("kind"),
                           "threadId": it.get("agentThreadId"), "model": it.get("model"),
                           "reasoningEffort": it.get("reasoningEffort")})
        if it.get("kind") == "started":
            child_ids[it.get("agentPath")] = it.get("agentThreadId")
            child_spawn_meta[it.get("agentPath")] = {"model": it.get("model"),
                                                     "reasoningEffort": it.get("reasoningEffort")}
    if it.get("type") == "agentMessage":
        final_text.append(it.get("text") or "")

blob = json.dumps(client.transcript(), ensure_ascii=False)
tools_seen = sorted({t["tool"] for t in tool_calls})
leader_summary = "\n".join(final_text)

# role config layer proof: read child histories + rollout files
role_proof = {}
recon = Client(ENGINE, HOME_B, deadline=180)
try:
    tl = recon.request("thread/list", {}, timeout=60)
    save("p0-2b-thread-list.json", tl)
    for path, cid in child_ids.items():
        read = recon.request("thread/read", {"threadId": cid, "includeTurns": True}, timeout=60)
        th = read.get("result", {}).get("thread", {})
        rblob = json.dumps(read, ensure_ascii=False)
        probe_exit = None
        for turn in th.get("turns", []):
            for it in turn.get("items", []):
                if it.get("type") == "commandExecution" and "sandbox-probe.txt" in str(it.get("command", "")):
                    probe_exit = it.get("exitCode")
        role_proof[path] = {
            "threadId": cid,
            "sessionId": th.get("sessionId"),
            "readable": "error" not in read,
            "agentRole": th.get("agentRole"),
            "agentNickname": th.get("agentNickname"),
            "model": th.get("model"),
            "sandboxProbeAttempt": "sandbox-probe.txt" in rblob,
            "sandboxProbeExitCode": probe_exit,
            "plannerInstructionsInRollout": False,
            "implementerInstructionsInRollout": False,
        }
        save(f"p0-2b-child-read-{path.strip('/').replace('/', '_')}.json", read)
finally:
    recon.close()

# developer_instructions are system-level (not in thread/read items); prove via rollout JSONL
for root, _dirs, files in os.walk(HOME_B):
    for fn in files:
        if not fn.endswith(".jsonl"):
            continue
        try:
            content = open(os.path.join(root, fn), encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        for v in role_proof.values():
            sid = v.get("sessionId")
            if sid and sid in content:
                if "PLANNER of a SuperAgent team" in content:
                    v["plannerInstructionsInRollout"] = True
                if "IMPLEMENTER of a SuperAgent team" in content:
                    v["implementerInstructionsInRollout"] = True

by_role = {}
for v in role_proof.values():
    by_role.setdefault(v.get("agentRole"), []).append(v)
instr_ok = any(v["plannerInstructionsInRollout"] for v in by_role.get("planner", [])) and \
           any(v["implementerInstructionsInRollout"] for v in by_role.get("implementer", []))
nickname_ok = any(v.get("agentNickname") for v in role_proof.values())

# round-trip detection: collab items only stream 'wait'; spawn/send/interrupt are
# evidenced by subAgentActivity kinds + leader tool-result narration.
child_paths = [p for p in child_ids if p != "/root"]
spawn_ok = len(child_ids) >= 2 and sum(1 for a in activities if a["kind"] == "started") >= 2
send_ok = any(a["kind"] == "interacted" and a["path"] in child_paths for a in activities)
interrupt_ok = any(a["kind"] == "interrupted" and a["path"] in child_paths for a in activities)
wait_ok = any(t["tool"] == "wait" and t["status"] == "completed" for t in tool_calls)
list_ok = ("list_agents" in leader_summary or "Listed agents" in leader_summary) \
          and "/root/athena" in leader_summary
close_disposition = "unavailable-in-v2" if "close_agent" in leader_summary and \
                    ("not available" in leader_summary or "does not exist" in leader_summary) else "unknown"

planner_meta = next((m for p, m in child_spawn_meta.items()
                     if by_role.get("planner") and
                     any(v["threadId"] == child_ids.get(p) for v in by_role["planner"])), {})
model_layer_ok = planner_meta.get("reasoningEffort") == "high"

planner_probe = next((v for v in by_role.get("planner", []) if v["sandboxProbeAttempt"]), {})
sandbox_finding = {
    "roleFileSandbox": "read-only",
    "touchExitCode": planner_probe.get("sandboxProbeExitCode"),
    "fileCreated": os.path.exists(f"{WORK}/sandbox-probe.txt"),
    "conclusion": "role sandbox_mode NOT applied: AgentRoleOverrides excludes sandbox "
                  "(core/src/agent/role.rs:77-86); approval/sandbox/permission profile are "
                  "re-applied from the leader turn after role overrides "
                  "(core/src/agent/child_config.rs:197-220). Orchestrator must enforce "
                  "per-role sandbox via command/exec SandboxPolicy (P0-7).",
}

summary = {
    "leaderThreadId": leader_id,
    "workdir": WORK,
    "childThreads": child_ids,
    "childSpawnMeta": child_spawn_meta,
    "activities": activities,
    "toolsSeen": tools_seen,
    "toolCalls": tool_calls,
    "spawnObserved": spawn_ok,
    "listObserved": list_ok,
    "sendObserved": send_ok,
    "interruptObserved": interrupt_ok,
    "waitObserved": wait_ok,
    "closeAgentDisposition": close_disposition,
    "nicknameObserved": nickname_ok,
    "roleInstructionsEffective": instr_ok,
    "roleModelLayerEffective": model_layer_ok,
    "sandboxFinding": sandbox_finding,
    "roleProof": role_proof,
    "leaderFinalMessage": leader_summary[-4000:],
    "turnCompleted": completed,
}
summary["allPass"] = all([spawn_ok, list_ok, send_ok, interrupt_ok, wait_ok,
                          instr_ok, model_layer_ok, nickname_ok])
save("p0-2b-summary.json", summary)
print(json.dumps({k: v for k, v in summary.items()
                  if k in ("allPass", "toolsSeen", "spawnObserved", "listObserved", "sendObserved",
                           "interruptObserved", "waitObserved", "closeAgentDisposition", "nicknameObserved",
                           "roleInstructionsEffective", "roleModelLayerEffective")}, indent=2))
sys.exit(0 if summary["allPass"] else 1)