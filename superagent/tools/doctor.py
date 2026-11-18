#!/usr/bin/env python3
"""SuperAgent doctor (P1-4 / #76).

Checks: engine presence/version/digest, auth presence (NEVER contents),
pack verification, product CODEX_HOME state, platform. Output is
credential-free by construction: auth.json is only stat'ed, never read.

Usage:
  python3 superagent/tools/doctor.py [--codex-home DIR] [--engine PATH]
                                     [--pack DIR] [--json]
Exit 0 iff all checks OK.
"""
import argparse
import hashlib
import json
import os
import platform
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pack as packtool

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def file_sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def run_checks(codex_home, engine, pack_dir):
    checks = []

    def add(name, ok, detail):
        checks.append({"check": name, "ok": bool(ok), "detail": detail})

    add("platform", platform.system() == "Darwin",
        f"{platform.system()} {platform.machine()}")

    if os.path.isfile(engine) and os.access(engine, os.X_OK):
        try:
            r = subprocess.run([engine, "--version"], capture_output=True,
                               text=True, timeout=30)
            version = r.stdout.strip() or r.stderr.strip()
            add("engine-version", r.returncode == 0, version)
        except (OSError, subprocess.TimeoutExpired) as exc:
            add("engine-version", False, str(exc))
        add("engine-digest", True, file_sha256(engine))
    else:
        add("engine-version", False, f"engine missing/not executable: {engine}")
        add("engine-digest", False, "engine missing")

    auth = os.path.join(codex_home, "auth.json")
    if os.path.isfile(auth):
        add("auth", os.path.getsize(auth) > 0,
            "auth.json present (contents never read or printed)")
    else:
        add("auth", False, "auth.json missing in product CODEX_HOME; "
            "copy it explicitly from your existing login (explicit-reuse mode)")

    try:
        man_path = os.path.join(pack_dir, packtool.MANIFEST)
        with open(man_path) as fh:
            man = json.load(fh)
        packtool.validate_manifest(man)
        digest, entries = packtool.compute_digest(pack_dir)
        packtool.check_licenses(pack_dir, entries)
        ok = digest == man["digest"] and man["file_count"] == len(entries)
        add("pack", ok, f"{man['name']} v{man['version']} digest={digest} "
            f"files={len(entries)}" if ok else "digest/file_count mismatch")
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as exc:
        add("pack", False, str(exc))

    install_record = os.path.join(codex_home, ".superagent", "install.json")
    if os.path.isfile(install_record):
        with open(install_record) as fh:
            rec = json.load(fh)
        add("installed-pack", True,
            f"{rec.get('pack')} v{rec.get('version')} digest={rec.get('digest')}")
    else:
        add("installed-pack", False, "no install record; run install_pack.py")

    add("codex-home-writable", os.path.isdir(codex_home) and os.access(codex_home, os.W_OK),
        codex_home)

    user_codex = os.path.join(os.path.expanduser("~"), ".codex")
    add("user-config-isolation",
        os.path.realpath(codex_home) != os.path.realpath(user_codex),
        "product CODEX_HOME is separate from ~/.codex")
    return checks


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--codex-home", default=os.path.join(
        os.path.expanduser("~"), "Library", "Application Support", "SuperAgent", "codex-home"))
    ap.add_argument("--engine", default=os.path.join(ROOT, "packaging", "superagent-engine"))
    ap.add_argument("--pack", default=os.path.join(ROOT, "pack"))
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()
    checks = run_checks(args.codex_home, args.engine, args.pack)
    all_ok = all(c["ok"] for c in checks)
    if args.json:
        print(json.dumps({"all_ok": all_ok, "checks": checks}, indent=2))
    else:
        for c in checks:
            print(f"{'OK  ' if c['ok'] else 'FAIL'} {c['check']}: {c['detail']}")
        print("ALL GREEN" if all_ok else "DOCTOR FAILED")
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())