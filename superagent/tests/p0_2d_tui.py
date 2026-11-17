#!/usr/bin/env python3
"""P0-2d (issue #65): TUI leader launch with >=2 role threads.

Launches the interactive TUI (bare CLI, prompt prefill) over a pty with the MAv2
home-b config, submits a delegation prompt, and collects rollout evidence:
leader session meta (source), spawn_agent function calls, child session files.
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
from spike_client import rollout_function_calls

ROOT = "/Users/fanjia/Desktop/code/codex"
ENGINE = f"{ROOT}/superagent/packaging/superagent-engine"
HOME_A = os.path.expanduser("~/Library/Application Support/SuperAgent/codex-home")
EV = sys.argv[1] if len(sys.argv) > 1 else f"{ROOT}/superagent/evidence/p0-2"
HOME_B = f"{EV}/home-b"
os.makedirs(EV, exist_ok=True)

WORK = tempfile.mkdtemp(prefix="p02d-")

# pre-trust the workdir so the TUI does not block on a trust dialog
cfg_path = f"{HOME_B}/config.toml"
with open(cfg_path, "a") as fh:
    fh.write(f'\n[projects."{WORK}"]\ntrust_level = "trusted"\n')

PROMPT = (
    "You are the SuperAgent leader. Delegate with the multi-agent tools: "
    "1) spawn_agent agent_type=planner task 'Produce a one-step plan to create tui.txt containing "
    "exactly TUI-TEAM-OK; do not create the file yourself'; wait_agent. "
    "2) spawn_agent agent_type=implementer task 'Create tui.txt containing exactly TUI-TEAM-OK per the plan'; "
    "wait_agent. 3) Reply DONE and quote the file content."
)

started_at = time.time()
argv = [ENGINE, PROMPT]
env = dict(os.environ, CODEX_HOME=HOME_B, TERM="xterm-256color")
pid, fd = pty.fork()
if pid == 0:
    os.chdir(WORK)
    os.execve(ENGINE, argv, env)

fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 45, 160, 0, 0))
screen_path = f"{EV}/p0-2d-tui-screen.log"
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

sessions_root = f"{HOME_B}/sessions"


def new_rollouts():
    out = []
    for root, _dirs, files in os.walk(sessions_root):
        for fn in files:
            if fn.endswith(".jsonl"):
                path = os.path.join(root, fn)
                if os.path.getmtime(path) >= started_at - 5:
                    out.append(path)
    return out


deadline = time.time() + 480
leader_file = None
leader_sid = None
spawn_calls = []
while time.time() < deadline and alive():
    drain(2.0)
    for path in new_rollouts():
        try:
            content = open(path, encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        if "TUI-TEAM-OK" in content and "spawn_agent" in content:
            leader_file = path
            first = content.splitlines()[0]
            try:
                meta = json.loads(first)
                leader_sid = meta.get("payload", {}).get("id")
            except json.JSONDecodeError:
                pass
            break
    if leader_file:
        _, calls = rollout_function_calls(HOME_B, leader_sid or "")
        spawn_calls = [c for c in calls if "spawn_agent" in str(c.get("call", {}).get("name", ""))]
        child_files = [p for p in new_rollouts() if p != leader_file]
        if len(spawn_calls) >= 2 and len(child_files) >= 2:
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

session_meta = None
if leader_file:
    try:
        session_meta = json.loads(open(leader_file, encoding="utf-8", errors="replace").readline())
    except (OSError, json.JSONDecodeError):
        pass
child_files = [p for p in new_rollouts() if p != leader_file] if leader_file else []
file_content = None
if os.path.exists(f"{WORK}/tui.txt"):
    file_content = open(f"{WORK}/tui.txt").read()

summary = {
    "launch": {"argv": [ENGINE, "<delegation prompt>"], "prompt": PROMPT, "cwd": WORK,
               "envCODEX_HOME": HOME_B, "mode": "interactive TUI (bare CLI, pty)",
               "startedAtEpoch": started_at},
    "leaderSessionFile": leader_file,
    "leaderSessionMeta": session_meta,
    "spawnCalls": spawn_calls,
    "childSessionFiles": child_files,
    "tuiTxtContent": file_content,
    "screenLog": screen_path,
}
summary["allPass"] = bool(leader_file and len(spawn_calls) >= 2 and len(child_files) >= 2)
with open(f"{EV}/p0-2d-tui-summary.json", "w") as fh:
    json.dump(summary, fh, ensure_ascii=False, indent=2, default=str)
print(json.dumps({"allPass": summary["allPass"], "leaderSessionFile": leader_file,
                  "spawnCalls": len(spawn_calls), "childSessionFiles": len(child_files),
                  "tuiTxtContent": file_content}, indent=2))
sys.exit(0 if summary["allPass"] else 1)