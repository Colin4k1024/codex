#!/usr/bin/env python3
"""P1-1 (#73) tests: pack manifest determinism, verification, version binding.

Model-free. Run: python3 superagent/tests/test_pack.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PACK = os.path.join(ROOT, "pack")
TOOL = os.path.join(ROOT, "tools", "pack.py")


def read_json(path):
    with open(path) as fh:
        return json.load(fh)


def write_json(path, obj):
    with open(path, "w") as fh:
        json.dump(obj, fh)


def run_tool(*args):
    r = subprocess.run([sys.executable, TOOL, *args], capture_output=True, text=True)
    return r.returncode, r.stdout.strip(), r.stderr.strip()


def copy_pack(dest, reverse=False):
    files = []
    for root, _, names in os.walk(PACK):
        for n in names:
            full = os.path.join(root, n)
            files.append((os.path.relpath(full, PACK), full))
    files.sort(key=lambda e: e[0], reverse=reverse)
    for rel, full in files:
        target = os.path.join(dest, rel)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        shutil.copyfile(full, target)


class PackTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="p11-pack-")
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)

    def test_01_committed_manifest_verifies(self):
        code, out, err = run_tool("verify", "--pack", PACK)
        self.assertEqual(code, 0, err)
        self.assertIn("OK:", out)

    def test_02_export_deterministic(self):
        copy_pack(self.tmp)
        code1, out1, _ = run_tool("export", "--pack", self.tmp)
        with open(os.path.join(self.tmp, "manifest.json")) as fh:
            man1 = fh.read()
        code2, out2, _ = run_tool("export", "--pack", self.tmp)
        with open(os.path.join(self.tmp, "manifest.json")) as fh:
            man2 = fh.read()
        self.assertEqual(code1, 0)
        self.assertEqual(code2, 0)
        self.assertEqual(man1, man2)
        self.assertEqual(json.loads(out1)["digest"], json.loads(out2)["digest"])

    def test_03_creation_order_independent(self):
        a = os.path.join(self.tmp, "a")
        b = os.path.join(self.tmp, "b")
        os.makedirs(a)
        os.makedirs(b)
        copy_pack(a, reverse=False)
        copy_pack(b, reverse=True)
        run_tool("export", "--pack", a)
        run_tool("export", "--pack", b)
        da = read_json(os.path.join(a, "manifest.json"))["digest"]
        db = read_json(os.path.join(b, "manifest.json"))["digest"]
        self.assertEqual(da, db)

    def test_04_clean_environment_verify(self):
        copy_pack(self.tmp)
        code, out, err = run_tool("verify", "--pack", self.tmp)
        self.assertEqual(code, 0, err)
        committed = read_json(os.path.join(PACK, "manifest.json"))
        fresh = read_json(os.path.join(self.tmp, "manifest.json"))
        self.assertEqual(committed["digest"], fresh["digest"])

    def test_05_tamper_detected(self):
        copy_pack(self.tmp)
        victim = os.path.join(self.tmp, "roles", "README.md")
        with open(victim, "ab") as fh:
            fh.write(b"x")
        code, _, err = run_tool("verify", "--pack", self.tmp)
        self.assertEqual(code, 1)
        self.assertIn("digest mismatch", err)

    def test_06_manifest_schema_enforced(self):
        copy_pack(self.tmp)
        man_path = os.path.join(self.tmp, "manifest.json")
        man = read_json(man_path)
        del man["digest"]
        write_json(man_path, man)
        code, _, err = run_tool("verify", "--pack", self.tmp)
        self.assertEqual(code, 1)
        self.assertIn("missing field", err)

    def test_07_pinned_digest_binding(self):
        digest = read_json(os.path.join(PACK, "manifest.json"))["digest"]
        code, _, err = run_tool("verify", "--pack", PACK, "--pinned", digest)
        self.assertEqual(code, 0, err)
        code, _, err = run_tool("verify", "--pack", PACK, "--pinned", "0" * 64)
        self.assertEqual(code, 1)
        self.assertIn("pinned digest mismatch", err)
        self.assertIn("never silently substitute", err)

    def test_08_symlink_rejected(self):
        copy_pack(self.tmp)
        os.symlink("/etc/hosts", os.path.join(self.tmp, "evil-link"))
        code, _, err = run_tool("verify", "--pack", self.tmp)
        self.assertEqual(code, 1)
        self.assertIn("symlink rejected", err)

    def test_09_license_coverage_enforced(self):
        copy_pack(self.tmp)
        lic_path = os.path.join(self.tmp, "LICENSES.json")
        lic = read_json(lic_path)
        lic["licenses"][0]["paths"] = ["roles/**"]
        write_json(lic_path, lic)
        code, _, err = run_tool("verify", "--pack", self.tmp)
        self.assertEqual(code, 1)
        self.assertIn("license coverage missing", err)

    def test_10_missing_licenses_file(self):
        copy_pack(self.tmp)
        os.remove(os.path.join(self.tmp, "LICENSES.json"))
        code, _, err = run_tool("verify", "--pack", self.tmp)
        self.assertEqual(code, 1)
        self.assertIn("LICENSES.json missing", err)

    def test_11_file_count_mismatch(self):
        copy_pack(self.tmp)
        man_path = os.path.join(self.tmp, "manifest.json")
        man = read_json(man_path)
        man["file_count"] += 1
        write_json(man_path, man)
        code, _, err = run_tool("verify", "--pack", self.tmp)
        self.assertEqual(code, 1)
        self.assertIn("file_count mismatch", err)


if __name__ == "__main__":
    unittest.main(verbosity=2)