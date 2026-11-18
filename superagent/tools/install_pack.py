#!/usr/bin/env python3
"""SuperAgent capability pack installer (P1-4 / #76).

Installs a verified pack into the product CODEX_HOME:
  pack/skills/*    -> <codex-home>/skills/*      (engine default discovery root,
                      loaded from disk every session — restart-persistent)
  pack/roles/*     -> <codex-home>/agents/*
  everything else  -> <codex-home>/.superagent/pack/*

Safety:
  - pack verified first (manifest schema + digest + license coverage, #73)
  - path traversal rejected: absolute paths, '..' components, symlinks,
    non-regular files (fail-closed, for both directory and .tar.gz packs)
  - whole-pack all-or-nothing swap: fully staged and re-verified before any
    target is touched; each old item moved aside, new renamed in; if any item
    fails, every already-swapped item is rolled back to its original state
  - never writes to the user's ~/.codex (target guard)

Usage:
  python3 superagent/tools/install_pack.py [--pack DIR_OR_TAR] [--codex-home DIR]
                                           [--pinned DIGEST]
"""
import argparse
import json
import os
import shutil
import sys
import tarfile
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pack as packtool


def is_safe_install_target(codex_home, user_home):
    """Reject the user's real ~/.codex (and anything inside it) as target."""
    target = os.path.realpath(codex_home)
    user_codex = os.path.realpath(os.path.join(user_home, ".codex"))
    return target != user_codex and not target.startswith(user_codex + os.sep)


def check_relpath(rel):
    if os.path.isabs(rel) or rel.startswith("/"):
        raise ValueError(f"path traversal rejected (absolute): {rel}")
    parts = rel.replace("\\", "/").split("/")
    if ".." in parts:
        raise ValueError(f"path traversal rejected ('..'): {rel}")
    if not rel or parts[0] in ("", "."):
        raise ValueError(f"invalid pack member path: {rel}")


def extract_tar(tar_path, dest):
    with tarfile.open(tar_path, "r:gz") as tf:
        for member in tf.getmembers():
            if member.name in (".", "./"):
                continue
            if member.name.startswith("./"):
                member.name = member.name[2:]
            check_relpath(member.name)
            if member.issym() or member.islnk():
                raise ValueError(f"symlink/hardlink rejected (fail-closed): {member.name}")
            if not (member.isfile() or member.isdir()):
                raise ValueError(f"non-regular member rejected: {member.name}")
        if sys.version_info >= (3, 12):
            tf.extractall(dest, filter="data")
        else:
            # Python < 3.12 has no data filter; the member pre-validation above
            # already rejects absolute/'..' paths, symlinks, hardlinks and
            # non-regular members, providing the same guarantees. Tar path
            # minimum recommended Python is 3.12; older versions use this fallback.
            tf.extractall(dest)


def stage_pack(pack_arg, staging):
    """Materialize the pack into staging/; return the pack root inside it."""
    if os.path.isdir(pack_arg):
        src = os.path.abspath(pack_arg)
        dst = os.path.join(staging, "pack")
        shutil.copytree(src, dst, symlinks=False)
        # copytree(symlinks=False) copies link TARGETS; reject links explicitly
        for root, dirs, files in os.walk(src):
            for name in dirs + files:
                if os.path.islink(os.path.join(root, name)):
                    raise ValueError(f"symlink rejected (fail-closed): {name}")
        return dst
    if pack_arg.endswith(".tar.gz") and os.path.isfile(pack_arg):
        dst = os.path.join(staging, "pack")
        os.makedirs(dst)
        extract_tar(pack_arg, dst)
        return dst
    raise ValueError(f"pack must be a directory or .tar.gz: {pack_arg}")


def verify_pack_dir(pack_dir, pinned=None):
    man_path = os.path.join(pack_dir, packtool.MANIFEST)
    if not os.path.isfile(man_path):
        raise ValueError(f"{packtool.MANIFEST} missing")
    with open(man_path) as fh:
        man = json.load(fh)
    packtool.validate_manifest(man)
    digest, entries = packtool.compute_digest(pack_dir)
    packtool.check_licenses(pack_dir, entries)
    if man["file_count"] != len(entries):
        raise ValueError("file_count mismatch")
    if digest != man["digest"]:
        raise ValueError(f"digest mismatch: manifest {man['digest']} vs {digest}")
    if pinned and pinned != man["digest"]:
        raise ValueError(f"pinned digest mismatch: run pins {pinned}, pack is {man['digest']}")
    return man


def swap_item(staged, target):
    """Move staged into target, moving any existing target aside. Returns the
    backup path (or None). The caller owns backup cleanup/rollback so the whole
    pack can be swapped all-or-nothing across items."""
    parent = os.path.dirname(target)
    os.makedirs(parent, exist_ok=True)
    backup = None
    if os.path.exists(target):
        backup = target + f".old-{os.getpid()}"
        os.rename(target, backup)
    try:
        os.rename(staged, target)
    except OSError:
        if backup is not None:
            os.rename(backup, target)
        raise
    return backup


def install(pack_arg, codex_home, pinned=None, user_home=None):
    user_home = user_home or os.path.expanduser("~")
    if not is_safe_install_target(codex_home, user_home):
        raise ValueError(f"refusing to install into user config dir: {codex_home}")
    codex_home = os.path.abspath(codex_home)
    os.makedirs(codex_home, exist_ok=True)
    staging = tempfile.mkdtemp(prefix=".staging-", dir=codex_home)
    try:
        pack_dir = stage_pack(pack_arg, staging)
        man = verify_pack_dir(pack_dir, pinned)

        installed = []
        for rel, _ in packtool.iter_files(pack_dir):
            top = rel.split("/", 1)[0]
            if top == "skills" and "/" in rel:
                target_rel = rel
            elif top == "roles" and "/" in rel:
                target_rel = "agents/" + rel.split("/", 1)[1]
            else:
                target_rel = ".superagent/pack/" + rel
            check_relpath(target_rel)
            installed.append((rel, target_rel))

        # Swap per top-level item (skill dir / agent file / pack subtree).
        items = {}
        for rel, target_rel in installed:
            parts = target_rel.split("/")
            if parts[0] in ("skills", "agents"):
                item = "/".join(parts[:2])
            else:
                item = ".superagent/pack"
            items.setdefault(item, None)
        # Whole-pack all-or-nothing swap: keep every backup until all items
        # succeed; on any failure restore all already-swapped items.
        swaps = []
        try:
            for item in sorted(items):
                staged_item = os.path.join(pack_dir, item)
                if item.startswith("agents/"):
                    staged_item = os.path.join(pack_dir, "roles", item.split("/", 1)[1])
                elif item == ".superagent/pack":
                    # stage the non-skills/roles remainder
                    rem = os.path.join(staging, "remainder")
                    os.makedirs(rem, exist_ok=True)
                    for rel, _ in packtool.iter_files(pack_dir):
                        top = rel.split("/", 1)[0]
                        if top in ("skills", "roles"):
                            continue
                        dst = os.path.join(rem, rel)
                        os.makedirs(os.path.dirname(dst), exist_ok=True)
                        shutil.copy2(os.path.join(pack_dir, rel), dst)
                    staged_item = rem
                target = os.path.join(codex_home, item)
                backup = swap_item(staged_item, target)
                swaps.append((target, backup))
        except OSError:
            for target, backup in reversed(swaps):
                if os.path.isdir(target):
                    shutil.rmtree(target, ignore_errors=True)
                elif os.path.lexists(target):
                    os.remove(target)
                if backup is not None:
                    os.rename(backup, target)
                else:
                    try:
                        os.rmdir(os.path.dirname(target))
                    except OSError:
                        pass
            raise
        else:
            for _target, backup in swaps:
                if backup is None:
                    continue
                if os.path.isdir(backup):
                    shutil.rmtree(backup)
                else:
                    os.remove(backup)

        record = {
            "pack": man["name"],
            "version": man["version"],
            "digest": man["digest"],
            "file_count": man["file_count"],
            "installed_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "codex_home": codex_home,
        }
        meta_dir = os.path.join(codex_home, ".superagent")
        os.makedirs(meta_dir, exist_ok=True)
        with open(os.path.join(meta_dir, "install.json"), "w") as fh:
            json.dump(record, fh, indent=2, sort_keys=True)
            fh.write("\n")
        return record
    finally:
        shutil.rmtree(staging, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    default_pack = os.path.join(os.path.dirname(os.path.dirname(
        os.path.abspath(__file__))), "pack")
    ap.add_argument("--pack", default=default_pack)
    ap.add_argument("--codex-home", default=os.path.join(
        os.path.expanduser("~"), "Library", "Application Support", "SuperAgent", "codex-home"))
    ap.add_argument("--pinned")
    args = ap.parse_args()
    try:
        record = install(args.pack, args.codex_home, args.pinned)
    except (ValueError, KeyError, json.JSONDecodeError, OSError) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1
    print(f"OK: installed {record['pack']} v{record['version']} "
          f"digest={record['digest']} -> {record['codex_home']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())