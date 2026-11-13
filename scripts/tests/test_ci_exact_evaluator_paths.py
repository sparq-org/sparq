"""Mandatory evaluator selection must never hide an uncertain diff."""

import importlib.util
import contextlib
import io
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import textwrap
import tomllib
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "ci_exact_evaluator_paths", Path(__file__).parents[1] / "ci_exact_evaluator_paths.py"
)
selector = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(selector)
BASE, HEAD = "a" * 40, "b" * 40
sys.path.insert(0, str(Path(__file__).resolve().parent))
from _workflow_trigger_paths import classifier_globs, trigger_paths  # noqa: E402
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
                ".github/workflows/zk-toolchain.yml", ".github/actions/zk-exact-setup/action.yml"]


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
    def test_proof_and_reproducibility_jobs_are_separate_and_classification_gated(self):
        workflow = (ROOT / ".github/workflows/zk-exact-evaluator.yml").read_text()
        jobs = dict(re.findall(r"^  ([\w-]+):\n(.*?)(?=^  [\w-]+:\n|\Z)",
                               workflow.split("jobs:\n", 1)[1], re.M | re.S))
        self.assertIn("exact-evaluator", jobs)
        self.assertIn("guest-reproducibility", jobs)
        self.assertIn("\npermissions:\n  contents: read\n", workflow)
        for name, output in (("exact-evaluator", "proof_required"), ("guest-reproducibility", "required")):
            with self.subTest(job=name):
                job = jobs[name]
                header = job.split("    steps:\n", 1)[0]
                self.assertNotIn("needs:", header)
                self.assertNotIn("permissions:", header)
                self.assertNotIn("continue-on-error:", job)
                self.assertEqual([line.strip() for line in header.splitlines()
                                  if line.strip().startswith("if:")], [
                    "if: github.event_name != 'pull_request' || github.event.pull_request.draft != true",
                ])
                steps = job.split("\n      - ")[1:]
                by_name = {step.splitlines()[0].removeprefix("name: "): step for step in steps}
                self.assertIn("Classify exact-evaluator inputs (fail closed)", by_name)
                self.assertIn("Require explicit classification output", by_name)
                classify = by_name["Classify exact-evaluator inputs (fail closed)"]
                self.assertIn("\n        id: changes\n", classify)
                self.assertIn("\n        run: python3 scripts/ci_exact_evaluator_paths.py\n", classify)
                self.assertIn("EXACT_BASE_SHA: ${{ steps.merge_parent.outputs.sha || github.event.pull_request.base.sha || github.event.merge_group.base_sha || github.event.before }}", classify)
                self.assertIn("EXACT_HEAD_SHA: ${{ steps.merge_parent.outputs.sha && github.sha || github.event.pull_request.head.sha || github.event.merge_group.head_sha || github.sha }}", classify)
                require = by_name["Require explicit classification output"]
                self.assertEqual([line.strip() for line in require.splitlines()[1:]
                                  if line.strip()], [
                    "env:", "EXECUTION_REQUIRED: ${{ steps.changes.outputs.required }}",
                    "PROOF_REQUIRED: ${{ steps.changes.outputs.proof_required }}",
                    "run: |", 'case "$EXECUTION_REQUIRED:$PROOF_REQUIRED" in',
                    "true:true|true:false|false:false) ;;",
                    '*) echo "Missing or invalid exact-evaluator classification" >&2; exit 1 ;;',
                    "esac",
                ])
                self.assertLess(steps.index(classify), steps.index(require))
                for step in steps:
                    title = step.splitlines()[0]
                    if title.startswith("uses: actions/checkout@") or title in (
                        "name: Classify exact-evaluator inputs (fail closed)",
                        "name: Require explicit classification output",
                    ):
                        expected = []
                    elif title == "name: Resolve pull request merge parent":
                        expected = ["if: github.event_name == 'pull_request'"]
                    elif title == "name: Successful irrelevant-diff classification":
                        expected = [f"if: steps.changes.outputs.{output} == 'false'"]
                    elif "uses: actions/upload-artifact@" in step:
                        expected = [f"if: always() && steps.changes.outputs.{output} == 'true'"]
                    else:
                        expected = [f"if: steps.changes.outputs.{output} == 'true'"]
                        self.assertLess(steps.index(require), steps.index(step))
                    self.assertEqual([line.strip() for line in step.splitlines()
                                      if line.startswith("        if:")], expected, title)
                self.assertIn("persist-credentials: false", steps[0])
                self.assertIn("fetch-depth: 2", steps[0])
                for action in re.findall(r"^\s+(?:- )?uses: (\S+)", job, re.M):
                    if action.startswith("./"):
                        self.assertEqual(action, "./.github/actions/zk-exact-setup")
                    else:
                        self.assertRegex(action, r"^[\w-]+/[\w-]+@[0-9a-f]{40}$")
        proof, repeat = jobs["exact-evaluator"], jobs["guest-reproducibility"]
        self.assertIn("\n    timeout-minutes: 120\n", proof)
        self.assertIn("\n    timeout-minutes: 90\n", repeat)
        self.assertIn("cache-key: zk-exact-evaluator-risc0-3.0.6-rust-1.97.0", proof)
        self.assertIn("cache-key: zk-guest-reproducibility-risc0-3.0.6-rust-1.97.0", repeat)
        self.assertNotIn("sparq-guest-determinism", proof)
        self.assertNotIn("ci_exact_evaluator_evidence.py", repeat)
        self.assertIn("ci_exact_evaluator_evidence.py", proof)
        self.assertEqual(repeat.count("python3 zk/sparql-evaluator/scripts/guest_determinism.py --output"), 1)
        self.assertIn("name: guest-reproducibility-${{ github.sha }}-${{ github.run_attempt }}", repeat)
        self.assertIn("path: ${{ runner.temp }}/sparq-guest-determinism", repeat)
        self.assertIn("if-no-files-found: error", repeat)

    def test_both_jobs_require_shared_toolchain_action(self):
        workflow = (ROOT / ".github/workflows/zk-exact-evaluator.yml").read_text()
        jobs = dict(re.findall(r"^  ([\w-]+):\n(.*?)(?=^  [\w-]+:\n|\Z)",
                               workflow.split("jobs:\n", 1)[1], re.M | re.S))
        for name, output, key in (
            ("exact-evaluator", "proof_required", "zk-exact-evaluator-risc0-3.0.6-rust-1.97.0"),
            ("guest-reproducibility", "required", "zk-guest-reproducibility-risc0-3.0.6-rust-1.97.0"),
        ):
            with self.subTest(job=name):
                steps = jobs[name].split("\n      - ")[1:]
                matches = [step for step in steps if
                           "\n        uses: ./.github/actions/zk-exact-setup\n" in step]
                self.assertEqual(len(matches), 1)
                expected = [
                    "name: Set up exact evaluator toolchains",
                    f"if: steps.changes.outputs.{output} == 'true'",
                    "uses: ./.github/actions/zk-exact-setup",
                    "with:", f"cache-key: {key}",
                ]
                if name == "exact-evaluator":
                    expected.append("clippy: 'true'")
                self.assertEqual([line.strip() for line in matches[0].splitlines()
                                  if line.strip()], expected)
                index = steps.index(matches[0])
                self.assertTrue(steps[0].startswith("uses: actions/checkout@"))
                self.assertTrue(any(step.startswith("name: Require explicit classification output\n")
                                    for step in steps[1:index]))
                self.assertNotIn("Swatinem/rust-cache@", jobs[name])
                self.assertNotIn("rustup ", jobs[name])
                self.assertNotIn("curl ", jobs[name])

    def test_shared_toolchain_action_retains_pins_shells_and_exports(self):
        action = (ROOT / ".github/actions/zk-exact-setup/action.yml").read_text()
        self.assertIn("\nruns:\n  using: composite\n  steps:\n", action)
        steps = action.split("\n    - ")[1:]
        self.assertEqual(len(steps), 3)
        for step, name in zip(steps[:2], ("Install checksum-pinned RISC Zero binaries and guest Rust",
                                     "Activate pinned host Rust")):
            header = step.split("      run: |\n", 1)[0]
            expected = [f"name: {name}", "shell: bash"]
            if name == "Activate pinned host Rust":
                expected += ["env:", "INSTALL_CLIPPY: ${{ inputs.clippy }}"]
            self.assertEqual([line.strip() for line in header.splitlines()
                              if line.strip() and not line.strip().startswith("#")], expected)
        install = steps[0]
        for digest, archive in (("615d961bfb81d318db5071d7548389c850e324ac7f421c075176daf26082a60a", "cli.tgz"),
                                ("100b597d605b706bec94e1d32bfaaf5851df8d75173f986d4b56a3e90916be53", "rust.tar.gz")):
            self.assertIn(f'echo "{digest}  $tool_root/{archive}" | sha256sum --check --strict', install)
        self.assertIn("https://github.com/risc0/risc0/releases/download/v3.0.6/", install)
        self.assertIn("https://github.com/risc0/rust/releases/download/r0.1.97.0/", install)
        for export in ('echo "$tool_root/bin" >> "$GITHUB_PATH"',
                       'echo "RISC0_HOME=$tool_root/home" >> "$GITHUB_ENV"',
                       'echo "RISC0_SERVER_PATH=$tool_root/bin/r0vm" >> "$GITHUB_ENV"'):
            self.assertIn(export, install)
        self.assertEqual([line.strip() for line in steps[1].split("      run: |\n", 1)[1].splitlines()
                          if line.strip()], ["rustup show active-toolchain", 'if [ "$INSTALL_CLIPPY" = true ]; then',
                                             "rustup component add clippy", "fi"])
        self.assertEqual([line.strip() for line in steps[2].splitlines() if line.strip()], [
            "uses: Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32 # v2",
            "with:", "key: ${{ inputs.cache-key }}", "workspaces: zk/sparql-evaluator",
            "save-if: ${{ github.ref == 'refs/heads/main' }}",
        ])
        clippy_input = action.split("  clippy:\n", 1)[1].split("runs:\n", 1)[0]
        self.assertIn("    default: 'false'\n", clippy_input)
        for event in ("pull_request", "push"):
            self.assertIn(".github/actions/zk-exact-setup/**",
                          trigger_paths(ROOT / ".github/workflows/zk-exact-evaluator.yml", event))
        self.assertTrue(selector.relevant_path(".github/actions/zk-exact-setup/action.yml"))
        self.assertTrue(".github/actions/zk-exact-setup/action.yml".startswith(selector.PR_PREFIXES))
        lint = (ROOT / ".github/workflows/actionlint.yml").read_text()
        for event in ("pull_request", "push"):
            trigger = re.search(rf"^  {event}:\n(.*?)(?=^  \S|\Z)", lint, re.M | re.S)
            self.assertIsNotNone(trigger)
            self.assertIn('      - ".github/actions/**"', trigger.group(1).splitlines())

    def test_workflow_always_creates_the_gate_and_checks_classification(self):
        workflow = (Path(__file__).parents[2] / ".github/workflows/zk-exact-evaluator.yml").read_text()
        self.assertIn("  merge_group:\n    types: [checks_requested]", workflow)
        self.assertIn("name: real exact-dataset guest and proof", workflow)
        self.assertIn("Require explicit classification output", workflow)
        self.assertIn("python3 scripts/ci_exact_evaluator_paths.py", workflow)
        self.assertIn("python3 scripts/ci_exact_evaluator_evidence.py", workflow)

    def test_classifier_outputs_split_pull_requests_and_preserve_other_events(self):
        cases = [
            ("pull_request", "crates/sparq-engine/src/exec.rs", (True, False)),
            ("pull_request", "crates/sparq-core/Cargo.toml", (True, False)),
            ("pull_request", "crates/sparq-substrate/src/join.rs", (True, False)),
            ("pull_request", "crates/sparq-engine/tests/fixtures/builtin_edges.json", (True, False)),
            ("pull_request", "zk/sparql-evaluator/model/src/lib.rs", (True, True)),
            ("pull_request", ".github/actions/zk-exact-setup/action.yml", (True, True)),
            ("pull_request", "crates/sparq-engine/src/exec.rs\0docs/design.md", (True, False)),
            ("pull_request", "crates/sparq-engine/src/exec.rs\0zk/sparql-evaluator/Cargo.toml", (True, True)),
            ("pull_request", "docs/design.md", (False, False)),
        ]
        for event in ("push", "merge_group", "workflow_dispatch"):
            cases.append((event, "crates/sparq-engine/src/exec.rs", (True, True)))
        for event in ("push", "merge_group"):
            cases.append((event, "docs/design.md", (False, False)))
        for event, paths, (required, proof) in cases:
            with self.subTest(event=event, paths=paths), tempfile.TemporaryDirectory() as tmp:
                output = Path(tmp) / "output"
                env = {"GITHUB_EVENT_NAME": event, "EXACT_BASE_SHA": BASE,
                       "EXACT_HEAD_SHA": HEAD, "GITHUB_OUTPUT": str(output)}
                with patch.dict(selector.os.environ, env), \
                        patch.object(sys, "argv", ["ci_exact_evaluator_paths.py"]), \
                        patch.object(selector.subprocess, "run", side_effect=[
                            completed(), completed(paths.encode() + b"\0")]) as run, \
                        contextlib.redirect_stdout(io.StringIO()):
                    selector.main()
                self.assertEqual(output.read_text(),
                                 f"required={str(required).lower()}\nproof_required={str(proof).lower()}\n")
                self.assertEqual(run.call_count, 0 if event == "workflow_dispatch" else 2)

    def test_uncertain_pull_request_requires_both_jobs(self):
        for outcome in (OSError(), subprocess.CalledProcessError(1, "git"),
                        subprocess.TimeoutExpired("git", 60), completed(b"crates/sparq-engine/src/exec.rs")):
            with self.subTest(outcome=outcome), patch.object(selector.subprocess, "run", side_effect=[
                completed(), outcome,
            ]):
                self.assertEqual(selector.execution_requirements("pull_request", BASE, HEAD), (True, True))
        with patch.object(selector.subprocess, "run") as run:
            self.assertEqual(selector.execution_requirements("pull_request", "", HEAD), (True, True))
            run.assert_not_called()

    def test_missing_or_invalid_classifier_outputs_fail_both_job_guards(self):
        workflow = (ROOT / ".github/workflows/zk-exact-evaluator.yml").read_text()
        jobs = dict(re.findall(r"^  ([\w-]+):\n(.*?)(?=^  [\w-]+:\n|\Z)",
                               workflow.split("jobs:\n", 1)[1], re.M | re.S))
        for name in ("exact-evaluator", "guest-reproducibility"):
            steps = jobs[name].split("\n      - ")[1:]
            step = next(s for s in steps if s.startswith("name: Require explicit classification output\n"))
            script = textwrap.dedent(step.split("        run: |\n", 1)[1])
            for required, proof, accepted in (("true", "true", True), ("true", "false", True),
                                               ("false", "false", True), ("false", "true", False),
                                               ("", "true", False), ("true", "", False), ("", "", False),
                                               ("TRUE", "false", False), ("false", "unknown", False)):
                with self.subTest(job=name, required=required, proof=proof):
                    result = subprocess.run(["bash", "-e", "-c", script], capture_output=True,
                                            env=dict(os.environ, EXECUTION_REQUIRED=required, PROOF_REQUIRED=proof))
                    self.assertEqual(result.returncode == 0, accepted)

    def test_host_activation_installs_clippy_only_when_requested(self):
        action = (ROOT / ".github/actions/zk-exact-setup/action.yml").read_text()
        step = action.split("\n    - ")[2]
        script = textwrap.dedent(step.split("      run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            rustup = directory / "rustup"
            rustup.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$RUSTUP_TRACE"\n')
            rustup.chmod(0o755)
            for clippy, expected in (("false", ["show active-toolchain"]),
                                     ("true", ["show active-toolchain", "component add clippy"])):
                with self.subTest(clippy=clippy):
                    trace = directory / clippy
                    subprocess.run(["/bin/bash", "-e", "-c", script], check=True,
                                   env=dict(os.environ, PATH=tmp, RUSTUP_TRACE=str(trace), INSTALL_CLIPPY=clippy))
                    self.assertEqual(trace.read_text().splitlines(), expected)

    def test_trigger_paths_filters_equal_the_classifier(self):
        # The push filter only spares a runner; it must never drop an input the classifier
        # keeps. The pull_request filter is the narrower zk-only set, checked after merge.
        workflow = ROOT / ".github/workflows/zk-exact-evaluator.yml"
        self.assertEqual(trigger_paths(workflow, "push"),
                         classifier_globs(selector.EXACT_PREFIXES, selector.EXACT_FILES))
        self.assertEqual(trigger_paths(workflow, "pull_request"),
                         classifier_globs(selector.PR_PREFIXES, selector.PR_FILES))
        self.assertEqual(trigger_paths(workflow, "merge_group"), [])

    def test_pull_request_inputs_cover_guest_dependency_crates(self):
        self.assertLessEqual(set(selector.PR_PREFIXES), set(selector.EXACT_PREFIXES))
        for path in selector.PR_FILES:
            self.assertTrue(selector.relevant_path(path), path)
            self.assertTrue((ROOT / path).is_file(), path)
        for path in ("crates/sparq-engine/src/exec.rs", "crates/sparq-core/Cargo.toml",
                     "crates/sparq-substrate/src/join.rs"):
            self.assertTrue(path.startswith(selector.PR_PREFIXES), path)
        for path in ("Cargo.lock", "Cargo.toml"):
            self.assertFalse(path.startswith(selector.PR_PREFIXES) or path in selector.PR_FILES,
                             path)

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
