#!/usr/bin/env python3
"""[GPT-6] Explicit remote compile/layout controls; importing this module runs none."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import tomllib

import verify


def run(argv, cwd, output, label, env, expected=0):
    start = time.monotonic()
    with (output / (label + ".stdout")).open("xb") as stdout, (output / (label + ".stderr")).open("xb") as stderr:
        child = subprocess.run(argv, cwd=cwd, env=env, stdout=stdout, stderr=stderr, timeout=600)
    binaries = []
    for line in (output / (label + ".stdout")).read_text().splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(event, dict) and event.get("reason") == "compiler-artifact" and event.get("executable"):
            executable = Path(event["executable"])
            digest = verify.sha(executable.read_bytes())
            retained = output / "retained-executables" / digest
            retained.parent.mkdir(exist_ok=True)
            if not retained.exists():
                shutil.copyfile(executable, retained)
            verify.require(verify.sha(retained.read_bytes()) == digest, "retained executable differs")
            binaries.append({"original":str(executable), "retained":str(retained), "sha256":digest})
    record = {"argv":argv, "cwd":str(cwd), "exit_code":child.returncode, "executables":binaries,
              "elapsed_seconds":time.monotonic() - start}
    (output / (label + ".json")).write_text(json.dumps(record, indent=2) + "\n")
    verify.require(child.returncode == expected, f"{label}: expected exit {expected}, got {child.returncode}")
    return record


def failure_messages(stdout):
    return [event["message"] for line in stdout.read_text().splitlines()
            if (event := json.loads(line)).get("reason") == "compiler-message"
            and event["message"]["level"] == "error"]


def assert_diagnostic(messages, expected):
    verify.require(any(expected in m["message"] and any(s["is_primary"] and s["file_name"].endswith("src/lib.rs")
                       for s in m["spans"]) for m in messages), "missing expected spanned source diagnostic")
    verify.require(not any("proc-macro derive panicked" in m["message"] for m in messages), "macro panic instead of diagnostic")


def campaign(output, target):
    output.mkdir(parents=True, exist_ok=False)
    verify.verify()
    native, support = verify.NATIVE, verify.SUPPORT
    env = os.environ | {"CARGO_TARGET_DIR":str(target), "CARGO_BUILD_JOBS":"1", "CARGO_INCREMENTAL":"0"}
    (output / "inputs.json").write_text(json.dumps({
        "native_lock_sha256":verify.sha((native / "Cargo.lock").read_bytes()),
        "provenance_sha256":verify.sha((support / "UPSTREAM.json").read_bytes()),
        "layout_control_sha256":verify.sha((support / "layout.rs").read_bytes()),
        "target":str(target), "scope":"remote diagnostics/layout controls; zero proofs; no performance claim",
    }, indent=2) + "\n")
    run(["cargo", "metadata", "--offline", "--locked", "--all-features", "--format-version", "1"], native, output, "native-metadata", env)
    verify.check_metadata(json.loads((output / "native-metadata.stdout").read_text()))
    package = native / "vendor/wasmer-derive-6.1.0"
    original, candidate = output / "original", output / "candidate"
    verify.reconstruct(package, support, original)
    shutil.copytree(package, candidate)
    initial = verify.inventory(candidate)
    harness = output / "harness"
    (harness / "src").mkdir(parents=True)
    manifest = f'''[package]
name = "sparq-wasmer-diagnostic-controls"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
wasmer-derive = {{ path = {json.dumps(str(candidate))} }}
wasmer-types = {{ version = "=6.1.0", default-features = false, features = ["std"] }}
syn = {{ version = "=1.0.109", features = ["full", "extra-traits"] }}
quote = "=1.0.47"
proc-macro2 = "=1.0.107"
# Test-only registry baseline helper; absent from the native runtime graph.
proc-macro-error2 = "=2.0.1"
'''
    (harness / "Cargo.toml").write_text(manifest)
    library = harness / "src/lib.rs"
    layout = (support / "layout.rs").read_text().replace("ORIGINAL_MODULE", str(original / "src/value_type.rs")).replace("CANDIDATE_MODULE", str(candidate / "src/value_type.rs"))
    library.write_text(layout)
    shutil.copyfile(native / "Cargo.lock", harness / "Cargo.lock")
    run(["cargo", "metadata", "--offline", "--format-version", "1"], harness, output, "test-metadata", env)
    # Resolution of the temporary package is allowed only within the exact
    # native registry inventory plus the two removed baseline diagnostic nodes.
    upstream = tomllib.loads((native / "Cargo.lock").read_text())["package"]
    removed = json.loads((support / "lock-delta.json").read_text())["removed_registry_packages"]
    allowed = {(p["name"], p["version"], p.get("source"), p.get("checksum")) for p in upstream + removed}
    selected = tomllib.loads((harness / "Cargo.lock").read_text())["package"]
    verify.require(all((p["name"], p["version"], p.get("source"), p.get("checksum")) in allowed
                       for p in selected if p.get("source")), "temporary graph introduced an unpinned registry package")
    run(["cargo", "test", "--offline", "--locked", "--message-format=json", "--lib"], harness, output, "valid-layout-and-diagnostics", env)
    # Deliberately delete the representation guard only in the retained copy.
    helper = candidate / "src/value_type.rs"
    before = helper.read_text()
    verify.require(before.count("    check_repr(input)?;") == 1, "representation mutation target")
    helper.write_text(before.replace("    check_repr(input)?;", ""))
    try:
        run(["cargo", "test", "--offline", "--locked", "--message-format=json", "--lib", "invalid_shapes_report_expected_errors"], harness, output, "guard-removal", env, 101)
        verify.require("invalid_shapes_report_expected_errors ... FAILED" in (output / "guard-removal.stdout").read_text(), "mutation failed before the assertion")
    finally:
        helper.write_text(before)
    run(["cargo", "test", "--offline", "--locked", "--message-format=json", "--lib"], harness, output, "restored-layout", env)
    fixtures = [
        ("missing-repr", "#[derive(wasmer_derive::ValueType)] struct Bad { a: u32 }", "ValueType can only be derived for #[repr(C)] or #[repr(transparent)] structs"),
        ("enum", "#[derive(wasmer_derive::ValueType)] #[repr(C)] enum Bad { A }", "ValueType can only be derived for structs"),
        ("union", "#[derive(wasmer_derive::ValueType)] #[repr(C)] union Bad { a: u32 }", "ValueType can only be derived for structs"),
        ("malformed-repr", "#[derive(wasmer_derive::ValueType)] #[repr(C =)] struct Bad { a: u32 }", "expected"),
        ("non-value-field", "#[derive(Clone, Copy)] struct Marker(u8); #[derive(Clone, Copy, wasmer_derive::ValueType)] #[repr(C)] struct Bad { a: Marker }", "ValueType"),
        ("packed-unaligned", "#[derive(Clone, Copy, wasmer_derive::ValueType)] #[repr(C, packed)] struct Bad { a: u8, b: u32 }", "unaligned"),
    ]
    for label, source, diagnostic in fixtures:
        library.write_text(source + "\n")
        run(["cargo", "check", "--offline", "--locked", "--message-format=json"], harness, output, label, env, 101)
        assert_diagnostic(failure_messages(output / (label + ".stdout")), diagnostic)
    # Empty token output would silently accept a missing-repr type that is never
    # used; the same real compile-fail discriminator must catch this adapter loss.
    entry = candidate / "src/lib.rs"
    before = entry.read_text()
    verify.require(before.count(".unwrap_or_else(syn::Error::into_compile_error)") == 1, "emitter mutation target")
    entry.write_text(before.replace(".unwrap_or_else(syn::Error::into_compile_error)", ".unwrap_or_else(|_| proc_macro2::TokenStream::new())"))
    library.write_text(fixtures[0][1] + "\n")
    try:
        run(["cargo", "check", "--offline", "--locked", "--message-format=json"], harness, output, "emitter-removal", env)
        verify.require(not failure_messages(output / "emitter-removal.stdout"), "emitter mutation unexpectedly rejected")
    finally:
        entry.write_text(before)
    library.write_text(layout)
    run(["cargo", "test", "--offline", "--locked", "--message-format=json", "--lib"], harness, output, "final-restored", env)
    verify.require(verify.inventory(candidate) == initial, "mutated candidate not restored")
    verify.verify()
    (output / "completion.json").write_text(json.dumps({"passed":True, "layout_test_functions":4,
        "valid_token_comparisons":6, "actual_compile_fail_fixtures":6, "guard_mutations_killed":2,
        "proofs":0, "source_inventory":initial}, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target-dir", type=Path, required=True)
    args = parser.parse_args()
    campaign(args.output.resolve(), args.target_dir.resolve())
