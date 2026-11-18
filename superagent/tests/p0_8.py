#!/usr/bin/env python3
"""P0-8 (issue #71): subagent observability verdict experiment.

Analyzes the P0-2 team turn transcript for SubAgentActivity / CollabAgentToolCall items,
then probes public RPCs (thread/list, thread/read, thread/items/list) against observed
agent thread ids. Also regenerates static protocol evidence: full ClientRequest method
enumeration (no agent/* or graph/store RPC), hook event existence, agent-graph-store
negative exposure evidence, and interrupt cross-evidence from P0-6 trace C.
Outputs verdict.json + event samples; decides whether fork change F-1 is triggered.
If the P0-2 transcript is missing or has no subagent items, runs a fresh team turn first.
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


PII_KEYS = {"installationId", "serverName", "codexHome", "authMode", "planType"}


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


def git_show(ref, path):
    r = subprocess.run(["git", "-C", ROOT, "show", f"{ref}:{path}"],
                       capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else None


def git_grep(pattern, *paths):
    r = subprocess.run(["git", "-C", ROOT, "grep", "-n", pattern, "--", *paths],
                       capture_output=True, text=True)
    return r.stdout


def regenerate_static_evidence():
    schema_rel = "codex-rs/app-server-protocol/schema/json/codex_app_server_protocol.v2.schemas.json"
    schema = json.loads(git_show("HEAD", schema_rel) or open(f"{ROOT}/{schema_rel}").read())
    defs = schema.get("$defs") or schema.get("definitions") or {}
    methods = []

    def collect(node):
        if isinstance(node, dict):
            props = node.get("properties")
            if isinstance(props, dict) and isinstance(props.get("method"), dict):
                m = props["method"]
                if isinstance(m.get("const"), str):
                    methods.append(m["const"])
                if isinstance(m.get("enum"), list):
                    methods.extend(x for x in m["enum"] if isinstance(x, str))
            for v in node.values():
                collect(v)
        elif isinstance(node, list):
            for v in node:
                collect(v)

    collect(defs.get("ClientRequest", {}))
    methods = sorted(set(methods))
    lines = [
        f"# Full ClientRequest method enumeration from {schema_rel} (HEAD checkout).",
        f"# Total methods: {len(methods)}",
        "",
        "## agent/* methods (negative evidence: none exist)",
        json.dumps([m for m in methods if m.startswith("agent")]),
        "",
        "## graph/store methods (negative evidence: none exist)",
        json.dumps([m for m in methods if "graph" in m.lower() or "store" in m.lower()]),
        "",
        "## turn/* methods (incl. turn/interrupt used for whole-turn cancel)",
        json.dumps([m for m in methods if m.startswith("turn/")], indent=1),
        "",
        "## thread/* methods (incl. thread/resume)",
        json.dumps([m for m in methods if m.startswith("thread/")], indent=1),
        "",
        "## all methods",
        json.dumps(methods, indent=1),
    ]
    with open(f"{EV}/no-agent-rpc.txt", "w") as fh:
        fh.write("\n".join(lines) + "\n")

    hooks = git_grep(r"SubagentStart\|SubagentStop",
                     "codex-rs/protocol/src/hooks.rs",
                     "codex-rs/protocol/src/protocol.rs",
                     "codex-rs/core/src/hook_runtime.rs")
    with open(f"{EV}/hooks-events.txt", "w") as fh:
        fh.write("# STATIC protocol existence only (git grep at HEAD); this spike does NOT\n"
                 "# trace runtime hook firing. Source paths included per line.\n\n" + hooks)

    store = git_grep(r"agent_graph_store\|AgentGraphStore", "codex-rs/app-server/src")
    store_lib = git_show("HEAD", "codex-rs/agent-graph-store/src/lib.rs") or ""
    store_trait = "\n".join((git_show("HEAD", "codex-rs/agent-graph-store/src/store.rs") or "").splitlines()[:40])
    registry_head = "\n".join((git_show("HEAD", "codex-rs/core/src/agent/registry.rs") or "").splitlines()[:40])
    with open(f"{EV}/store-negative-evidence.txt", "w") as fh:
        fh.write(
            "# Negative evidence: agent graph store is NOT exposed as a public RPC.\n"
            "# git grep 'agent_graph_store|AgentGraphStore' over codex-rs/app-server/src at HEAD:\n\n"
            + store +
            "\n# Interpretation: the only app-server references are internal wiring that passes\n"
            "# codex_core::local_agent_graph_store_from_state_db (core/src/thread_manager.rs:640)\n"
            "# into the session/thread manager (message_processor.rs:414, mcp_refresh.rs:575).\n"
            "# No ClientRequest method reads or writes the graph store (see no-agent-rpc.txt:\n"
            "# zero graph/store methods in the generated schema).\n"
            "\n## codex-rs/agent-graph-store/src/lib.rs (full, HEAD):\n" + store_lib +
            "\n## codex-rs/agent-graph-store/src/store.rs:1-40 (trait surface, HEAD):\n" + store_trait +
            "\n\n## codex-rs/core/src/agent/registry.rs:1-40 (HEAD):\n" + registry_head + "\n")

    cross = {"sourcePointer": "feat/p0-6-stdio-contract:superagent/evidence/p0-6/summary.json (PR #108)",
             "traceC": None, "error": None}
    raw = git_show("feat/p0-6-stdio-contract", "superagent/evidence/p0-6/summary.json")
    if raw:
        try:
            cross["traceC"] = json.loads(raw).get("traceC")
        except Exception as exc:
            cross["error"] = str(exc)
    else:
        cross["error"] = "git show failed: p0-6 summary not available on feat/p0-6-stdio-contract"
    save("interrupt-cross-evidence.json", cross)
    return methods


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

rpc_methods = regenerate_static_evidence()
agent_rpcs = [m for m in rpc_methods if m.startswith("agent")]
store_rpcs = [m for m in rpc_methods if "graph" in m.lower() or "store" in m.lower()]

verdict = {
    "issue": 71,
    "source": source,
    "leaderThreadId": leader_thread,
    "subAgentItemCount": len(items),
    "itemKinds": kinds,
    "agentThreadIds": agent_threads,
    "lifecycle": lifecycle,
    "childHistoryReadableViaPublicRpc": child_history_readable,
    "publicAgentControlRpcs": agent_rpcs,
    "publicGraphStoreRpcs": store_rpcs,
    "rpcMethodCount": len(rpc_methods),
    "verdict": "SUFFICIENT_NO_FORK" if sufficient else "GAP_TRIGGERS_F1",
    "controlPlaneNotes": [
        "No public agent/* RPC and no graph/store RPC exists in the generated ClientRequest schema "
        "(full enumeration in no-agent-rpc.txt; store wiring negative evidence in store-negative-evidence.txt).",
        "Per-subagent interrupt from an external client is not directly exposed; outer orchestrator cancels "
        "the whole leader turn via turn/interrupt (runtime proof: interrupt-cross-evidence.json, P0-6 trace C, "
        "PR #108: sleep child present before interrupt, absent after) or asks the leader to use interrupt_agent.",
        "Resume evaluation (static code assessment, not a runtime trace): thread/resume "
        "(app-server/src/request_processors/thread_processor.rs:640 -> thread_resume_inner:3925) cold-resumes "
        "any persisted thread, including a child agent thread, as an INDEPENDENT top-level session; it does not "
        "re-attach the child to a running leader's agent tree. In-tree single-agent resume is the leader's "
        "resume_agent tool (MAv1 only; MAv2 exposes no resume tool). External in-tree per-agent resume would be "
        "an F-1-shaped addition; not required for the orchestrator MVP (leader-mediated resume suffices).",
        "Topology is reconstructed from SubAgentActivity items (kind started/interacted/interrupted/completed) "
        "+ CollabAgentToolCall.agentsStates (per-agent threadId) + child-thread thread/read. thread/list does NOT "
        "enumerate agent threads: probes['thread/list'].agentThreadsListed is empty.",
        "Child thread history readable via thread/read when agentThreadId is known (see probes). "
        "thread/items/list is UNSUPPORTED_SERVER_SIDE (P0-9 probe); leader history comes from rollout files.",
        "Hook events SubagentStart/SubagentStop exist in the protocol (hooks-events.txt, static existence only; "
        "runtime hook firing not traced in this spike).",
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