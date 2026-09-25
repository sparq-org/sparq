#!/usr/bin/env python3
"""[OPUS-5.5] Registry-only compatibility gate for stable VERSION parsing.

``scan`` is cheap and runs on every change. It forbids the vendored fork's
label-retaining methods (``parse_query_with_versions`` and
``parse_update_with_versions``) in every Rust file under ``crates/`` except the
dedicated differential ``crates/sparq-engine/tests/parser_versions_fork.rs``.
The match is textual and deliberately includes comments: no other file needs
those names, so a mention elsewhere is reworded rather than exempted. Longer
identifiers such as ``from_query_with_versions`` do not match. The scan also
inventories ``parse_versioned_*`` production callers and requires each caller
crate to be classified as registry-compiled or scan-only.

``gate`` is heavy and runs only when the fail-closed selector requires it:

1. Root fork graph: from the checkout, ``cargo test --locked`` runs the stable
   contract and the fork differential with ``spargebra``'s
   ``standard-unicode-escaping`` feature off and then on. Each run sets
   ``SPARQ_EXPECT_STANDARD_UNICODE_ESCAPING``, so feature unification that
   yields the other mode fails the stable contract. Root manifests and locks
   are never edited.
2. Registry graph: a fresh consumer outside the checkout, with its own
   ``[workspace]`` and no ``[patch]``, depends on crates.io ``spargebra``
   ``=0.4.6`` and on this checkout's ``sparq-engine`` (``explain-json``,
   ``params``, ``window-functions``), ``sparq-text`` and ``sparq-shacl``. Cargo
   runs from the consumer directory, so root Cargo configuration is not
   inherited, under the compiler pinned by ``rust-toolchain.toml``. The lock is
   seeded from the root lock and resolved once; ``cargo metadata`` must show
   one official registry parser with the expected features, and the unchanged
   stable contract (included through ``#[path]``) runs ``--locked`` in both
   modes.

Evidence is written even when a step fails. Each command's output goes to a
log file under a timeout and a size cap, never into memory. An existing
``--output`` path is refused. Only the temporary directory this script
creates is ever deleted.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import tomllib

ROOT = Path(__file__).resolve().parent.parent

FORK_METHODS = re.compile(r"\bparse_(?:query|update)_with_versions\b")
# A call, possibly split across lines; `fn` excludes the helper definitions.
VERSIONED_CALL = re.compile(r"(?<!fn )\bparse_versioned_(?:query|update)\s*\(")
FEATURE_CFG = re.compile(r'\bfeature\s*=\s*"')
TEST_FUNCTION = re.compile(r"#\[test\]\s*\n\s*fn\s+([A-Za-z_]\w*)")
TEST_OK = re.compile(r"^test (?:[A-Za-z_]\w*::)*([A-Za-z_]\w*) \.\.\. ok$")

HELPER = "crates/sparq-engine/src/versioned_parse.rs"
STABLE_CONTRACT = "crates/sparq-engine/tests/parser_versions.rs"
CORPUS = "crates/sparq-engine/tests/parser_versions/cases.rs"
FORK_DIFFERENTIAL = "crates/sparq-engine/tests/parser_versions_fork.rs"
MODE_VARIABLE = "SPARQ_EXPECT_STANDARD_UNICODE_ESCAPING"
MODE_TEST = "requested_unicode_escaping_mode_is_the_linked_mode"

# Migrated callers that the registry consumer compiles against crates.io
# `spargebra`. The first entry is the engine, built with ENGINE_FEATURES.
REGISTRY_COMPILED = ("sparq-engine", "sparq-text", "sparq-shacl")
# Migrated callers covered only by the static scan. Compiling them would pull
# large optional stacks (HTTP server, Python bindings, Solid, LWS, vectors).
SCAN_ONLY = ("sparq-vectors", "sparq-server", "sparq-solid", "sparq-py", "sparq-lws-core")
ENGINE_FEATURES = ("explain-json", "params", "window-functions")

PARSER = "spargebra"
PARSER_VERSION = "0.4.6"
REGISTRY_SOURCE = "registry+https://github.com/rust-lang/crates.io-index"
# crates.io checksum of spargebra 0.4.6 as Cargo records it in a lock file.
PARSER_CHECKSUM = "46715eb957d1fe960cbbc0b713da8f78e2cb19df315b48fb09c9467a1c84f656"
# Features requested by the workspace dependency that every sparq crate inherits.
PARSER_FEATURES = frozenset({"sep-0006", "sparql-12"})
ESCAPING_FEATURE = "standard-unicode-escaping"
CONSUMER = "sparq-registry-parser-consumer"
MODES = (("verbatim", False), ("standard-unicode-escaping", True))

SOURCE_LOCKS = ("Cargo.lock", "zk/sparql-evaluator/Cargo.lock",
                "zk/sparql-evaluator/methods/guest/Cargo.lock")
SOURCE_FILES = (HELPER, STABLE_CONTRACT, CORPUS, FORK_DIFFERENTIAL, "Cargo.toml",
                "crates/sparq-engine/Cargo.toml", "crates/sparq-text/Cargo.toml",
                "crates/sparq-shacl/Cargo.toml", "vendor/spargebra/Cargo.toml", "rust-toolchain.toml",
                "scripts/check_registry_parser.py", "scripts/tests/test_registry_parser.py",
                "scripts/ci_exact_evaluator_paths.py", ".github/workflows/zk-exact-evaluator.yml")
# Compiler overrides would make the pinned-toolchain evidence meaningless.
FORBIDDEN_ENVIRONMENT = ("RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC",
                         "CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
                         "CARGO_BUILD_TARGET")
RECORDED_ENVIRONMENT = ("CARGO_BUILD_JOBS", "CARGO_INCREMENTAL", "CARGO_NET_OFFLINE", "RUSTFLAGS",
                        "CARGO_ENCODED_RUSTFLAGS", "RUSTUP_TOOLCHAIN", "CARGO_TARGET_DIR")

# Per-command bounds. A cold debug build of the engine graph needs minutes;
# the job-level timeout in the workflow bounds the whole gate.
GIT_TIMEOUT = 120
PROBE_TIMEOUT = 900  # rustup may install the pinned toolchain on first use.
RESOLVE_TIMEOUT = 900
METADATA_TIMEOUT = 900
TEST_TIMEOUT = 1800
MAX_OUTPUT_BYTES = 128 * 1024 * 1024
MAX_PROBE_BYTES = 1024 * 1024
POLL_SECONDS = 2.0
HEARTBEAT_SECONDS = 60.0

LIMITATIONS = (
    f"Scan-only crates ({', '.join(SCAN_ONLY)}) are checked textually; they are not "
    "compiled against the registry parser.",
    "The textual scan cannot see method names synthesized by macros in scan-only crates.",
    "Cargo configuration under CARGO_HOME still applies. A [patch] or source replacement "
    "there is detected by the resolved-graph source and lock checksum checks, not prevented.",
    "The root fork graph is identified statically (root manifest and lock) and by the fork "
    "differential, whose fork-only methods cannot compile against any other parser.",
)


class GateError(ValueError):
    """A gate requirement is not met."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def read_small(path: Path, limit: int = MAX_PROBE_BYTES) -> str:
    with path.open("rb") as handle:
        return handle.read(limit).decode("utf-8", "replace")


# --------------------------------------------------------------------------
# Static scan
# --------------------------------------------------------------------------


def rust_files(root: Path) -> list[Path]:
    """Every `.rs` file under `crates/`, skipping Cargo target directories."""
    files = []
    for directory, subdirectories, names in os.walk(root / "crates"):
        # Cargo marks every target directory with CACHEDIR.TAG; sources never are.
        if "CACHEDIR.TAG" in names:
            subdirectories.clear()
            continue
        subdirectories.sort()
        files.extend(Path(directory) / name for name in sorted(names) if name.endswith(".rs"))
    return files


def caller_count(text: str) -> int:
    """`parse_versioned_*` calls outside full-line comments (doc examples included)."""
    code = "\n".join("" if line.lstrip().startswith("//") else line for line in text.splitlines())
    return len(VERSIONED_CALL.findall(code))


def scan(root: Path) -> dict:
    """Forbidden fork-method mentions, contract hygiene and the caller inventory."""
    violations, errors, callers = [], [], {}
    fork_lines = 0
    files = rust_files(root)
    if not files:
        errors.append("no Rust sources found under crates/")
    for path in files:
        relative = path.relative_to(root).as_posix()
        text = path.read_text(encoding="utf-8", errors="replace")
        for number, line in enumerate(text.splitlines(), 1):
            if not FORK_METHODS.search(line):
                continue
            if relative == FORK_DIFFERENTIAL:
                fork_lines += 1
            else:
                violations.append({"path": relative, "line": number, "text": line.strip()[:200]})
        parts = relative.split("/")
        if len(parts) > 3 and parts[2] == "src":
            count = caller_count(text)
            if count:
                callers[relative] = count
    fork = root / FORK_DIFFERENTIAL
    if fork.is_file():
        fork_text = fork.read_text(encoding="utf-8", errors="replace")
        for method in ("parse_query_with_versions", "parse_update_with_versions"):
            if not re.search(rf"\.{method}\s*\(", fork_text):
                errors.append(f"{FORK_DIFFERENTIAL} no longer calls the fork's {method}")
    else:
        errors.append(f"missing the dedicated fork differential {FORK_DIFFERENTIAL}")
    for relative in (STABLE_CONTRACT, CORPUS):
        path = root / relative
        if not path.is_file():
            errors.append(f"missing {relative}")
        elif FEATURE_CFG.search(path.read_text(encoding="utf-8", errors="replace")):
            errors.append(f"{relative} must stay free of crate-feature cfgs; the registry "
                          "consumer compiles it inside another crate")
    stable = root / STABLE_CONTRACT
    if stable.is_file() and MODE_VARIABLE not in stable.read_text(encoding="utf-8", errors="replace"):
        errors.append(f"{STABLE_CONTRACT} no longer checks {MODE_VARIABLE}")
    by_crate: dict[str, int] = {}
    for relative, count in callers.items():
        crate = relative.split("/")[1]
        by_crate[crate] = by_crate.get(crate, 0) + count
    for crate in sorted(set(by_crate) - set(REGISTRY_COMPILED) - set(SCAN_ONLY)):
        errors.append(f"{crate} calls parse_versioned_*; classify it in REGISTRY_COMPILED or SCAN_ONLY")
    return {
        "passed": not violations and not errors,
        "violations": violations,
        "errors": errors,
        "files_scanned": len(files),
        "fork_differential": FORK_DIFFERENTIAL,
        "fork_differential_lines": fork_lines,
        "callers": callers,
        "registry_compiled_callers": {crate: by_crate.get(crate, 0) for crate in REGISTRY_COMPILED},
        "scan_only_callers": {crate: by_crate.get(crate, 0) for crate in SCAN_ONLY},
    }


def require_scan(root: Path, evidence: dict) -> dict:
    result = evidence["scan"] = scan(root)
    if not result["passed"]:
        raise GateError(f"{len(result['violations'])} forbidden fork-method mentions; errors: {result['errors']}")
    return {"files_scanned": result["files_scanned"], "production_calls": sum(result["callers"].values())}


def print_scan(result: dict) -> None:
    for violation in result["violations"]:
        print(f"{violation['path']}:{violation['line']}: fork-only parser method outside "
              f"{FORK_DIFFERENTIAL}: {violation['text']}", file=sys.stderr)
    for error in result["errors"]:
        print(f"error: {error}", file=sys.stderr)
    print(f"Scanned {result['files_scanned']} Rust files; "
          f"{sum(result['callers'].values())} parse_versioned_* production calls.")
    print(f"Registry-compiled callers: {result['registry_compiled_callers']}")
    print(f"Scan-only callers: {result['scan_only_callers']}")
    print("Registry parser scan passed." if result["passed"] else "Registry parser scan FAILED.")


# --------------------------------------------------------------------------
# Bounded commands and step records
# --------------------------------------------------------------------------


@dataclass
class RunResult:
    exit_code: int | None
    timed_out: bool
    oversized: bool
    elapsed_seconds: float


def terminate(process: subprocess.Popen) -> None:
    """Kill the command's whole process group (rustc children included)."""
    try:
        if hasattr(os, "killpg"):
            os.killpg(process.pid, signal.SIGKILL)
        else:
            process.kill()
    except (ProcessLookupError, PermissionError):
        pass
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired:
        pass


def written_bytes(log_path: Path, stdout_path: Path | None) -> int:
    """Bytes in the log plus, when stdout is split off, the stdout file."""
    return log_path.stat().st_size + (stdout_path.stat().st_size if stdout_path else 0)


def run_bounded(argv: list[str], *, cwd: Path, env: dict, log_path: Path, timeout: float,
                stdout_path: Path | None = None, max_bytes: int = MAX_OUTPUT_BYTES,
                poll: float = POLL_SECONDS) -> RunResult:
    """Run `argv` with output in files, a wall-clock limit and an output cap.

    Both limits are checked at every poll and again once the command has exited,
    so a command that crosses either between polls and then exits still fails.
    """
    started = time.monotonic()
    timed_out = oversized = False
    exit_code = None
    with log_path.open("wb") as log:
        out = stdout_path.open("wb") if stdout_path else log
        try:
            process = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=out,
                                       stderr=log if stdout_path else subprocess.STDOUT,
                                       start_new_session=True)
            try:
                heartbeat = HEARTBEAT_SECONDS
                while True:
                    try:
                        exit_code = process.wait(timeout=poll)
                    except subprocess.TimeoutExpired:
                        elapsed = time.monotonic() - started
                        timed_out = elapsed > timeout
                        oversized = written_bytes(log_path, stdout_path) > max_bytes
                        if timed_out or oversized:
                            break
                        if elapsed >= heartbeat:
                            print(f"[registry-parser] {log_path.stem}: running for {int(elapsed)}s", flush=True)
                            heartbeat += HEARTBEAT_SECONDS
                    else:
                        # [OPUS-5.5] An exit after the deadline but before the next poll still overran.
                        timed_out = time.monotonic() - started > timeout
                        break
            finally:
                if process.poll() is None:
                    terminate(process)
        finally:
            if stdout_path:
                out.close()
    # [OPUS-5.5] A fast command can pass the cap and exit before any poll sees it;
    # measure the final files too, keeping any failure already recorded.
    oversized = oversized or written_bytes(log_path, stdout_path) > max_bytes
    return RunResult(exit_code, timed_out, oversized, time.monotonic() - started)


def show_tail(path: Path, failed: bool) -> None:
    """Echo a bounded tail of a log to the job output; the full log is uploaded."""
    if not path.is_file():
        return
    limit = 64 * 1024 if failed else 8 * 1024
    with path.open("rb") as handle:
        handle.seek(0, os.SEEK_END)
        handle.seek(max(0, handle.tell() - limit))
        sys.stdout.write(handle.read().decode("utf-8", "replace"))
    sys.stdout.write("\n")
    sys.stdout.flush()


class Recorder:
    """Runs and records every step, so failures still leave complete evidence."""

    def __init__(self, output: Path, runner=run_bounded):
        self.output = output
        self.runner = runner
        self.steps: list[dict] = []

    def log(self, name: str) -> Path:
        return self.output / "logs" / f"{name}.log"

    def command(self, name: str, argv: list[str], *, cwd: Path, env: dict, timeout: float,
                stdout_name: str | None = None) -> bool:
        log = self.log(name)
        record = {"name": name, "kind": "command", "argv": argv, "cwd": str(cwd),
                  "timeout_seconds": timeout, "log": log.relative_to(self.output).as_posix()}
        if stdout_name:
            record["stdout"] = stdout_name
        print(f"[registry-parser] {name}: {shlex.join(argv)}", flush=True)
        try:
            result = self.runner(argv, cwd=cwd, env=env, log_path=log, timeout=timeout,
                                 stdout_path=self.output / stdout_name if stdout_name else None)
        except OSError as error:
            record.update(exit_code=None, passed=False, detail=f"could not run: {error}")
        else:
            record.update(exit_code=result.exit_code, timed_out=result.timed_out,
                          output_limit_exceeded=result.oversized,
                          elapsed_seconds=round(result.elapsed_seconds, 3),
                          passed=result.exit_code == 0 and not result.timed_out and not result.oversized)
        self.steps.append(record)
        show_tail(log, failed=not record["passed"])
        if not record["passed"]:
            print(f"[registry-parser] {name}: FAILED (exit {record['exit_code']}, "
                  f"timed out {record.get('timed_out')}, output limit {record.get('output_limit_exceeded')}, "
                  f"{record.get('detail', 'see log')})", flush=True)
        return record["passed"]

    def check(self, name: str, action) -> tuple[bool, object]:
        try:
            detail = action()
        except (GateError, OSError, ValueError, KeyError, TypeError, IndexError) as error:
            self.steps.append({"name": name, "kind": "check", "passed": False,
                               "detail": f"{type(error).__name__}: {error}"})
            print(f"[registry-parser] {name}: FAILED: {error}", flush=True)
            return False, None
        self.steps.append({"name": name, "kind": "check", "passed": True, "detail": detail})
        print(f"[registry-parser] {name}: passed", flush=True)
        return True, detail


def planned_steps() -> list[str]:
    """Every configured step, in order; the evidence counts attempts against it."""
    steps = ["build-environment", "git-revision", "git-status", "static-scan", "source-identity"]
    steps += [f"toolchain-{place}-{tool}" for place in ("root", "detached") for tool in ("rustc", "cargo")]
    steps += ["toolchain-pin", "fork-graph-identity"]
    for mode, _ in MODES:
        steps += [f"root-{mode}", f"root-{mode}-tests"]
    steps += ["consumer-seed", "consumer-resolve", "consumer-lock"]
    for mode, _ in MODES:
        steps += [f"registry-metadata-{mode}", f"registry-graph-{mode}", f"registry-{mode}",
                  f"registry-{mode}-tests"]
    steps += ["consumer-lock-stable", "source-locks-unchanged"]
    return steps


# --------------------------------------------------------------------------
# Environment, identity and toolchain
# --------------------------------------------------------------------------


def pinned_channel(root: Path) -> str:
    channel = tomllib.loads((root / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    if not re.fullmatch(r"\d+\.\d+\.\d+", channel):
        raise GateError(f"rust-toolchain.toml must pin an exact release, not {channel!r}")
    return channel


def ensure_detached(directory: Path, root: Path) -> None:
    """Refuse a consumer location that could inherit this checkout's Cargo setup."""
    directory, root = directory.resolve(), root.resolve()
    if directory.is_relative_to(root) or root.is_relative_to(directory):
        raise GateError(f"the registry consumer must live outside the checkout: {directory}")
    for ancestor in (directory, *directory.parents):
        for name in ("config", "config.toml"):
            if (ancestor / ".cargo" / name).exists():
                raise GateError(f"ancestor Cargo configuration would be inherited: {ancestor / '.cargo' / name}")


def environment_summary(root: Path, work: Path, target: Path, environ: dict) -> dict:
    present = [key for key in FORBIDDEN_ENVIRONMENT if key in environ]
    if present:
        raise GateError(f"compiler overrides would bypass the pinned toolchain: {present}")
    resolved = target.resolve()
    if resolved == root.resolve() or root.resolve().is_relative_to(resolved):
        raise GateError("the target directory must not contain the checkout")
    ensure_detached(work, root)
    return {"toolchain_channel": pinned_channel(root), "work_directory": str(work),
            "target_directory": str(target),
            "inherited": {key: environ.get(key) for key in RECORDED_ENVIRONMENT}}


def cargo_environment(environ: dict, channel: str) -> dict:
    env = {key: value for key, value in environ.items() if key not in {"CARGO_TARGET_DIR", MODE_VARIABLE}}
    # Outside the checkout rust-toolchain.toml is not discovered; pin explicitly.
    env.update(RUSTUP_TOOLCHAIN=channel, CARGO_TERM_COLOR="never", CARGO_TERM_PROGRESS_WHEN="never")
    return env


def mode_environment(env: dict, escaping: bool) -> dict:
    return {**env, MODE_VARIABLE: "1" if escaping else "0"}


def feature_arguments(escaping: bool) -> list[str]:
    return ["--features", f"{PARSER}/{ESCAPING_FEATURE}"] if escaping else []


def lock_hashes(root: Path) -> dict:
    return {relative: sha256_file(root / relative) if (root / relative).is_file() else None
            for relative in SOURCE_LOCKS}


def compare_locks(before: dict | None, after: dict) -> dict:
    if not before or any(value is None for value in before.values()):
        raise GateError(f"source locks were missing before the gate: {before}")
    if after != before:
        raise GateError(f"source locks changed during the gate: {before} -> {after}")
    return {"unchanged": sorted(before)}


def source_identity(root: Path, commands_passed: bool, revision_log: Path, status_log: Path,
                    callers: dict) -> dict:
    if not commands_passed:
        raise GateError("git could not report the checkout identity and status")
    identity = read_small(revision_log).split()
    if len(identity) != 2 or not all(re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", sha) for sha in identity):
        raise GateError("could not read the checkout and tree identity")
    if status_log.stat().st_size:
        raise GateError("tracked checkout changes invalidate source provenance")
    return {"checkout_sha": identity[0], "tree_sha": identity[1],
            "sha256": {relative: sha256_file(root / relative) for relative in SOURCE_FILES},
            "callers_sha256": {relative: sha256_file(root / relative) for relative in callers}}


def release(log: Path) -> str:
    match = re.search(r"(?m)^release:\s*(\S+)\s*$", read_small(log))
    if not match:
        raise GateError(f"no release line in {log.name}")
    return match.group(1)


def toolchain_pin(recorder: Recorder, channel: str) -> dict:
    probes = {f"{place}_{tool}": recorder.log(f"toolchain-{place}-{tool}")
              for place in ("root", "detached") for tool in ("rustc", "cargo")}
    releases = {name: release(log) for name, log in probes.items()}
    wrong = {name: value for name, value in releases.items() if value != channel}
    if wrong:
        raise GateError(f"actual compiler differs from the pinned {channel}: {wrong}")
    return {"channel": channel, "releases": releases,
            "version_output": {name: read_small(log).strip() for name, log in probes.items()}}


def fork_graph_identity(root: Path) -> dict:
    """The root graph selects the vendored fork through its patch table."""
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    patch = manifest.get("patch", {}).get("crates-io", {}).get(PARSER, {})
    if "path" not in patch or (root / patch["path"]).resolve() != (root / "vendor/spargebra").resolve():
        raise GateError("root manifest no longer patches spargebra with vendor/spargebra")
    if manifest["workspace"]["dependencies"][PARSER].get("version") != f"={PARSER_VERSION}":
        raise GateError(f"root workspace must pin {PARSER} ={PARSER_VERSION}")
    vendored = tomllib.loads((root / "vendor/spargebra/Cargo.toml").read_text())
    if vendored["package"]["version"] != PARSER_VERSION or ESCAPING_FEATURE not in vendored["features"]:
        raise GateError("vendored parser version or escaping feature changed")
    entries = [package for package in lock_packages((root / "Cargo.lock").read_text())
               if package.get("name") == PARSER]
    if len(entries) != 1 or entries[0].get("source") is not None or entries[0].get("version") != PARSER_VERSION:
        raise GateError("root lock must select exactly one vendored path parser")
    return {"parser": "vendor/spargebra", "version": PARSER_VERSION, "lock_source": "path"}


# --------------------------------------------------------------------------
# Test logs
# --------------------------------------------------------------------------


def declared_tests(path: Path) -> list[str]:
    return TEST_FUNCTION.findall(path.read_text(encoding="utf-8", errors="replace"))


def passed_tests(log: Path) -> set[str]:
    passed = set()
    with log.open(encoding="utf-8", errors="replace") as handle:
        for line in handle:
            match = TEST_OK.match(line.rstrip("\r\n"))
            if match:
                passed.add(match.group(1))
    return passed


def check_tests(log: Path, sources: list[Path]) -> dict:
    """Every configured test, including the mode expectation, reported ok."""
    configured = sorted({name for source in sources for name in declared_tests(source)})
    if MODE_TEST not in configured:
        raise GateError(f"the escaping-mode expectation test {MODE_TEST} is not configured")
    passed = passed_tests(log)
    missing = [name for name in configured if name not in passed]
    result = {"configured_tests": len(configured), "passed_tests": len(configured) - len(missing),
              "missing": missing}
    if missing:
        raise GateError(f"{len(missing)} of {len(configured)} configured tests did not report ok: {missing}")
    return result


def root_test_command(root: Path, target: Path, escaping: bool) -> list[str]:
    return ["cargo", "test", "--locked", "--no-fail-fast", "--manifest-path", str(root / "Cargo.toml"),
            "--target-dir", str(target), "-p", "sparq-engine", "--test", "parser_versions",
            "--test", "parser_versions_fork", *feature_arguments(escaping)]


def consumer_test_command(target: Path, escaping: bool) -> list[str]:
    return ["cargo", "test", "--locked", "--no-fail-fast", "--target-dir", str(target),
            "--test", "stable_contract", *feature_arguments(escaping)]


# --------------------------------------------------------------------------
# Registry consumer
# --------------------------------------------------------------------------


def quoted(path: Path) -> str:
    """A TOML basic string or Rust string literal for an absolute path."""
    text = path.as_posix()
    if any(ord(character) < 0x20 or ord(character) == 0x7F for character in text):
        raise GateError(f"unsupported control character in checkout path {text!r}")
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"'


def consumer_manifest(root: Path) -> str:
    features = ", ".join(f'"{feature}"' for feature in ENGINE_FEATURES)
    engine, *others = REGISTRY_COMPILED
    lines = [
        "# Generated by scripts/check_registry_parser.py. A detached workspace with no",
        "# [patch]: the sparq crates come from the checkout, spargebra from crates.io.",
        "[workspace]",
        'resolver = "2"',
        "",
        "[package]",
        f'name = "{CONSUMER}"',
        'version = "0.0.0"',
        'edition = "2021"',
        "publish = false",
        "",
        "[dependencies]",
        f'{PARSER} = "={PARSER_VERSION}"',
        f"{engine} = {{ path = {quoted(root / 'crates' / engine)}, features = [{features}] }}",
        *(f"{crate} = {{ path = {quoted(root / 'crates' / crate)} }}" for crate in others),
    ]
    return "\n".join(lines) + "\n"


def consumer_library() -> str:
    uses = "".join(f"pub use {crate.replace('-', '_')};\n" for crate in REGISTRY_COMPILED)
    return ("//! Registry-only parser consumer generated by `scripts/check_registry_parser.py`.\n"
            "//!\n"
            "//! Linking these crates compiles their `parse_versioned_*` callers against\n"
            "//! the official crates.io `spargebra`, not the vendored fork.\n" + uses)


def contract_shim(root: Path) -> str:
    return ("//! The engine's stable-parser contract, compiled unchanged against registry `spargebra`.\n"
            f"#[path = {quoted(root / STABLE_CONTRACT)}]\n"
            "mod contract;\n")


def seed_consumer(root: Path, consumer: Path) -> dict:
    ensure_detached(consumer.parent, root)
    consumer.mkdir()
    (consumer / "src").mkdir()
    (consumer / "tests").mkdir()
    manifest = consumer_manifest(root)
    parsed = tomllib.loads(manifest)
    if "patch" in parsed or "workspace" not in parsed:
        raise GateError("consumer manifest must be its own workspace without [patch]")
    (consumer / "Cargo.toml").write_text(manifest, encoding="utf-8")
    (consumer / "src/lib.rs").write_text(consumer_library(), encoding="utf-8")
    (consumer / "tests/stable_contract.rs").write_text(contract_shim(root), encoding="utf-8")
    shutil.copyfile(root / "Cargo.lock", consumer / "Cargo.lock")
    return {"directory": str(consumer), "manifest_sha256": sha256_file(consumer / "Cargo.toml"),
            "seed_lock_sha256": sha256_file(consumer / "Cargo.lock"), "seeded_from": "Cargo.lock"}


def lock_packages(text: str) -> list[dict]:
    packages = tomllib.loads(text).get("package", [])
    if not isinstance(packages, list):
        raise GateError("malformed Cargo.lock package table")
    return packages


def package_key(package: dict) -> tuple:
    return (package.get("name"), package.get("version"), package.get("source"), package.get("checksum"))


def check_consumer_lock(seed_text: str, resolved_text: str) -> dict:
    """The resolution adds the official registry parser and moves nothing else."""
    seed, resolved = lock_packages(seed_text), lock_packages(resolved_text)
    parsers = [package for package in resolved if package.get("name") == PARSER]
    if len(parsers) != 1:
        raise GateError(f"consumer lock must contain exactly one {PARSER}, found {len(parsers)}")
    parser = parsers[0]
    if package_key(parser)[1:] != (PARSER_VERSION, REGISTRY_SOURCE, PARSER_CHECKSUM):
        raise GateError(f"consumer lock parser is not the official registry release: {package_key(parser)}")
    seeded = {package_key(package) for package in seed}
    drift = sorted(f"{package.get('name')} {package.get('version')} {package.get('source') or 'path'}"
                   for package in resolved
                   if package.get("name") not in {PARSER, CONSUMER} and package_key(package) not in seeded)
    if drift:
        raise GateError(f"resolution changed packages other than the parser: {drift}")
    return {"seed_packages": len(seed), "resolved_packages": len(resolved),
            "parser": {"version": parser["version"], "source": parser["source"], "checksum": parser["checksum"]},
            "seed_packages_not_in_consumer_graph": len(seeded - {package_key(p) for p in resolved})}


def load_selector():
    path = Path(__file__).with_name("ci_exact_evaluator_paths.py")
    spec = importlib.util.spec_from_file_location("ci_exact_evaluator_paths", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_metadata(path: Path) -> dict:
    if path.stat().st_size > MAX_OUTPUT_BYTES:
        raise GateError(f"{path.name} exceeds the metadata size limit")
    return json.loads(path.read_bytes())


def check_registry_graph(metadata: dict, root: Path, escaping: bool) -> dict:
    """Cargo's actual resolved graph links one official parser everywhere."""
    root = root.resolve()
    packages = metadata["packages"]
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    members = [package["name"] for package in packages if package["id"] in metadata["workspace_members"]]
    if members != [CONSUMER]:
        raise GateError(f"the consumer must be the only workspace member, found {members}")
    parsers = [package for package in packages if package["name"] == PARSER]
    if len(parsers) != 1:
        raise GateError(f"resolved graph must contain exactly one {PARSER}, found {len(parsers)}")
    parser = parsers[0]
    if (parser["source"], parser["version"]) != (REGISTRY_SOURCE, PARSER_VERSION):
        raise GateError(f"resolved parser is not the registry release: {parser['source']} {parser['version']}")
    if Path(parser["manifest_path"]).resolve().is_relative_to(root):
        raise GateError("resolved registry parser manifest lies inside the checkout")
    features = set(nodes[parser["id"]]["features"]) - {"default"}
    expected = set(PARSER_FEATURES) | ({ESCAPING_FEATURE} if escaping else set())
    if features != expected:
        raise GateError(f"{PARSER} features {sorted(features)} differ from the expected {sorted(expected)}")
    local = {}
    for package in packages:
        if package["source"] is None and package["name"] != CONSUMER:
            directory = Path(package["manifest_path"]).resolve().parent
            if not directory.is_relative_to(root / "crates"):
                raise GateError(f"unexpected path package outside crates/: {package['name']}")
            local[package["name"]] = {"id": package["id"], "path": directory.relative_to(root).as_posix()}
    for crate in REGISTRY_COMPILED:
        if local.get(crate, {}).get("path") != f"crates/{crate}":
            raise GateError(f"{crate} is not compiled from this checkout")
        linked = [dep["pkg"] for dep in nodes[local[crate]["id"]]["deps"] if dep["name"] == PARSER]
        if linked != [parser["id"]]:
            raise GateError(f"{crate} does not link the registry parser")
    engine_features = nodes[local[REGISTRY_COMPILED[0]]["id"]]["features"]
    missing = sorted(set(ENGINE_FEATURES) - set(engine_features))
    if missing:
        raise GateError(f"sparq-engine lacks the required features {missing}")
    selector = load_selector()
    uncovered = sorted(entry["path"] for entry in local.values()
                       if not selector.relevant_path(f"{entry['path']}/Cargo.toml", "registry-parser"))
    if uncovered:
        raise GateError(f"the CI selector would not rerun this gate for compiled path packages {uncovered}")
    return {"parser": {"id": parser["id"], "source": parser["source"], "version": parser["version"],
                       "features": sorted(nodes[parser["id"]]["features"])},
            "engine_features": sorted(engine_features),
            "path_packages": {name: entry["path"] for name, entry in sorted(local.items())},
            "path_package_manifests_sha256": {
                entry["path"] + "/Cargo.toml": sha256_file(root / entry["path"] / "Cargo.toml")
                for entry in local.values()},
            "package_count": len(packages)}


def retain_consumer(consumer: Path, destination: Path) -> dict:
    """Copy the generated consumer (manifest, lock, sources) into the evidence."""
    retained = {}
    for relative in ("Cargo.toml", "Cargo.lock", "src/lib.rs", "tests/stable_contract.rs"):
        source = consumer / relative
        if source.is_file():
            copy = destination / relative
            copy.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, copy)
            retained[relative] = sha256_file(copy)
    return retained


# --------------------------------------------------------------------------
# Gate
# --------------------------------------------------------------------------


def run_phases(root: Path, work: Path, target: Path, recorder: Recorder, evidence: dict) -> None:
    ok, summary = recorder.check("build-environment",
                                 lambda: environment_summary(root, work, target, dict(os.environ)))
    if not ok:
        return
    evidence["environment"] = summary
    env = cargo_environment(dict(os.environ), summary["toolchain_channel"])

    revision = recorder.command("git-revision", ["git", "rev-parse", "HEAD", "HEAD^{tree}"], cwd=root,
                                env=env, timeout=GIT_TIMEOUT)
    status = recorder.command("git-status", ["git", "status", "--porcelain", "--untracked-files=no"],
                              cwd=root, env=env, timeout=GIT_TIMEOUT)
    scanned, _ = recorder.check("static-scan", lambda: require_scan(root, evidence))
    ok, evidence["source"] = recorder.check("source-identity", lambda: source_identity(
        root, revision and status, recorder.log("git-revision"), recorder.log("git-status"),
        evidence.get("scan", {}).get("callers", {})))
    if not (ok and scanned):
        return

    for place, cwd in (("root", root), ("detached", work)):
        for tool in ("rustc", "cargo"):
            recorder.command(f"toolchain-{place}-{tool}", [tool, "-vV"], cwd=cwd, env=env, timeout=PROBE_TIMEOUT)
    ok, evidence["toolchain"] = recorder.check("toolchain-pin",
                                               lambda: toolchain_pin(recorder, summary["toolchain_channel"]))
    if not ok:
        return

    root_fork = evidence["root_fork"] = {"modes": {}}
    ok, root_fork["identity"] = recorder.check("fork-graph-identity", lambda: fork_graph_identity(root))
    if ok:
        for mode, escaping in MODES:
            name = f"root-{mode}"
            ran = recorder.command(name, root_test_command(root, target, escaping), cwd=root,
                                   env=mode_environment(env, escaping), timeout=TEST_TIMEOUT)
            passed, tests = recorder.check(f"{name}-tests", lambda name=name: check_tests(
                recorder.log(name), [root / STABLE_CONTRACT, root / FORK_DIFFERENTIAL]))
            root_fork["modes"][mode] = {"passed": ran and passed, "tests": tests}

    registry = evidence["registry"] = {"modes": {}}
    consumer = work / "consumer"
    ok, registry["consumer"] = recorder.check("consumer-seed", lambda: seed_consumer(root, consumer))
    if not ok or not recorder.command("consumer-resolve", ["cargo", "update", "--workspace"], cwd=consumer,
                                      env=env, timeout=RESOLVE_TIMEOUT):
        return
    ok, registry["lock"] = recorder.check("consumer-lock", lambda: check_consumer_lock(
        (root / "Cargo.lock").read_text(), (consumer / "Cargo.lock").read_text()))
    if not ok:
        return
    resolved = sha256_file(consumer / "Cargo.lock")
    registry["resolved_lock_sha256"] = resolved
    for mode, escaping in MODES:
        metadata = f"registry-metadata-{mode}"
        recorder.command(metadata, ["cargo", "metadata", "--locked", "--format-version", "1",
                                    *feature_arguments(escaping)],
                         cwd=consumer, env=env, timeout=METADATA_TIMEOUT, stdout_name=f"{metadata}.json")
        graph_ok, graph = recorder.check(f"registry-graph-{mode}", lambda metadata=metadata, escaping=escaping:
                                         check_registry_graph(load_metadata(recorder.output / f"{metadata}.json"),
                                                              root, escaping))
        name = f"registry-{mode}"
        ran = recorder.command(name, consumer_test_command(target, escaping), cwd=consumer,
                               env=mode_environment(env, escaping), timeout=TEST_TIMEOUT)
        passed, tests = recorder.check(f"{name}-tests", lambda name=name: check_tests(
            recorder.log(name), [root / STABLE_CONTRACT]))
        registry["modes"][mode] = {"passed": graph_ok and ran and passed, "graph": graph, "tests": tests}

    def lock_stable():
        if sha256_file(consumer / "Cargo.lock") != resolved:
            raise GateError("a --locked run changed the consumer lock")
        return {"sha256": resolved}
    recorder.check("consumer-lock-stable", lock_stable)


def finalize(evidence: dict, recorder: Recorder) -> None:
    planned = planned_steps()
    attempted = [step["name"] for step in recorder.steps]
    passed = [step["name"] for step in recorder.steps if step["passed"]]
    evidence["steps"] = recorder.steps
    evidence["counts"] = {
        "configured_steps": len(planned),
        "attempted_steps": len(attempted),
        "passed_steps": len(passed),
        "failed_steps": [name for name in attempted if name not in passed],
        "not_attempted": [name for name in planned if name not in attempted],
        "configured_modes": {"root-fork": len(MODES), "registry": len(MODES)},
        "passed_modes": {
            "root-fork": sum(bool(mode["passed"]) for mode in evidence.get("root_fork", {}).get("modes", {}).values()),
            "registry": sum(bool(mode["passed"]) for mode in evidence.get("registry", {}).get("modes", {}).values()),
        },
    }
    evidence["completed"] = (evidence["error"] is None and attempted == planned and len(passed) == len(planned))


def gate(root: Path, output: Path, target_dir: Path | None = None, runner=run_bounded) -> int:
    """Run every configured step and write evidence, whatever the outcome."""
    root = root.resolve()
    output = output.absolute()
    output.mkdir()  # An existing path is refused: never reuse or delete evidence.
    (output / "logs").mkdir()
    recorder = Recorder(output, runner)
    evidence = {"schema": "sparq-registry-parser-gate-v1", "completed": False, "error": None,
                "checkout": str(root), "limitations": list(LIMITATIONS),
                "consumer_scope": {"registry_compiled": list(REGISTRY_COMPILED), "scan_only": list(SCAN_ONLY),
                                   "engine_features": list(ENGINE_FEATURES)}}
    try:
        evidence["source_locks_before"] = lock_hashes(root)
        # The only directory this script deletes is the one it creates here.
        with tempfile.TemporaryDirectory(prefix="sparq-registry-parser-", ignore_cleanup_errors=True) as temporary:
            work = Path(temporary).resolve()
            target = target_dir.absolute() if target_dir else work / "target"
            try:
                run_phases(root, work, target, recorder, evidence)
            finally:
                try:
                    evidence["retained_consumer"] = retain_consumer(work / "consumer", output / "consumer")
                except OSError as error:
                    evidence["retained_consumer"] = {"error": str(error)}
    except (GateError, OSError, ValueError, KeyError, TypeError) as error:
        evidence["error"] = f"{type(error).__name__}: {error}"
    finally:
        try:
            after = lock_hashes(root)
        except OSError as error:
            after = {"error": str(error)}
        evidence["source_locks_after"] = after
        recorder.check("source-locks-unchanged", lambda: compare_locks(evidence.get("source_locks_before"), after))
        finalize(evidence, recorder)
        (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    counts = evidence["counts"]
    print(f"[registry-parser] {counts['passed_steps']}/{counts['configured_steps']} configured steps passed; "
          f"failed {counts['failed_steps']}; not attempted {counts['not_attempted']}; evidence {output}")
    return 0 if evidence["completed"] else 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("scan", help="static fork-method scan and caller inventory (cheap)")
    heavy = commands.add_parser("gate", help="root fork graph and registry consumer (heavy)")
    heavy.add_argument("--output", type=Path, required=True, help="new evidence directory; an existing path is refused")
    heavy.add_argument("--target-dir", type=Path, help="Cargo target directory (default: inside the owned temporary directory)")
    args = parser.parse_args(argv)
    if args.command == "scan":
        result = scan(ROOT)
        print_scan(result)
        return 0 if result["passed"] else 1
    try:
        return gate(ROOT, args.output, args.target_dir)
    except FileExistsError:
        print(f"refusing to reuse the existing output path {args.output}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
