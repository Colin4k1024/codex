#!/usr/bin/env python3
"""P1-2 / #74 F2: genuine TUI-path persona-driven autonomous teaming probe.

Launches the interactive TUI (bare CLI over a pty, prompt prefill) with the
production pack overlay home. The user prompt describes a TASK only — it does
NOT mention spawn_agent, agent_type, role names, or "spawn all five". Whether
the leader teams up with the five persona roles is driven by the resident
persona (root_agent_usage_hint_text) alone.

Evidence collected:
  - TUI screen log (pty capture);
  - leader rollout: session_meta.originator == codex-tui (genuine TUI path),
    persona marker present in the leader's own rollout;
  - child rollouts: session_meta.agent_role / parent_thread_id;
  - post-run app-server thread/read reconciliation of every child turn status
    (a child turn that failed, e.g. modelNotFound, fails the probe).

Honesty contract: if the TUI leader does not autonomously spawn all five
persona roles with completed turns, this probe reports NOT_PROVEN / FAIL with
the observed facts. It never fabricates teaming, and a scripted stdio run
(tests/p1_2.py) is NOT accepted as proof of this TUI criterion.
"""
import errno
import fcntl
import json
import os
import pty
import select
import shutil
import signal
import struct
import sys
import tempfile
import termios
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from spike_client import Client

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ENGINE = os.environ.get("SUPERAGENT_ENGINE", f"{ROOT}/superagent/packaging/superagent-engine")
HOME_A = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p1-2"
os.makedirs(EV, exist_ok=True)
HOME = f"{EV}/home-p1-2-tui"
if os.path.exists(HOME):
    shutil.rmtree(HOME)
os.makedirs(f"{HOME}/agents", exist_ok=True)

ROLES = ["planner", "implementer", "verifier", "reviewer", "reporter"]
PERSONA_MARKER = "SuperAgent meta-agent (leader), a coordinator not an executor"
TASK_MARKER = "HELLO-P12"

# Hard wall-clock guard (pty cleanup can block if the TUI ignores signals).
signal.alarm(780)

with open(f"{HOME}/config.toml", "w") as fh:
    fh.write("# P1-2 TUI dedicated home: production pack overlay\n")
    fh.write(open(f"{ROOT}/superagent/pack/config/agents-overlay.toml").read())
for r in ROLES:
    shutil.copyfile(f"{ROOT}/superagent/pack/roles/{r}.toml", f"{HOME}/agents/{r}.toml")
if os.path.exists(f"{HOME_A}/auth.json"):
    shutil.copyfile(f"{HOME_A}/auth.json", f"{HOME}/auth.json")

WORK = tempfile.mkdtemp(prefix="p12tui-")
with open(f"{HOME}/config.toml", "a") as fh:
    fh.write(f'\n[projects."{WORK}"]\ntrust_level = "trusted"\n')

# TASK-ONLY prompt: no spawn_agent / agent_type / role names / "all five".
# The five-role composition must come from the resident persona alone.
PROMPT = (
    "Use your team to prepare everything needed to create hello.txt containing "
    "exactly HELLO-P12: a one-step plan, the exact diff to apply, a one-line "
    "acceptance checklist, a one-line review verdict, and a one-line evidence "
    "summary. Do not create or modify any files. When the team deliverables are "
    "in, list what each teammate produced."
)

started_at = time.time()
argv = [ENGINE, "--no-daemon", PROMPT]
env = dict(os.environ, CODEX_HOME=HOME, TERM="xterm-256color")
pid, fd = pty.fork()
if pid == 0:
    os.chdir(WORK)
    os.execve(ENGINE, argv, env)

fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 45, 160, 0, 0))
screen_path = f"{EV}/p1-2-tui-screen.log"
screen = open(screen_path, "wb")


def drain(timeout=0.5):
    while True:
        r, _, _ = select.select([fd], [], [], timeout)
        if not r:
            return
        try:
            data = os.read(fd, 65536)
        except OSError as exc:
            if exc.errno == errno.EIO:
                return
            raise
        if not data:
            return
        screen.write(data)
        screen.flush()


def alive():
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


time.sleep(6)
drain(1.0)
os.write(fd, b"\r")  # submit prefilled prompt (or accept a startup dialog)
time.sleep(3)
drain(1.0)
os.write(fd, b"\r")

sessions_root = f"{HOME}/sessions"


def rollout_metas():
    """Map rollout path -> first session_meta payload (file owner)."""
    out = {}
    for root, _dirs, files in os.walk(sessions_root):
        for fn in files:
            if not fn.endswith(".jsonl"):
                continue
            path = os.path.join(root, fn)
            try:
                with open(path, encoding="utf-8", errors="replace") as fh:
                    meta = json.loads(fh.readline())
                if meta.get("type") == "session_meta":
                    out[path] = meta.get("payload", {})
            except (OSError, json.JSONDecodeError):
                continue
    return out


def has_task_complete(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            for line in fh:
                if '"task_complete"' in line:
                    return True
    except OSError:
        pass
    return False


deadline = time.time() + 600
leader_file = None
leader_meta = None
leader_sid = None
child_files = {}
while time.time() < deadline and alive():
    drain(2.0)
    metas = rollout_metas()
    if leader_file is None:
        for path, m in metas.items():
            if m.get("originator") == "codex-tui" and TASK_MARKER in open(
                    path, encoding="utf-8", errors="replace").read():
                leader_file = path
                leader_meta = m
                leader_sid = m.get("id")
                break
    if leader_file:
        child_files = {}
        for path, m in metas.items():
            if path == leader_file:
                continue
            if m.get("parent_thread_id") == leader_sid and m.get("agent_role") in ROLES:
                child_files.setdefault(m.get("agent_role"), []).append(
                    {"path": path, "threadId": m.get("id"), "meta": m})
        if (len(child_files) >= 5 and has_task_complete(leader_file)
                and all(has_task_complete(c["path"])
                        for cs in child_files.values() for c in cs)):
            break
    time.sleep(3)

drain(1.0)
try:
    os.write(fd, b"/quit\r")
except OSError:
    pass
time.sleep(4)
drain(1.0)
if alive():
    try:
        os.write(fd, b"\x03")
    except OSError:
        pass
    time.sleep(2)
if alive():
    os.kill(pid, signal.SIGTERM)
    time.sleep(2)
if alive():
    os.kill(pid, signal.SIGKILL)
try:
    os.waitpid(pid, 0)
except ChildProcessError:
    pass
screen.close()

# --- Post-run reconciliation: authoritative child turn statuses -------------
child_proof = {}
leader_turn_status = None
recon = Client(ENGINE, HOME, deadline=240)
try:
    if leader_sid:
        read = recon.request("thread/read", {"threadId": leader_sid, "includeTurns": True},
                             timeout=60)
        lth = read.get("result", {}).get("thread", {})
        leader_turn_status = [t.get("status") for t in lth.get("turns") or []]
        with open(f"{EV}/p1-2-tui-leader-read.json", "w") as fh:
            json.dump(read, fh, ensure_ascii=False, indent=2, default=str)
    for role, cs in child_files.items():
        for c in cs:
            read = recon.request("thread/read", {"threadId": c["threadId"],
                                                 "includeTurns": True}, timeout=60)
            th = read.get("result", {}).get("thread", {})
            turns = th.get("turns") or []
            statuses = [t.get("status") for t in turns]
            errors = [t.get("error") for t in turns if t.get("error")]
            child_proof[c["threadId"]] = {
                "role": role, "threadId": c["threadId"],
                "model": th.get("model"), "agentNickname": th.get("agentNickname"),
                "turnStatuses": statuses, "turnErrors": errors,
                "turnsCompleted": bool(turns) and all(s == "completed" for s in statuses)
                                  and not errors,
                "rolloutFile": c["path"],
            }
            with open(f"{EV}/p1-2-tui-child-read-{role}.json", "w") as fh:
                json.dump(read, fh, ensure_ascii=False, indent=2, default=str)
finally:
    recon.close()

leader_content = (open(leader_file, encoding="utf-8", errors="replace").read()
                  if leader_file else "")
persona_in_leader = PERSONA_MARKER in leader_content
spawned_roles = sorted(child_files.keys())
spawn_ok = all(r in child_files for r in ROLES)
turns_ok = bool(child_proof) and all(v["turnsCompleted"] for v in child_proof.values())
tui_path_ok = bool(leader_meta and leader_meta.get("originator") == "codex-tui")
prompt_is_task_only = not any(k in PROMPT for k in
                              ["spawn_agent", "agent_type", "planner", "implementer",
                               "verifier", "reviewer", "reporter", "ALL FIVE"])
proven = bool(tui_path_ok and prompt_is_task_only and persona_in_leader
              and spawn_ok and turns_ok)

summary = {
    "issue": 74,
    "criterion": "TUI leader persona-driven autonomous teaming (F2)",
    "mode": "interactive TUI (bare CLI, pty, prompt prefill)",
    "launch": {"argv": [ENGINE, "--no-daemon", "<task-only prompt>"], "prompt": PROMPT,
               "cwd": WORK, "envCODEX_HOME": HOME, "startedAtEpoch": started_at},
    "promptIsTaskOnly": prompt_is_task_only,
    "leaderSessionFile": leader_file,
    "leaderOriginator": (leader_meta or {}).get("originator"),
    "leaderSessionSource": (leader_meta or {}).get("source"),
    "leaderTurnStatuses": leader_turn_status,
    "personaInLeaderRollout": persona_in_leader,
    "spawnedRoles": spawned_roles,
    "allFiveRolesSpawned": spawn_ok,
    "childProof": child_proof,
    "allChildTurnsCompleted": turns_ok,
    "tuiAutonomousTeaming": "PROVEN" if proven else "NOT_PROVEN",
    "allPass": proven,
    "screenLog": screen_path,
    "honestyNote": "PROVEN only if a genuine TUI leader (originator codex-tui), given a "
                   "task-only prompt with no spawn/role instructions, autonomously spawned "
                   "all five persona roles and every child turn completed. Otherwise "
                   "NOT_PROVEN with observed facts; never fabricated.",
}
with open(f"{EV}/p1-2-tui-conclusion.json", "w") as fh:
    json.dump(summary, fh, ensure_ascii=False, indent=2, default=str)
print(json.dumps({"allPass": proven, "tuiAutonomousTeaming": summary["tuiAutonomousTeaming"],
                  "leaderOriginator": summary["leaderOriginator"],
                  "personaInLeaderRollout": persona_in_leader,
                  "spawnedRoles": spawned_roles, "allChildTurnsCompleted": turns_ok},
                 ensure_ascii=False))
sys.exit(0 if proven else 1)