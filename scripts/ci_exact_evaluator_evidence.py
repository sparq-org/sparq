#!/usr/bin/env python3
"""[GPT-6] Export source-bound artifacts from the mandatory real evaluator gate."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = "zk/sparql-evaluator/Cargo.toml"
GUEST_MANIFEST = "zk/sparql-evaluator/methods/guest/Cargo.toml"
V1_RECEIPTS = {"v1-verifier-select", "v1-holder-bag", "v1-verifier-false-ask"}
V2_RECEIPTS = {"v2-verifier-catalog", "v2-holder-false-ask"}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def capture(args: list[str], root: Path = ROOT) -> str:
    return subprocess.check_output(args, cwd=root, text=True, timeout=120).strip()


def snapshot(root: Path) -> dict:
    """Hash every tracked input, rather than infer source identity from a cache."""
    if capture(["git", "status", "--porcelain", "--untracked-files=no"], root):
        raise ValueError("tracked checkout changes invalidate source provenance")
    if capture(["git", "ls-files", "--others", "--exclude-standard"], root):
        raise ValueError("untracked checkout inputs invalidate source provenance")
    paths = subprocess.check_output(["git", "ls-files", "-z"], cwd=root).split(b"\0")
    files = {}
    for raw in paths:
        if not raw:
            continue
        relative = os.fsdecode(raw)
        path = root / relative
        # Preserve symlink identity without following a checkout-external target.
        content = os.fsencode(os.readlink(path)) if path.is_symlink() else path.read_bytes()
        files[relative] = digest(content)
    if not files:
        raise ValueError("empty tracked source inventory")
    return {"checkout_sha": capture(["git", "rev-parse", "HEAD"], root),
            "tree_sha": capture(["git", "rev-parse", "HEAD^{tree}"], root),
            "tracked_sha256": files}


def event_identity(env: dict, checkout: str) -> dict:
    sha = env.get("GITHUB_SHA", "")
    if not re.fullmatch(r"[0-9a-f]{40}", sha) or sha != checkout:
        raise ValueError("GitHub checkout SHA is missing or differs from the checked-out commit")
    event = json.loads(Path(env["GITHUB_EVENT_PATH"]).read_text())
    name = env["GITHUB_EVENT_NAME"]
    result = {"event": name, "github_sha": sha, "checkout_sha": checkout,
              "pr_head_sha": None, "pr_base_sha": None, "merge_group_head_sha": None,
              "merge_group_base_sha": None, "run_id": env["GITHUB_RUN_ID"],
              "run_attempt": env["GITHUB_RUN_ATTEMPT"], "repository": env["GITHUB_REPOSITORY"]}
    if name == "pull_request":
        result.update(pr_head_sha=event["pull_request"]["head"]["sha"],
                      pr_base_sha=event["pull_request"]["base"]["sha"],
                      pr_number=event["number"])
    elif name == "merge_group":
        result.update(merge_group_head_sha=event["merge_group"]["head_sha"],
                      merge_group_base_sha=event["merge_group"]["base_sha"])
        if result["merge_group_head_sha"] != checkout:
            raise ValueError("merge-group head does not match the actual checkout")
    elif name not in {"push", "workflow_dispatch"}:
        raise ValueError("unsupported GitHub event identity")
    for key, value in result.items():
        if key.endswith("_sha") and value is not None and not re.fullmatch(r"[0-9a-f]{40}", value):
            raise ValueError(f"invalid {key}")
    return result


def campaign_identity(env: dict, checkout: str, local: bool) -> dict:
    if not local:
        return event_identity(env, checkout)
    if env.get("GITHUB_ACTIONS") or env.get("GITHUB_EVENT_NAME"):
        raise ValueError("local identity cannot replace hosted event provenance")
    return {"event": "local", "checkout_sha": checkout,
            "pr_head_sha": None, "pr_base_sha": None,
            "scope": "Local execution only; no hosted run or PR-head attestation."}


def artifact_pin(directory: Path) -> dict:
    artifact = (directory / "guest.bin").read_bytes()
    pin = json.loads((directory / "pin.json").read_text())
    if not 0 < len(artifact) <= 32 * 1024 * 1024:
        raise ValueError("missing or oversized exported guest")
    if set(pin) != {"sha256", "image_id"} or any(
        not isinstance(pin[key], list) or len(pin[key]) != count
        or any(type(word) is not int or not 0 <= word <= maximum for word in pin[key])
        for key, count, maximum in [("sha256", 32, 255), ("image_id", 8, 2**32 - 1)]
    ):
        raise ValueError("invalid exported artifact pin")
    if bytes(pin["sha256"]).hex() != digest(artifact):
        raise ValueError("exported guest hash differs from its pin")
    return pin


def receipts(directory: Path, pin: dict, expected: set[str]) -> dict:
    paths = {path.stem: path for path in (directory / "receipts").glob("*.json")}
    if set(paths) != expected:
        raise ValueError("missing or unexpected genuine receipt fixture exports")
    result = {}
    for name, path in sorted(paths.items()):
        data = path.read_bytes()
        record = json.loads(data)
        if (record.get("schema") != "sparq-synthetic-receipt-evidence-v1"
                or record.get("fixture") != name or record.get("synthetic_inputs") is not True
                or record.get("pin") != pin or not record.get("request") or not record.get("receipt")):
            raise ValueError(f"invalid or mismatched receipt evidence: {name}")
        result[name] = {"sha256": digest(data), "bytes": len(data)}
    return result


def observed_hal(log: str) -> list[str]:
    plain = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", log)
    targets = sorted(set(re.findall(
        r"(?m)^[^\n]*\bDEBUG\s+(risc0_(?:circuit_rv32im::prove|zkp)::hal::(?:cpu|cuda))\s*:", plain
    )))
    if not targets:
        raise ValueError("actual prover HAL execution diagnostics are missing")
    return targets


def run_logged(args: list[str], name: str, output: Path, env: dict, commands: list[dict]) -> None:
    started = time.monotonic()
    with (output / f"{name}.log").open("wb") as log:
        process = subprocess.Popen(args, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT)
        assert process.stdout is not None
        for line in process.stdout:
            log.write(line)
            sys.stdout.buffer.write(line)
            sys.stdout.buffer.flush()
        status = process.wait()
    commands.append({"name": name, "argv": args, "status": status,
                     "elapsed_seconds": time.monotonic() - started})
    if status:
        raise ValueError(f"{name} failed with exit status {status}")


def build_metadata(manifest: str, env: dict) -> dict:
    return json.loads(subprocess.check_output([
        "cargo", "metadata", "--manifest-path", manifest, "--locked", "--offline",
        "--format-version", "1",
    ], cwd=ROOT, env=env, timeout=120))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="new evidence directory")
    parser.add_argument("--local", action="store_true", help="record local-only identity; forbidden in GitHub Actions")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir()  # Existing evidence must never be overwritten or reused.
    (output / "receipts").mkdir()
    commands: list[dict] = []
    try:
        env = os.environ.copy()
        forbidden = [key for key in ("RISC0_SKIP_BUILD", "SPARQ_ACCEPTED_GUEST_ARTIFACT",
                                     "SPARQ_ACCEPTED_GUEST_PIN", "RUSTC_WRAPPER",
                                     "RUSTC_WORKSPACE_WRAPPER") if key in env]
        if forbidden or env.get("RISC0_DEV_MODE", "0") != "0":
            raise ValueError(f"ambiguous/mock build or artifact overrides: {forbidden}")
        before = snapshot(ROOT)
        identity = campaign_identity(env, before["checkout_sha"], args.local)
        (output / "source.json").write_text(json.dumps(before, indent=2) + "\n")
        server = Path(env["RISC0_SERVER_PATH"]).resolve(strict=True)
        guest_compilers = list((Path(env["RISC0_HOME"]) / "toolchains").glob("*/bin/rustc"))
        if len(guest_compilers) != 1:
            raise ValueError("guest compiler installation must be unambiguous")
        toolchains = {"host_rustc": capture(["rustc", "-vV"]),
                      "host_cargo": capture(["cargo", "-V"]),
                      "guest_rustc": capture([str(guest_compilers[0]), "-vV"]),
                      "r0vm_version": capture([str(server), "--version"]),
                      "r0vm_sha256": digest(server.read_bytes()),
                      "guest_rustc_sha256": digest(guest_compilers[0].read_bytes())}
        if not ("1.97.1" in toolchains["host_rustc"]
                and "1.97.0" in toolchains["guest_rustc"]
                and toolchains["r0vm_version"].split()[-1] == "3.0.6"):
            raise ValueError("actual toolchain does not match the pinned profile")
        if env.get("RISC0_BUILD_LOCKED") != "1":
            raise ValueError("locked guest-build mode is required")
        build_environment = {key: env.get(key) for key in [
            "CARGO_BUILD_JOBS", "CARGO_INCREMENTAL", "RISC0_BUILD_LOCKED",
            "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_PROFILE_RELEASE_DEBUG",
        ]}
        target = Path(env["CARGO_TARGET_DIR"]).resolve()
        if target == ROOT or ROOT.is_relative_to(target):
            raise ValueError("target directory must not contain the checkout")
        # Rebuild every path dependency, including the patched SDK. Registry
        # dependencies retain their locked cache; compiler/lock identities are recorded.
        for label, manifest, package_target in [("host", MANIFEST, target),
                                               ("guest", GUEST_MANIFEST, target / "sparq-exact-guest")]:
            run_logged(["cargo", "fetch", "--locked", "--manifest-path", manifest],
                       f"fetch-{label}-locked-graph", output, env, commands)
            metadata = build_metadata(manifest, env)
            (output / f"{label}-metadata.json").write_text(json.dumps(metadata) + "\n")
            local = sorted({p["name"] for p in metadata["packages"] if p["source"] is None})
            clean = ["cargo", "clean", "--manifest-path", manifest, "--target-dir", str(package_target)]
            for package in local:
                clean.extend(["-p", package])
            run_logged(clean, f"rebuild-{label}-local-packages", output, env, commands)
        cargo = ["cargo", "run", "--locked", "--manifest-path", MANIFEST,
                 "-p", "sparq-proved-evaluator", "--example", "export_guest", "--"]
        run_logged(cargo + [str(output / "artifact")], "export-guest", output, env, commands)
        pin = artifact_pin(output / "artifact")
        env["SPARQ_EVALUATOR_EVIDENCE_DIR"] = str(output)
        # These filters expose kernel dimensions/target names only. Never enable
        # broad witness/preflight TRACE logs; all test input here is synthetic.
        env["RUST_LOG"] = "risc0_circuit_rv32im::prove::hal=debug,risc0_zkp::hal=debug"
        test = ["cargo", "test", "--locked", "--manifest-path", MANIFEST]
        run_logged(test + ["-p", "sparq-proved-evaluator-model", "--features", "evaluate"],
                   "native", output, env, commands)
        run_logged(test + ["-p", "sparq-proved-evaluator", "--", "--nocapture", "--test-threads=1"],
                   "actual-guest-and-proofs", output, env, commands)
        run_logged(["cargo", "clippy", "--locked", "--manifest-path", MANIFEST, "--workspace",
                    "--all-targets", "--features", "sparq-proved-evaluator-model/evaluate", "--",
                    "-D", "warnings"], "lint", output, env, commands)
        run_logged(cargo + [str(output / "artifact-after")], "confirm-guest", output, env, commands)
        if pin != artifact_pin(output / "artifact-after"):
            raise ValueError("exported program changed during the campaign")
        if snapshot(ROOT) != before:
            raise ValueError("source identity changed during the campaign")
        expected = V1_RECEIPTS | (V2_RECEIPTS if (ROOT / "zk/sparql-evaluator/model/src/v2.rs").exists() else set())
        receipt_files = receipts(output, pin, expected)
        kernel_log = (output / "actual-guest-and-proofs.log").read_text(errors="replace")
        observed = observed_hal(kernel_log)
        result = {"schema": "sparq-evaluator-campaign-v1", "completed": True,
                  "identity": identity, "toolchains": toolchains, "commands": commands,
                  "build_environment": build_environment, "diagnostic_filter": env["RUST_LOG"],
                  "artifact_pin": pin, "receipts": receipt_files,
                  "hal_execution_targets_observed": observed,
                  "hal_observation_limit": "Logged execution targets, not hardware inference; unlogged stages are not attributed.",
                  "source_inventory_sha256": digest((output / "source.json").read_bytes()),
                  "privacy_scope": "Synthetic fixtures; experimental upstream privacy assumptions remain."}
        (output / "evidence.json").write_text(json.dumps(result, indent=2) + "\n")
        return 0
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        (output / "failure.json").write_text(json.dumps({"completed": False, "error": str(error),
                                                        "commands": commands}, indent=2) + "\n")
        print(f"Exact evaluator evidence rejected: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
