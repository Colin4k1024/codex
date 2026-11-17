#!/usr/bin/env python3
"""P0-6 (issue #69): app-server stdio full-link traces.

Trace A: real model task (initialize -> thread/start -> turn/start -> event pump -> turn/completed)
Trace B: typed approval handlers, decline without side effects; unknown server requests blocked
Trace B2: deferred approval decision — event pump keeps consuming while approval is held open
Trace C: turn/interrupt cancel with child-process cleanup evidence
Trace D: engine crash mid-turn -> fresh connection reconciliation, orphan-aware (uncertain handling)
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
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p0-6"
os.makedirs(EV, exist_ok=True)


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


def is_note(method):
    return lambda n: n.get("method") == method


def turn_id_from(note):
    turn = note.get("params", {}).get("turn", {})
    return turn.get("id") or note.get("params", {}).get("turnId")


def pgrep(pattern):
    r = subprocess.run(["pgrep", "-fl", pattern], capture_output=True, text=True)
    return r.stdout.strip()


def decline_handler(msg):
    method = msg.get("method", "")
    if method == "item/commandExecution/requestApproval":
        return {"result": {"decision": "decline"}}
    if method == "execCommandApproval":
        return {"result": {"decision": "deny"}}
    if method == "item/fileChange/requestApproval":
        return {"result": {"decision": "decline"}}
    if method == "applyPatchApproval":
        return {"result": {"decision": "deny"}}
    if method == "item/permissions/requestApproval":
        return {"result": {"decision": "deny"}}
    if method == "mcpServer/elicitation/request":
        return {"result": {"action": "decline"}}
    if method == "item/tool/requestUserInput":
        return {"result": {"answers": {}}}
    return {"error": {"code": -32601, "message": f"unsupported server request: {method}; blocked, never auto-approved"}}


summary = {}

# ---------------- Trace A: real model task ----------------
work_a = tempfile.mkdtemp(prefix="p06-a-")
client = Client(ENGINE, HOME_A, deadline=420)
try:
    ts = client.request("thread/start", {"cwd": work_a}, timeout=60)
    thread_id = ts["result"]["thread"]["id"]
    rid = client.send_request("turn/start", {
        "threadId": thread_id,
        "input": [{"type": "text", "text":
                   "Create a file named p06.txt in the current directory whose content is exactly "
                   "SUPERAGENT_P06_OK (no trailing newline). Then reply DONE."}],
    })
    started = client.wait_notification(is_note("turn/started"), timeout=120)
    completed_note = client.wait_notification(is_note("turn/completed"), timeout=360)
    resp = client.wait_response(rid, timeout=30)
    content = None
    path = f"{work_a}/p06.txt"
    if os.path.exists(path):
        content = open(path).read()
    summary["traceA"] = {
        "pass": content == "SUPERAGENT_P06_OK" and "error" not in resp,
        "threadId": thread_id,
        "turnStarted": started,
        "turnCompleted": completed_note,
        "turnStartResponse": resp,
        "fileContent": content,
        "notificationCount": len(client.notifications),
        "maxFrameBytesSeen": client.max_frame_seen,
        "frameViolations": client.frame_violations,
    }
    save("trace-a-transcript.json", {"results": [ts, resp], **client.transcript()})
finally:
    client.close()

# ---------------- Trace B: approval decline ----------------
home_b = f"{EV}/home-b"
os.makedirs(home_b, exist_ok=True)
with open(f"{home_b}/config.toml", "w") as fh:
    fh.write('# P0-6 trace B: force command approval via read-only sandbox escalation\n'
             '# ("untrusted" approval_policy was removed upstream; on-request + read-only\n'
             '# makes any write command escalate to item/commandExecution/requestApproval)\n'
             'approval_policy = "on-request"\nsandbox_mode = "read-only"\n')
auth_src = f"{HOME_A}/auth.json"
if os.path.exists(auth_src):
    shutil.copyfile(auth_src, f"{home_b}/auth.json")
work_b = tempfile.mkdtemp(prefix="p06-b-")
client = Client(ENGINE, home_b, deadline=420, server_request_handler=decline_handler)
try:
    ts = client.request("thread/start", {"cwd": work_b}, timeout=60)
    thread_id = ts["result"]["thread"]["id"]
    rid = client.send_request("turn/start", {
        "threadId": thread_id,
        "input": [{"type": "text", "text":
                   "Run the shell command `touch approval-probe.txt` in the current directory using your "
                   "command execution tool. If the command is denied by the user, do not retry and do not "
                   "create the file with any other tool; reply exactly DENIED."}],
    })
    client.wait_notification(is_note("turn/completed"), timeout=360)
    resp = client.wait_response(rid, timeout=30)
    probe_exists = os.path.exists(f"{work_b}/approval-probe.txt")
    decisions = client.server_request_decisions
    declined = any(d["method"] in ("item/commandExecution/requestApproval", "execCommandApproval")
                   and d["payload"].get("result", {}).get("decision") in ("decline", "deny")
                   for d in decisions)
    summary["traceB"] = {
        "pass": declined and not probe_exists,
        "approvalDeclined": declined,
        "sideEffectFileExists": probe_exists,
        "serverRequestDecisions": decisions,
        "turnStartResponse": resp,
    }
    save("trace-b-transcript.json", {"results": [ts, resp], **client.transcript()})
finally:
    client.close()

# ---------------- Trace B2: deferred approval, pump keeps consuming ----------------
def defer_handler(msg):
    if msg.get("method") in ("item/commandExecution/requestApproval", "execCommandApproval"):
        return {"__defer__": True}
    return decline_handler(msg)


work_b2 = tempfile.mkdtemp(prefix="p06-b2-")
client = Client(ENGINE, home_b, deadline=420, server_request_handler=defer_handler)
try:
    ts = client.request("thread/start", {"cwd": work_b2}, timeout=60)
    thread_id = ts["result"]["thread"]["id"]
    rid = client.send_request("turn/start", {
        "threadId": thread_id,
        "input": [{"type": "text", "text":
                   "Run the shell command `touch approval-probe.txt` in the current directory using your "
                   "command execution tool. If the command is denied by the user, do not retry and do not "
                   "create the file with any other tool; reply exactly DENIED."}],
    })
    pending_note = client.wait_notification(
        lambda n: n.get("method") == "item/started"
        and "approval" in str(n.get("params", {}).get("item", {}).get("type", "")).lower(),
        timeout=240)
    deadline = time.time() + 120
    while not client.pending_server_requests and time.time() < deadline:
        time.sleep(0.5)
    pending_ids = list(client.pending_server_requests)
    count_before = len(client.notifications)
    time.sleep(8)
    count_after = len(client.notifications)
    unrelated = [n.get("method") for n in client.notifications[count_before:count_after]]
    for pid in pending_ids:
        client.respond_server_request(pid, {"result": {"decision": "decline"}})
    client.wait_notification(is_note("turn/completed"), timeout=360)
    resp = client.wait_response(rid, timeout=30)
    probe_exists = os.path.exists(f"{work_b2}/approval-probe.txt")
    summary["traceB2"] = {
        "pass": bool(pending_ids) and count_after > count_before and not probe_exists,
        "deferredRequestIds": pending_ids,
        "notificationsBeforeHold": count_before,
        "notificationsAfterHold": count_after,
        "methodsConsumedWhileApprovalPending": unrelated,
        "pumpConsumedWhilePending": count_after > count_before,
        "sideEffectFileExists": probe_exists,
        "serverRequestDecisions": client.server_request_decisions,
        "turnStartResponse": resp,
    }
    save("trace-b2-transcript.json", {"results": [ts, resp], "pendingApprovalNote": pending_note,
                                      **client.transcript()})
finally:
    client.close()

# ---------------- Trace B3: fileChange approval decline ----------------
work_b3 = tempfile.mkdtemp(prefix="p06-b3-")
client = Client(ENGINE, home_b, deadline=420, server_request_handler=decline_handler)
try:
    ts = client.request("thread/start", {"cwd": work_b3}, timeout=60)
    thread_id = ts["result"]["thread"]["id"]
    rid = client.send_request("turn/start", {
        "threadId": thread_id,
        "input": [{"type": "text", "text":
                   "Use your apply_patch / file editing tool (NOT a shell command) to create a file "
                   "named patch-probe.txt containing PATCHED. If the change is denied by the user, "
                   "do not retry with any tool; reply exactly PATCH-DENIED."}],
    })
    client.wait_notification(is_note("turn/completed"), timeout=360)
    resp = client.wait_response(rid, timeout=30)
    probe_exists = os.path.exists(f"{work_b3}/patch-probe.txt")
    decisions = client.server_request_decisions
    file_declined = any(d["method"] in ("item/fileChange/requestApproval", "applyPatchApproval")
                        and d["payload"].get("result", {}).get("decision") in ("decline", "deny")
                        for d in decisions)
    summary["traceB3"] = {
        "pass": file_declined and not probe_exists,
        "fileChangeDeclined": file_declined,
        "sideEffectFileExists": probe_exists,
        "serverRequestDecisions": decisions,
        "turnStartResponse": resp,
        "note": "If no fileChange approval request was raised (model used shell instead), "
                "this trace is INCONCLUSIVE and must be rerun with a stronger prompt.",
    }
    save("trace-b3-transcript.json", {"results": [ts, resp], **client.transcript()})
finally:
    client.close()

# ---------------- Trace B4: user-input request decline ----------------
work_b4 = tempfile.mkdtemp(prefix="p06-b4-")
client = Client(ENGINE, home_b, experimental=True, deadline=420, server_request_handler=decline_handler)
try:
    ts = client.request("thread/start", {"cwd": work_b4}, timeout=60)
    thread_id = ts["result"]["thread"]["id"]
    rid = client.send_request("turn/start", {
        "threadId": thread_id,
        "input": [{"type": "text", "text":
                   "Use the request_user_input tool to ask the user one question: 'Proceed with the probe?' "
                   "with options yes/no. If the tool is unavailable or the user provides no answer, "
                   "reply exactly NO-INPUT. Do not use any other tool."}],
    })
    client.wait_notification(is_note("turn/completed"), timeout=360)
    resp = client.wait_response(rid, timeout=30)
    decisions = client.server_request_decisions
    input_requests = [d for d in decisions if d["method"] == "item/tool/requestUserInput"]
    summary["traceB4"] = {
        "pass": bool(input_requests),
        "userInputRequestObserved": bool(input_requests),
        "answersReturned": [d["payload"] for d in input_requests],
        "serverRequestDecisions": decisions,
        "turnStartResponse": resp,
        "note": "Empty answers {} = declined input, no side effects. If the tool is not exposed to the "
                "model in this build, record UNAVAILABLE and narrow the conclusion accordingly.",
    }
    save("trace-b4-transcript.json", {"results": [ts, resp], **client.transcript()})
finally:
    client.close()

# ---------------- Trace C: cancel ----------------
work_c = tempfile.mkdtemp(prefix="p06-c-")
client = Client(ENGINE, HOME_A, deadline=420)
try:
    ts = client.request("thread/start", {"cwd": work_c}, timeout=60)
    thread_id = ts["result"]["thread"]["id"]
    rid = client.send_request("turn/start", {
        "threadId": thread_id,
        "input": [{"type": "text", "text":
                   "Use your command execution tool to run `sleep 60` and wait for it to finish. "
                   "Do nothing else before it finishes."}],
    })
    started = client.wait_notification(is_note("turn/started"), timeout=120)
    turn_id = turn_id_from(started)
    before = ""
    for _ in range(60):
        before = pgrep("sleep 60")
        if before:
            break
        time.sleep(1)
    interrupt_resp = client.request("turn/interrupt", {"threadId": thread_id, "turnId": turn_id}, timeout=30)
    completed_note = client.wait_notification(is_note("turn/completed"), timeout=120)
    try:
        resp = client.wait_response(rid, timeout=60)
    except TimeoutError:
        resp = {"error": {"message": "turn/start response not received after interrupt"}}
    time.sleep(3)
    after = pgrep("sleep 60")
    cleanup_seconds = 3
    for _ in range(20):
        if not after:
            break
        time.sleep(1)
        cleanup_seconds += 1
        after = pgrep("sleep 60")
    status = completed_note.get("params", {}).get("turn", {}).get("status")
    summary["traceC"] = {
        "pass": bool(before) and not after and "error" not in interrupt_resp,
        "turnId": turn_id,
        "sleepBeforeInterrupt": before,
        "sleepAfterInterrupt": after,
        "cleanupSeconds": cleanup_seconds,
        "interruptResponse": interrupt_resp,
        "turnCompleted": completed_note,
        "turnStatus": status,
        "turnStartResponse": resp,
    }
    save("trace-c-transcript.json", {"results": [ts, interrupt_resp, resp], **client.transcript()})
finally:
    client.close()

# ---------------- Trace D: crash + reconciliation ----------------
work_d = tempfile.mkdtemp(prefix="p06-d-")
client = Client(ENGINE, HOME_A, deadline=420)
thread_id = None
last_notes = []
try:
    ts = client.request("thread/start", {"cwd": work_d}, timeout=60)
    thread_id = ts["result"]["thread"]["id"]
    rid = client.send_request("turn/start", {
        "threadId": thread_id,
        "input": [{"type": "text", "text":
                   "Use your command execution tool to run `sleep 45` and wait for it to finish. "
                   "Then create d.txt containing RECONCILED."}],
    })
    client.wait_notification(is_note("turn/started"), timeout=120)
    for _ in range(60):
        if pgrep("sleep 45"):
            break
        time.sleep(1)
    last_notes = list(client.notifications)
    client.proc.kill()
    client.proc.wait(timeout=10)
    crash_exit = client.proc.returncode
    orphan_after_crash = pgrep("sleep 45")
    save("trace-d-precrash.json", {"threadId": thread_id, "notifications": last_notes,
                                  "stderrTail": client.stderr_lines[-50:], "crashExitCode": crash_exit,
                                  "orphanSleepAfterCrash": orphan_after_crash})
finally:
    try:
        client.close()
    except Exception:
        pass

recon = Client(ENGINE, HOME_A, deadline=180)
try:
    orphan_poll_seconds = 0
    orphan_now = orphan_after_crash
    for _ in range(20):
        if not orphan_now:
            break
        time.sleep(1)
        orphan_poll_seconds += 1
        orphan_now = pgrep("sleep 45")
    read = recon.request("thread/read", {"threadId": thread_id, "includeTurns": True}, timeout=60)
    turns = read.get("result", {}).get("thread", {}).get("turns", [])
    last_turn = turns[-1] if turns else None
    last_status = (last_turn or {}).get("status")
    terminal = last_status in ("completed", "interrupted", "failed", "aborted")
    orphan_cleared = not orphan_now
    terminal_proven = terminal and orphan_cleared
    if terminal_proven:
        disposition = "reconciled-terminal: turn status terminal AND no orphaned child process remains"
    elif terminal and not orphan_cleared:
        disposition = ("uncertain: turn status terminal but orphaned child process still present; "
                       "execution outcome not provably terminated; await operator decision, no blind replay")
    else:
        disposition = "uncertain: cannot prove execution outcome; await operator decision, no blind replay"
    summary["traceD"] = {
        "pass": "error" not in read,
        "threadId": thread_id,
        "crashExitCode": crash_exit,
        "orphanSleepAfterCrash": orphan_after_crash,
        "orphanPollSeconds": orphan_poll_seconds,
        "orphanAfterPoll": orphan_now,
        "orphanCleared": orphan_cleared,
        "reconciledLastTurnStatus": last_status,
        "terminalProven": terminal_proven,
        "disposition": disposition,
        "threadReadResponse": read,
    }
    save("trace-d-reconcile.json", {"results": [read], **recon.transcript()})
finally:
    recon.close()

summary["allPass"] = all(summary[k]["pass"] for k in ("traceA", "traceB", "traceB2", "traceB3", "traceC", "traceD"))
summary["traceB4ReportedSeparately"] = "user-input tool may be unavailable in this build; see traceB4.note"
save("summary.json", summary)
print(json.dumps({k: (v["pass"] if isinstance(v, dict) and "pass" in v else v) for k, v in summary.items()}, indent=2))
sys.exit(0 if summary["allPass"] else 1)