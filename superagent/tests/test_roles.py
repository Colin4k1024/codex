#!/usr/bin/env python3
"""P1-2 / #74 content-layer tests (model-free).

Validates the leader persona, the five role config-layer files, and the
agents overlay: structure, overlay<->file consistency, model slugs against the
engine catalog, resident-intro token budget (tiktoken o200k_base <= 400),
delivery contracts, and credential absence.

Runtime acceptance (leader persona injection, five-role spawn, child turn
completion, role isolation, budget caps) is proven separately by
tests/p1_2.py (app-server stdio) and tests/p1_2_tui.py (TUI pty); these tests
do NOT claim runtime acceptance.
"""
import json
import re
import tomllib
import unittest
from pathlib import Path

import tiktoken

TESTS = Path(__file__).resolve().parent
SUPERAGENT = TESTS.parent
ROOT = SUPERAGENT.parent
PACK = SUPERAGENT / "pack"
ROLES = PACK / "roles"
OVERLAY = PACK / "config" / "agents-overlay.toml"
PERSONA = ROLES / "leader-persona.md"
MODELS = ROOT / "codex-rs" / "models-manager" / "models.json"

ROLE_NAMES = ["planner", "implementer", "verifier", "reviewer", "reporter"]
INTRO_TOKEN_CAP = 400
ENCODING = "o200k_base"

CREDENTIAL_PATTERNS = [
    re.compile(r"sk-[A-Za-z0-9]{16,}"),
    re.compile(r"(?i)(api[_-]?key|secret|password|passwd|token)\s*[:=]\s*['\"][^'\"]{8,}"),
    re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    re.compile(r"(?i)bearer\s+[A-Za-z0-9._\-]{20,}"),
]


def load_toml(path):
    with open(path, "rb") as fh:
        return tomllib.load(fh)


def resident_intro():
    txt = PERSONA.read_text(encoding="utf-8")
    section = txt.split("## 2.", 1)[1]
    return section.split("\n", 1)[1].strip()


def catalog_slugs():
    data = json.loads(MODELS.read_text(encoding="utf-8"))
    return {m["slug"] for m in data["models"]}


class RoleContentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.overlay = load_toml(OVERLAY)
        cls.roles = {name: load_toml(ROLES / f"{name}.toml") for name in ROLE_NAMES}
        cls.slugs = catalog_slugs()
        cls.enc = tiktoken.get_encoding(ENCODING)

    def test_five_role_files_exist_and_parse(self):
        for name in ROLE_NAMES:
            self.assertTrue((ROLES / f"{name}.toml").is_file(), name)
            self.assertIn("description", self.roles[name])
            self.assertIn("developer_instructions", self.roles[name])
            self.assertIn("model", self.roles[name])

    def test_role_models_in_catalog(self):
        for name, role in self.roles.items():
            self.assertIn(role["model"], self.slugs, f"{name} model not in catalog")

    def test_role_delivery_contract_present(self):
        for name, role in self.roles.items():
            self.assertIn("Delivery contract:", role["developer_instructions"], name)
            self.assertIn("800 tokens", role["developer_instructions"], name)

    def test_overlay_enables_mav2_and_agents(self):
        self.assertTrue(self.overlay["features"]["multi_agent_v2"]["enabled"])
        self.assertTrue(self.overlay["agents"]["enabled"])
        self.assertGreaterEqual(self.overlay["agents"]["max_concurrent_threads_per_session"], 5)

    def test_overlay_declares_all_five_roles_consistently(self):
        for name in ROLE_NAMES:
            entry = self.overlay["agents"][name]
            self.assertEqual(entry["config_file"], f"./agents/{name}.toml")
            self.assertEqual(entry["description"], self.roles[name]["description"])
            self.assertIsInstance(entry["nickname_candidates"], list)
            self.assertTrue(entry["nickname_candidates"])
            self.assertTrue(all(isinstance(n, str) and n for n in entry["nickname_candidates"]))

    def test_overlay_has_no_spike_approval_or_sandbox(self):
        # Production overlay must not hardcode never-approval / sandbox; those
        # belong to the orchestrator (P0-7) and user config.
        self.assertNotIn("approval_policy", self.overlay)
        self.assertNotIn("sandbox_mode", self.overlay)

    def test_resident_intro_matches_overlay_and_under_cap(self):
        intro = resident_intro()
        hint = self.overlay["features"]["multi_agent_v2"]["root_agent_usage_hint_text"].strip()
        self.assertEqual(intro, hint, "persona §2 must equal overlay root hint text")
        n = len(self.enc.encode(hint))
        self.assertLessEqual(n, INTRO_TOKEN_CAP, f"resident intro {n} tokens > {INTRO_TOKEN_CAP}")

    def test_persona_has_full_and_resident_sections(self):
        txt = PERSONA.read_text(encoding="utf-8")
        self.assertIn("## 1.", txt)
        self.assertIn("## 2.", txt)
        for kw in ["INTAKE", "DELEGATE", "VERIFY", "REPORT"]:
            self.assertIn(kw, txt)

    def test_no_credentials_in_any_content_file(self):
        files = [PERSONA, OVERLAY] + [ROLES / f"{n}.toml" for n in ROLE_NAMES]
        for path in files:
            text = path.read_text(encoding="utf-8")
            for pat in CREDENTIAL_PATTERNS:
                self.assertIsNone(pat.search(text), f"credential-like pattern in {path.name}")

    def test_role_files_declare_sandbox_constraint(self):
        for name in ROLE_NAMES:
            text = (ROLES / f"{name}.toml").read_text(encoding="utf-8")
            self.assertIn("Sandbox is orchestrator-enforced", text, name)
            self.assertNotIn("sandbox_mode =", text, f"{name} must not set role-level sandbox_mode")


def measure():
    enc = tiktoken.get_encoding(ENCODING)
    intro = resident_intro()
    overlay = load_toml(OVERLAY)
    hint = overlay["features"]["multi_agent_v2"]["root_agent_usage_hint_text"].strip()
    return {
        "encoding": ENCODING,
        "cap_tokens": INTRO_TOKEN_CAP,
        "resident_intro_tokens": len(enc.encode(intro)),
        "overlay_hint_tokens": len(enc.encode(hint)),
        "intro_equals_overlay_hint": intro == hint,
        "within_cap": len(enc.encode(hint)) <= INTRO_TOKEN_CAP,
        "role_models": {n: load_toml(ROLES / f"{n}.toml")["model"] for n in ROLE_NAMES},
        "note": "Content-layer measurement only; runtime acceptance is proven separately by tests/p1_2.py (stdio) and tests/p1_2_tui.py (TUI).",
    }


if __name__ == "__main__":
    import sys
    if "--measure" in sys.argv:
        out = SUPERAGENT / "evidence" / "p1-2" / "tokenizer-measurement.json"
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(measure(), ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(measure(), ensure_ascii=False, sort_keys=True))
    else:
        unittest.main(verbosity=2)