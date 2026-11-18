#!/usr/bin/env python3
"""P1-2 / #74 runtime acceptance probe (app-server stdio path).

Proves, against the real engine + production pack overlay:
  - leader persona resident intro is injected into the LEADER's own session
    (root_agent_usage_hint_text -> leader developer message);
  - leader spawns all five declared roles (planner/implementer/verifier/
    reviewer/reporter) via a scripted delegation prompt;
  - EVERY child turn reaches status=completed with no error (F1: a failed
    child turn, e.g. modelNotFound, fails the probe — thread-read
    status:{type:notLoaded} is the thread-read load status, NOT the turn
    status; the turn status lives in thread.turns[i].status);
  - per-role config layer applied: child model matches the role toml model;
  - role isolation: each child's OWN rollout carries ONLY its own role
    instructions (no foreign role instructions);
  - budget (F3): tokens measured from each child's OWN rollout file, keyed by
    token_usage_record.payload.thread_id == child threadId (the leader rollout
    contains every child sessionId plus the parent's cumulative total and must
    never be used for child budgets). Measured against design caps
    (evidence/acceptance/issue-63.md risk 5):
      resident intro <= 400 (tiktoken o200k_base),
      handoff summary <= 800 (child final_answer text, tiktoken),
      single fragment <= 900 (largest measurable plaintext fragment:
        role developer_instructions and child final_answer; the
        leader->child task payload is encrypted at rest by the engine and
        is reported NOT_PROVEN, never guessed),
      per-round new tokens <= 2000 (per token_usage_record round:
        new = (input_tokens - cached_input_tokens) + output_tokens;
        cached input is reused context, not new spend).

NOT claimed here (reported as NOT_PROVEN in the conclusion, not deleted):
  - TUI leader persona-driven autonomous teaming: this probe is stdio-scripted
    with an explicit "spawn ALL FIVE" prompt. The genuine TUI attempt lives in
    tests/p1_2_tui.py and is reported separately.
  - per-role sandbox: the engine does not apply role-level sandbox_mode
    (AgentRoleOverrides in core/src/agent/role.rs carries only model/reasoning/
    verbosity/developer_instructions; child sandbox comes from the parent turn
    runtime via child_config.rs; P0-2e runtime proof). Orchestrator-enforced
    via explicit SandboxPolicy (P0-7).

This probe does NOT edit code; content-layer structure is proven by
test_roles.py. Exit code 0 only if every gated check genuinely passes.
"""
import json
import os
import shutil
import sys
import tempfile
import tomllib

import tiktoken

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spike_client import Client

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ENGINE = os.environ.get("SUPERAGENT_ENGINE", f"{ROOT}/superagent/packaging/superagent-engine")
HOME_A = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p1-2"
os.makedirs(EV, exist_ok=True)
HOME = f"{EV}/home-p1-2"
# Fresh home per run: stale rollouts from earlier runs must never contaminate
# persona/isolation/budget measurements.
if os.path.exists(HOME):
    shutil.rmtree(HOME)
os.makedirs(f"{HOME}/agents", exist_ok=True)

ROLES = ["planner", "implementer", "verifier", "reviewer", "reporter"]
# Single source of truth: the role toml models shipped in the pack.
ROLE_MODEL = {}
ROLE_INSTRUCTIONS = {}
for r in ROLES:
    with open(f"{ROOT}/superagent/pack/roles/{r}.toml", "rb") as fh:
        role_toml = tomllib.load(fh)
    ROLE_MODEL[r] = role_toml["model"]
    ROLE_INSTRUCTIONS[r] = role_toml["developer_instructions"]
MARKER = {r: f"{r.upper()} of a SuperAgent team" for r in ROLES}
PERSONA_MARKER = "SuperAgent meta-agent (leader), a coordinator not an executor"

with open(f"{ROOT}/superagent/pack/config/agents-overlay.toml", "rb") as fh:
    OVERLAY = tomllib.load(fh)
RESIDENT_INTRO = OVERLAY["features"]["multi_agent_v2"]["root_agent_usage_hint_text"].strip()

with open(f"{HOME}/config.toml", "w") as fh:
    fh.write("# P1-2 dedicated home: production pack overlay\n")
    fh.write(open(f"{ROOT}/superagent/pack/config/agents-overlay.toml").read())
for r in ROLES:
    shutil.copyfile(f"{ROOT}/superagent/pack/roles/{r}.toml", f"{HOME}/agents/{r}.toml")
if os.path.exists(f"{HOME_A}/auth.json"):
    shutil.copyfile(f"{HOME_A}/auth.json", f"{HOME}/auth.json")

WORK = tempfile.mkdtemp(prefix="p12-")
ENC = tiktoken.get_encoding("o200k_base")


def ntok(text):
    return len(ENC.encode(text or ""))


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
    leader_done = client.wait_notification(turn_done, timeout=1100)
    resp = client.wait_response(rid, timeout=60)
    save("p1-2-transcript.json", {"results": [ts, resp], **client.transcript()})
finally:
    client.close()

notes = client.notifications
leader_turn_status = leader_done.get("params", {}).get("turn", {}).get("status")
child_ids = {}
child_meta = {}
activities = []
final_text = []
# Child turn/completed notifications observed on the leader connection are
# recorded as a cross-check; thread/read below remains the authority.
child_turn_notes = {}
for n in notes:
    if n.get("method") == "turn/completed":
        p = n.get("params", {})
        if p.get("threadId") != leader_id:
            child_turn_notes[p.get("threadId")] = p.get("turn", {}).get("status")
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
child_reads = {}
recon = Client(ENGINE, HOME, deadline=240)
try:
    for path, cid in child_ids.items():
        read = recon.request("thread/read", {"threadId": cid, "includeTurns": True}, timeout=60)
        child_reads[cid] = read
        th = read.get("result", {}).get("thread", {})
        turns = th.get("turns") or []
        turn_statuses = [t.get("status") for t in turns]
        turn_errors = [t.get("error") for t in turns if t.get("error")]
        final_answers = []
        for t in turns:
            for it in t.get("items") or []:
                if it.get("type") == "agentMessage" and it.get("phase") == "final_answer":
                    final_answers.append(it.get("text") or "")
        role_proof[path] = {
            "threadId": cid, "sessionId": th.get("sessionId"),
            "agentRole": th.get("agentRole"), "agentNickname": th.get("agentNickname"),
            "model": th.get("model"), "readable": "error" not in read,
            "turnStatuses": turn_statuses,
            "turnErrors": turn_errors,
            "turnsCompleted": bool(turns) and all(s == "completed" for s in turn_statuses)
                              and not turn_errors,
            "turnCompletedNotification": child_turn_notes.get(cid),
            "finalAnswer": "\n".join(final_answers),
        }
        save(f"p1-2-child-read-{path.strip('/').replace('/', '_')}.json", read)
finally:
    recon.close()

# --- Rollout instrumentation keyed to each session's OWN file ---------------
# A rollout file's first session_meta.id is its owner. token_usage_record rows
# carry payload.thread_id; only rows for the session's own thread count.
rollout_by_owner = {}
for root, _dirs, files in os.walk(f"{HOME}/sessions"):
    for fn in files:
        if not fn.endswith(".jsonl"):
            continue
        path = os.path.join(root, fn)
        try:
            with open(path, encoding="utf-8", errors="replace") as fh:
                first = fh.readline()
            meta = json.loads(first)
            if meta.get("type") == "session_meta":
                rollout_by_owner[meta.get("payload", {}).get("id")] = path
        except (OSError, json.JSONDecodeError):
            continue


def own_rollout_records(session_id):
    path = rollout_by_owner.get(session_id)
    if not path:
        return None, []
    recs = []
    with open(path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            try:
                recs.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    return path, recs


leader_file, leader_recs = own_rollout_records(leader_id)
leader_content = open(leader_file, encoding="utf-8", errors="replace").read() if leader_file else ""
persona_ok = PERSONA_MARKER in leader_content

CAPS = {"residentIntro": 400, "handoffSummary": 800, "singleFragment": 900,
        "perRoundNewTokens": 2000}
resident_intro_tokens = ntok(RESIDENT_INTRO)
budget_roles = {}
for path, v in role_proof.items():
    sid = v.get("sessionId")
    role = v.get("agentRole")
    cfile, crecs = own_rollout_records(sid)
    content = open(cfile, encoding="utf-8", errors="replace").read() if cfile else ""
    markers = {r for r in ROLES if MARKER[r] in content}
    v["ownRolloutFile"] = cfile
    v["ownInstructionsInRollout"] = role in markers
    v["foreignInstructionsInRollout"] = sorted(markers - {role})
    v["leaderPersonaInChildRollout"] = PERSONA_MARKER in content

    rounds = []
    for rec in crecs:
        if rec.get("type") != "token_usage_record":
            continue
        p = rec.get("payload", {})
        if p.get("thread_id") != v["threadId"]:
            continue  # never count another thread's (e.g. parent's) usage
        u = p.get("usage") or {}
        inp = u.get("input_tokens", 0)
        cached = u.get("cached_input_tokens", 0)
        out = u.get("output_tokens", 0)
        rounds.append({"turnId": p.get("turn_id"), "responseId": p.get("response_id"),
                       "inputTokens": inp, "cachedInputTokens": cached,
                       "outputTokens": out, "totalTokens": u.get("total_tokens"),
                       "newTokens": (inp - cached) + out})
    thread_totals = [rec["payload"].get("thread_token_usage", {}).get("total_tokens")
                     for rec in crecs
                     if rec.get("type") == "token_usage_record"
                     and rec["payload"].get("thread_id") == v["threadId"]]
    thread_totals = [t for t in thread_totals if isinstance(t, int)]
    v["totalTokens"] = max(thread_totals) if thread_totals else None
    v["usageRounds"] = rounds

    handoff_tokens = ntok(v["finalAnswer"]) if v["finalAnswer"] else None
    role_instr_tokens = ntok(ROLE_INSTRUCTIONS.get(role, ""))
    caps = {
        "handoffSummary": {
            "tokens": handoff_tokens, "cap": CAPS["handoffSummary"],
            "status": ("NOT_PROVEN" if handoff_tokens is None else
                       "PASS" if handoff_tokens <= CAPS["handoffSummary"] else "FAIL"),
            "source": "child final_answer text (thread/read), tiktoken o200k_base",
        },
        "singleFragment": {
            "measuredFragments": {"roleDeveloperInstructions": role_instr_tokens,
                                  "childFinalAnswer": handoff_tokens},
            "cap": CAPS["singleFragment"],
            "status": ("NOT_PROVEN" if handoff_tokens is None else
                       "PASS" if max(role_instr_tokens, handoff_tokens) <= CAPS["singleFragment"]
                       else "FAIL"),
            "notProven": "leader->child task payload is encrypted at rest by the "
                         "engine (rollout + notifications); its plaintext token "
                         "count cannot be measured and is NOT guessed",
        },
        "perRoundNewTokens": {
            "rounds": rounds, "cap": CAPS["perRoundNewTokens"],
            "definition": "new = (input_tokens - cached_input_tokens) + output_tokens "
                          "per token_usage_record round from the child's OWN rollout",
            "status": ("NOT_PROVEN" if not rounds else
                       "PASS" if all(r["newTokens"] <= CAPS["perRoundNewTokens"] for r in rounds)
                       else "FAIL"),
        },
    }
    budget_roles[path] = caps

by_role = {}
for v in role_proof.values():
    by_role.setdefault(v.get("agentRole"), []).append(v)

spawned_roles = sorted(by_role.keys())
spawn_ok = all(r in by_role for r in ROLES)
model_ok = all(any(v.get("model") == ROLE_MODEL[r] for v in by_role.get(r, [])) for r in ROLES)
isolation_ok = all(v["ownInstructionsInRollout"] and not v["foreignInstructionsInRollout"]
                   for v in role_proof.values()) and len(role_proof) >= 5
turns_ok = bool(role_proof) and all(v["turnsCompleted"] for v in role_proof.values())
budget_measured = sum(1 for v in role_proof.values() if isinstance(v.get("totalTokens"), int))
cap_statuses = [c["status"] for caps in budget_roles.values() for c in caps.values()]
budget_ok = (budget_measured >= 5 and bool(cap_statuses)
             and all(s == "PASS" for s in cap_statuses)
             and resident_intro_tokens <= CAPS["residentIntro"])

summary = {
    "issue": 74,
    "leaderThreadId": leader_id,
    "leaderTurnStatus": leader_turn_status,
    "spawnedRoles": spawned_roles,
    "childCount": len(role_proof),
    "personaInjected": persona_ok,
    "personaInjectionSource": "leader's OWN rollout (root_agent_usage_hint_text developer message)",
    "allFiveRolesSpawned": spawn_ok,
    "perRoleModelApplied": model_ok,
    "roleModels": ROLE_MODEL,
    "roleIsolation": isolation_ok,
    "childTurnsCompleted": turns_ok,
    "budgetMeasuredChildren": budget_measured,
    "budgetCaps": CAPS,
    "residentIntroTokens": resident_intro_tokens,
    "residentIntroCapStatus": "PASS" if resident_intro_tokens <= CAPS["residentIntro"] else "FAIL",
    "budgetByRole": budget_roles,
    "budgetMeasured": budget_ok,
    "roleProof": role_proof,
    "leaderSummary": leader_summary,
    "allPass": bool(spawn_ok and model_ok and isolation_ok and persona_ok and turns_ok and budget_ok),
    "notProven": {
        "tuiLeaderPersonaAutonomousTeaming":
            "NOT_PROVEN by this probe: stdio-scripted with an explicit 'spawn ALL FIVE' "
            "prompt. Genuine TUI attempt: tests/p1_2_tui.py (reported separately).",
        "perRoleSandbox":
            "NOT_PROVEN / orchestrator-only: engine AgentRoleOverrides (core/src/agent/role.rs) "
            "does not carry sandbox/approval; child sandbox inherits the parent turn runtime "
            "(core/src/agent/child_config.rs). P0-2e runtime proof: role sandbox_mode ignored. "
            "Sandbox is orchestrator-enforced via explicit SandboxPolicy (P0-7).",
        "leaderToChildTaskFragmentTokens":
            "NOT_PROVEN: spawn task payload encrypted at rest; plaintext token count unmeasurable.",
    },
    "note": "Runtime teaming via app-server stdio (same engine+config path as TUI), "
            "scripted delegation prompt. allPass requires every child turn completed and "
            "every measurable budget cap PASS; NOT_PROVEN items are listed explicitly.",
}
save("p1-2-runtime-conclusion.json", summary)
print(json.dumps({k: summary[k] for k in
                  ["allPass", "spawnedRoles", "personaInjected", "perRoleModelApplied",
                   "roleIsolation", "childTurnsCompleted", "budgetMeasured", "childCount"]},
                 ensure_ascii=False))
sys.exit(0 if summary["allPass"] else 1)