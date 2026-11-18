#!/usr/bin/env python3
"""SuperAgent capability pack manifest tool (P1-1 / #73).

Deterministic digest export + verification + run version binding.

Usage:
  python3 superagent/tools/pack.py export [--pack DIR] [--name N] [--version V]
  python3 superagent/tools/pack.py verify [--pack DIR] [--pinned DIGEST]

Digest canonicalization: all regular files under pack/ except manifest.json,
relative POSIX paths sorted lexicographically; per-file sha256; digest =
sha256 of joined "<path>\\t<filehash>\\n" lines. Symlinks are rejected
(fail-closed). LICENSES.json must cover every file (license completeness).

Version binding: a run record pins manifest.digest (see
pack/templates/run-record.json). `verify --pinned DIGEST` fails explicitly on
mismatch; callers must never fall back to a different pack silently.
"""
import argparse
import fnmatch
import hashlib
import json
import os
import subprocess
import sys

SCHEMA_VERSION = 1
REQUIRED_FIELDS = {
    "schema_version": int,
    "name": str,
    "version": str,
    "source_commit": str,
    "digest": str,
    "file_count": int,
}
MANIFEST = "manifest.json"
LICENSES = "LICENSES.json"


def iter_files(pack):
    for root, dirs, files in os.walk(pack):
        dirs.sort()
        for name in sorted(files):
            full = os.path.join(root, name)
            rel = os.path.relpath(full, pack).replace(os.sep, "/")
            if rel == MANIFEST:
                continue
            if os.path.islink(full):
                raise ValueError(f"symlink rejected (fail-closed): {rel}")
            if not os.path.isfile(full):
                raise ValueError(f"non-regular file rejected: {rel}")
            yield rel, full


def file_sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def compute_digest(pack):
    entries = [(rel, file_sha256(full)) for rel, full in iter_files(pack)]
    entries.sort(key=lambda e: e[0])
    canonical = "".join(f"{rel}\t{sha}\n" for rel, sha in entries)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest(), entries


def check_licenses(pack, entries):
    lic_path = os.path.join(pack, LICENSES)
    if not os.path.isfile(lic_path):
        raise ValueError(f"{LICENSES} missing")
    with open(lic_path) as fh:
        lic = json.load(fh)
    groups = lic.get("licenses")
    if not isinstance(groups, list) or not groups:
        raise ValueError(f"{LICENSES}: non-empty 'licenses' list required")
    for g in groups:
        for key in ("id", "source", "paths"):
            if key not in g:
                raise ValueError(f"{LICENSES}: entry missing '{key}'")
        if not isinstance(g["paths"], list) or not g["paths"]:
            raise ValueError(f"{LICENSES}: entry paths must be a non-empty list")
    for rel, _ in entries:
        if not any(fnmatch.fnmatch(rel, pat) for g in groups for pat in g["paths"]):
            raise ValueError(f"license coverage missing for: {rel}")


def validate_manifest(man):
    for field, typ in REQUIRED_FIELDS.items():
        if field not in man:
            raise ValueError(f"manifest missing field: {field}")
        if not isinstance(man[field], typ):
            raise ValueError(f"manifest field {field} must be {typ.__name__}")
    if man["schema_version"] != SCHEMA_VERSION:
        raise ValueError(f"unsupported schema_version: {man['schema_version']}")
    if len(man["digest"]) != 64 or any(c not in "0123456789abcdef" for c in man["digest"]):
        raise ValueError("manifest digest must be 64 hex chars")


def source_commit(pack):
    try:
        r = subprocess.run(["git", "-C", pack, "rev-parse", "HEAD"],
                           capture_output=True, text=True, timeout=10)
        if r.returncode == 0:
            return r.stdout.strip()
    except Exception:
        pass
    return "unknown"


def cmd_export(args):
    pack = os.path.abspath(args.pack)
    digest, entries = compute_digest(pack)
    check_licenses(pack, entries)
    man_path = os.path.join(pack, MANIFEST)
    existing = {}
    if os.path.isfile(man_path):
        with open(man_path) as fh:
            existing = json.load(fh)
    man = {
        "schema_version": SCHEMA_VERSION,
        "name": args.name or existing.get("name", "superagent-meta-agent"),
        "version": args.version or existing.get("version", "0.1.0"),
        "source_commit": source_commit(pack),
        "digest": digest,
        "file_count": len(entries),
    }
    with open(man_path, "w") as fh:
        json.dump(man, fh, ensure_ascii=False, indent=2, sort_keys=True)
        fh.write("\n")
    print(json.dumps(man, ensure_ascii=False, sort_keys=True))
    return 0


def cmd_verify(args):
    pack = os.path.abspath(args.pack)
    man_path = os.path.join(pack, MANIFEST)
    if not os.path.isfile(man_path):
        print(f"FAIL: {MANIFEST} missing", file=sys.stderr)
        return 1
    with open(man_path) as fh:
        man = json.load(fh)
    validate_manifest(man)
    digest, entries = compute_digest(pack)
    check_licenses(pack, entries)
    if man["file_count"] != len(entries):
        raise ValueError(f"file_count mismatch: manifest {man['file_count']} vs actual {len(entries)}")
    if digest != man["digest"]:
        raise ValueError(f"digest mismatch: manifest {man['digest']} vs recomputed {digest}")
    if args.pinned and args.pinned != man["digest"]:
        raise ValueError(
            f"pinned digest mismatch: run pins {args.pinned}, pack is {man['digest']}; "
            "explicit block — never silently substitute a different pack version")
    print(f"OK: {man['name']} v{man['version']} digest={digest} files={len(entries)}")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("export", "verify"):
        p = sub.add_parser(name)
        p.add_argument("--pack", default=os.path.join(os.path.dirname(os.path.dirname(
            os.path.abspath(__file__))), "pack"))
        if name == "export":
            p.add_argument("--name")
            p.add_argument("--version")
        else:
            p.add_argument("--pinned", help="digest pinned by a run record; mismatch fails")
    args = ap.parse_args()
    try:
        return cmd_export(args) if args.cmd == "export" else cmd_verify(args)
    except (ValueError, KeyError, json.JSONDecodeError) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())