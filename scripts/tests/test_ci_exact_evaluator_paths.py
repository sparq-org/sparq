"""Mandatory evaluator selection must never hide an uncertain diff."""

import importlib.util
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "ci_exact_evaluator_paths", Path(__file__).parents[1] / "ci_exact_evaluator_paths.py"
)
selector = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(selector)
BASE, HEAD = "a" * 40, "b" * 40
EXACT_INPUTS = ["zk/sparql-evaluator/fixtures/case.json", "crates/sparq-engine/src/exec.rs",
                "crates/sparq-core/Cargo.toml", "crates/sparq-substrate/src/lib.rs",
                "crates/sparq-canon/src/lib.rs", "crates/sparq-canon/Cargo.toml",
                "vendor/spargebra/src/parser.rs", "vendor/spargebra-shim/Cargo.toml",
                "crates/sparq-query-protocol/src/lib.rs", ".cargo/config.toml", "Cargo.lock",
                "Cargo.toml", "rust-toolchain.toml", ".github/workflows/zk-exact-evaluator.yml",
                "scripts/ci_exact_evaluator_paths.py", "scripts/ci_exact_evaluator_evidence.py",
                "scripts/tests/test_ci_exact_evaluator_evidence.py", "vendor/zk-sdk/risc0-build/src/lib.rs",
                "bench/zk-bindings/exact_ci.py", "bench/zk-bindings/corpus.py",
                "bench/zk-bindings/exact-originals.json", "crates/sparq-conformance/examples/proof_corpus.rs",
                "bench/differential-divergences.json", "crates/sparq-bench/src/fuzz.rs",
                ".github/workflows/zk-toolchain.yml"]


ROOT = Path(__file__).resolve().parents[2]
EVALUATOR = ROOT / "zk/sparql-evaluator"


DEP_KINDS = ("dependencies", "dev-dependencies", "build-dependencies")


def _manifest(crate: Path) -> dict:
    return tomllib.loads((crate / "Cargo.toml").read_text())


def _owning_workspace(crate: Path, data: dict):
    """The workspace root Cargo would use for `crate`, or None outside any workspace."""
    if "workspace" in data:
        return crate
    explicit = data.get("package", {}).get("workspace")
    if explicit is not None:
        return (crate / explicit).resolve()
    for parent in crate.parents:
        manifest = parent / "Cargo.toml"
        if manifest.is_file() and "workspace" in tomllib.loads(manifest.read_text()):
            return parent
    return None


def _members(crate: Path, data: dict):
    for pattern in data.get("workspace", {}).get("members", []):
        matches = sorted(p for p in crate.glob(pattern) if (p / "Cargo.toml").is_file())
        if not matches:
            raise AssertionError(f"workspace member {pattern!r} in {crate} matches no crate")
        yield from matches


def _path_deps(crate: Path, data: dict):
    """Yield every local crate one manifest can build, resolving `workspace = true`."""
    tables = [(data.get(k, {}), crate) for k in DEP_KINDS]
    tables += [(t.get(k, {}), crate) for t in data.get("target", {}).values() for k in DEP_KINDS]
    workspace_deps = {}
    ws = _owning_workspace(crate, data)
    if ws is not None:
        ws_data = data if ws == crate else _manifest(ws)
        workspace_deps = ws_data.get("workspace", {}).get("dependencies", {})
        # Workspace dependencies, patches and replacements resolve from the workspace root.
        tables += [(t, ws) for t in ws_data.get("patch", {}).values()]
        tables += [(ws_data.get("replace", {}), ws), (workspace_deps, ws)]
    for table, base in tables:
        for name, spec in table.items():
            if isinstance(spec, dict) and spec.get("workspace") is True:
                spec, base_dir = workspace_deps.get(name, {}), ws
            else:
                base_dir = base
            if isinstance(spec, dict) and "path" in spec:
                yield (base_dir / spec["path"]).resolve()


def path_closure(seeds):
    """Every local crate reachable from `seeds` through members, guests and path deps."""
    seen, todo = set(), [Path(d).resolve() for d in seeds]
    while todo:
        crate = todo.pop()
        if crate in seen:
            continue
        seen.add(crate)
        data = _manifest(crate)
        todo += list(_members(crate, data))
        todo += [crate / m for m in data.get("package", {}).get("metadata", {})
                 .get("risc0", {}).get("methods", [])]
        todo += list(_path_deps(crate, data))
    return seen


def evaluator_path_closure():
    """Every local crate the evaluator workspace and both guest workspaces can build."""
    return path_closure([EVALUATOR, *(EVALUATOR / "methods" / g for g in ("guest", "guest-authrdf"))])


def completed(stdout=b""):
    return subprocess.CompletedProcess([], 0, stdout)


class ExactEvaluatorSelection(unittest.TestCase):
    def test_workflow_always_creates_the_gate_and_checks_classification(self):
        workflow = (Path(__file__).parents[2] / ".github/workflows/zk-exact-evaluator.yml").read_text()
        self.assertIn("  merge_group:\n    types: [checks_requested]", workflow)
        self.assertIn("name: real exact-dataset guest and proof", workflow)
        self.assertNotIn("    paths:", workflow)
        self.assertIn("Require explicit classification output", workflow)
        self.assertIn("python3 scripts/ci_exact_evaluator_paths.py", workflow)
        self.assertIn("python3 scripts/ci_exact_evaluator_evidence.py", workflow)

    def test_every_execution_input_is_relevant(self):
        for path in EXACT_INPUTS:
            self.assertTrue(selector.relevant_path(path), path)

    def test_every_local_crate_in_the_evaluator_closure_is_relevant(self):
        # A hand list drifts: derive the evaluator's local crate closure from its manifests.
        closure = evaluator_path_closure()
        self.assertIn((ROOT / "vendor/spargebra-shim").resolve(), closure)
        self.assertIn((ROOT / "crates/sparq-query-protocol").resolve(), closure)
        missing = sorted(str(c.relative_to(ROOT)) for c in closure
                         if not selector.relevant_path(f"{c.relative_to(ROOT).as_posix()}/Cargo.toml"))
        self.assertEqual(missing, [], "evaluator inputs missing from EXACT_PREFIXES")

    def test_closure_resolves_inherited_workspace_path_dependencies(self):
        # `dep = { workspace = true }` resolves through the owning workspace root,
        # relative to that root, including deps inherited by a non-root member.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            def crate(rel, body):
                (root / rel).mkdir(parents=True, exist_ok=True)
                (root / rel / "Cargo.toml").write_text(body)
            crate(".", '[workspace]\nmembers = ["a"]\n'
                       '[workspace.dependencies]\nleaf = { path = "libs/leaf" }\n'
                       'r = { path = "libs/other", package = "renamed" }\n')
            crate("a", '[package]\nname = "a"\n[dependencies]\nleaf = { workspace = true }\n'
                       '[build-dependencies]\nr = { workspace = true }\n')
            crate("libs/leaf", '[package]\nname = "leaf"\n')
            crate("libs/other", '[package]\nname = "renamed"\n')
            crate("seed", '[package]\nname = "seed"\nworkspace = ".."\n'
                          '[dependencies]\na = { path = "../a" }\n')
            closure = path_closure([root / "seed"])
            for rel in ("a", "libs/leaf", "libs/other"):
                self.assertIn((root / rel).resolve(), closure, rel)

    def test_unknown_scope_is_an_error_not_a_skip(self):
        # An empty diff never consults relevant_path, so the scope is checked first.
        with patch.object(selector.subprocess, "run", side_effect=[completed(), completed()]):
            with self.assertRaisesRegex(ValueError, "unknown selection scope"):
                selector.requires_execution("merge_group", BASE, HEAD, "registry")
        with self.assertRaisesRegex(ValueError, "unknown selection scope"):
            selector.relevant_path("Cargo.toml", "proof")

    def test_explicit_irrelevant_diff_only_skips_heavy_steps(self):
        with patch.object(selector.subprocess, "run", side_effect=[
            subprocess.CompletedProcess([], 0),
            subprocess.CompletedProcess([], 0, b"docs/design.md\0site/page.tsx\0"),
        ]) as run:
            self.assertFalse(selector.requires_execution("merge_group", BASE, HEAD))
            self.assertIn("--no-renames", run.call_args.args[0])

    def test_relevant_deletion_or_rename_requires_execution(self):
        with patch.object(selector.subprocess, "run", side_effect=[
            subprocess.CompletedProcess([], 0),
            subprocess.CompletedProcess([], 0, b"zk/sparql-evaluator/model/src/lib.rs\0docs/moved.txt\0"),
        ]):
            self.assertTrue(selector.requires_execution("pull_request", BASE, HEAD))

    def test_failed_or_truncated_diff_requires_execution(self):
        for outcome in [subprocess.CalledProcessError(1, "git"),
                        subprocess.TimeoutExpired("git", 60),
                        subprocess.CompletedProcess([], 0, b"docs/design.md")]:
            with patch.object(selector.subprocess, "run", side_effect=[
                subprocess.CompletedProcess([], 0), outcome,
            ]):
                self.assertTrue(selector.requires_execution("merge_group", BASE, HEAD))

    def test_failed_fetch_and_missing_context_require_execution(self):
        with patch.object(selector.subprocess, "run", side_effect=OSError()):
            self.assertTrue(selector.requires_execution("push", BASE, HEAD))
        with patch.object(selector.subprocess, "run") as run:
            for event, base, head in [("pull_request", "", HEAD), ("push", "0" * 40, HEAD),
                                      ("merge_group", BASE, "--bad"), ("workflow_dispatch", "", "")]:
                self.assertTrue(selector.requires_execution(event, base, head))
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
