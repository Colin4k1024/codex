#!/usr/bin/env python3
"""P1-3 / #75 runtime acceptance probe.

Proves against the real engine:
  - team-delivery skill is discoverable after install (skills/list, MODEL-FREE);
  - skill is triggerable: a turn injects the <skill> fragment (name=team-delivery);
  - the injected fragment is user-role (NOT developer_instructions) and <=900 tokens;
  - low-trust boundary: the skill body never appears in developer_instructions.

Discovery mode (model-free, no quota):  python3 p1_3.py <ev> --discovery-only
Full mode (quota-gated turn):           python3 p1_3.py <ev>

Content-layer structure + static budget are proven by test_team_delivery.py.
Skill fragment mechanism: codex-rs/ext/skills/src/fragments.rs SkillInstructions
(role="user", kind="skills.selected_skill_instructions", markers <skill>...</skill>,
body <name>NAME</name><path>...</path>CONTENTS). Non-authorizing guardian context:
codex-rs/core/src/context/contextual_user_message.rs.
"""
import json
import os
import re
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from spike_client import Client

ROOT = os.path.dirname(os.path.dirname(HERE))
ENGINE = os.environ.get("SUPERAGENT_ENGINE", f"{ROOT}/superagent/packaging/superagent-engine")
HOME_A = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p1-3"
DISCOVERY_ONLY = "--discovery-only" in sys.argv
os.makedirs(EV, exist_ok=True)
HOME = f"{EV}/home-p1-3"
os.makedirs(HOME, exist_ok=True)

FRAGMENT_CAP = 900
SKILL_NAME = "team-delivery"
NAME_MARKER = f"<name>{SKILL_NAME}</name>"


def save(name, obj):
    with open(f"{EV}/{name}", "w") as fh:
        json.dump(obj, fh, ensure_ascii=False, indent=2, default=str)


# Place pack skills into the dedicated home's CODEX_HOME/skills (engine default
# discovery root; persistent across restarts, no extraRoots re-injection — #76).
# The installer itself is #76's deliverable; here we replicate its skills placement.
shutil.copytree(f"{ROOT}/superagent/pack/skills", f"{HOME}/skills", dirs_exist_ok=True)
with open(f"{HOME}/config.toml", "w") as fh:
    fh.write('model = "gpt-6-astra"\n')
if os.path.exists(f"{HOME_A}/auth.json"):
    shutil.copyfile(f"{HOME_A}/auth.json", f"{HOME}/auth.json")


def skill_in_listing(listing):
    for entry in listing.get("result", {}).get("data", []):
        for s in entry.get("skills", []):
            if s.get("name") == SKILL_NAME and s.get("enabled"):
                return True
    return False


# --- Part A: discovery (model-free) ---
disc = Client(ENGINE, HOME, deadline=120)
try:
    listing = disc.request("skills/list", {}, timeout=60)
    save("p1-3-skills-list.json", listing)
finally:
    disc.close()
discoverable = skill_in_listing(listing)
print(f"skill_discoverable={discoverable}")

if DISCOVERY_ONLY:
    conclusion = {"skill_discoverable": discoverable,
                  "mechanism": "pack skills installed to CODEX_HOME/skills (engine default "
                               "discovery root); skills/list lists team-delivery without extraRoots",
                  "mode": "discovery-only (model-free)",
                  "allPass": discoverable}
    save("p1-3-discovery-conclusion.json", conclusion)
    print(json.dumps(conclusion, indent=2))
    sys.exit(0 if discoverable else 1)

# --- Part B: trigger (quota-gated) ---
import tiktoken
ENC = tiktoken.get_encoding("o200k_base")
WORK = tempfile.mkdtemp(prefix="p13-")
PROMPT = (
    f"Use the ${SKILL_NAME} skill. Invoke it explicitly, then reply with the six workflow "
    "phase names it defines (intake/plan/implement/verify/review/report) and the handoff "
    "summary token cap. Do NOT spawn agents, run commands, or create files; only read the "
    "skill and summarize its workflow."
)


def is_note(method):
    return lambda n: n.get("method") == method


client = Client(ENGINE, HOME, deadline=900)
try:
    ts = client.request("thread/start", {"cwd": WORK}, timeout=60)
    leader_id = ts["result"]["thread"]["id"]
    rid = client.send_request("turn/start", {"threadId": leader_id,
                                             "input": [{"type": "text", "text": PROMPT}]})
    client.wait_notification(is_note("turn/started"), timeout=120)

    def turn_done(n):
        return (n.get("method") == "turn/completed"
                and n.get("params", {}).get("threadId") == leader_id)
    client.wait_notification(turn_done, timeout=800)
    resp = client.wait_response(rid, timeout=60)
    save("p1-3-transcript.json", {"results": [ts, resp], **client.transcript()})
finally:
    client.close()

# Scan rollout JSONL for the injected skill fragment and its role.
fragment_texts = []
fragment_roles = set()
developer_texts = []
for dirpath, _dirs, files in os.walk(HOME):
    for f in files:
        if not f.endswith(".jsonl"):
            continue
        with open(os.path.join(dirpath, f), encoding="utf-8", errors="replace") as fh:
            for line in fh:
                if NAME_MARKER not in line and "developer" not in line:
                    continue
                try:
                    rec = json.loads(line)
                except Exception:
                    continue
                blob = json.dumps(rec, ensure_ascii=False)
                role = rec.get("role") or rec.get("payload", {}).get("role")
                if NAME_MARKER in blob:
                    fragment_texts.append(blob)
                    if role:
                        fragment_roles.add(role)
                if role == "developer":
                    developer_texts.append(blob)

triggered = len(fragment_texts) > 0
fragment_tokens = max((len(ENC.encode(t)) for t in fragment_texts), default=0)
# The full rollout record wraps the fragment; measure the <skill>...</skill> span.
skill_spans = []
for t in fragment_texts:
    for m in re.finditer(r"<skill>.*?</skill>", t, re.DOTALL):
        skill_spans.append(m.group(0))
span_tokens = max((len(ENC.encode(s)) for s in skill_spans), default=fragment_tokens)
role_user = ("user" in fragment_roles) and ("developer" not in fragment_roles)
no_dev_elevation = not any(NAME_MARKER in d for d in developer_texts)
within_budget = span_tokens <= FRAGMENT_CAP

conclusion = {
    "skill_discoverable": discoverable,
    "skill_triggered": triggered,
    "fragment_roles": sorted(fragment_roles),
    "fragment_role_user_not_developer": role_user,
    "skill_span_tokens_o200k": span_tokens,
    "fragment_within_budget_900": within_budget,
    "no_developer_elevation": no_dev_elevation,
    "mechanism": "SkillInstructions fragment role=user kind=skills.selected_skill_instructions "
                 "(ext/skills/src/fragments.rs); non-authorizing guardian context "
                 "(core/src/context/contextual_user_message.rs); low-trust skill body never "
                 "in developer_instructions",
    "allPass": bool(discoverable and triggered and role_user and within_budget and no_dev_elevation),
}
save("p1-3-runtime-conclusion.json", conclusion)
print(json.dumps(conclusion, indent=2))
sys.exit(0 if conclusion["allPass"] else 1)