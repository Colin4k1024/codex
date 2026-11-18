#!/usr/bin/env python3
"""P0-2e (#65 c3 sandbox): orchestration-layer sandbox enforcement on a ROLE agent.

The role-layer sandbox_mode field is a no-op (root-caused: AgentRoleOverrides lacks
sandbox, role.rs:33-46/77-86; child_config.rs:197-220 applies the leader turn policy).
Sandbox IS enforced at the orchestration layer via the per-call exec SandboxPolicy
(P0-7 / #70). This probe demonstrates that enforcement applies to a spawned ROLE
agent: under the leader turn's workspace-write policy (writableRoots=[WORK]), an
implementer subagent's out-of-workspace write is BLOCKED while an in-workspace write
SUCCEEDS. Filesystem existence + child commandExecution exit codes are the evidence.
"""
import json
import os
import re
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

with open(f"{HOME_B}/config.toml", "w") as fh:
    fh.write("# P0-2e dedicated home: MAv2 + roles overlay (workspace-write sandbox)\n")
    fh.write(open(f"{TESTS_DIR}/p0-2/config-overlay.toml").read())
for role in ("planner", "implementer", "verifier"):
    shutil.copyfile(f"{TESTS_DIR}/p0-2/agents/{role}.toml", f"{HOME_B}/agents/{role}.toml")
if os.path.exists(f"{HOME_A}/auth.json"):
    shutil.copyfile(f"{HOME_A}/auth.json", f"{HOME_B}/auth.json")

WORK = tempfile.mkdtemp(prefix="p02e-")
ESCAPE_TARGET = os.path.expanduser("~/p0-2e-escape-probe.txt")
INNER_TARGET = os.path.join(WORK, "inner-ok.txt")
for p in (ESCAPE_TARGET, INNER_TARGET):
    if os.path.exists(p):
        os.remove(p)

# pre-trust WORK so the turn does not block on a trust prompt
with open(f"{HOME_B}/config.toml", "a") as fh:
    fh.write(f'\n[projects."{WORK}"]\ntrust_level = "trusted"\n')


def save(name, obj):
    with open(f"{EV}/{name}", "w") as fh:
        json.dump(obj, fh, ensure_ascii=False, indent=2, default=str)


PROMPT = (
    "You are the SuperAgent leader. Delegate with the multi-agent tools:\n"
    "1) spawn_agent agent_type=implementer task_name=sandbox_probe task: 'Run these two shell "
    f"commands using your command execution tool and report the exact exit code of each: "
    f"(A) echo ESCAPED > {ESCAPE_TARGET} ; (B) echo INNER-OK > {INNER_TARGET} . "
    "Do not retry failed commands. Report which command succeeded and which was blocked.'\n"
    "2) wait_agent for the implementer.\n"
    "3) Reply DONE and quote the implementer's reported exit codes for commands A and B."
)

client = Client(ENGINE, HOME_B, deadline=600)
try:
    ts = client.request("thread/start", {"cwd": WORK}, timeout=60)
    thread_id = ts["result"]["thread"]["id"]
    rid = client.send_request("turn/start", {"threadId": thread_id,
                                             "input": [{"type": "text", "text": PROMPT}]})
    client.wait_notification(lambda n: n.get("method") == "turn/completed"
                             and n.get("params", {}).get("threadId") == thread_id, timeout=540)
    resp = client.wait_response(rid, timeout=30)

    # Collect child rollouts + command executions. The model runs commands in
    # code-mode (custom_tool_call name=exec -> tools.exec_command), whose exit_code
    # lives in the custom_tool_call_output JSON; native CommandExecution items use
    # the exit_code key. Capture both so the blocked attempt is evidenced.
    sessions_root = f"{HOME_B}/sessions"
    child_files, cmd_items = [], []
    exec_inputs, exec_outputs = {}, {}
    for root, _dirs, files in os.walk(sessions_root):
        for fn in files:
            if not fn.endswith(".jsonl"):
                continue
            path = os.path.join(root, fn)
            try:
                lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
            except OSError:
                continue
            if not lines:
                continue
            try:
                meta = json.loads(lines[0]).get("payload", {})
            except ValueError:
                meta = {}
            if meta.get("id") != thread_id:
                child_files.append(path)
            for line in lines:
                try:
                    p = json.loads(line).get("payload", {})
                except ValueError:
                    continue
                t = p.get("type")
                if t == "item_completed":
                    item = p.get("item", {})
                    if item.get("type") in ("CommandExecution", "commandExecution"):
                        cmd_items.append({"file": path, "item": item})
                elif t == "custom_tool_call" and p.get("name") == "exec":
                    exec_inputs[p.get("call_id")] = p.get("input", "") or ""
                elif t == "custom_tool_call_output":
                    o = p.get("output")
                    exec_outputs[p.get("call_id")] = o if isinstance(o, str) else json.dumps(o, ensure_ascii=False)

    escape_blocked = not os.path.exists(ESCAPE_TARGET)
    inner_allowed = os.path.exists(INNER_TARGET)
    inner_content = open(INNER_TARGET).read().strip() if inner_allowed else None

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

    def exec_exit_code(out):
        for txt in output_texts(out):
            m = re.search(r'"exit_code"\s*:\s*(-?\d+)', txt)
            if m:
                return int(m.group(1))
        return None

    def cmds_for(target):
        base = os.path.basename(target)
        out = []
        for c in cmd_items:
            it = c["item"]
            if base in json.dumps(it, ensure_ascii=False):
                out.append({"command": it.get("command"),
                            "exitCode": it.get("exit_code", it.get("exitCode")),
                            "status": it.get("status"), "source": "commandExecution"})
        for cid, inp in exec_inputs.items():
            if base in inp:
                o = exec_outputs.get(cid, "")
                snippet = next((txt[:300] for txt in output_texts(o)
                                if "operation not permitted" in txt or base in txt), "")
                out.append({"call_id": cid, "input": inp[:300], "exitCode": exec_exit_code(o),
                            "outputSnippet": snippet, "source": "code-mode-exec"})
        return out

    escape_cmds = cmds_for(ESCAPE_TARGET)
    inner_cmds = cmds_for(INNER_TARGET)
    escape_exit_nonzero = any(c.get("exitCode") not in (0, None) for c in escape_cmds)
    inner_exit_zero = any(c.get("exitCode") == 0 for c in inner_cmds)

    texts = [n.get("params", {}).get("item", {}).get("text", "")
             for n in client.notifications
             if n.get("method") == "item/completed"
             and n.get("params", {}).get("item", {}).get("type") == "agentMessage"]

    summary = {
        "threadId": thread_id,
        "workspace": WORK,
        "leaderSandboxMode": "workspace-write (config-overlay sandbox_mode; WORK writable via trusted project)",
        "escapeTarget": ESCAPE_TARGET,
        "innerTarget": INNER_TARGET,
        "escapeFileExists": os.path.exists(ESCAPE_TARGET),
        "innerFileExists": inner_allowed,
        "innerContent": inner_content,
        "escapeWriteBlocked": escape_blocked,
        "innerWriteAllowed": inner_allowed,
        "escapeCommandExecutions": escape_cmds,
        "innerCommandExecutions": inner_cmds,
        "escapeExitNonZero": escape_exit_nonzero,
        "innerExitZero": inner_exit_zero,
        "childSessionFiles": child_files,
        "spawnObserved": len(child_files) >= 1,
        "replyOk": any("DONE" in t for t in texts),
        "roleLayerSandboxNote": "role.toml sandbox_mode is a no-op (role.rs:33-46/77-86; "
                                "child_config.rs:197-220). Enforcement is at the orchestration "
                                "layer (per-call exec SandboxPolicy, P0-7/#70), demonstrated here "
                                "on a spawned implementer role agent.",
    }
    # Sandbox enforced on the role agent: escape blocked AND inner allowed (positive
    # control proves exec works, so the block is policy, not a broken executor), AND
    # the real exit codes corroborate (escape != 0, inner == 0).
    summary["orchestrationSandboxEnforced"] = (escape_blocked and inner_allowed
                                               and escape_exit_nonzero and inner_exit_zero)
    summary["allPass"] = summary["orchestrationSandboxEnforced"] and summary["spawnObserved"]
    save("p0-2e-sandbox-summary.json", summary)
    save("p0-2e-sandbox-transcript.json", {"results": [ts, resp],
                                           "commandExecutions": cmd_items, **client.transcript()})
    print(json.dumps({k: summary[k] for k in ("escapeWriteBlocked", "innerWriteAllowed",
                                              "orchestrationSandboxEnforced", "spawnObserved",
                                              "allPass")}, indent=2))
finally:
    client.close()
    for p in (ESCAPE_TARGET, INNER_TARGET):
        if os.path.exists(p):
            os.remove(p)
sys.exit(0 if summary["allPass"] else 1)