#!/usr/bin/env python3
"""P1-4 (#76) installer/doctor acceptance tests. Model-free, temp dirs only."""
import argparse
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.join(os.path.dirname(HERE), "tools")
sys.path.insert(0, TOOLS)
import pack as packtool
import install_pack
import doctor

SKILL_MD = "---\nname: p1-4-greet\ndescription: installer fixture skill\n---\n\n# Greet\nSay hello.\n"


def make_pack(root, name="fixture-pack"):
    pack = os.path.join(root, name)
    os.makedirs(os.path.join(pack, "skills", "p1-4-greet"))
    with open(os.path.join(pack, "skills", "p1-4-greet", "SKILL.md"), "w") as fh:
        fh.write(SKILL_MD)
    os.makedirs(os.path.join(pack, "roles"))
    with open(os.path.join(pack, "roles", "leader.md"), "w") as fh:
        fh.write("# leader role\n")
    os.makedirs(os.path.join(pack, "workflows"))
    with open(os.path.join(pack, "workflows", "README.md"), "w") as fh:
        fh.write("# workflows\n")
    with open(os.path.join(pack, "LICENSES.json"), "w") as fh:
        json.dump({"licenses": [{"id": "Apache-2.0", "source": "original",
                                 "paths": ["**"]}]}, fh)
    packtool.cmd_export(argparse.Namespace(pack=pack, name=name, version="0.1.0"))
    return pack


def tree_state(root):
    state = {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames.sort()
        for f in sorted(filenames):
            full = os.path.join(dirpath, f)
            rel = os.path.relpath(full, root)
            with open(full, "rb") as fh:
                state[rel] = hashlib.sha256(fh.read()).hexdigest()
    return state


def make_tar(pack, tar_path, extra=None):
    with tarfile.open(tar_path, "w:gz") as tf:
        tf.add(pack, arcname=".")
        if extra:
            for name, data, typ in extra:
                info = tarfile.TarInfo(name)
                info.size = len(data) if data else 0
                info.type = typ
                if typ == tarfile.SYMTYPE:
                    info.linkname = data.decode()
                    tf.addfile(info)
                else:
                    tf.addfile(info, io.BytesIO(data))


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="p1-4-test-")
        self.home = os.path.join(self.tmp, "codex-home")
        self.pack = make_pack(self.tmp)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_install_dir_pack(self):
        rec = install_pack.install(self.pack, self.home)
        self.assertTrue(os.path.isfile(os.path.join(self.home, "skills", "p1-4-greet", "SKILL.md")))
        self.assertTrue(os.path.isfile(os.path.join(self.home, "agents", "leader.md")))
        self.assertTrue(os.path.isfile(os.path.join(self.home, ".superagent", "pack", "workflows", "README.md")))
        with open(os.path.join(self.home, ".superagent", "install.json")) as fh:
            saved = json.load(fh)
        self.assertEqual(saved["digest"], rec["digest"])
        leftovers = [d for d in os.listdir(self.home) if d.startswith((".staging-", ".old-"))]
        self.assertEqual(leftovers, [])

    def test_install_tar_pack(self):
        tar = os.path.join(self.tmp, "pack.tar.gz")
        make_tar(self.pack, tar)
        install_pack.install(tar, self.home)
        self.assertTrue(os.path.isfile(os.path.join(self.home, "skills", "p1-4-greet", "SKILL.md")))

    def test_tar_traversal_rejected(self):
        tar = os.path.join(self.tmp, "evil.tar.gz")
        make_tar(self.pack, tar, extra=[("../evil.txt", b"pwned", tarfile.REGTYPE)])
        with self.assertRaises(ValueError):
            install_pack.install(tar, self.home)
        self.assertFalse(os.path.exists(os.path.join(self.tmp, "evil.txt")))
        self.assertFalse(os.path.exists(os.path.join(self.home, "skills")))

    def test_tar_absolute_path_rejected(self):
        tar = os.path.join(self.tmp, "abs.tar.gz")
        make_tar(self.pack, tar, extra=[("/tmp/p1-4-evil", b"x", tarfile.REGTYPE)])
        with self.assertRaises(ValueError):
            install_pack.install(tar, self.home)

    def test_tar_symlink_rejected(self):
        tar = os.path.join(self.tmp, "sym.tar.gz")
        make_tar(self.pack, tar, extra=[("skills/evil-link", b"/etc/passwd", tarfile.SYMTYPE)])
        with self.assertRaises(ValueError):
            install_pack.install(tar, self.home)
        self.assertFalse(os.path.exists(os.path.join(self.home, "skills")))

    def test_dir_symlink_rejected(self):
        os.symlink("/etc/passwd", os.path.join(self.pack, "skills", "p1-4-greet", "link"))
        with self.assertRaises(ValueError):
            install_pack.install(self.pack, self.home)

    def test_tampered_pack_rejected_before_write(self):
        with open(os.path.join(self.pack, "skills", "p1-4-greet", "SKILL.md"), "a") as fh:
            fh.write("tampered\n")
        with self.assertRaises(ValueError):
            install_pack.install(self.pack, self.home)
        self.assertFalse(os.path.exists(os.path.join(self.home, "skills")))

    def test_pinned_mismatch_rejected(self):
        with self.assertRaises(ValueError):
            install_pack.install(self.pack, self.home, pinned="0" * 64)
        self.assertFalse(os.path.exists(os.path.join(self.home, "skills")))

    def test_user_codex_guard(self):
        user_home = os.path.join(self.tmp, "user")
        os.makedirs(os.path.join(user_home, ".codex"))
        self.assertFalse(install_pack.is_safe_install_target(
            os.path.join(user_home, ".codex"), user_home))
        self.assertFalse(install_pack.is_safe_install_target(
            os.path.join(user_home, ".codex", "sub"), user_home))
        self.assertTrue(install_pack.is_safe_install_target(self.home, user_home))
        with self.assertRaises(ValueError):
            install_pack.install(self.pack, os.path.join(user_home, ".codex"),
                                 user_home=user_home)

    def test_reinstall_idempotent(self):
        install_pack.install(self.pack, self.home)
        first = tree_state(self.home)
        install_pack.install(self.pack, self.home)
        second = tree_state(self.home)
        self.assertEqual({k: v for k, v in first.items() if k != ".superagent/install.json"},
                         {k: v for k, v in second.items() if k != ".superagent/install.json"})
        self.assertFalse(any(".old-" in k for k in second))

    def test_user_config_untouched(self):
        user_home = os.path.join(self.tmp, "user")
        user_codex = os.path.join(user_home, ".codex")
        os.makedirs(user_codex)
        for name, content in [("config.toml", b"model = \"x\"\n"),
                              ("auth.json", b"SECRET-USER-TOKEN"),
                              ("AGENTS.md", b"# user agents\n")]:
            with open(os.path.join(user_codex, name), "wb") as fh:
                fh.write(content)
        before = tree_state(user_codex)
        install_pack.install(self.pack, self.home, user_home=user_home)
        self.assertEqual(tree_state(user_codex), before)


class DoctorTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="p1-4-doctor-")
        self.home = os.path.join(self.tmp, "codex-home")
        self.pack = make_pack(self.tmp)
        install_pack.install(self.pack, self.home)
        self.engine = os.path.join(self.tmp, "stub-engine")
        with open(self.engine, "w") as fh:
            fh.write("#!/bin/bash\necho 'stub-engine 0.0.0-test'\n")
        os.chmod(self.engine, 0o755)
        with open(os.path.join(self.home, "auth.json"), "w") as fh:
            fh.write('{"SECRET_TOKEN": "sk-DOCTOR-MUST-NOT-LEAK-12345"}')

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_doctor(self, *extra):
        return subprocess.run(
            [sys.executable, os.path.join(TOOLS, "doctor.py"),
             "--codex-home", self.home, "--engine", self.engine,
             "--pack", self.pack, *extra],
            capture_output=True, text=True, timeout=60)

    def test_doctor_all_green_and_credential_free(self):
        r = self.run_doctor()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("ALL GREEN", r.stdout)
        self.assertNotIn("DOCTOR-MUST-NOT-LEAK", r.stdout + r.stderr)
        self.assertNotIn("SECRET_TOKEN", r.stdout + r.stderr)
        rj = self.run_doctor("--json")
        self.assertEqual(rj.returncode, 0)
        self.assertNotIn("DOCTOR-MUST-NOT-LEAK", rj.stdout + rj.stderr)
        self.assertTrue(json.loads(rj.stdout)["all_ok"])

    def test_doctor_fails_missing_engine(self):
        r = subprocess.run(
            [sys.executable, os.path.join(TOOLS, "doctor.py"),
             "--codex-home", self.home, "--engine", os.path.join(self.tmp, "nope"),
             "--pack", self.pack],
            capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 1)
        self.assertIn("FAIL", r.stdout)

    def test_doctor_fails_tampered_pack(self):
        with open(os.path.join(self.pack, "skills", "p1-4-greet", "SKILL.md"), "a") as fh:
            fh.write("tamper\n")
        r = self.run_doctor()
        self.assertEqual(r.returncode, 1)
        self.assertIn("FAIL pack", r.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)