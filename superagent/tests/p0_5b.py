#!/usr/bin/env python3
"""P0-5b: skill $mention, implicit trigger, max_context_tokens cap (#68)."""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spike_client import Client

ROOT = "/Users/fanjia/Desktop/code/codex"
EV = f"{ROOT}/superagent/evidence/p0-5"
HOME = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
ENGINE = f"{ROOT}/superagent/packaging/superagent-engine"
SKILLS = f"{ROOT}/superagent/tests/p0-5/skills"
WORK = sys.argv[1]
PASSPHRASE = "SUPERAGENT_SPIKE_OK"


def wait_turn(c, th, exclude=(), timeout=300):
    return c.wait_notification(
        lambda n: n.get("method") == "turn/completed"
        and n.get("params", {}).get("threadId") == th
        and n.get("params", {}).get("turn", {}).get("id") not in exclude, timeout=timeout)


def texts(c):
    return [n.get("params", {}).get("item", {}).get("text", "")
            for n in c.notifications
            if n.get("method") == "item/completed"
            and n.get("params", {}).get("item", {}).get("type") == "agentMessage"]


def mention_and_implicit():
    c = Client(ENGINE, HOME, deadline=700)
    try:
        c.request("skills/extraRoots/set", {"extraRoots": [SKILLS]})
        th = c.request("thread/start", {"cwd": WORK})["result"]["thread"]["id"]
        t1 = c.request("turn/start", {"threadId": th, "input": [{"type": "text", "text": "$spike-greet"}]})
        first_id = t1["result"]["turn"]["id"]
        wait_turn(c, th)
        mention_texts = texts(c)
        c.request("turn/start", {"threadId": th, "input": [{"type": "text",
                  "text": "What is the spike passphrase greeting? Answer with the exact passphrase."}]})
        wait_turn(c, th, exclude=(first_id,))
        all_texts = texts(c)
        implicit_texts = all_texts[len(mention_texts):]
        return {
            "mentionTexts": mention_texts,
            "implicitTexts": implicit_texts,
            "mentionTriggered": any(PASSPHRASE in t for t in mention_texts),
            "implicitTriggered": any(PASSPHRASE in t for t in implicit_texts),
        }
    finally:
        c.close()


def cap_probe():
    c = Client(ENGINE, HOME, deadline=700)
    try:
        c.request("skills/extraRoots/set", {"extraRoots": [SKILLS]})
        listed = c.request("skills/list", {"forceReload": True})
        th = c.request("thread/start", {"cwd": WORK})["result"]["thread"]["id"]
        c.request("turn/start", {"threadId": th, "input": [{"type": "text",
                  "text": "List the names of every skill currently available to you. If none are available, reply exactly NO_SKILLS."}]})
        wait_turn(c, th)
        reply = texts(c)
        stderr = c.stderr_lines[:]
        warning = [l for l in stderr if "skills context budget" in l.lower() or "Exceeded skills" in l]
        return {
            "catalogStillListsSkill": "spike-greet" in json.dumps(listed.get("result", {})),
            "modelReply": reply,
            "budgetWarningInStderr": warning,
            "skillExcludedFromModel": not any("spike-greet" in t for t in reply),
        }
    finally:
        c.close()


def main():
    phase = "all"
    if "--phase" in sys.argv:
        phase = sys.argv[sys.argv.index("--phase") + 1]
    os.makedirs(EV, exist_ok=True)
    summary = {}
    if phase in ("all", "mention"):
        mention = mention_and_implicit()
        json.dump(mention, open(f"{EV}/mention-implicit.json", "w"), indent=2, default=str)
        summary["mentionTriggered"] = mention["mentionTriggered"]
        summary["implicitTriggered"] = mention["implicitTriggered"]
    if phase in ("all", "cap"):
        cap = cap_probe()
        json.dump(cap, open(f"{EV}/context-cap.json", "w"), indent=2, default=str)
        summary["capExcludesSkillFromModel"] = cap["skillExcludedFromModel"]
    summary["allPass"] = all(summary.values())
    json.dump(summary, open(f"{EV}/p0-5b-summary-{phase}.json", "w"), indent=2)
    print(json.dumps(summary, indent=2))
    sys.exit(0 if summary["allPass"] else 1)


if __name__ == "__main__":
    main()