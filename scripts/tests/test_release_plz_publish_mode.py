"""[OPUS-5.5] scripts/release-plz-publish-mode.py: the crates.io flip activates fail-closed."""

from __future__ import annotations

import importlib.util
import tempfile
import tomllib
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
_spec = importlib.util.spec_from_file_location(
    "release_plz_publish_mode", REPO_ROOT / "scripts" / "release-plz-publish-mode.py"
)
mode = importlib.util.module_from_spec(_spec)
assert _spec.loader is not None
_spec.loader.exec_module(mode)

CONFIG = (
    "[workspace]\npublish = false\ngit_only = true\n"
    '[[package]]\nname = "a"\npublish = true\n'
    '[[package]]\nname = "b"\npublish = true\n'
    '[[package]]\nname = "c"\npublish = false\n'
)


def _fetch(table):
    def fetch(name):
        return table[name]

    return fetch


class DecideTests(unittest.TestCase):
    def test_pre_bootstrap_is_tag_only(self):
        got = mode.decide(["a", "b"], "0.1.4", _fetch({"a": (404, set()), "b": (404, set())}))
        self.assertEqual(got[:2], ("tag-only", False))

    def test_partial_bootstrap_refuses(self):
        with self.assertRaises(mode.Refusal):
            mode.decide(["a", "b"], "0.1.4", _fetch({"a": (200, {"0.1.4"}), "b": (404, set())}))

    def test_already_published_version_is_tag_only(self):
        table = {"a": (200, {"0.1.4"}), "b": (200, {"0.1.4"})}
        self.assertEqual(mode.decide(["a", "b"], "0.1.4", _fetch(table))[:2], ("tag-only", True))

    def test_new_version_after_bootstrap_publishes(self):
        table = {"a": (200, {"0.1.4"}), "b": (200, {"0.1.4", "0.1.5"})}
        self.assertEqual(mode.decide(["a", "b"], "0.1.5", _fetch(table))[:2], ("publish", True))

    def test_unexpected_status_refuses(self):
        with self.assertRaises(mode.Refusal):
            mode.decide(["a"], "0.1.4", _fetch({"a": (500, set())}))


class ConfigTests(unittest.TestCase):
    def test_publish_set(self):
        self.assertEqual(mode.publish_set(tomllib.loads(CONFIG)), ["a", "b"])

    def test_tag_only_disables_every_publish_key(self):
        out = tomllib.loads(mode.effective_config(CONFIG, "tag-only"))
        self.assertFalse(any(p.get("publish") for p in out["package"]))
        self.assertTrue(out["workspace"]["git_only"])

    def test_publish_mode_drops_git_only(self):
        out = tomllib.loads(mode.effective_config(CONFIG, "publish"))
        self.assertFalse(out["workspace"]["git_only"])
        self.assertEqual([p["publish"] for p in out["package"]], [True, True, False])

    def test_main_writes_outputs_and_refuses_partial(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "release-plz.toml").write_text(CONFIG, encoding="utf-8")
            (root / "Cargo.toml").write_text(
                '[workspace]\n[workspace.package]\nversion = "0.1.4"\n', encoding="utf-8"
            )
            out = root / "eff.toml"
            code = mode.main(
                ["--repo-root", str(root), "--out", str(out)],
                fetch=_fetch({"a": (404, set()), "b": (404, set())}),
            )
            self.assertEqual(code, 0)
            self.assertFalse(any(p.get("publish") for p in tomllib.loads(out.read_text())["package"]))
            code = mode.main(
                ["--repo-root", str(root), "--out", str(out)],
                fetch=_fetch({"a": (200, set()), "b": (404, set())}),
            )
            self.assertEqual(code, 1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
