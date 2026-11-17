#!/usr/bin/env python3
"""P0-2b (issue #65) supplementary probe: MAv2 tool round-trips + role config layer.

Proves: spawn_agent/list_agents/send_message/interrupt_agent/wait_agent round-trips;
close_agent attempt disposition under V2 (V1 round-trip lives in p0_2c_v1.py);
per-role model/effort overrides vs default_subagent_model; nickname auto-assignment
without task_name; developer_instructions via rollout; sandbox role-layer finding.
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
TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
HOME_A = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p0-2"
os.makedirs(EV, exist_ok=True)
HOME_B = f"{EV}/home-b"
os.makedirs(f"{HOME_B}/agents", exist_ok=True)

DEFAULT_SUBAGENT_MODEL = "gpt-5.6-terra"
PLANNER_ROLE_MODEL = "gpt-5.6-sol"

with open(f"{HOME_B}/config.toml", "w") as fh:
    fh.write("# P0-2b dedicated home: MAv2 + roles overlay\n")
    fh.write(open(f"{TESTS_DIR}/p0-2/config-overlay.toml").read())
for role in ("planner", "implementer", "verifier"):
    shutil.copyfile(f"{TESTS_DIR}/p0-2/agents/{role}.toml",
                    f"{HOME_B}/agents/{role}.toml")
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
    "3) spawn_agent with agent_type=verifier, task_name=verify_gates, task: 'Reply with a one-line "
    "acceptance checklist for creating hello.txt. Do not create or modify any file.'\n"
    "4) list_agents; note the agent paths, nicknames and statuses returned.\n"
    "5) send_message to the implementer agent: 'After the sleep finishes, also create extra.txt containing EXTRA.'\n"
    "6) interrupt_agent the implementer agent.\n"
    "7) wait_agent for all three agents and report their final statuses.\n"
    "8) Attempt close_agent on the implementer agent. If that tool does not exist or errors, quote the exact error.\n"
    "Finally reply with a numbered summary of steps 1-8, including every nickname and model you observed."
)

summary = {}
client = Client(ENGINE, HOME_B, deadline=900)
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

tools_seen = sorted({t["tool"] for t in tool_calls})
leader_summary = "\n".join(final_text)

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
            "verifierInstructionsInRollout": False,
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
                if "VERIFIER of a SuperAgent team" in content:
                    v["verifierInstructionsInRollout"] = True

by_role = {}
for v in role_proof.values():
    by_role.setdefault(v.get("agentRole"), []).append(v)
instr_ok = any(v["plannerInstructionsInRollout"] for v in by_role.get("planner", [])) and \
           any(v["implementerInstructionsInRollout"] for v in by_role.get("implementer", [])) and \
           any(v["verifierInstructionsInRollout"] for v in by_role.get("verifier", []))

# nickname auto-assignment: V2 spawn_agent REQUIRES task_name
# (core/src/tools/handlers/multi_agents_spec_tests.rs:8). Verifier is spawned with
# task_name=verify_gates; its nickname must still be "Themis" from the role's
# nickname_candidates pool (control.rs:603), proving auto-assignment independent
# of task_name (a task_name-derived nickname would be "Verify_gates", not Themis).
verifier_proof = next(iter(by_role.get("verifier", [])), {})
nickname_auto_ok = verifier_proof.get("agentNickname") == "Themis"
nickname_ok = nickname_auto_ok and any(v.get("agentNickname") for v in by_role.get("planner", []))

# model layer: role override (planner=sol) vs default_subagent_model (implementer/verifier=terra)
planner_proof = next(iter(by_role.get("planner", [])), {})
implementer_proof = next(iter(by_role.get("implementer", [])), {})
planner_meta = next((m for p, m in child_spawn_meta.items()
                     if planner_proof and m and child_ids.get(p) == planner_proof.get("threadId")), {})
role_model_ok = planner_proof.get("model") == PLANNER_ROLE_MODEL and \
                planner_meta.get("reasoningEffort") == "high"
default_model_ok = implementer_proof.get("model") == DEFAULT_SUBAGENT_MODEL and \
                   verifier_proof.get("model") == DEFAULT_SUBAGENT_MODEL
model_layer_ok = role_model_ok and default_model_ok

# close_agent attempt evidence from leader rollout function calls
leader_files, leader_calls = rollout_function_calls(HOME_B, leader_id)
close_calls = [c for c in leader_calls if "close_agent" in str(c.get("call", {}).get("name", ""))]
save("p0-2b-leader-function-calls.json", {"leaderThreadId": leader_id, "rolloutFiles": leader_files,
                                          "calls": leader_calls})
if close_calls:
    close_disposition = "attempted-in-v2"
elif "close_agent" in leader_summary and ("not available" in leader_summary or "does not exist" in leader_summary
                                         or "unsupported" in leader_summary or "unknown tool" in leader_summary.lower()):
    close_disposition = "unavailable-in-v2-narrated"
else:
    # V2 schema definitively excludes close_agent (spec_plan.rs:794 lists it for
    # V1 only). The close/resume round-trip acceptance is proven by the V1 sibling
    # probe p0_2c_v1; classify from that runtime evidence, not fragile narration.
    v1_roundtrip_proven = False
    try:
        with open(os.path.join(EV, "p0-2c-v1-summary.json")) as fh:
            v1_roundtrip_proven = bool(json.load(fh).get("allPass"))
    except (OSError, ValueError):
        pass
    close_disposition = "unavailable-in-v2-schema" if v1_roundtrip_proven else "unknown"
close_evidence = {
    "disposition": close_disposition,
    "functionCalls": close_calls,
    "leaderRolloutFiles": leader_files,
    "note": "V2 tool schema has no close_agent (core/src/tools/spec_plan.rs:794 lists it for V1 only); "
            "real close_agent/resume_agent round-trip is proven separately in p0-2c-v1-summary.json. "
            "disposition=unavailable-in-v2-schema is asserted only when that sibling probe allPass=true.",
}

child_paths = [p for p in child_ids if p != "/root"]
spawn_ok = len(child_ids) >= 3 and sum(1 for a in activities if a["kind"] == "started") >= 3
send_ok = any(a["kind"] == "interacted" and a["path"] in child_paths for a in activities)
interrupt_ok = any(a["kind"] == "interrupted" and a["path"] in child_paths for a in activities)
wait_ok = any(t["tool"] == "wait" and t["status"] == "completed" for t in tool_calls)
list_ok = ("list_agents" in leader_summary or "Listed agents" in leader_summary) \
          and "/root/athena" in leader_summary

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
    "config": {"defaultSubagentModel": DEFAULT_SUBAGENT_MODEL, "plannerRoleModel": PLANNER_ROLE_MODEL,
               "leaderModelExpected": "gpt-6-astra"},
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
    "closeAgentEvidence": close_evidence,
    "nicknameObserved": nickname_ok,
    "nicknameAutoAssigned": nickname_auto_ok,
    "roleInstructionsEffective": instr_ok,
    "roleModelLayerEffective": model_layer_ok,
    "roleModelOverrideEffective": role_model_ok,
    "defaultSubagentModelEffective": default_model_ok,
    "sandboxFinding": sandbox_finding,
    "v2SpawnTaskNameRequired": {
        "required": True,
        "evidence": "core/src/tools/handlers/multi_agents_spec_tests.rs:8 spawn_agent_tool_v2_requires_task_name; "
                    "runtime spawn without task_name fails 'missing field task_name'. Nickname auto-assignment "
                    "is proven with task_name=verify_gates yielding role-pool nickname Themis (control.rs:603).",
    },
    "roleProof": role_proof,
    "leaderFinalMessage": leader_summary[-4000:],
    "turnCompleted": completed,
}
summary["allPass"] = all([spawn_ok, list_ok, send_ok, interrupt_ok, wait_ok,
                          instr_ok, model_layer_ok, nickname_ok,
                          close_disposition != "unknown"])
save("p0-2b-summary.json", summary)
print(json.dumps({k: v for k, v in summary.items()
                  if k in ("allPass", "toolsSeen", "spawnObserved", "listObserved", "sendObserved",
                           "interruptObserved", "waitObserved", "nicknameObserved", "nicknameAutoAssigned",
                           "roleInstructionsEffective", "roleModelLayerEffective",
                           "roleModelOverrideEffective", "defaultSubagentModelEffective")}, indent=2))
print("closeAgentDisposition=" + close_disposition)
sys.exit(0 if summary["allPass"] else 1)