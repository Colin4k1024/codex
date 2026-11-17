#!/usr/bin/env python3
"""P0-3b: external message-board MCP caller-bound identity probe (#66).

Authorized member reads succeed; non-member and cross-board callers are denied.
Usage: python3 p0_3b_mcp.py --url http://127.0.0.1:7799/mcp --token TOKEN \
    --member-thread LEADER_ID --foreign-thread OTHER_SESSION_LEADER_ID --out probe.json
"""
import argparse
import json
import urllib.request
import uuid


class McpHttp:
    def __init__(self, url, token):
        self.url = url
        self.token = token
        self.session = None
        self.next_id = 1

    def post(self, obj):
        headers = {
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
            "Authorization": f"Bearer {self.token}",
        }
        if self.session:
            headers["Mcp-Session-Id"] = self.session
        req = urllib.request.Request(self.url, data=json.dumps(obj).encode(), headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                sid = resp.headers.get("Mcp-Session-Id")
                if sid:
                    self.session = sid
                body = resp.read().decode()
                ctype = resp.headers.get("Content-Type", "")
                return resp.status, self.parse(body, ctype)
        except urllib.error.HTTPError as exc:
            return exc.code, exc.read().decode()[:2000]

    @staticmethod
    def parse(body, ctype):
        if "text/event-stream" in ctype:
            for line in body.splitlines():
                if line.startswith("data:"):
                    try:
                        return json.loads(line[5:].strip())
                    except json.JSONDecodeError:
                        continue
            return {"_raw": body[:2000]}
        try:
            return json.loads(body)
        except json.JSONDecodeError:
            return {"_raw": body[:2000]}

    def request(self, method, params):
        rid = self.next_id
        self.next_id += 1
        status, body = self.post({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        return {"httpStatus": status, "body": body}

    def notify(self, method, params):
        status, body = self.post({"jsonrpc": "2.0", "method": method, "params": params})
        return {"httpStatus": status, "body": body}

    def call_tool(self, name, arguments, thread_id, session_id):
        return self.request("tools/call", {
            "name": name,
            "arguments": arguments,
            "_meta": {"threadId": thread_id, "sessionId": session_id},
        })


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", required=True)
    ap.add_argument("--token", required=True)
    ap.add_argument("--member-thread", required=True)
    ap.add_argument("--foreign-thread", required=True)
    ap.add_argument("--channel", default="run-spike-p03")
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    client = McpHttp(args.url, args.token)
    trace = {}
    trace["initialize"] = client.request("initialize", {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "p0-3b-probe", "version": "0"},
    })
    trace["initialized"] = client.notify("notifications/initialized", {})
    trace["toolsList"] = client.request("tools/list", {})

    member = args.member_thread
    trace["memberGetChannels"] = client.call_tool("get_channels", {}, member, member)
    trace["memberReadThread"] = client.call_tool(
        "read_thread", {"channel_name": args.channel}, member, member)

    stranger = str(uuid.uuid4())
    trace["nonMemberDenied"] = client.call_tool("get_channels", {}, stranger, member)
    trace["crossBoardDenied"] = client.call_tool(
        "read_thread", {"channel_name": args.channel}, args.foreign_thread, member)
    trace["missingMetaDenied"] = client.request("tools/call", {
        "name": "get_channels", "arguments": {},
    })

    def ok(entry):
        body = entry.get("body", {})
        return entry.get("httpStatus") == 200 and isinstance(body, dict) and "result" in body

    def denied(entry):
        body = entry.get("body", {})
        if entry.get("httpStatus") != 200:
            return True
        if isinstance(body, dict):
            if "error" in body:
                return True
            result = body.get("result", {})
            if result.get("isError"):
                return True
        return False

    summary = {
        "memberReadOk": ok(trace["memberGetChannels"]) and ok(trace["memberReadThread"]),
        "nonMemberDenied": denied(trace["nonMemberDenied"]),
        "crossBoardDenied": denied(trace["crossBoardDenied"]),
        "missingMetaDenied": denied(trace["missingMetaDenied"]),
    }
    summary["allPass"] = all(summary.values())
    out = {"summary": summary, "trace": trace}
    with open(args.out, "w") as fh:
        json.dump(out, fh, indent=2, ensure_ascii=False, default=str)
    print(json.dumps(summary, indent=2))
    raise SystemExit(0 if summary["allPass"] else 1)


if __name__ == "__main__":
    main()