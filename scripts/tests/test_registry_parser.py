"""[OPUS-5.5] Registry-only parser gate: scan, consumer, graph, evidence, CI wiring.

These tests never invoke Cargo, rustc or the network. Commands run through a
fake runner, except for the bounded-runner tests, which use small Python
subprocesses to exercise timeouts and output caps for real.
"""

import copy
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import tomllib
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
ROOT = SCRIPTS.parent
WORKFLOW = ROOT / ".github/workflows/zk-exact-evaluator.yml"


def load(name):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    # [GPT-6] Match normal imports so dataclass annotations can find their module.
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


registry = load("check_registry_parser")
selector = load("ci_exact_evaluator_paths")
summary_gate = load("ci_summary_gate")

CONTRACT_FILES = (registry.STABLE_CONTRACT, registry.CORPUS, registry.FORK_DIFFERENTIAL)
PARSER_ID = f"{registry.REGISTRY_SOURCE}#spargebra@0.4.6"
REGISTRY_MANIFEST = "/cargo-home/registry/src/index.crates.io-1949cf8c6b5b557f/spargebra-0.4.6/Cargo.toml"
LOCAL_CRATES = ("sparq-core", "sparq-substrate", "sparq-engine", "sparq-text", "sparq-shacl")
# A realistic regression: a migrated caller reverted to the fork-only method.
MUTATION = re.compile(r"(?:sparq_engine::|crate::)?parse_versioned_(query|update)\(\s*([^,]+?),\s*")


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    return path


def contract_tree(root):
    """A minimal checkout holding the real contract, corpus and differential."""
    for relative in CONTRACT_FILES:
        write(root, relative, (ROOT / relative).read_text())
    return root


def line_of(text, needle):
    return text[:text.index(needle)].count("\n") + 1


def revert_first_call(text):
    """Rewrite the first non-comment `parse_versioned_*` call to the fork method."""
    for match in MUTATION.finditer(text):
        line_start = text.rfind("\n", 0, match.start()) + 1
        if text[line_start:match.start()].lstrip().startswith("//"):
            continue
        kind, parser = match.group(1), match.group(2)
        return text[:match.start()] + f"{parser}.parse_{kind}_with_versions(" + text[match.end():], kind
    raise AssertionError("no production call to mutate")


def fake_metadata(consumer, escaping, root=ROOT):
    """`cargo metadata` output for the consumer graph, as Cargo would shape it."""
    consumer_id = f"path+file://{consumer}#{registry.CONSUMER}@0.0.0"
    packages = [{"name": registry.CONSUMER, "id": consumer_id, "source": None, "version": "0.0.0",
                 "manifest_path": str(consumer / "Cargo.toml")}]
    nodes = []
    link = [{"name": "spargebra", "pkg": PARSER_ID, "dep_kinds": [{"kind": None, "target": None}]}]
    for crate in LOCAL_CRATES:
        crate_id = f"path+file://{root}/crates/{crate}#0.1.1"
        packages.append({"name": crate, "id": crate_id, "source": None, "version": "0.1.1",
                         "manifest_path": str(root / "crates" / crate / "Cargo.toml")})
        features = ["default"]
        if crate == "sparq-engine":
            features = sorted({"default", "digest", "parallel", "regex", *registry.ENGINE_FEATURES})
        nodes.append({"id": crate_id, "deps": copy.deepcopy(link) if crate in registry.REGISTRY_COMPILED else [],
                      "features": features})
    packages.append({"name": "spargebra", "id": PARSER_ID, "source": registry.REGISTRY_SOURCE,
                     "version": "0.4.6", "manifest_path": REGISTRY_MANIFEST})
    parser_features = {"default", *registry.PARSER_FEATURES} | ({registry.ESCAPING_FEATURE} if escaping else set())
    nodes.append({"id": PARSER_ID, "deps": [], "features": sorted(parser_features)})
    nodes.append({"id": consumer_id, "deps": copy.deepcopy(link), "features": []})
    return {"version": 1, "packages": packages, "workspace_members": [consumer_id],
            "resolve": {"nodes": nodes, "root": consumer_id}}


def resolved_lock(seed):
    """The seeded root lock after Cargo swaps in the registry parser."""
    block = 'name = "spargebra"\nversion = "0.4.6"\n'
    assert seed.count(block) == 1
    registry_block = block + f'source = "{registry.REGISTRY_SOURCE}"\nchecksum = "{registry.PARSER_CHECKSUM}"\n'
    return (seed.replace(block, registry_block)
            + f'\n[[package]]\nname = "{registry.CONSUMER}"\nversion = "0.0.0"\n')


class FakeTools:
    """Stands in for git, rustc and cargo, recording every invocation."""

    def __init__(self, channel, fail=(), timeout=(), unified_escaping=False):
        self.channel, self.fail, self.timeout = channel, set(fail), set(timeout)
        self.unified_escaping = unified_escaping
        self.calls = []

    def __call__(self, argv, *, cwd, env, log_path, timeout, stdout_path=None):
        name = log_path.stem
        cwd = Path(cwd)
        self.calls.append({"name": name, "argv": list(argv), "cwd": cwd, "env": dict(env), "timeout": timeout})
        text = ""
        if argv[0] == "git":
            text = f"{'a' * 40}\n{'b' * 40}\n" if "rev-parse" in argv else ""
        elif argv[1:] == ["-vV"]:
            text = f"{argv[0]} {self.channel} (fake)\nrelease: {self.channel}\nhost: x86_64-unknown-linux-gnu\n"
        elif argv[1] == "update":
            (cwd / "Cargo.lock").write_text(resolved_lock((cwd / "Cargo.lock").read_text()))
            text = "Adding spargebra v0.4.6\n"
        elif argv[1] == "metadata":
            escaping = "--features" in argv or self.unified_escaping
            stdout_path.write_text(json.dumps(fake_metadata(cwd, escaping)))
        elif argv[1] == "test":
            fork = "parser_versions_fork" in argv
            sources = [ROOT / registry.STABLE_CONTRACT] + ([ROOT / registry.FORK_DIFFERENTIAL] if fork else [])
            names = [test for source in sources for test in registry.declared_tests(source)]
            if name in self.fail:
                names.remove(registry.MODE_TEST)
                text += f"test {registry.MODE_TEST} ... FAILED\n"
            prefix = "" if fork else "contract::"
            text += "".join(f"test {prefix}{test} ... ok\n" for test in names)
        log_path.write_text(text)
        if name in self.timeout:
            return registry.RunResult(None, True, False, 1.0)
        return registry.RunResult(101 if name in self.fail else 0, False, False, 0.25)


class StaticScan(unittest.TestCase):
    def test_real_checkout_passes_and_inventories_every_migrated_caller(self):
        result = registry.scan(ROOT)
        self.assertTrue(result["passed"], result)
        self.assertGreater(result["fork_differential_lines"], 0)
        self.assertEqual(result["registry_compiled_callers"],
                         {"sparq-engine": 5, "sparq-text": 1, "sparq-shacl": 1})
        self.assertEqual(result["scan_only_callers"], {"sparq-vectors": 1, "sparq-server": 1, "sparq-solid": 3,
                                                       "sparq-py": 1, "sparq-lws-core": 1})
        self.assertEqual(sum(result["callers"].values()), 14)
        for path in result["callers"]:
            # A caller edit must rerun this gate through the fail-closed selector.
            self.assertTrue(selector.relevant_path(path, "registry-parser"), path)

    def test_every_migrated_caller_reverted_to_the_fork_method_is_rejected(self):
        callers = registry.scan(ROOT)["callers"]
        self.assertTrue(callers)
        for relative in callers:
            with self.subTest(relative), tempfile.TemporaryDirectory() as directory:
                root = contract_tree(Path(directory))
                mutated, kind = revert_first_call((ROOT / relative).read_text())
                write(root, relative, mutated)
                result = registry.scan(root)
                self.assertFalse(result["passed"])
                self.assertEqual([(v["path"], v["line"]) for v in result["violations"]],
                                 [(relative, line_of(mutated, f"parse_{kind}_with_versions"))])

    def test_realistic_forms_are_rejected_everywhere_but_the_differential(self):
        stable_addition = ('\n#[test]\nfn fork_only_call() {\n'
                           '    let _ = SparqlParser::new().parse_query_with_versions("ASK {}");\n}\n')
        one_liner = 'fn f() { let _ = spargebra::SparqlParser::new().parse_query_with_versions(""); }\n'
        with tempfile.TemporaryDirectory() as directory:
            root = contract_tree(Path(directory))
            write(root, "crates/sparq-server/src/handler.rs",
                  "use spargebra::{SparqlParser, Update};\n\n"
                  "fn parse(text: &str, base: &str) -> Result<(Update, Vec<String>), String> {\n"
                  "    SparqlParser::new()\n"
                  "        .with_base_iri(base)\n"
                  "        .map_err(|error| error.to_string())?\n"
                  "        .parse_update_with_versions(text)\n"
                  "        .map_err(|error| error.to_string())\n"
                  "}\n")
            write(root, "crates/sparq-cli/src/main.rs",
                  "fn main() {\n"
                  "    let query = std::env::args().nth(1).unwrap_or_default();\n"
                  "    let parsed = spargebra::SparqlParser::parse_query_with_versions(spargebra::SparqlParser::new(), &query);\n"
                  "    let parse = spargebra::SparqlParser::parse_update_with_versions;\n"
                  "    println!(\"{parsed:?} {}\", parse(spargebra::SparqlParser::new(), \"\").is_ok());\n"
                  "}\n")
            write(root, "crates/sparq-py/src/versions.rs",
                  "pub fn labels(parser: spargebra::SparqlParser, text: &str) -> Vec<String> {\n"
                  "    parser.r#parse_query_with_versions(text).map(|(_, labels)| labels).unwrap_or_default()\n"
                  "}\n")
            # Comments are not exempt: nothing outside the differential needs the names.
            write(root, "crates/sparq-solid/src/notes.rs",
                  "// Previously this used SparqlParser::parse_query_with_versions directly.\n"
                  "/// See [`spargebra::SparqlParser::parse_update_with_versions`].\n"
                  "pub fn noop() {}\n")
            stable = root / registry.STABLE_CONTRACT
            stable.write_text(stable.read_text() + stable_addition)
            write(root, "crates/sparq-engine/tests/ebv_dialects_extra.rs",
                  "#[test]\nfn labels() {\n    let _ = spargebra::SparqlParser::new().parse_update_with_versions(\"\");\n}\n")
            # The exemption is one exact path, not a file name.
            for relative in ["crates/sparq-text/tests/parser_versions_fork.rs",
                             "crates/sparq-engine/tests/parser_versions/parser_versions_fork.rs",
                             "crates/sparq-core/build.rs", "crates/sparq-engine/examples/labels.rs",
                             "crates/sparq-engine/benches/parse.rs"]:
                write(root, relative, one_liner)
            # Longer identifiers that merely contain the names are not fork calls.
            write(root, "crates/sparq-lws-core/src/fine.rs",
                  "pub fn wrap(query: spargebra::Query, versions: Vec<String>) -> Result<sparq_engine::PreparedQuery, String> {\n"
                  "    sparq_engine::PreparedQuery::from_query_with_versions(query, versions)\n"
                  "}\n"
                  "fn my_parse_query_with_versions_helper() {}\n"
                  "const METRIC: &str = \"parse_update_with_versions_total\";\n")
            # Cargo target directories are generated output, not crate sources.
            write(root, "crates/sparq-engine/target/CACHEDIR.TAG", "Signature: 8a477f597d28d172789f06886806bc55\n")
            write(root, "crates/sparq-engine/target/debug/build/out/generated.rs", one_liner)
            result = registry.scan(root)
            stable_line = line_of(stable.read_text(), "parse_query_with_versions")
        self.assertFalse(result["passed"])
        self.assertEqual(result["errors"], [])
        self.assertEqual(sorted((v["path"], v["line"]) for v in result["violations"]), sorted([
            ("crates/sparq-server/src/handler.rs", 7),
            ("crates/sparq-cli/src/main.rs", 3), ("crates/sparq-cli/src/main.rs", 4),
            ("crates/sparq-py/src/versions.rs", 2),
            ("crates/sparq-solid/src/notes.rs", 1), ("crates/sparq-solid/src/notes.rs", 2),
            (registry.STABLE_CONTRACT, stable_line),
            ("crates/sparq-engine/tests/ebv_dialects_extra.rs", 3),
            ("crates/sparq-text/tests/parser_versions_fork.rs", 1),
            ("crates/sparq-engine/tests/parser_versions/parser_versions_fork.rs", 1),
            ("crates/sparq-core/build.rs", 1), ("crates/sparq-engine/examples/labels.rs", 1),
            ("crates/sparq-engine/benches/parse.rs", 1),
        ]))

    def test_scan_requires_the_differential_and_a_registry_safe_contract(self):
        def replace(relative, old, new):
            return lambda root: (root / relative).write_text((root / relative).read_text().replace(old, new))

        cases = [
            (lambda root: (root / registry.FORK_DIFFERENTIAL).unlink(), "missing the dedicated fork differential"),
            (replace(registry.FORK_DIFFERENTIAL, ".parse_update_with_versions(", ".parse_update("),
             "no longer calls the fork's parse_update_with_versions"),
            (replace(registry.CORPUS, "use spargebra::SparqlParser;",
                     'use spargebra::SparqlParser;\n#[cfg(feature = "sparql-12")]\nconst GATED: () = ();'),
             "free of crate-feature cfgs"),
            (replace(registry.STABLE_CONTRACT, registry.MODE_VARIABLE, "SPARQ_MODE"),
             f"no longer checks {registry.MODE_VARIABLE}"),
            (lambda root: write(root, "crates/sparq-geo/src/rewrite.rs",
                                "fn f(q: &str) {\n    let _ = sparq_engine::parse_versioned_query(\n"
                                "        spargebra::SparqlParser::new(), q);\n}\n"),
             "sparq-geo calls parse_versioned_"),
        ]
        for change, message in cases:
            with self.subTest(message), tempfile.TemporaryDirectory() as directory:
                root = contract_tree(Path(directory))
                write(root, "crates/sparq-engine/src/lib.rs", "pub fn f() {}\n")
                self.assertTrue(registry.scan(root)["passed"])
                change(root)
                result = registry.scan(root)
                self.assertFalse(result["passed"])
                self.assertTrue(any(message in error for error in result["errors"]), result["errors"])

    def test_caller_inventory_skips_definitions_and_documentation(self):
        text = ("/// let (q, v) = sparq_engine::parse_versioned_query(parser, text)?;\n"
                "pub fn parse_versioned_query(parser: SparqlParser, query: &str) {}\n"
                "let first = crate::parse_versioned_update(\n    SparqlParser::new(), text);\n"
                "let second = parse_versioned_query (parser, text);\n"
                "pub use versioned_parse::{parse_versioned_query, parse_versioned_update};\n")
        self.assertEqual(registry.caller_count(text), 2)

    def test_require_scan_returns_counts_and_raises_on_a_fork_method(self):
        # [OPUS-5.5] The gate's static-scan step: the differential's fork calls are
        # allowed, one anywhere else fails the step, and evidence is kept either way.
        with tempfile.TemporaryDirectory() as directory:
            root = contract_tree(Path(directory))
            write(root, "crates/sparq-text/src/lib.rs",
                  "pub fn f(q: &str) {\n    let _ = sparq_engine::parse_versioned_query(\n"
                  "        spargebra::SparqlParser::new(), q);\n}\n")
            evidence = {}
            self.assertEqual(registry.require_scan(root, evidence), {"files_scanned": 4, "production_calls": 1})
            self.assertTrue(evidence["scan"]["passed"])
            self.assertGreater(evidence["scan"]["fork_differential_lines"], 0)
            self.assertEqual(evidence["scan"]["callers"], {"crates/sparq-text/src/lib.rs": 1})
            write(root, "crates/sparq-text/src/fork.rs",
                  "pub fn g(q: &str) {\n    let _ = spargebra::SparqlParser::new().parse_update_with_versions(q);\n}\n")
            evidence = {}
            with self.assertRaisesRegex(registry.GateError, r"^1 forbidden fork-method mentions; errors: \[\]$"):
                registry.require_scan(root, evidence)
            self.assertFalse(evidence["scan"]["passed"])
            self.assertEqual([(v["path"], v["line"]) for v in evidence["scan"]["violations"]],
                             [("crates/sparq-text/src/fork.rs", 2)])


class ConsumerAndGraph(unittest.TestCase):
    def test_consumer_is_a_patch_free_workspace_reusing_the_unchanged_contract(self):
        text = registry.consumer_manifest(ROOT)
        manifest = tomllib.loads(text)
        self.assertIn("workspace", manifest)
        self.assertNotIn("patch", manifest)
        self.assertIsNone(re.search(r"(?m)^\s*\[patch", text))
        dependencies = manifest["dependencies"]
        self.assertFalse([name for name, dependency in dependencies.items()
                          if isinstance(dependency, dict) and "vendor/" in dependency["path"]])
        self.assertEqual(dependencies["spargebra"], "=0.4.6")
        self.assertEqual(dependencies["sparq-engine"]["features"], ["explain-json", "params", "window-functions"])
        for crate in ("sparq-engine", "sparq-text", "sparq-shacl"):
            self.assertEqual(Path(dependencies[crate]["path"]), ROOT / "crates" / crate)
        library = registry.consumer_library()
        for crate in ("sparq_engine", "sparq_text", "sparq_shacl"):
            self.assertIn(f"pub use {crate};", library)
        included = Path(re.fullmatch(r'(?s).*#\[path = "([^"]+)"\]\nmod contract;\n', registry.contract_shim(ROOT))[1])
        self.assertEqual(included, ROOT / registry.STABLE_CONTRACT)
        # The contract's own `#[path]` resolves beside it, so the corpus is reused too.
        self.assertIn('#[path = "parser_versions/cases.rs"]', included.read_text())
        self.assertTrue((included.parent / "parser_versions/cases.rs").is_file())

    def test_paths_are_quoted_for_toml_and_rust(self):
        awkward = Path('/tmp/sp ace/qu"ote\\slash')
        self.assertEqual(Path(tomllib.loads(f"path = {registry.quoted(awkward)}")["path"]), awkward)
        with self.assertRaisesRegex(registry.GateError, "control character"):
            registry.quoted(Path("/tmp/line\nbreak"))

    def test_seeding_is_fresh_detached_and_leaves_existing_directories_alone(self):
        with tempfile.TemporaryDirectory() as directory:
            consumer = Path(directory) / "consumer"
            seeded = registry.seed_consumer(ROOT, consumer)
            self.assertEqual((consumer / "Cargo.lock").read_bytes(), (ROOT / "Cargo.lock").read_bytes())
            self.assertEqual(seeded["seed_lock_sha256"], registry.sha256_file(ROOT / "Cargo.lock"))
            (consumer / "keep.txt").write_text("user data")
            with self.assertRaises(FileExistsError):
                registry.seed_consumer(ROOT, consumer)
            self.assertEqual((consumer / "keep.txt").read_text(), "user data")
            (Path(directory) / ".cargo").mkdir()
            (Path(directory) / ".cargo/config.toml").write_text("[patch.crates-io]\n")
            with self.assertRaisesRegex(registry.GateError, "would be inherited"):
                registry.ensure_detached(Path(directory) / "other", ROOT)
        for inside in (ROOT, ROOT / "crates", ROOT.parent):
            with self.assertRaisesRegex(registry.GateError, "outside the checkout"):
                registry.ensure_detached(inside, ROOT)

    def test_lock_admits_only_the_official_registry_parser(self):
        seed = (ROOT / "Cargo.lock").read_text()
        resolved = resolved_lock(seed)
        parser_source = f'name = "spargebra"\nversion = "0.4.6"\nsource = "{registry.REGISTRY_SOURCE}"\n'
        self.assertEqual(resolved.count(parser_source), 1)
        result = registry.check_consumer_lock(seed, resolved)
        self.assertEqual(result["parser"], {"version": "0.4.6", "source": registry.REGISTRY_SOURCE,
                                            "checksum": registry.PARSER_CHECKSUM})
        for label, text, message in [
            ("unresolved path parser", seed, "not the official registry release"),
            ("wrong checksum", resolved.replace(registry.PARSER_CHECKSUM, "0" * 64), "not the official"),
            ("patched git parser", resolved.replace(parser_source, parser_source.replace(
                registry.REGISTRY_SOURCE, "git+https://github.com/oxigraph/oxigraph#abc")), "not the official"),
            ("second parser", resolved + '\n[[package]]\nname = "spargebra"\nversion = "0.4.7"\n', "exactly one"),
            ("drifted dependency", resolved + '\n[[package]]\nname = "oxrdf"\nversion = "9.9.9"\n'
             f'source = "{registry.REGISTRY_SOURCE}"\n', "other than the parser"),
        ]:
            with self.subTest(label), self.assertRaisesRegex(registry.GateError, message):
                registry.check_consumer_lock(seed, text)

    def test_resolved_graph_links_one_registry_parser_with_the_mode_features(self):
        consumer = Path("/tmp/sparq-registry-parser-test/consumer")
        for escaping in (False, True):
            result = registry.check_registry_graph(fake_metadata(consumer, escaping), ROOT, escaping)
            self.assertEqual(result["parser"]["source"], registry.REGISTRY_SOURCE)
            self.assertEqual(escaping, registry.ESCAPING_FEATURE in result["parser"]["features"])
            self.assertEqual(result["path_packages"]["sparq-text"], "crates/sparq-text")
            self.assertIn("crates/sparq-engine/Cargo.toml", result["path_package_manifests_sha256"])

        def parser(metadata):
            return next(p for p in metadata["packages"] if p["name"] == "spargebra")

        def node(metadata, name):
            package_id = next(p["id"] for p in metadata["packages"] if p["name"] == name)
            return next(n for n in metadata["resolve"]["nodes"] if n["id"] == package_id)

        def unified(metadata):
            node(metadata, "spargebra")["features"].append(registry.ESCAPING_FEATURE)

        def geo(metadata):
            metadata["packages"].append({"name": "sparq-geo", "id": "geo", "source": None,
                                         "manifest_path": str(ROOT / "crates/sparq-geo/Cargo.toml")})

        cases = [
            ("feature unification enables escaping", False, unified, "differ from the expected"),
            ("escaping requested but absent", True,
             lambda m: node(m, "spargebra")["features"].remove(registry.ESCAPING_FEATURE), "differ from the expected"),
            ("vendored path parser", False, lambda m: parser(m).update(
                source=None, manifest_path=str(ROOT / "vendor/spargebra/Cargo.toml")), "not the registry release"),
            ("git patch", False, lambda m: parser(m).update(source="git+https://github.com/oxigraph/oxigraph#abc"),
             "not the registry release"),
            ("replacement inside checkout", False,
             lambda m: parser(m).update(manifest_path=str(ROOT / "vendor/spargebra/Cargo.toml")), "inside the checkout"),
            ("second parser", False, lambda m: m["packages"].append(dict(parser(m), id="other", version="0.4.7")),
             "exactly one"),
            ("engine feature missing", False, lambda m: node(m, "sparq-engine")["features"].remove("params"),
             "lacks the required features"),
            ("caller links another parser", False,
             lambda m: node(m, "sparq-text")["deps"][0].update(pkg="other"), "does not link"),
            ("engine becomes a member", False,
             lambda m: m["workspace_members"].append(node(m, "sparq-engine")["id"]), "only workspace member"),
            ("uncovered path package", False, geo, "selector would not rerun"),
        ]
        for label, escaping, change, message in cases:
            metadata = fake_metadata(consumer, escaping)
            change(metadata)
            with self.subTest(label), self.assertRaisesRegex(registry.GateError, message):
                registry.check_registry_graph(metadata, ROOT, escaping)

    def test_root_graph_is_the_vendored_fork(self):
        self.assertEqual(registry.fork_graph_identity(ROOT)["parser"], "vendor/spargebra")

    def test_commands_are_locked_and_mode_specific(self):
        target = Path("/tmp/target")
        for escaping in (False, True):
            root_run = registry.root_test_command(ROOT, target, escaping)
            consumer_run = registry.consumer_test_command(target, escaping)
            for command in (root_run, consumer_run):
                self.assertIn("--locked", command)
                self.assertEqual(command[command.index("--target-dir") + 1], str(target))
                self.assertEqual("spargebra/standard-unicode-escaping" in command, escaping)
            self.assertEqual(root_run[root_run.index("-p") + 1], "sparq-engine")
            self.assertIn("parser_versions_fork", root_run)
            self.assertEqual(consumer_run[consumer_run.index("--test") + 1], "stable_contract")
            self.assertEqual(registry.mode_environment({}, escaping)[registry.MODE_VARIABLE], "1" if escaping else "0")
        env = registry.cargo_environment({"CARGO_TARGET_DIR": "/elsewhere", registry.MODE_VARIABLE: "1",
                                          "RUSTUP_TOOLCHAIN": "stable"}, "1.97.1")
        self.assertNotIn("CARGO_TARGET_DIR", env)
        self.assertNotIn(registry.MODE_VARIABLE, env)
        self.assertEqual(env["RUSTUP_TOOLCHAIN"], "1.97.1")

    def test_toolchain_pin_is_exact_and_compared_with_actual_releases(self):
        self.assertRegex(registry.pinned_channel(ROOT), r"^\d+\.\d+\.\d+$")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, "rust-toolchain.toml", '[toolchain]\nchannel = "stable"\n')
            with self.assertRaisesRegex(registry.GateError, "exact release"):
                registry.pinned_channel(root)
            recorder = registry.Recorder(root)
            for place in ("root", "detached"):
                for tool in ("rustc", "cargo"):
                    version = "1.88.0" if (place, tool) == ("detached", "rustc") else "1.97.1"
                    write(root, f"logs/toolchain-{place}-{tool}.log", f"{tool} {version}\nrelease: {version}\n")
            with self.assertRaisesRegex(registry.GateError, "detached_rustc"):
                registry.toolchain_pin(recorder, "1.97.1")

    def test_test_logs_must_report_every_configured_test_including_the_mode_check(self):
        sources = [ROOT / registry.STABLE_CONTRACT]
        names = registry.declared_tests(sources[0])
        self.assertIn(registry.MODE_TEST, names)
        self.assertEqual(len(registry.declared_tests(ROOT / registry.FORK_DIFFERENTIAL)), 3)
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "run.log"
            log.write_text("".join(f"test contract::{name} ... ok\n" for name in names))
            self.assertEqual(registry.check_tests(log, sources)["passed_tests"], len(names))
            for outcome in ("FAILED", "ignored"):
                log.write_text("".join(f"test {name} ... {outcome if name == registry.MODE_TEST else 'ok'}\n"
                                       for name in names))
                with self.assertRaisesRegex(registry.GateError, registry.MODE_TEST):
                    registry.check_tests(log, sources)


class BoundedRunner(unittest.TestCase):
    def run_python(self, directory, code, **options):
        log = Path(directory) / "run.log"
        options.setdefault("poll", 0.05)
        result = registry.run_bounded([sys.executable, "-c", code], cwd=Path(directory), env=dict(os.environ),
                                      log_path=log, **options)
        return result, log

    def test_output_goes_to_files(self):
        with tempfile.TemporaryDirectory() as directory:
            stdout = Path(directory) / "out.json"
            result, log = self.run_python(directory, "import sys; print('{}'); print('diagnostic', file=sys.stderr)",
                                          timeout=60, stdout_path=stdout)
            self.assertEqual((result.exit_code, result.timed_out, result.oversized), (0, False, False))
            self.assertEqual(stdout.read_text().strip(), "{}")
            self.assertIn("diagnostic", log.read_text())

    def test_timeout_and_output_cap_kill_the_command(self):
        with tempfile.TemporaryDirectory() as directory:
            result, _ = self.run_python(directory, "import time; time.sleep(60)", timeout=0.3)
            self.assertTrue(result.timed_out)
            self.assertIsNone(result.exit_code)
            self.assertLess(result.elapsed_seconds, 30)
            result, log = self.run_python(
                directory, "import sys, time\nsys.stdout.write('x' * 200000)\nsys.stdout.flush()\ntime.sleep(60)",
                timeout=60, max_bytes=1000)
            self.assertTrue(result.oversized)
            self.assertIsNone(result.exit_code)

    def test_quick_successful_exit_past_the_output_cap_fails(self):
        # [OPUS-5.5] The poll outlasts the command, so it exits before any poll and
        # only the post-exit size check can see the output.
        stdout_big = "import sys; sys.stdout.write('x' * 2000)"
        stderr_big = "import sys; sys.stderr.write('x' * 2000)"
        cases = [
            ("combined stdout", stdout_big, False),
            ("combined stderr", stderr_big, False),
            ("split stdout", stdout_big, True),
            ("split stderr", stderr_big, True),
            ("split files each under the cap", "import sys; sys.stdout.write('x' * 600); "
             "sys.stderr.write('x' * 600)", True),
        ]
        for label, code, split in cases:
            with self.subTest(label), tempfile.TemporaryDirectory() as directory:
                stdout = Path(directory) / "out.json" if split else None
                result, _ = self.run_python(directory, code, timeout=3600, poll=3600, max_bytes=1000,
                                            stdout_path=stdout)
                self.assertEqual((result.exit_code, result.timed_out, result.oversized), (0, False, True))

    def test_exit_after_the_deadline_is_timed_out_and_keeps_the_output_failure(self):
        # [OPUS-5.5] A fake clock, not wall time: the command starts at 0 and exits at 100.
        readings = iter([0.0])
        clock = SimpleNamespace(monotonic=lambda: next(readings, 100.0))
        with tempfile.TemporaryDirectory() as directory, patch.object(registry, "time", clock):
            result, _ = self.run_python(directory, "import sys; sys.stdout.write('x' * 2000)",
                                        timeout=5, poll=3600, max_bytes=1000)
        self.assertEqual((result.exit_code, result.timed_out, result.oversized), (0, True, True))

    def test_quick_exits_past_a_limit_fail_the_recorded_step(self):
        # [OPUS-5.5] The Recorder turns either flag into a failed step despite exit 0.
        for flags in ((True, False), (False, True)):
            with self.subTest(flags), tempfile.TemporaryDirectory() as directory:
                output = Path(directory)
                (output / "logs").mkdir()
                recorder = registry.Recorder(output, lambda *_, **__: registry.RunResult(0, *flags, 0.1))
                self.assertFalse(recorder.command("quick", ["tool"], cwd=output, env={}, timeout=5))
                self.assertEqual(recorder.steps[0]["exit_code"], 0)

    def test_missing_tool_is_a_recorded_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            (output / "logs").mkdir()
            recorder = registry.Recorder(output)
            self.assertFalse(recorder.command("missing", ["sparq-no-such-tool-for-tests"], cwd=output,
                                              env=dict(os.environ), timeout=5))
            self.assertIsNone(recorder.steps[0]["exit_code"])
            self.assertIn("could not run", recorder.steps[0]["detail"])


class GateEvidence(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.base = Path(self.directory.name)
        self.target = self.base / "user-target"
        self.target.mkdir()
        (self.target / "keep.txt").write_text("cache")
        self.channel = registry.pinned_channel(ROOT)
        self.locks = registry.lock_hashes(ROOT)
        environment = patch.dict(os.environ)
        environment.start()
        self.addCleanup(environment.stop)
        for key in registry.FORBIDDEN_ENVIRONMENT:
            os.environ.pop(key, None)

    def run_gate(self, name, tools):
        output = self.base / name
        status = registry.gate(ROOT, output, self.target, runner=tools)
        evidence = json.loads((output / "evidence.json").read_text())
        # Neither the user-supplied target nor any source lock is touched.
        self.assertEqual((self.target / "keep.txt").read_text(), "cache")
        self.assertEqual(registry.lock_hashes(ROOT), self.locks)
        self.assertEqual(evidence["source_locks_before"], self.locks)
        self.assertEqual(evidence["source_locks_after"], self.locks)
        self.assertFalse(Path(evidence["environment"]["work_directory"]).exists())
        return status, evidence, output

    def test_successful_gate_records_graphs_compiler_and_both_modes(self):
        tools = FakeTools(self.channel)
        status, evidence, output = self.run_gate("success", tools)
        planned = registry.planned_steps()
        self.assertEqual(status, 0)
        self.assertTrue(evidence["completed"])
        self.assertEqual([step["name"] for step in evidence["steps"]], planned)
        self.assertEqual(evidence["counts"]["configured_steps"], len(planned))
        self.assertEqual(evidence["counts"]["passed_steps"], len(planned))
        self.assertEqual(evidence["counts"]["passed_modes"], {"root-fork": 2, "registry": 2})
        self.assertEqual(set(evidence["toolchain"]["releases"].values()), {self.channel})
        self.assertEqual(evidence["source"]["checkout_sha"], "a" * 40)
        self.assertEqual(set(evidence["source"]["callers_sha256"]), set(registry.scan(ROOT)["callers"]))
        for mode, escaping in registry.MODES:
            graph = evidence["registry"]["modes"][mode]["graph"]
            self.assertEqual(escaping, registry.ESCAPING_FEATURE in graph["parser"]["features"])
            self.assertTrue((output / f"registry-metadata-{mode}.json").is_file())
        retained = output / "consumer"
        self.assertNotIn("patch", tomllib.loads((retained / "Cargo.toml").read_text()))
        self.assertIn(registry.PARSER_CHECKSUM, (retained / "Cargo.lock").read_text())
        self.assertTrue((retained / "tests/stable_contract.rs").is_file())
        for call in tools.calls:
            env, argv = call["env"], call["argv"]
            self.assertEqual(env["RUSTUP_TOOLCHAIN"], self.channel)
            self.assertNotIn("CARGO_TARGET_DIR", env)
            if argv[0] == "cargo" and argv[1] in {"test", "metadata"}:
                self.assertIn("--locked", argv)
            if argv[0] == "cargo" and argv[1] == "test":
                escaping = "spargebra/standard-unicode-escaping" in argv
                self.assertEqual(env[registry.MODE_VARIABLE], "1" if escaping else "0")
                self.assertEqual(argv[argv.index("--target-dir") + 1], str(self.target))
            detached = call["name"].startswith(("consumer-", "registry-", "toolchain-detached"))
            self.assertEqual(not call["cwd"].resolve().is_relative_to(ROOT), detached, call["name"])

    def test_failed_mode_keeps_partial_evidence_and_separate_counts(self):
        failing = "registry-standard-unicode-escaping"
        status, evidence, output = self.run_gate("failure", FakeTools(self.channel, fail={failing}))
        self.assertEqual(status, 1)
        self.assertFalse(evidence["completed"])
        counts = evidence["counts"]
        self.assertEqual(counts["failed_steps"], [failing, f"{failing}-tests"])
        self.assertEqual(counts["attempted_steps"], counts["configured_steps"])
        self.assertEqual(counts["passed_steps"], counts["configured_steps"] - 2)
        self.assertEqual(counts["configured_modes"], {"root-fork": 2, "registry": 2})
        self.assertEqual(counts["passed_modes"], {"root-fork": 2, "registry": 1})
        failed = next(step for step in evidence["steps"] if step["name"] == failing)
        self.assertEqual(failed["exit_code"], 101)
        self.assertIn(registry.MODE_TEST, (output / failed["log"]).read_text())

    def test_unexpected_unified_escaping_fails_the_verbatim_graph(self):
        status, evidence, _ = self.run_gate("unified", FakeTools(self.channel, unified_escaping=True))
        self.assertEqual(status, 1)
        self.assertEqual(evidence["counts"]["failed_steps"], ["registry-graph-verbatim"])
        self.assertEqual(evidence["counts"]["passed_modes"]["registry"], 1)

    def test_timed_out_root_run_and_failed_resolution_are_recorded(self):
        tools = FakeTools(self.channel, fail={"consumer-resolve"}, timeout={"root-verbatim"})
        status, evidence, output = self.run_gate("resolve", tools)
        self.assertEqual(status, 1)
        steps = {step["name"]: step for step in evidence["steps"]}
        self.assertTrue(steps["root-verbatim"]["timed_out"])
        self.assertFalse(steps["root-verbatim"]["passed"])
        self.assertEqual(steps["consumer-resolve"]["exit_code"], 101)
        self.assertIn("consumer-lock", evidence["counts"]["not_attempted"])
        self.assertIn("registry-verbatim", evidence["counts"]["not_attempted"])
        self.assertTrue(steps["source-locks-unchanged"]["passed"])
        # The seeded consumer is still retained for diagnosis.
        self.assertTrue((output / "consumer/Cargo.toml").is_file())

    def test_existing_output_or_compiler_override_is_refused(self):
        existing = self.base / "existing"
        existing.mkdir()
        (existing / "keep.txt").write_text("user evidence")
        tools = FakeTools(self.channel)
        with self.assertRaises(FileExistsError):
            registry.gate(ROOT, existing, self.target, runner=tools)
        self.assertEqual(registry.main(["gate", "--output", str(existing)]), 2)
        self.assertEqual([path.name for path in existing.iterdir()], ["keep.txt"])
        self.assertEqual(tools.calls, [])
        os.environ["RUSTC_WRAPPER"] = "sccache"
        status = registry.gate(ROOT, self.base / "wrapped", self.target, runner=tools)
        evidence = json.loads((self.base / "wrapped/evidence.json").read_text())
        self.assertEqual(status, 1)
        self.assertEqual(evidence["steps"][0]["name"], "build-environment")
        self.assertIn("RUSTC_WRAPPER", evidence["steps"][0]["detail"])
        self.assertEqual(tools.calls, [])


def workflow_jobs():
    text = WORKFLOW.read_text()
    body = text[text.index("\njobs:\n") + len("\njobs:\n"):]
    headers = list(re.finditer(r"(?m)^  ([A-Za-z0-9_-]+):[ \t]*$", body))
    return {match[1]: body[match.end():headers[index + 1].start() if index + 1 < len(headers) else len(body)]
            for index, match in enumerate(headers)}


def job_steps(job):
    return re.split(r"(?m)^      - ", job)[1:]


def github_needs_outcome(job_if, needed_result):
    """GitHub Actions: with no status function in `if`, failed needs skip the job."""
    if job_if and re.search(r"\b(?:always|failure|cancelled)\(\)", job_if):
        return "runs"
    return "runs" if needed_result == "success" else "skipped"


class WorkflowWiring(unittest.TestCase):
    def test_proof_job_needs_the_registry_gate_without_a_bypass(self):
        jobs = workflow_jobs()
        exact, gate = jobs["exact-evaluator"], jobs["registry-parser"]
        self.assertRegex(exact, r"(?m)^    needs: \[?registry-parser\]?\s*$")
        self.assertIn(f"name: {summary_gate.EXACT_EVALUATOR_CHECK}\n", exact)
        self.assertRegex(exact, r"(?m)^    timeout-minutes: 120$")
        for job in (exact, gate):
            self.assertIsNone(re.search(r"(?m)^    if:", job))
        self.assertIsNone(re.search(r"(?m)^    needs:", gate))
        text = WORKFLOW.read_text()
        self.assertNotIn("failure()", text)
        self.assertNotIn("cancelled()", text)
        self.assertNotIn("continue-on-error", text)
        for job in jobs.values():
            for step in job_steps(job):
                if "always()" in step:
                    self.assertIn("actions/upload-artifact@", step)

    def test_a_failed_or_skipped_registry_gate_fails_the_mandatory_check(self):
        exact_if = re.search(r"(?m)^    if:(.*)$", workflow_jobs()["exact-evaluator"])
        for event in ("pull_request", "merge_group"):
            context = summary_gate.TierContext(event_name=event)
            for needed in ("failure", "cancelled", "skipped"):
                self.assertEqual(github_needs_outcome(exact_if and exact_if[1], needed), "skipped")
                runs = [{"name": summary_gate.EXACT_EVALUATOR_CHECK, "status": "completed", "conclusion": "skipped"}]
                self.assertEqual(summary_gate.exact_evaluator_status(runs, context), "failed")
            self.assertEqual(github_needs_outcome(None, "success"), "runs")
            runs = [{"name": summary_gate.EXACT_EVALUATOR_CHECK, "status": "completed", "conclusion": "success"}]
            self.assertEqual(summary_gate.exact_evaluator_status(runs, context), "ok")
        # A bypass condition would let the proof job run past a failed gate.
        self.assertEqual(github_needs_outcome("always()", "failure"), "runs")

    def test_registry_job_runs_cheap_checks_always_and_heavy_work_only_when_selected(self):
        steps = job_steps(workflow_jobs()["registry-parser"])
        self.assertRegex(workflow_jobs()["registry-parser"], r"(?m)^    timeout-minutes: (\d+)$")
        timeout = int(re.search(r"(?m)^    timeout-minutes: (\d+)$", workflow_jobs()["registry-parser"])[1])
        self.assertLess(timeout, 120)
        checkout = steps[0]
        self.assertRegex(checkout, r"uses: actions/checkout@[0-9a-f]{40} ")
        self.assertIn("persist-credentials: false", checkout)

        def step(marker):
            matches = [candidate for candidate in steps if marker in candidate]
            self.assertEqual(len(matches), 1, marker)
            return matches[0], steps.index(matches[0])

        unit, unit_index = step("-p 'test_registry_parser.py'")
        scan, scan_index = step("check_registry_parser.py scan")
        classify, _ = step("--scope registry-parser")
        required, _ = step("true|false) ;;")
        heavy, heavy_index = step("check_registry_parser.py gate")
        upload, _ = step("actions/upload-artifact@")
        for cheap in (unit, scan, classify, required):
            self.assertNotIn("if:", cheap)
        self.assertLess(max(unit_index, scan_index), heavy_index)
        self.assertIn("id: changes", classify)
        self.assertIn("if: steps.changes.outputs.required == 'true'\n", heavy)
        self.assertIn("id: gate", heavy)
        self.assertIn("--target-dir", heavy)
        self.assertIn("sparq-registry-parser-target", heavy)
        self.assertNotIn("zk/sparql-evaluator/target", heavy)
        self.assertIn("if: always() && steps.changes.outputs.required == 'true' "
                      "&& steps.gate.outcome != 'skipped'", upload)
        self.assertRegex(upload, r"actions/upload-artifact@[0-9a-f]{40} ")
        self.assertIn("if-no-files-found: error", upload)
        self.assertIn("sparq-registry-parser-evidence", upload)


if __name__ == "__main__":
    unittest.main()
