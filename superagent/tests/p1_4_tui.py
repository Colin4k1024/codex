#!/usr/bin/env python3
"""P1-4 (#76) TUI launch probe: start bin/superagent in a real pty (controlling
terminal + window size), answer terminal capability queries, verify the TUI
renders its composer, exit cleanly. No model turn is started.

Usage: p1_4_tui.py <screen-log-path>
"""
import fcntl
import os
import pty
import re
import select
import signal
import struct
import sys
import tempfile
import termios
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
LAUNCHER = os.path.join(ROOT, "superagent", "bin", "superagent")
MARKERS = [b"Ask Codex to do anything", b"OpenAI Codex"]


def respond(data):
    out = b""
    if b"\x1b[c" in data or b"\x1b[0c" in data:
        out += b"\x1b[?62;22c"
    if b"\x1b[?u" in data:
        out += b"\x1b[?0u"
    if b"]10;?" in data:
        out += b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\"
    if b"]11;?" in data:
        out += b"\x1b]11;rgb:0000/0000/0000\x1b\\"
    for m in re.finditer(rb"\x1b\[\?(\d+)\$p", data):
        out += b"\x1b[?" + m.group(1) + b";2$y"
    return out


def main():
    out_path = sys.argv[1] if len(sys.argv) > 1 else "/tmp/p1-4-tui-screen.log"
    work = tempfile.mkdtemp(prefix="p1-4-tui-")
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(work)
        env = dict(os.environ, TERM="xterm-256color")
        os.execve(LAUNCHER, [LAUNCHER], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 45, 160, 0, 0))
    buf = b""
    deadline = time.time() + 30
    rendered = False
    while time.time() < deadline:
        r, _, _ = select.select([fd], [], [], 0.5)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            buf += chunk
            resp = respond(chunk)
            if resp:
                os.write(fd, resp)
            if any(m in buf for m in MARKERS):
                rendered = True
                break
        try:
            os.kill(pid, 0)
        except OSError:
            break
    time.sleep(1)
    while True:
        r, _, _ = select.select([fd], [], [], 0.5)
        if not r:
            break
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            break
        if not chunk:
            break
        buf += chunk
    try:
        os.write(fd, b"/quit\r")
    except OSError:
        pass
    time.sleep(3)
    while True:
        r, _, _ = select.select([fd], [], [], 0.5)
        if not r:
            break
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            break
        if not chunk:
            break
        buf += chunk
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.kill(pid, 0)
        except OSError:
            break
        try:
            os.kill(pid, sig)
        except OSError:
            break
        time.sleep(2)
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    os.close(fd)
    with open(out_path, "wb") as fh:
        fh.write(buf)
    error_screen = b"Error: this CLI has no complete local package" in buf
    ok = rendered and not error_screen
    print(f"tui_rendered={rendered} error_screen={error_screen} bytes={len(buf)}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())