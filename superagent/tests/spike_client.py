#!/usr/bin/env python3
"""SuperAgent P0 spike client: drive superagent-engine app-server over stdio.

Usage:
  python3 spike_client.py --engine PATH --home CODEX_HOME \
      --calls '[{"method":"config/read","params":{}}]' [--out transcript.json] \
      [--deadline 120] [--experimental]

Performs initialize/initialized, then runs calls sequentially. Server requests
are rejected with -32601 by default (spike baseline: never auto-approve).
Notifications are recorded. Transcript JSON is written to --out or stdout.
"""
import argparse
import json
import os
import subprocess
import sys
import threading
import time
import queue


class Client:
    def __init__(self, engine, home, experimental=False, deadline=120,
                 server_request_handler=None, max_frame_bytes=16 * 1024 * 1024):
        env = dict(os.environ, CODEX_HOME=home)
        self.proc = subprocess.Popen(
            [engine, "app-server", "--listen", "stdio://"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            env=env, text=True, bufsize=1,
        )
        self.responses = {}
        self.notifications = []
        self.server_requests = []
        self.server_request_decisions = []
        self.server_request_handler = server_request_handler
        self.max_frame_bytes = max_frame_bytes
        self.max_frame_seen = 0
        self.frame_violations = []
        self.lock = threading.Lock()
        self.cv = threading.Condition(self.lock)
        self.send_lock = threading.Lock()
        self.next_id = 1
        self.deadline = time.time() + deadline
        self.reader = threading.Thread(target=self._read_loop, daemon=True)
        self.reader.start()
        self.stderr_lines = []
        self.err_reader = threading.Thread(target=self._read_stderr, daemon=True)
        self.err_reader.start()
        caps = {"experimentalApi": True} if experimental else {}
        resp = self.request("initialize", {
            "clientInfo": {"name": "superagent-spike", "title": "SuperAgent P0 Spike", "version": "0.1.0"},
            "capabilities": caps,
        })
        self.notify("initialized", {})
        self.initialize_result = resp

    def _read_stderr(self):
        for line in self.proc.stderr:
            self.stderr_lines.append(line.rstrip())
            if len(self.stderr_lines) > 2000:
                del self.stderr_lines[:1000]

    def _read_loop(self):
        for line in self.proc.stdout:
            line = line.strip()
            if not line:
                continue
            with self.cv:
                self.max_frame_seen = max(self.max_frame_seen, len(line.encode("utf-8", "replace")))
                if len(line.encode("utf-8", "replace")) > self.max_frame_bytes:
                    self.frame_violations.append(len(line))
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                with self.cv:
                    self.notifications.append({"_raw": line[:4000]})
                continue
            if "id" in msg and "method" in msg:
                if self.server_request_handler is not None:
                    try:
                        payload = self.server_request_handler(msg)
                    except Exception as exc:
                        payload = {"error": {"code": -32603, "message": f"handler failed: {exc}"}}
                else:
                    payload = {"error": {"code": -32601,
                                         "message": "spike client does not implement server requests"}}
                with self.cv:
                    self.server_requests.append(msg)
                    self.server_request_decisions.append({"method": msg.get("method"), "payload": payload})
                    reply = {"jsonrpc": "2.0", "id": msg["id"]}
                    reply.update(payload)
                    self._send(reply)
                    self.cv.notify_all()
                continue
            with self.cv:
                if "id" in msg:
                    self.responses[msg["id"]] = msg
                else:
                    self.notifications.append(msg)
                self.cv.notify_all()

    def _send(self, obj):
        with self.send_lock:
            self.proc.stdin.write(json.dumps(obj) + "\n")
            self.proc.stdin.flush()

    def notify(self, method, params):
        self._send({"jsonrpc": "2.0", "method": method, "params": params})

    def send_request(self, method, params):
        with self.lock:
            rid = self.next_id
            self.next_id += 1
        self._send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        return rid

    def wait_response(self, rid, timeout=None):
        wait_until = min(self.deadline, time.time() + (timeout or 60))
        with self.cv:
            while rid not in self.responses:
                remaining = wait_until - time.time()
                if remaining <= 0:
                    raise TimeoutError(f"response id={rid} timed out")
                self.cv.wait(remaining)
            return self.responses.pop(rid)

    def wait_notification(self, predicate, timeout=60):
        wait_until = min(self.deadline, time.time() + timeout)
        with self.cv:
            while True:
                for note in self.notifications:
                    if predicate(note):
                        return note
                remaining = wait_until - time.time()
                if remaining <= 0:
                    raise TimeoutError("notification predicate timed out")
                self.cv.wait(remaining)

    def request(self, method, params, timeout=None):
        return self.wait_response(self.send_request(method, params), timeout)

    def close(self):
        try:
            self.proc.stdin.close()
        except Exception:
            pass
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait(timeout=5)

    def transcript(self):
        with self.lock:
            return {
                "initialize": self.initialize_result,
                "notifications": list(self.notifications),
                "notificationCount": len(self.notifications),
                "serverRequests": list(self.server_requests),
                "serverRequestDecisions": list(self.server_request_decisions),
                "maxFrameBytesSeen": self.max_frame_seen,
                "frameLimitBytes": self.max_frame_bytes,
                "frameViolations": list(self.frame_violations),
                "stderrTail": self.stderr_lines[-50:],
                "exitCode": self.proc.returncode,
            }


def rollout_function_calls(home, session_id):
    """Extract function_call / function_call_output pairs from rollout JSONL."""
    calls = {}
    files = []
    for root, _dirs, names in os.walk(home):
        for fn in names:
            if not fn.endswith(".jsonl"):
                continue
            path = os.path.join(root, fn)
            try:
                content = open(path, encoding="utf-8", errors="replace").read()
            except OSError:
                continue
            if session_id not in content:
                continue
            files.append(path)
            for lineno, line in enumerate(content.splitlines(), 1):
                try:
                    rec = json.loads(line)
                except json.JSONDecodeError:
                    continue
                payload = rec.get("payload", {})
                if payload.get("type") == "function_call":
                    calls.setdefault(payload.get("call_id"), {})["call"] = {
                        "file": path, "line": lineno, "name": payload.get("name"),
                        "arguments": str(payload.get("arguments"))[:600],
                    }
                elif payload.get("type") == "function_call_output":
                    calls.setdefault(payload.get("call_id"), {})["output"] = {
                        "file": path, "line": lineno,
                        "output": str(payload.get("output"))[:800],
                    }
    return files, list(calls.values())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--engine", required=True)
    ap.add_argument("--home", required=True)
    ap.add_argument("--calls", default="[]")
    ap.add_argument("--out")
    ap.add_argument("--deadline", type=float, default=120)
    ap.add_argument("--experimental", action="store_true")
    ap.add_argument("--wait-turn-completed", action="store_true",
                    help="after each turn/start, block until turn/completed for that thread before continuing")
    args = ap.parse_args()

    calls = json.loads(args.calls)
    client = Client(args.engine, args.home, args.experimental, args.deadline)
    results = []
    error = None

    def resolve_refs(obj):
        if isinstance(obj, str) and obj.startswith("$ref:"):
            _, idx, path = obj.split(":", 2)
            cur = results[int(idx)]["response"]
            for part in path.split("."):
                cur = cur[part] if isinstance(cur, dict) else cur[int(part)]
            return cur
        if isinstance(obj, dict):
            return {k: resolve_refs(v) for k, v in obj.items()}
        if isinstance(obj, list):
            return [resolve_refs(v) for v in obj]
        return obj

    try:
        for call in calls:
            params = resolve_refs(call.get("params", {}))
            resp = client.request(call["method"], params, call.get("timeout"))
            results.append({"method": call["method"], "response": resp})
            if "error" in resp and not call.get("allowError"):
                error = f"{call['method']} returned error: {resp['error']}"
                break
            if args.wait_turn_completed and call["method"] == "turn/start" and "error" not in resp:
                thread_id = params.get("threadId")
                client.wait_notification(
                    lambda n: n.get("method") == "turn/completed"
                    and n.get("params", {}).get("threadId") == thread_id,
                    timeout=max(1.0, client.deadline - time.time()),
                )
    except Exception as exc:
        error = str(exc)
    finally:
        client.close()
    out = {"results": results, "error": error, **client.transcript()}
    text = json.dumps(out, ensure_ascii=False, indent=2, default=str)
    if args.out:
        with open(args.out, "w") as fh:
            fh.write(text)
        print(f"transcript -> {args.out} ({len(results)} calls, error={error})")
    else:
        print(text)
    sys.exit(1 if error else 0)


if __name__ == "__main__":
    main()