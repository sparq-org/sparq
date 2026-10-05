"""Hermetic tests for scripts/publish-strip.py (the crates.io packaging-time feature strip).

Needs tomlkit (.github/requirements/publish-strip.txt), like the script itself.
"""

from __future__ import annotations

import importlib.util
import tempfile
import tomllib
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "publish_strip", REPO_ROOT / "scripts" / "publish-strip.py"
)
strip = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(strip)


def _write(root: Path, rel: str, text: str) -> None:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def _fixture(root: Path, *, research_optional: bool = True, default_research=False) -> None:
    _write(
        root,
        "Cargo.toml",
        '[workspace]\nmembers = ["crates/core", "crates/server", "crates/zk", "crates/conf"]\n'
        '[workspace.package]\nversion = "0.2.0"\n'
        '[patch.crates-io]\nspargebra = { path = "vendor/shim" }\n',
    )
    _write(root, "crates/core/Cargo.toml", '[package]\nname = "core"\nversion.workspace = true\n')
    _write(
        root,
        "crates/zk/Cargo.toml",
        '[package]\nname = "zk"\nversion.workspace = true\npublish = false\n',
    )
    optional = ", optional = true" if research_optional else ""
    default = '"zk-authz"' if default_research else '"http"'
    _write(
        root,
        "crates/server/Cargo.toml",
        '[package]\nname = "server"\nversion.workspace = true\n'
        "[features]\n"
        f"default = [{default}]\n"
        'http = ["dep:quinn-ish"]\n'
        'zk-authz = ["dep:zk", "dep:big"]\n'
        'zk-plus = ["zk-authz"]\n'
        'weak = ["core?/x"]\n'
        'probe = ["zk?/trace"]\n'
        "[dependencies]\n"
        'core = { path = "../core", version = "0.2.0" }\n'
        f'zk = {{ path = "../zk", version = "0.2.0"{optional} }}\n'
        'big = { version = "1", optional = true }\n'
        'quinn-ish = { version = "1", optional = true }\n'
        "[dev-dependencies]\n"
        'zk = { path = "../zk", version = "0.2.0" }\n'
        '[[example]]\nname = "zk_demo"\nrequired-features = ["zk-plus"]\n',
    )
    _write(
        root,
        "crates/conf/Cargo.toml",
        '[package]\nname = "conf"\nversion.workspace = true\npublish = false\n'
        '[dependencies]\nserver = { path = "../server", features = ["zk-authz"] }\n',
    )


class PublishStripTest(unittest.TestCase):
    def test_strips_optional_research_edge_and_dependent_features(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _fixture(root)
            self.assertEqual(strip.main(["--root", str(root)]), 0)
            server = tomllib.loads((root / "crates/server/Cargo.toml").read_text())
            self.assertNotIn("zk", server["dependencies"])
            # `big` was only reachable through the removed feature.
            self.assertNotIn("big", server["dependencies"])
            self.assertIn("quinn-ish", server["dependencies"])
            self.assertNotIn("zk", server.get("dev-dependencies", {}))
            self.assertEqual(
                set(server["features"]), {"default", "http", "weak", "probe"}
            )
            self.assertEqual(server["features"]["probe"], [])
            self.assertEqual(server["features"]["weak"], ["core?/x"])
            self.assertNotIn("example", server)
            self.assertIn("examples/zk_demo.rs", server["package"]["exclude"])
            ws = tomllib.loads((root / "Cargo.toml").read_text())
            self.assertEqual(ws["workspace"]["members"], ["crates/core", "crates/server"])
            self.assertNotIn("patch", ws)

    def test_check_mode_writes_nothing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _fixture(root)
            before = (root / "crates/server/Cargo.toml").read_text()
            self.assertEqual(strip.main(["--check", "--root", str(root)]), 0)
            self.assertEqual((root / "crates/server/Cargo.toml").read_text(), before)

    def test_refuses_non_optional_edge_to_unpublished_crate(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _fixture(root, research_optional=False)
            self.assertEqual(strip.main(["--check", "--root", str(root)]), 1)

    def test_refuses_stripping_a_default_feature(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _fixture(root, default_research=True)
            self.assertEqual(strip.main(["--check", "--root", str(root)]), 1)

    def test_live_workspace_strip_plan_is_sound(self) -> None:
        members = strip.load_workspace(REPO_ROOT)
        removed_deps, _, removed_feats = strip.plan(members)
        published = {n for n, (_, _, pub) in members.items() if pub}
        for name in published:
            self.assertNotIn("default", removed_feats[name], name)
            for dep in removed_deps[name]:
                self.assertNotIn(dep, published, f"{name} strips published crate {dep}")


    def test_book_crate_table_matches_publish_set(self) -> None:
        import re

        members = strip.load_workspace(REPO_ROOT)
        published = {n for n, (_, _, pub) in members.items() if pub}
        page = (REPO_ROOT / "book/src/getting-started/rust-crates.md").read_text()
        rows = set(re.findall(r"^\| \[`(sparq-[a-z0-9-]+)`\]", page, re.MULTILINE))
        self.assertEqual(rows, published)


if __name__ == "__main__":
    unittest.main()
