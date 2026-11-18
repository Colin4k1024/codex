#!/usr/bin/env python3
"""Snapshot user ~/.codex for the zero-change acceptance (#76).

Config surface (top-level files + skills/agents/prompts subtrees) is hashed;
bulk state (sessions/history/logs) is recorded as path+size+mtime metadata.
Usage: p1_4_snapshot.py <user-home> <out-file>
"""
import hashlib
import json
import os
import sys

CONFIG_DIRS = {"skills", "agents", "prompts"}


def main():
    home, out = sys.argv[1], sys.argv[2]
    codex = os.path.join(home, ".codex")
    hashed, meta = {}, {}
    if os.path.isdir(codex):
        for name in sorted(os.listdir(codex)):
            full = os.path.join(codex, name)
            if os.path.isfile(full) and not os.path.islink(full):
                with open(full, "rb") as fh:
                    hashed[name] = hashlib.sha256(fh.read()).hexdigest()
            elif os.path.isdir(full) and name in CONFIG_DIRS:
                for root, dirs, files in os.walk(full):
                    dirs.sort()
                    for f in sorted(files):
                        p = os.path.join(root, f)
                        if os.path.islink(p):
                            continue
                        rel = os.path.relpath(p, codex)
                        with open(p, "rb") as fh:
                            hashed[rel] = hashlib.sha256(fh.read()).hexdigest()
        for root, dirs, files in os.walk(codex):
            dirs.sort()
            for f in sorted(files):
                p = os.path.join(root, f)
                rel = os.path.relpath(p, codex)
                if rel in hashed:
                    continue
                try:
                    st = os.stat(p)
                    meta[rel] = [st.st_size, int(st.st_mtime)]
                except OSError:
                    meta[rel] = None
    with open(out + ".config.json", "w") as fh:
        json.dump(hashed, fh, sort_keys=True, indent=1)
    with open(out + ".bulk.json", "w") as fh:
        json.dump(meta, fh, sort_keys=True)
    print(f"snapshot: {len(hashed)} hashed -> {out}.config.json, "
          f"{len(meta)} metadata -> {out}.bulk.json")


if __name__ == "__main__":
    main()