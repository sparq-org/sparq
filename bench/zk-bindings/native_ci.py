#!/usr/bin/env python3
"""[GPT-6] Mandatory finite native RDF replay; no build or simulated proofs."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tomllib

from corpus import digest, encoded, exhaustive, load, plan
import run as controller


# This is the unchanged finite domain executed in the retained 89a campaign.
# Changing its oracle, query bytes, classifications or jobs requires a reviewed
# new pin, not a smaller denominator or a successful tool-availability skip.
PLAN_SHA256 = "4963a1ff1aa9a68de9a396432f6163b313587ef20910eb6655d1badde90added"
EXPECTED_COUNTS = {"required_accepted": 72, "weaker_verified_required_rejected": 468,
                   "empty_graph_admission": 36}
REPLAY = {"name": "same-verifier-replay", "observed": "rejected",
          "error_class": "Replay", "stage": "proof_verifier"}
WEAKER_CONTROLS = [
    {"name": "honest-preparation", "observed": "rejected", "stage": "support", "error_class": "Support"},
    {"name": "same-context-weaker-statement", "observed": "accepted", "stage": "proof_verifier",
     "verified_count": 1, "omitted": "all requested RDF preimages"},
    {"name": "required-statement", "observed": "rejected", "stage": "proof_verifier", "error_class": "Verification"},
]


def require(condition, message):
    if not condition:
        raise ValueError(message)


def native_plan():
    value = plan(exhaustive(), ["native_rdf"], "real", coverage="exhaustive_tiny_v1")
    require(digest(value) == PLAN_SHA256, "native finite domain differs from reviewed pin")
    require(len(value["jobs"]) == 576 and len(value["classifications"]) == 80,
            "native finite domain denominator changed")
    return value


def byte_array(value):
    return isinstance(value, list) and bool(value) and all(type(v) is int and 0 <= v <= 255 for v in value)


def check_native_outcome(job, outcome, output):
    """Stronger native-specific obligations, in addition to shared protocol checks."""
    inventory = controller.check_outcome(job, outcome, output)
    require(outcome.get("implementation", {}).get("input_sha256") == hashlib.sha256(encoded(job)).hexdigest(),
            "adapter input bytes not bound to retained job")
    artifacts = outcome["artifacts"]
    names = [a["path"] for a in artifacts]
    if not job["triples"]:
        require(not job["expected_accept"] and outcome["stage"] == "admission"
                and outcome["error_class"] == "Capacity"
                and inventory["proofs"] == inventory["verified"] == 0
                and names == [] and outcome["controls"] == []
                and outcome.get("unsupported_reason") == "existing bounded nonempty signed-graph profile; admission-only refusal, no attempted proof",
                "empty graph must remain a distinct admission-only exclusion")
        return "empty_graph_admission", None
    positive = job["expected_accept"]
    proof_name = "proof.bin" if positive else "weaker-proof.bin"
    require(inventory == {"proofs": 1, "verified": int(positive), "stage": "proof_verifier"},
            "nonempty binding lacks exactly one actual required/weaker proof")
    require(len(names) == 2 and set(names) == {"public.json", proof_name},
            "native proof/public artifact inventory differs")
    require(outcome["controls"] == ([REPLAY] if positive else WEAKER_CONTROLS),
            "missing actual replay or independently verified weaker-statement control")
    require(outcome["error_class"] == (None if positive else "Verification"),
            "required verifier rejection must use Verification")
    require((output / proof_name).stat().st_size <= 2 * 1024 * 1024, "proof artifact exceeds native wire budget")
    proof = (output / proof_name).read_bytes()
    require(bool(proof), "empty proof artifact")
    public = load(output / "public.json")
    expected_nonce = hashlib.sha256(b"sparq/native-rdf/binding-job/nonce/v1\0" + encoded(job)).digest()
    require(public.get("nonce") == list(expected_nonce), "native proof nonce differs from complete job")
    context = public.get("context", {})
    require(context.get("query") == job["query"]
            and context.get("rows") == [dict(zip(job["variables"], job["rows"][0]))]
            and context.get("protocol") == list(b"sparq/native-rdf/public-bgp/v1")
            and context.get("semantics") == "nonempty-distinct-successful-support"
            and context.get("support") == public.get("support"),
            "retained native public claim differs from expected query/row/support")
    key = public.get("issuer_key")
    require(byte_array(key) and context.get("roles") == [[
        "urn:sparq:proof-binding:synthetic-issuer", key,
        "urn:sparq:proof-binding:synthetic-status", 1, 0,
        list(hashlib.blake2b(bytes([0]), digest_size=64).digest())]],
        "retained issuer/status policy differs from synthetic fixture contract")
    return ("required_accepted" if positive else "weaker_verified_required_rejected"), hashlib.sha256(proof).hexdigest()


def verify_campaign(output):
    """Validate retained records and identities; this does not rerun cryptography."""
    expected = native_plan()
    campaign = output / "campaign"
    require(load(campaign / "plan.json") == expected, "persisted plan differs from full native domain")
    report = load(campaign / "report.json")
    require(report.get("passed") is True and report.get("complete_declared_domain") is True
            and report.get("plan_sha256") == PLAN_SHA256 and report.get("coverage_gaps") == []
            and report.get("all_backend_profiles_complete") is False,
            "campaign failed, omitted coverage, or overstated backend completeness")
    records = report.get("records", [])
    require(len(records) == 576, "missing or additional native job record")
    counts, proofs, nonces, artifact_hashes = Counter(), set(), set(), []
    for ordinal, (job, record) in enumerate(zip(expected["jobs"], records)):
        directory = campaign / f"{ordinal:06}-{job['id'][:12]}"
        require((directory / "job.json").read_bytes() == encoded(job)
                and record.get("job_id") == job["id"] and record.get("backend") == "native_rdf"
                and record.get("expected_accept") is job["expected_accept"]
                and record.get("status") == "passed"
                and record.get("input_sha256") == hashlib.sha256(encoded(job)).hexdigest()
                and load(directory / "record.json") == record,
                "job record omitted, reordered, substituted or inconsistent")
        outcome = load(directory / "adapter/outcome.json")
        require(outcome == record.get("outcome"), "outcome differs from report")
        kind, proof_hash = check_native_outcome(job, outcome, directory / "adapter")
        require(record.get("inventory") == controller.check_outcome(job, outcome, directory / "adapter"),
                "recorded proof inventory differs from actual outcome")
        counts[kind] += 1
        if proof_hash:
            require(proof_hash not in proofs, "proof artifact reused across distinct claims")
            proofs.add(proof_hash)
            nonce = tuple(load(directory / "adapter/public.json")["nonce"])
            require(nonce not in nonces, "proof nonce reused across distinct claims")
            nonces.add(nonce)
        artifact_hashes.extend({"path": str((directory / "adapter" / a["path"]).relative_to(output)),
                                "sha256": a["sha256"]} for a in outcome["artifacts"])
    require(dict(counts) == EXPECTED_COUNTS and len(proofs) == len(nonces) == 540
            and len(artifact_hashes) == 1080, "native proof/rejection denominators differ")
    totals = expected["totals"]["native_rdf"] | {
        "executed_jobs": 576, "passed_jobs": 576, "genuine_proofs": 540,
        "verified_proofs": 72, "negative_stages": {"admission": 36, "proof_verifier": 468}}
    require(report.get("totals") == {"native_rdf": totals}, "aggregate native counts differ from per-job evidence")
    return {"schema": "sparq.native-bindings-ci.v1", "plan_sha256": PLAN_SHA256,
            "report_sha256": controller.file_hash(campaign / "report.json"),
            "counts": dict(counts), "distinct_proofs": len(proofs), "distinct_nonces": len(nonces),
            "classified_other_cases": expected["classifications"], "artifacts": artifact_hashes,
            "claim": "This declared finite native domain only; admission exclusions and actual verifier negatives are distinct. The artifact checker does not rerun cryptography."}


def check_build_artifact(root, binary, events_path, metadata_path):
    """Bind retained Cargo records to this package/target and supplied executable.

    Cargo's compiler-artifact and metadata-v1 formats provide the package ID,
    manifest, target and actual executable path. This is a build-record linkage,
    not an independent reproduction or signed build attestation.
    """
    manifest = (root / "zk/native-composition/Cargo.toml").resolve(strict=True)
    package = tomllib.loads(manifest.read_text())["package"]
    require(events_path.stat().st_size <= 32 * 1024 * 1024
            and metadata_path.stat().st_size <= 2 * 1024 * 1024, "Cargo record capacity exceeded")
    metadata = load(metadata_path)
    require(metadata.get("version") == 1, "unsupported Cargo metadata format")
    packages = [p for p in metadata.get("packages", []) if p.get("name") == package["name"]
                and p.get("version") == package["version"]
                and Path(p.get("manifest_path", "")).resolve(strict=True) == manifest]
    require(len(packages) == 1, "missing or ambiguous native Cargo package")
    package_id = packages[0].get("id")
    require(isinstance(package_id, str) and bool(package_id)
            and package_id in metadata.get("workspace_members", [])
            and Path(metadata.get("workspace_root", "")).resolve(strict=True) == manifest.parent,
            "Cargo package is not the detached native workspace member")
    source = (manifest.parent / "src/bin/native-bindings.rs").resolve(strict=True)
    targets = [t for t in packages[0].get("targets", []) if t.get("name") == "native-bindings"
               and t.get("kind") == ["bin"] and Path(t.get("src_path", "")).resolve(strict=True) == source]
    require(len(targets) == 1, "missing or ambiguous native binding target")
    events = [json.loads(line) for line in events_path.read_text().splitlines() if line.strip()]
    finished = [e for e in events if e.get("reason") == "build-finished"]
    require(len(finished) == 1 and finished[0].get("success") is True, "Cargo build did not finish successfully")
    artifacts = [e for e in events if e.get("reason") == "compiler-artifact"
                 and e.get("package_id") == package_id and e.get("target", {}).get("name") == "native-bindings"]
    require(len(artifacts) == 1, "missing or ambiguous native compiler artifact")
    artifact = artifacts[0]
    require(Path(artifact.get("manifest_path", "")).resolve(strict=True) == manifest
            and artifact.get("target", {}).get("kind") == ["bin"]
            and Path(artifact["target"].get("src_path", "")).resolve(strict=True) == source
            and artifact.get("profile", {}).get("test") is False
            and "native-binding" in artifact.get("features", []),
            "compiler artifact package, target or feature differs")
    executable = artifact.get("executable")
    require(isinstance(executable, str) and Path(executable).is_absolute()
            and Path(executable).resolve(strict=True) == binary.resolve(strict=True),
            "Cargo executable differs from supplied binary")
    return {"cargo_events_sha256": controller.file_hash(events_path),
            "cargo_metadata_sha256": controller.file_hash(metadata_path),
            "package_id": package_id, "manifest_path": str(manifest), "target": artifact["target"],
            "executable": str(binary.resolve(strict=True)), "executable_sha256": controller.file_hash(binary),
            "cargo_profile": artifact["profile"], "cargo_fresh": artifact.get("fresh"),
            "independent_or_signed_build_attestation": False}


def execute(binary, circom, output, cargo_events=None, cargo_metadata=None):
    root = Path(__file__).resolve().parents[2]
    binary, circom = binary.resolve(strict=True), circom.resolve(strict=True)
    output.mkdir(parents=True, exist_ok=False)
    require((cargo_events is None) == (cargo_metadata is None), "both Cargo records are required together")
    build = None if cargo_events is None else check_build_artifact(root, binary, cargo_events, cargo_metadata)
    source = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    version = subprocess.run([str(circom), "--version"], capture_output=True, check=True, timeout=30)
    require(version.stdout == b"circom compiler 2.2.2\n" and not version.stderr, "unpinned Circom compiler")
    adapter = {"argv": [str(binary)], "executable_sha256": controller.file_hash(binary),
               "checkout": str(root), "source_commit": source,
               "tools": {"circom": {"path": str(circom), "sha256": controller.file_hash(circom),
                                      "version_stdout": version.stdout.decode(), "version_stderr": ""}}}
    provenance = {"schema": "sparq.native-bindings-ci-source.v1", "source_commit": source,
                  "observed_checkout_not_binary_build_attestation": True,
                  "cargo_build_record": build,
                  "adapter": adapter, "rustc_verbose": subprocess.check_output(["rustc", "-vV"], text=True),
                  "source_hashes": {name: controller.file_hash(root / name) for name in (
                      "bench/zk-bindings/corpus.py", "bench/zk-bindings/run.py", "bench/zk-bindings/native_ci.py",
                      "bench/zk-bindings/protocol.json", "zk/native-composition/Cargo.toml",
                      "zk/native-composition/Cargo.lock", "zk/native-composition/src/bin/native-bindings.rs",
                      "zk/native-composition/src/rdf.rs", "zk/native-composition/src/rdf/binding_tests.rs")},
                  "build_provenance": "CI builds the supplied release executable immediately before this step; local callers supply a binary. Refer to CI build records; checkout/tool observations alone do not attest compilation."}
    controller.write(output / "source.json", provenance)
    controller.write(output / "plan.json", native_plan())
    controller.write(output / "adapters.json", {"native_rdf": adapter})
    require(controller.run(output / "plan.json", output / "adapters.json", output / "campaign", 180, 16 * 1024 * 1024),
            "native finite campaign did not pass")
    summary = verify_campaign(output)
    summary["source_sha256"] = controller.file_hash(output / "source.json")
    controller.write(output / "native-summary.json", summary)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--circom", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cargo-events", type=Path, help="CI compiler-artifact JSONL; requires metadata")
    parser.add_argument("--cargo-metadata", type=Path, help="Cargo metadata-v1 JSON; requires events")
    args = parser.parse_args()
    execute(args.binary, args.circom, args.output, args.cargo_events, args.cargo_metadata)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"native binding campaign failed: {error}", file=sys.stderr)
        sys.exit(1)
