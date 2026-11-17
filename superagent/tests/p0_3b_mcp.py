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


def structured(entry):
    body = entry.get("body", {})
    if not isinstance(body, dict):
        return None
    result = body.get("result", {})
    if not isinstance(result, dict):
        return None
    sc = result.get("structuredContent")
    if isinstance(sc, dict):
        return sc
    for c in result.get("content", []):
        if c.get("type") == "text":
            try:
                v = json.loads(c["text"])
                if isinstance(v, dict):
                    return v
            except (json.JSONDecodeError, KeyError):
                pass
    return None


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
    channels = (structured(trace["memberGetChannels"]) or {}).get("results", [])
    chan = next((c for c in channels if c.get("channel_name") == args.channel), {})
    last_msg = chan.get("last_message_id")

    trace["memberListThreads"] = client.call_tool(
        "list_threads", {"channel_name": args.channel}, member, member)
    threads = (structured(trace["memberListThreads"]) or {}).get("results", [])
    thread_id = threads[0].get("thread_id") or threads[0].get("id") if threads else None

    trace["memberReadThread"] = client.call_tool(
        "read_thread", {"thread_id": thread_id}, member, member) if thread_id else \
        {"httpStatus": 0, "body": {"error": {"message": "no thread_id from list_threads"}}}
    trace["memberReadPost"] = client.call_tool(
        "read_post", {"message_id": last_msg}, member, member) if last_msg else \
        {"httpStatus": 0, "body": {"error": {"message": "no last_message_id"}}}

    write_text = (f"P03B-EXTERNAL-WRITE-OK reply-to={last_msg} "
                  "Bounded summary: external caller-bound write probe; one reference, under 200 tokens.")
    trace["memberPost"] = client.call_tool(
        "post", {"channel_name": args.channel, "text": write_text}, member, member)
    post_sc = structured(trace["memberPost"]) or {}
    new_msg = post_sc.get("message_id") or post_sc.get("id")
    trace["memberPostReadback"] = client.call_tool(
        "read_post", {"message_id": new_msg}, member, member) if new_msg else \
        {"httpStatus": 0, "body": {"error": {"message": "post returned no message_id"}}}

    trace["memberSubscribe"] = client.call_tool(
        "subscribe", {"channel_name": args.channel}, member, member)

    stranger = str(uuid.uuid4())
    trace["nonMemberDenied"] = client.call_tool("get_channels", {}, stranger, member)
    trace["crossBoardDenied"] = client.call_tool(
        "read_thread", {"thread_id": thread_id or "unknown"}, args.foreign_thread, member)
    trace["foreignWriteDenied"] = client.call_tool(
        "post", {"channel_name": args.channel, "text": "P03B-FOREIGN-WRITE-SHOULD-NOT-LAND"},
        args.foreign_thread, member)
    trace["missingMetaDenied"] = client.request("tools/call", {
        "name": "get_channels", "arguments": {},
    })

    def ok(entry):
        body = entry.get("body", {})
        if entry.get("httpStatus") != 200 or not isinstance(body, dict):
            return False
        if "error" in body:
            return False
        result = body.get("result")
        return isinstance(result, dict) and not result.get("isError", False)

    def denied(entry):
        body = entry.get("body", {})
        if entry.get("httpStatus") != 200:
            return True
        if isinstance(body, dict):
            if "error" in body:
                return True
            result = body.get("result", {})
            if isinstance(result, dict) and result.get("isError"):
                return True
        return False

    readback = structured(trace["memberPostReadback"]) or {}
    readback_posts = readback.get("results") or readback.get("posts") or []
    readback_text = json.dumps(readback, ensure_ascii=False)
    write_landed = ok(trace["memberPost"]) and ok(trace["memberPostReadback"]) \
        and "P03B-EXTERNAL-WRITE-OK" in readback_text
    foreign_absent = "P03B-FOREIGN-WRITE-SHOULD-NOT-LAND" not in readback_text

    summary = {
        "memberReadOk": ok(trace["memberGetChannels"]) and ok(trace["memberListThreads"])
                        and ok(trace["memberReadThread"]) and ok(trace["memberReadPost"]),
        "memberWriteOk": write_landed,
        "subscribeConfirmed": ok(trace["memberSubscribe"]),
        "nonMemberDenied": denied(trace["nonMemberDenied"]),
        "crossBoardDenied": denied(trace["crossBoardDenied"]),
        "foreignWriteDenied": denied(trace["foreignWriteDenied"]) and foreign_absent,
        "missingMetaDenied": denied(trace["missingMetaDenied"]),
        "crossBoardDenialReason": (trace["crossBoardDenied"].get("body", {}) or {}).get("error", {}).get("message")
            if isinstance(trace["crossBoardDenied"].get("body"), dict) else None,
        "channelBefore": chan,
        "newMessageId": new_msg,
    }
    summary["allPass"] = all(v for k, v in summary.items()
                             if k not in ("crossBoardDenialReason", "channelBefore", "newMessageId"))
    out = {"summary": summary, "trace": trace}
    with open(args.out, "w") as fh:
        json.dump(out, fh, indent=2, ensure_ascii=False, default=str)
    print(json.dumps(summary, indent=2))
    raise SystemExit(0 if summary["allPass"] else 1)


if __name__ == "__main__":
    main()