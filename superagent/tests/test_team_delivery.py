#!/usr/bin/env python3
"""P1-3 (#75) team-delivery skill + contracts budget/compliance tests.

Model-free. Measures injected-fragment and per-round token budgets with tiktoken
(o200k_base and cl100k_base), validates the acceptance template, and asserts the
low-trust / no-developer-elevation trust boundary. Run:
    python3 superagent/tests/test_team_delivery.py
    python3 superagent/tests/test_team_delivery.py --measure <evidence-dir>
"""
import json
import os
import re
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
PACK = os.path.join(ROOT, "superagent", "pack")
SKILL = os.path.join(PACK, "skills", "team-delivery", "SKILL.md")
WORKFLOW = os.path.join(PACK, "workflows", "team-delivery.md")
HANDOFF = os.path.join(PACK, "templates", "handoff-contract.md")
ACCEPT = os.path.join(PACK, "templates", "acceptance-criteria.json")

FRAGMENT_CAP = 900
PER_ROUND_CAP = 2000
HANDOFF_CAP = 800
HUMAN_REVIEW_THRESHOLD = 1000

import tiktoken
ENCODINGS = ["o200k_base", "cl100k_base"]


def toks(text, enc="o200k_base"):
    return len(tiktoken.get_encoding(enc).encode(text))


def read(path):
    with open(path, encoding="utf-8") as fh:
        return fh.read()


def skill_body():
    return read(SKILL).split("---", 2)[2].strip()


def handoff_summary():
    m = re.search(r"## Summary example.*?\n\n((?:>.*\n?)+)", read(HANDOFF))
    return m.group(1) if m else ""


def max_toks(text):
    return max(toks(text, e) for e in ENCODINGS)


CRED_PATTERNS = [
    r"sk-[A-Za-z0-9]{16,}",
    r"(?i)(api_key|apikey|secret|password|passwd|token)\s*[:=]\s*['\"][A-Za-z0-9_\-]{12,}",
    r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
    r"(?i)bearer\s+[A-Za-z0-9_\-\.]{16,}",
]


class TeamDeliveryTests(unittest.TestCase):
    def test_skill_frontmatter(self):
        txt = read(SKILL)
        self.assertTrue(txt.startswith("---"))
        fm = txt.split("---", 2)[1]
        self.assertRegex(fm, r"(?m)^name:\s*team-delivery\s*$")
        self.assertRegex(fm, r"(?m)^description:\s*\S")

    def test_injected_fragment_budget(self):
        whole = read(SKILL)
        for e in ENCODINGS:
            self.assertLessEqual(toks(whole, e), FRAGMENT_CAP,
                                 f"SKILL.md {e} exceeds {FRAGMENT_CAP}")
            self.assertLessEqual(toks(skill_body(), e), FRAGMENT_CAP)

    def test_all_contract_fragments_budget(self):
        for path in (WORKFLOW, HANDOFF, ACCEPT):
            self.assertLessEqual(max_toks(read(path)), FRAGMENT_CAP,
                                 f"{path} fragment exceeds {FRAGMENT_CAP}")

    def test_per_round_new_tokens_budget(self):
        skill = max_toks(read(SKILL))
        for path in (WORKFLOW, HANDOFF, ACCEPT):
            self.assertLessEqual(skill + max_toks(read(path)), PER_ROUND_CAP,
                                 f"skill + {path} exceeds per-round {PER_ROUND_CAP}")

    def test_handoff_summary_budget(self):
        summary = handoff_summary()
        self.assertTrue(summary.strip(), "handoff summary example missing")
        for e in ENCODINGS:
            self.assertLessEqual(toks(summary, e), HANDOFF_CAP)

    def test_workflow_phases(self):
        combined = (read(SKILL) + read(WORKFLOW)).lower()
        for phase in ("intake", "plan", "implement", "verify", "review", "report"):
            self.assertIn(phase, combined, f"missing phase {phase}")

    def test_acceptance_template_structure(self):
        data = json.loads(read(ACCEPT))
        self.assertEqual(data["workflow"], "team-delivery")
        gates = data["gates"]
        for node in ("plan", "implement", "verify", "review", "report"):
            g = gates[node]
            self.assertTrue(g["command_source"], f"{node} missing command_source")
            self.assertTrue(g["command"], f"{node} missing command")
            self.assertIn("schema", g["artifact"], f"{node} artifact missing schema")
            self.assertIn("exit_code", g["success"], f"{node} success not exit-code gated")
        for name in ("gate-result", "node-dag", "review", "report"):
            self.assertIn(name, data["schemas"])
        self.assertEqual(data["schemas"]["gate-result"]["properties"]["exit_code"]["type"],
                         "integer")
        self.assertIn("waiting_input", data["missing_gate_policy"])

    def test_success_by_gate_evidence_not_self_report(self):
        combined = (read(SKILL) + read(WORKFLOW) + read(ACCEPT)).lower()
        self.assertIn("gate evidence", combined)
        self.assertTrue("never by a model" in combined
                        or "model self-report is never acceptance" in combined
                        or "never succeed on a model claim" in combined)

    def test_low_trust_no_developer_elevation(self):
        for path in (SKILL, WORKFLOW, HANDOFF):
            txt = read(path).lower()
            self.assertIn("developer_instructions", txt, f"{path} missing trust boundary")
            self.assertIn("low", txt, f"{path} missing low-trust designation")
            self.assertTrue("never" in txt or "not" in txt,
                            f"{path} must negate developer elevation")
        self.assertNotRegex(read(SKILL), r"(?m)^\s*developer_instructions\s*=")

    def test_long_material_via_file_or_mcp(self):
        combined = (read(SKILL) + read(WORKFLOW)).lower()
        self.assertIn("mcp", combined)
        self.assertTrue("on demand" in combined or "on-demand" in combined)
        self.assertTrue("read" in combined)

    def test_budget_rules_declared(self):
        txt = read(SKILL)
        self.assertIn("900", txt)
        self.assertIn("2000", txt)
        self.assertIn("800", txt)
        self.assertIn("1000", txt)
        self.assertIn("human review", txt.lower())

    def test_referenced_files_exist(self):
        for rel in ("workflows/team-delivery.md", "templates/handoff-contract.md",
                    "templates/acceptance-criteria.json"):
            self.assertTrue(os.path.isfile(os.path.join(PACK, rel)), rel)
        self.assertTrue(os.path.isfile(os.path.join(HERE, "test_team_delivery.py")))

    def test_no_credentials(self):
        for dirpath, _dirs, files in os.walk(PACK):
            for f in files:
                if not f.endswith((".md", ".json", ".toml")):
                    continue
                txt = read(os.path.join(dirpath, f))
                for pat in CRED_PATTERNS:
                    self.assertIsNone(re.search(pat, txt),
                                      f"credential pattern in {f}: {pat}")


def measure(evdir):
    os.makedirs(evdir, exist_ok=True)
    skill = read(SKILL)
    rows = {}
    for label, path in (("skill_injected_fragment", SKILL),
                        ("workflow_on_demand", WORKFLOW),
                        ("handoff_on_demand", HANDOFF),
                        ("acceptance_on_demand", ACCEPT)):
        txt = read(path)
        rows[label] = {e: toks(txt, e) for e in ENCODINGS}
    summary = handoff_summary()
    result = {
        "caps": {"fragment": FRAGMENT_CAP, "per_round": PER_ROUND_CAP,
                 "handoff_summary": HANDOFF_CAP,
                 "human_review_threshold": HUMAN_REVIEW_THRESHOLD},
        "fragments": rows,
        "handoff_summary_example": {e: toks(summary, e) for e in ENCODINGS},
        "per_round_skill_plus_each": {
            os.path.basename(p): max_toks(skill) + max_toks(read(p))
            for p in (WORKFLOW, HANDOFF, ACCEPT)},
        "all_fragments_within_cap": all(max_toks(read(p)) <= FRAGMENT_CAP
                                        for p in (SKILL, WORKFLOW, HANDOFF, ACCEPT)),
        "handoff_within_cap": max_toks(summary) <= HANDOFF_CAP,
        "per_round_within_cap": all(max_toks(skill) + max_toks(read(p)) <= PER_ROUND_CAP
                                    for p in (WORKFLOW, HANDOFF, ACCEPT)),
        "encodings": ENCODINGS,
    }
    result["allPass"] = (result["all_fragments_within_cap"]
                         and result["handoff_within_cap"]
                         and result["per_round_within_cap"])
    out = os.path.join(evdir, "budget-measurement.json")
    with open(out, "w", encoding="utf-8") as fh:
        json.dump(result, fh, indent=2, ensure_ascii=False)
    print(json.dumps(result, indent=2, ensure_ascii=False))
    return result["allPass"]


if __name__ == "__main__":
    if "--measure" in sys.argv:
        i = sys.argv.index("--measure")
        evdir = sys.argv[i + 1] if len(sys.argv) > i + 1 else os.path.join(
            ROOT, "superagent", "evidence", "p1-3")
        ok = measure(evdir)
        sys.exit(0 if ok else 1)
    unittest.main(verbosity=2)