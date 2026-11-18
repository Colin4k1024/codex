#!/usr/bin/env python3
"""Model-free fake stdio engine for P2-2 (#79) adapter acceptance tests.

Speaks the app-server JSONL stdio contract (initialize / thread/* / turn/* /
command/exec / server requests) with deterministic, controlled behavior.
NO model, NO network. Deliberately NOT named "codex" (macOS endpoint security
SIGKILLs processes named codex).

Scenarios:
  echo         normal flow: turn/start -> started + 3 deltas + completed
  slow         turn/start -> deltaCount deltas (default 5000) + completed
  crash        turn/start -> turn/started, persist inProgress state, exit(9)
  approval_hold turn/start(holdApproval) -> server request held open; a second
               concurrent turn completes while the approval is pending
  reconcile    thread/read returns --turn-status (fresh-connection recovery)
  ignore_eof   does not exit on stdin EOF (exercises the close/kill path)
  stderr_flood writes 3000 stderr lines at startup (ring-buffer bound test)

Test hooks (requests): test/env, test/ping, test/hang, test/triggerUnknown,
test/triggerBigLog, test/triggerOversize, test/emitServerRequest.
"""
import argparse
import json
import os
import subprocess
import sys
import threading
import time

ap = argparse.ArgumentParser()
ap.add_argument("--scenario", default="echo")
ap.add_argument("--state-file")
ap.add_argument("--turn-status", default="inProgress")
args = ap.parse_args()

emit_lock = threading.Lock()


def emit(obj):
    with emit_lock:
        sys.stdout.write(json.dumps(obj) + "\n")
        sys.stdout.flush()


def note(method, params):
    emit({"jsonrpc": "2.0", "method": method, "params": params})


def respond(rid, result):
    emit({"jsonrpc": "2.0", "id": rid, "result": result})


def rpc_error(rid, code, message):
    emit({"jsonrpc": "2.0", "id": rid, "error": {"code": code, "message": message}})


state = {"threads": {}}
if args.state_file and os.path.exists(args.state_file):
    try:
        with open(args.state_file) as fh:
            state.update(json.load(fh))
    except Exception:
        pass
state["argv"] = sys.argv
state["codexHome"] = os.environ.get("CODEX_HOME")
state["pid"] = os.getpid()


def save_state():
    if args.state_file:
        with open(args.state_file, "w") as fh:
            json.dump(state, fh)


if args.scenario == "stderr_flood":
    for i in range(3000):
        sys.stderr.write(f"stderr-line-{i}\n")
    sys.stderr.flush()

children = {}  # processId -> Popen
pending_turns = {}  # server-request id -> (turn request id, threadId)
counter = 0


def complete_turn(rid, tid, decision=None):
    state.setdefault("threads", {}).setdefault(tid, {})["turns"] = [
        {"id": "turn-1", "status": "completed"}
    ]
    save_state()
    note("turn/completed", {"threadId": tid, "turn": {"id": "turn-1", "status": "completed"},
                            "approvalDecision": decision})
    respond(rid, {"turn": {"id": "turn-1", "status": "completed"}})


def do_exec(rid, params):
    argv = params.get("command") or []
    pid_file = params.get("pidFile")
    proc = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    process_id = params.get("processId") or str(proc.pid)
    children[process_id] = proc
    if pid_file:
        with open(pid_file, "w") as fh:
            fh.write(str(proc.pid))
    out, _ = proc.communicate()
    children.pop(process_id, None)
    if pid_file and os.path.exists(pid_file):
        os.remove(pid_file)
    respond(rid, {"exitCode": proc.returncode, "stdout": out.decode(errors="replace")[:4000]})


def wait_reply(srv_id):
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except Exception:
            continue
        if "method" not in msg and msg.get("id") == srv_id:
            return msg
    return None


def handle(msg):
    global counter
    method = msg.get("method")
    rid = msg.get("id")
    params = msg.get("params") or {}
    if method == "initialize":
        respond(rid, {"userAgent": "fake-engine/0.1", "codexHome": os.environ.get("CODEX_HOME")})
        return
    if method == "initialized":
        return
    if method == "thread/start":
        counter += 1
        tid = params.get("threadId") or f"thr-{counter}"
        state.setdefault("threads", {})[tid] = {"turns": []}
        save_state()
        respond(rid, {"thread": {"id": tid}})
        return
    if method == "thread/read":
        tid = params.get("threadId")
        turns = state.get("threads", {}).get(tid, {}).get("turns", [])
        if args.scenario == "reconcile":
            turns = [{"id": "turn-1", "status": args.turn_status}]
        respond(rid, {"thread": {"id": tid, "turns": turns}})
        return
    if method == "turn/start":
        tid = params.get("threadId")
        state.setdefault("threads", {}).setdefault(tid, {})["turns"] = [
            {"id": "turn-1", "status": "inProgress"}
        ]
        save_state()
        note("turn/started", {"threadId": tid, "turn": {"id": "turn-1"}})
        if args.scenario == "crash":
            sys.stdout.flush()
            sys.stderr.flush()
            os._exit(9)
        if args.scenario == "approval_hold" and params.get("holdApproval"):
            emit({"jsonrpc": "2.0", "id": "srv-approval-1",
                  "method": "item/commandExecution/requestApproval",
                  "params": {"threadId": tid, "command": "touch probe.txt"}})
            pending_turns["srv-approval-1"] = (rid, tid)
            return
        if args.scenario == "slow":
            n = int(params.get("deltaCount", 5000))
            for i in range(n):
                note("item/agentMessage/delta", {"threadId": tid, "delta": f"d{i} "})
            complete_turn(rid, tid)
            return
        for i in range(3):
            note("item/agentMessage/delta", {"threadId": tid, "delta": f"chunk{i} "})
        complete_turn(rid, tid)
        return
    if method == "command/exec":
        # Threaded so command/exec/terminate can be processed while the
        # program is still running (waits-program-exit semantics).
        threading.Thread(target=do_exec, args=(rid, params), daemon=True).start()
        return
    if method == "command/exec/terminate":
        proc = children.get(params.get("processId"))
        if proc is None:
            respond(rid, {"terminated": False, "reason": "unknown processId"})
            return
        try:
            proc.kill()
        except Exception:
            pass
        proc.wait()
        respond(rid, {"terminated": True})
        return
    if method == "test/emitServerRequest":
        srv_id = params.get("requestId", "srv-1")
        emit({"jsonrpc": "2.0", "id": srv_id,
              "method": params.get("method", "evil/unknown"),
              "params": params.get("params", {})})
        reply = wait_reply(srv_id)
        respond(rid, {"reply": reply})
        return
    if method == "test/env":
        respond(rid, {"codexHome": os.environ.get("CODEX_HOME"), "argv": sys.argv, "pid": os.getpid()})
        return
    if method == "test/ping":
        respond(rid, {"pong": True})
        return
    if method == "test/hang":
        time.sleep(float(params.get("seconds", 5)))
        respond(rid, {"done": True})
        return
    if method == "test/triggerUnknown":
        note("totally/unknown", {"x": 1})
        with emit_lock:
            sys.stdout.write("this is not json\n")
            sys.stdout.flush()
        respond(rid, {"triggered": True})
        return
    if method == "test/triggerBigLog":
        chunks = int(params.get("chunks", 40))
        size = int(params.get("chunkBytes", 30000))
        for i in range(chunks):
            note("test/biglogChunk", {"i": i, "data": "x" * size})
        respond(rid, {"emitted": chunks})
        return
    if method == "test/triggerOversize":
        size = int(params.get("bytes", 200000))
        note("test/oversize", {"data": "y" * size})
        respond(rid, {"emitted": size})
        return
    if rid is not None:
        rpc_error(rid, -32601, f"unknown method: {method}")


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except Exception:
            continue
        if "method" in msg:
            handle(msg)
        else:
            # Reply to one of our server requests (deferred approval path).
            sid = msg.get("id")
            if sid in pending_turns:
                rid, tid = pending_turns.pop(sid)
                decision = (msg.get("result") or {}).get("decision")
                complete_turn(rid, tid, decision)
    # stdin EOF
    if args.scenario == "ignore_eof":
        while True:
            time.sleep(3600)
    save_state()
    sys.exit(0)


main()