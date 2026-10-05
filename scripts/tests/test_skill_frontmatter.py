#!/usr/bin/env python3
# Hermetic tests for scripts/check-skill-frontmatter.py (bead sq-tstv).
#
# Locks in the two things sq-tstv changed:
#   1. The default glob now covers BOTH skills/**/SKILL.md AND .claude/agents/*.md,
#      so an unquoted colon in a subagent `description:` value fails the gate the
#      same way it does for a SKILL.md (the sparq-site.md "GitHub Pages app: …"
#      render bug that motivated this bead).
#   2. For .claude/agents/<role>.md the name-match heuristic checks `name` against
#      the FILE stem (<role>), not the parent directory (which is always `agents`),
#      so the valid agent files do not emit spurious advisory warnings.
#
# Hermetic w.r.t. git/network: drives the script's pure check_file() against
# FIXTURE files in a tempdir, and exercises default_paths() by chdir-ing into a
# fixture tree. No subprocess, no live repo state. Requires PyYAML (same dep the
# gate itself needs).
#
# Run:  python3 scripts/tests/test_skill_frontmatter.py
# (stdlib + PyYAML; also discoverable by `pytest`.)

from __future__ import annotations

import importlib.util
import os
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, REPO_ROOT / "scripts" / filename)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


fm = _load("check_skill_frontmatter", "check-skill-frontmatter.py")


# Frontmatter blocks (without and with the unquoted-colon defect). The colon-space
# inside an unquoted scalar is exactly what makes PyYAML read a nested mapping key.
GOOD_AGENT = (
    "---\n"
    "name: my-agent\n"
    'description: "Does the X thing: then the Y thing. Use for X."\n'
    "model: opus\n"
    "---\n"
    "body\n"
)
BAD_AGENT_UNQUOTED_COLON = (
    "---\n"
    "name: my-agent\n"
    "description: Does the X thing: then the Y thing. Use for X.\n"
    "model: opus\n"
    "---\n"
    "body\n"
)
GOOD_SKILL = "---\nname: foo\ndescription: A foo skill.\n---\nbody\n"


def _write(d: Path, rel: str, text: str) -> str:
    p = d / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text, encoding="utf-8")
    return str(p)


class CheckFileTest(unittest.TestCase):
    def test_unquoted_colon_in_agent_description_is_an_error(self):
        with tempfile.TemporaryDirectory() as t:
            d = Path(t)
            path = _write(d, ".claude/agents/my-agent.md", BAD_AGENT_UNQUOTED_COLON)
            errors, _ = fm.check_file(path)
            self.assertTrue(
                any("YAML parse error" in e for e in errors),
                f"expected a YAML parse error, got {errors!r}",
            )

    def test_quoted_agent_description_is_clean(self):
        with tempfile.TemporaryDirectory() as t:
            d = Path(t)
            path = _write(d, ".claude/agents/my-agent.md", GOOD_AGENT)
            errors, warnings = fm.check_file(path)
            self.assertEqual(errors, [], f"unexpected errors: {errors!r}")
            # name == file stem ('my-agent'), so no name-mismatch warning, and the
            # parent dir 'agents' must NOT trigger the directory-match heuristic.
            self.assertEqual(warnings, [], f"unexpected warnings: {warnings!r}")

    def test_agent_name_mismatch_warns_against_file_stem_not_directory(self):
        with tempfile.TemporaryDirectory() as t:
            d = Path(t)
            # name 'other' != file stem 'my-agent' -> advisory warning (not error),
            # and it must reference the FILE, not the 'agents' directory.
            text = GOOD_AGENT.replace("name: my-agent", "name: other")
            path = _write(d, ".claude/agents/my-agent.md", text)
            errors, warnings = fm.check_file(path)
            self.assertEqual(errors, [], f"unexpected errors: {errors!r}")
            self.assertTrue(
                any("!= file 'my-agent'" in w for w in warnings),
                f"expected a file-stem mismatch warning, got {warnings!r}",
            )
            self.assertFalse(
                any("directory" in w for w in warnings),
                f"agent files must not warn against the directory, got {warnings!r}",
            )

    def test_skill_directory_match_still_warns(self):
        with tempfile.TemporaryDirectory() as t:
            d = Path(t)
            # A nested skill whose name != its surface directory still warns.
            text = GOOD_SKILL.replace("name: foo", "name: bar")
            path = _write(d, "skills/foo/SKILL.md", text)
            _errors, warnings = fm.check_file(path)
            self.assertTrue(
                any("!= directory 'foo'" in w for w in warnings),
                f"expected a directory mismatch warning, got {warnings!r}",
            )


class DefaultPathsTest(unittest.TestCase):
    def test_default_glob_covers_both_families(self):
        prev = os.getcwd()
        with tempfile.TemporaryDirectory() as t:
            d = Path(t)
            _write(d, "skills/foo/SKILL.md", GOOD_SKILL)
            _write(d, ".claude/agents/my-agent.md", GOOD_AGENT)
            # A non-matching .md in agents-adjacent locations must NOT be picked up.
            _write(d, ".claude/notes.md", "irrelevant\n")
            try:
                os.chdir(d)
                paths = fm.default_paths()
            finally:
                os.chdir(prev)
            self.assertIn("skills/foo/SKILL.md", paths)
            self.assertIn(".claude/agents/my-agent.md", paths)
            self.assertNotIn(".claude/notes.md", paths)


if __name__ == "__main__":
    unittest.main()
