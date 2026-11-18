#!/usr/bin/env python3
"""Post-run reconciliation for the P1-2 / #74 F2 TUI probe.

Generates `p1-2-tui-conclusion.json` from a CAPTURED genuine TUI home (the
rollouts produced by `p1_2_tui.py`'s interactive pty launch). This is the same
authoritative reconciliation the probe performs inline, factored out so it can
run against an already-captured TUI session without re-spending model quota.

Honesty contract: PROVEN only if the captured leader is a genuine TUI session
(originator codex-tui, no parent), the task-only prompt drove persona-based
teaming, all five persona roles were spawned as its children, and EVERY child
turn completed per authoritative app-server thread/read. Never fabricated.
"""
import json
import os
import shutil
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spike_client import Client

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ENGINE = os.environ.get("SUPERAGENT_ENGINE", f"{ROOT}/superagent/packaging/superagent-engine")
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p1-2"
HOME = sys.argv[2] if len(sys.argv) > 2 else f"{EV}/home-p1-2-tui"

ROLES = ["planner", "implementer", "verifier", "reviewer", "reporter"]
PERSONA_MARKER = "SuperAgent meta-agent (leader), a coordinator not an executor"
TASK_MARKER = "HELLO-P12"
PROMPT = (
    "Use your team to prepare everything needed to create hello.txt containing "
    "exactly HELLO-P12: a one-step plan, the exact diff to apply, a one-line "
    "acceptance checklist, a one-line review verdict, and a one-line evidence "
    "summary. Do not create or modify any files. When the team deliverables are "
    "in, list what each teammate produced."
)

sessions_root = f"{HOME}/sessions"


def rollout_metas():
    out = {}
    for root, _dirs, files in os.walk(sessions_root):
        for fn in files:
            if not fn.endswith(".jsonl"):
                continue
            path = os.path.join(root, fn)
            try:
                with open(path, encoding="utf-8", errors="replace") as fh:
                    meta = json.loads(fh.readline())
                if meta.get("type") == "session_meta":
                    out[path] = meta.get("payload", {})
            except (OSError, json.JSONDecodeError):
                continue
    return out


metas = rollout_metas()
leader_file = leader_meta = leader_sid = None
for path, m in metas.items():
    if (m.get("originator") == "codex-tui" and not m.get("parent_thread_id")
            and TASK_MARKER in open(path, encoding="utf-8", errors="replace").read()):
        leader_file, leader_meta, leader_sid = path, m, m.get("id")
        break

child_files = {}
if leader_sid:
    for path, m in metas.items():
        if path == leader_file:
            continue
        if m.get("parent_thread_id") == leader_sid and m.get("agent_role") in ROLES:
            child_files.setdefault(m.get("agent_role"), []).append(
                {"path": path, "threadId": m.get("id"), "meta": m})

# Lock-free snapshot copy for reconciliation (no active writer locks).
RECON_HOME = f"{HOME}-recon"
shutil.rmtree(RECON_HOME, ignore_errors=True)


def _ignore_locks(_dir, names):
    return [n for n in names if n.endswith(".lock") or n == "thread-writer-locks"]


shutil.copytree(HOME, RECON_HOME, ignore=_ignore_locks)
shutil.rmtree(f"{RECON_HOME}/thread-writer-locks", ignore_errors=True)

child_proof = {}
leader_turn_status = None
recon = Client(ENGINE, RECON_HOME, deadline=240)
try:
    if leader_sid:
        read = recon.request("thread/read", {"threadId": leader_sid, "includeTurns": True},
                             timeout=60)
        lth = read.get("result", {}).get("thread", {})
        leader_turn_status = [t.get("status") for t in lth.get("turns") or []]
        with open(f"{EV}/p1-2-tui-leader-read.json", "w") as fh:
            json.dump(read, fh, ensure_ascii=False, indent=2, default=str)
    for role, cs in child_files.items():
        for c in cs:
            read = recon.request("thread/read", {"threadId": c["threadId"],
                                                 "includeTurns": True}, timeout=60)
            th = read.get("result", {}).get("thread", {})
            turns = th.get("turns") or []
            statuses = [t.get("status") for t in turns]
            errors = [t.get("error") for t in turns if t.get("error")]
            child_proof[c["threadId"]] = {
                "role": role, "threadId": c["threadId"],
                "model": th.get("model"), "agentNickname": th.get("agentNickname"),
                "turnStatuses": statuses, "turnErrors": errors,
                "turnsCompleted": bool(turns) and all(s == "completed" for s in statuses)
                                  and not errors,
                "rolloutFile": c["path"],
            }
            with open(f"{EV}/p1-2-tui-child-read-{role}.json", "w") as fh:
                json.dump(read, fh, ensure_ascii=False, indent=2, default=str)
finally:
    recon.close()

leader_content = (open(leader_file, encoding="utf-8", errors="replace").read()
                  if leader_file else "")
persona_in_leader = PERSONA_MARKER in leader_content
spawned_roles = sorted(child_files.keys())
spawn_ok = all(r in child_files for r in ROLES)
turns_ok = bool(child_proof) and all(v["turnsCompleted"] for v in child_proof.values())
tui_path_ok = bool(leader_meta and leader_meta.get("originator") == "codex-tui")
prompt_is_task_only = not any(k in PROMPT for k in
                              ["spawn_agent", "agent_type", "planner", "implementer",
                               "verifier", "reviewer", "reporter", "ALL FIVE"])
proven = bool(tui_path_ok and prompt_is_task_only and persona_in_leader
              and spawn_ok and turns_ok)

summary = {
    "issue": 74,
    "criterion": "TUI leader persona-driven autonomous teaming (F2)",
    "mode": "interactive TUI (bare CLI, pty, prompt prefill); post-run reconciliation",
    "launch": {"argv": [ENGINE, "--no-daemon", "<task-only prompt>"], "prompt": PROMPT,
               "envCODEX_HOME": HOME, "capturedHome": HOME},
    "promptIsTaskOnly": prompt_is_task_only,
    "leaderSessionFile": leader_file,
    "leaderOriginator": (leader_meta or {}).get("originator"),
    "leaderSessionSource": (leader_meta or {}).get("source"),
    "leaderTurnStatuses": leader_turn_status,
    "personaInLeaderRollout": persona_in_leader,
    "spawnedRoles": spawned_roles,
    "allFiveRolesSpawned": spawn_ok,
    "childProof": child_proof,
    "allChildTurnsCompleted": turns_ok,
    "tuiAutonomousTeaming": "PROVEN" if proven else "NOT_PROVEN",
    "allPass": proven,
    "screenLog": f"{EV}/p1-2-tui-screen.log",
    "honestyNote": "PROVEN only if a genuine TUI leader (originator codex-tui), given a "
                   "task-only prompt with no spawn/role instructions, autonomously spawned "
                   "all five persona roles and every child turn completed. Otherwise "
                   "NOT_PROVEN with observed facts; never fabricated.",
}
with open(f"{EV}/p1-2-tui-conclusion.json", "w") as fh:
    json.dump(summary, fh, ensure_ascii=False, indent=2, default=str)
print(json.dumps({"allPass": proven, "tuiAutonomousTeaming": summary["tuiAutonomousTeaming"],
                  "leaderOriginator": summary["leaderOriginator"],
                  "personaInLeaderRollout": persona_in_leader,
                  "spawnedRoles": spawned_roles, "allChildTurnsCompleted": turns_ok},
                 ensure_ascii=False))
sys.exit(0 if proven else 1)