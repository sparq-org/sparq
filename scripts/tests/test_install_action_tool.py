#!/usr/bin/env python3
# Hermetic tests for scripts/check-install-action-tool.py (bead sq-ur7o).
#
# Two layers:
#   1. The gate's PURE logic (the same fixtures as its built-in --self-test, asserted
#      here so a regression in the splitter / with:tool detection is caught by pytest
#      and by the docs-quality ci-scripts job).
#   2. A LIVE invariant: the repo's own .github/workflows/*.yml MUST currently pass
#      the gate (every SHA-pinned install-action carries `with: tool:`). This is the
#      committed-files check — no git/network — so it stays deterministic.
#
# Run:  python3 scripts/tests/test_install_action_tool.py
# (stdlib only; no pytest required — also discoverable by `pytest`.)

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(
        name, REPO_ROOT / "scripts" / filename
    )
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


gate = _load("check_install_action_tool", "check-install-action-tool.py")


class PureLogic(unittest.TestCase):
    """scan_text() fixtures: a SHA-pinned install-action without `with: tool:` is the
    only thing that trips the gate."""

    def test_good_with_tool(self):
        self.assertEqual(gate.scan_text(gate._GOOD), [])

    def test_bad_no_with(self):
        self.assertEqual(len(gate.scan_text(gate._BAD_NO_WITH)), 1)

    def test_bad_with_no_tool(self):
        self.assertEqual(len(gate.scan_text(gate._BAD_WITH_NO_TOOL)), 1)

    def test_good_extra_with_keys(self):
        self.assertEqual(gate.scan_text(gate._GOOD_EXTRA_WITH), [])

    def test_tag_pinned_out_of_scope(self):
        # @cargo-llvm-cov / @v2.81.10 are tags, not SHAs → not flagged even with no
        # `with: tool:` (a tag ref can still carry the selector).
        self.assertEqual(gate.scan_text(gate._TAG_PINNED_OK), [])

    def test_other_action_ignored(self):
        self.assertEqual(gate.scan_text(gate._OTHER_ACTION_OK), [])

    def test_mixed_one_bad(self):
        self.assertEqual(len(gate.scan_text(gate._MIXED)), 1)

    def test_no_with_leak(self):
        # The first step's `with: tool:` must not satisfy the second, with:-less step.
        self.assertEqual(len(gate.scan_text(gate._NO_LEAK)), 1)

    def test_sha_only_matches_40_hex(self):
        # A 39-hex ref is not a SHA pin (defensive): treated as a tag → out of scope.
        almost = gate._BAD_NO_WITH.replace(
            "7a79fe8c3a13344501c80d99cae481c1c9085912",
            "7a79fe8c3a13344501c80d99cae481c1c908591",  # 39 chars
        )
        self.assertEqual(gate.scan_text(almost), [])


class LiveWorkflows(unittest.TestCase):
    def test_repo_workflows_pass(self):
        """The repo's own workflows must currently satisfy the gate. If a future edit
        re-introduces a selector-less SHA-pinned install-action, this fails first."""
        offences = gate.scan_workflows(REPO_ROOT)
        self.assertEqual(
            offences,
            [],
            msg="SHA-pinned install-action step(s) missing `with: tool:`: "
            + ", ".join(f"{p}: {o}" for p, o in offences),
        )

    def test_gate_actually_sees_the_install_actions(self):
        """Guard against a VACUOUS pass: the gate must in fact find install-action
        steps in the repo (so an all-green is meaningful, not 'matched nothing')."""
        wf_dir = REPO_ROOT / ".github" / "workflows"
        seen = 0
        for path in wf_dir.glob("*.yml"):
            text = path.read_text(encoding="utf-8", errors="ignore")
            for block in gate.split_steps(text):
                uses = gate._step_uses(block)
                if uses and uses[0] == gate.ACTION and gate._SHA_RE.match(uses[1]):
                    seen += 1
        self.assertGreater(
            seen,
            0,
            "expected the repo to contain SHA-pinned taiki-e/install-action steps; "
            "found none — the live check would be vacuously green.",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
