#!/usr/bin/env python3
"""P0-4b: goal budget-limit behavior + queue idle dispatch (#67)."""
import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spike_client import Client

ROOT = "/Users/fanjia/Desktop/code/codex"
EV = f"{ROOT}/superagent/evidence/p0-4"
HOME = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
ENGINE = f"{ROOT}/superagent/packaging/superagent-engine"
WORK = sys.argv[1]


def turn_completed(client, thread_id, exclude_turn_ids=(), timeout=600):
    return client.wait_notification(
        lambda n: n.get("method") == "turn/completed"
        and n.get("params", {}).get("threadId") == thread_id
        and n.get("params", {}).get("turn", {}).get("id") not in exclude_turn_ids,
        timeout=timeout,
    )


def agent_texts(client):
    return [n.get("params", {}).get("item", {}).get("text", "")
            for n in client.notifications
            if n.get("method") == "item/completed"
            and n.get("params", {}).get("item", {}).get("type") == "agentMessage"]


def budget_probe():
    c = Client(ENGINE, HOME, deadline=700)
    try:
        th = c.request("thread/start", {"cwd": WORK})["result"]["thread"]["id"]
        c.request("thread/goal/set", {"threadId": th,
                  "objective": "Budget-limit probe: write a long essay.", "tokenBudget": 500})
        c.request("turn/start", {"threadId": th, "input": [{"type": "text",
                  "text": "Write a 3000-word essay about the history of the ocean. Do not use any tools. Just write."}]})
        done = turn_completed(c, th)
        goal = c.request("thread/goal/get", {"threadId": th})["result"]["goal"]
        out = {"threadId": th, "turnStatus": done.get("params", {}).get("turn", {}).get("status"),
               "goal": goal,
               "goalNotifications": [n["method"] for n in c.notifications if "goal" in n.get("method", "")],
               "stderrTail": c.stderr_lines[-20:]}
        out["budgetLimitedObserved"] = goal.get("status") == "budgetLimited"
        out["accountingObserved"] = (goal.get("tokensUsed") or 0) >= 500
        return out
    finally:
        c.close()


def queue_probe():
    c = Client(ENGINE, HOME, experimental=True, deadline=700)
    try:
        th = c.request("thread/start", {"cwd": WORK})["result"]["thread"]["id"]
        first = c.request("turn/start", {"threadId": th, "input": [{"type": "text",
                  "text": "Use the shell tool to run 'sleep 15', then reply with exactly FIRST_TURN_DONE."}]})
        first_id = first["result"]["turn"]["id"]
        add = c.request("thread/queue/add", {"threadId": th,
                "input": [{"type": "text", "text": "Reply with exactly QUEUE_DISPATCH_OK."}],
                "clientUserMessageId": "p04b-q1"})
        listed = c.request("thread/queue/list", {"threadId": th})
        turn_completed(c, th, timeout=300)
        second = turn_completed(c, th, exclude_turn_ids=(first_id,), timeout=300)
        second_id = second.get("params", {}).get("turn", {}).get("id")
        final_list = c.request("thread/queue/list", {"threadId": th})
        texts = agent_texts(c)
        out = {"threadId": th, "firstTurnId": first_id, "secondTurnId": second_id,
               "queueAdd": add.get("result"), "queuedWhileBusy": listed.get("result"),
               "finalQueue": final_list.get("result"),
               "secondTurnStatus": second.get("params", {}).get("turn", {}).get("status"),
               "agentTexts": texts,
               "dispatchOk": any("QUEUE_DISPATCH_OK" in t for t in texts),
               "queueDrained": not (final_list.get("result", {}).get("data") or [])}
        return out
    finally:
        c.close()


def main():
    os.makedirs(EV, exist_ok=True)
    budget = budget_probe()
    json.dump(budget, open(f"{EV}/budget-limit.json", "w"), indent=2, default=str)
    queue = queue_probe()
    json.dump(queue, open(f"{EV}/queue-dispatch.json", "w"), indent=2, default=str)
    summary = {
        "budgetLimitedObserved": budget["budgetLimitedObserved"],
        "budgetAccountingObserved": budget["accountingObserved"],
        "queueIdleDispatchOk": queue["dispatchOk"] and queue["queueDrained"],
    }
    summary["allPass"] = all(summary.values())
    json.dump(summary, open(f"{EV}/p0-4b-summary.json", "w"), indent=2)
    print(json.dumps(summary, indent=2))
    sys.exit(0 if summary["allPass"] else 1)


if __name__ == "__main__":
    main()