#!/usr/bin/env python3
"""Create the deterministic, sanitized AC-SPARQL publication archive.

The creator has no network behavior and invokes zstd directly without a shell.  It
preserves the EC2-original manifest, installs the three reviewed support reports,
constructs a new run-root manifest only after final derived output exists, writes a
sorted normalized tar stream, compresses it deterministically, and calls the publication
exporter's full member/hash/sanitization verifier before admitting the container.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import tarfile
import tempfile
from pathlib import Path
from typing import Any, Mapping, Sequence

import export_publication as publication


def load_json(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise publication.PublicationError(f"cannot read {label}: {path}: {error}") from error
    if not isinstance(value, dict):
        publication.fail(f"{label} must be an object")
    return value


def immutable_file(path: Path, payload: bytes, mode: str) -> None:
    publication.reconcile_immutable_outputs({path: payload}, mode)


def install_archive(candidate: Path, output: Path, mode: str) -> None:
    """Install a large archive immutably without materializing it in memory."""

    candidate_hash = publication.sha256_file(candidate)
    candidate_bytes = candidate.stat().st_size
    if output.exists():
        if not output.is_file() or output.is_symlink():
            publication.fail(f"archive target is not a regular non-symlink file: {output}")
        if output.stat().st_size != candidate_bytes or publication.sha256_file(output) != candidate_hash:
            publication.fail(f"immutable archive target differs; use a new run ID: {output}")
        return
    if mode == "check":
        publication.fail(f"archive check found missing output: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    try:
        os.link(candidate, output)
    except FileExistsError:  # pragma: no cover - race defence
        if (
            not output.is_file()
            or output.is_symlink()
            or output.stat().st_size != candidate_bytes
            or publication.sha256_file(output) != candidate_hash
        ):
            publication.fail(f"archive publication race produced a differing target: {output}")


def preserve_ec2_manifest(run_root: Path, expected_sha256: str, mode: str) -> Path:
    if not publication.HEX64.fullmatch(expected_sha256):
        publication.fail("--ec2-manifest-sha256 must be lowercase 64-hex")
    current = run_root / "MANIFEST.sha256"
    preserved = run_root / "provenance/ec2-original-MANIFEST.sha256"
    if preserved.exists():
        if not preserved.is_file() or preserved.is_symlink() or publication.sha256_file(preserved) != expected_sha256:
            publication.fail("preserved EC2-original manifest is missing, unsafe, or hash-mismatched")
        return preserved
    if mode == "check":
        publication.fail("archive check requires provenance/ec2-original-MANIFEST.sha256")
    if not current.is_file() or current.is_symlink() or publication.sha256_file(current) != expected_sha256:
        publication.fail("run-root MANIFEST.sha256 is not the declared EC2-original manifest")
    preserved.parent.mkdir(parents=True, exist_ok=True)
    try:
        os.link(current, preserved)
    except FileExistsError:  # pragma: no cover - race defence
        if publication.sha256_file(preserved) != expected_sha256:
            publication.fail("concurrent EC2-manifest preservation produced different bytes")
    current.unlink()
    return preserved


def install_report(source: Path, target: Path, mode: str) -> None:
    if not source.is_file() or source.is_symlink():
        publication.fail(f"support report is missing or unsafe: {source}")
    immutable_file(target, source.read_bytes(), mode)


def collect_members(run_root: Path) -> dict[str, Path]:
    required = ("DONE", "started-at.txt", "finished-at.txt", "environment.txt")
    members: dict[str, Path] = {}

    def add(path: Path) -> None:
        if not path.is_file() or path.is_symlink():
            publication.fail(f"archive input must be a regular non-symlink file: {path}")
        rel = publication.ensure_inside(run_root, path, "archive input")
        publication.safe_posix_member(rel, f"archive input {rel!r}")
        publication.scan_sensitive_name(rel)
        if ".partial" in path.name:
            publication.fail(f"partial file must be retained outside the sanitized archive: {rel}")
        if rel in members:
            publication.fail(f"archive input repeats logical path {rel}")
        with path.open("rb") as source:
            publication.hash_and_scan_member(source, rel)
        members[rel] = path

    for name in required:
        add(run_root / name)

    raw_root = run_root / "raw"
    raw_files = sorted(raw_root.rglob("*")) if raw_root.is_dir() else []
    for path in raw_files:
        if path.is_file() or path.is_symlink():
            add(path)
    jsonl = [name for name in members if name.startswith("raw/") and name.endswith(".jsonl")]
    if len(jsonl) != 1280:
        publication.fail(f"sanitized archive requires 1,280 canonical raw JSONL files, got {len(jsonl)}")

    derived_root = run_root / "derived"
    manifest_path = derived_root / "manifest.json"
    analyzer_manifest = load_json(manifest_path, "final analyzer manifest")
    if analyzer_manifest.get("schema_version") != publication.ANALYZER_SCHEMA_VERSION:
        publication.fail("final analyzer manifest schema changed")
    outputs = publication.require_list(analyzer_manifest.get("outputs"), "final analyzer outputs")
    if set(outputs) != publication.REQUIRED_ANALYZER_OUTPUTS:
        publication.fail("final analyzer output set is incomplete")
    add(manifest_path)
    for name in sorted(outputs):
        if Path(name).name != name:
            publication.fail(f"unsafe final analyzer output path: {name!r}")
        add(derived_root / name)
    declared_inputs = publication.require_list(analyzer_manifest.get("input_files"), "final analyzer input_files")
    if len(declared_inputs) != 1280:
        publication.fail("final analyzer manifest does not identify 1,280 raw inputs")
    for index, value in enumerate(declared_inputs):
        descriptor = publication.require_object(value, f"final analyzer input_files[{index}]")
        name = Path(publication.require_string(descriptor.get("path"), f"input {index}.path")).name
        expected = publication.require_string(descriptor.get("sha256"), f"input {index}.sha256")
        member = members.get(f"raw/{name}")
        if member is None or not publication.HEX64.fullmatch(expected) or publication.sha256_file(member) != expected:
            publication.fail(f"final analyzer input is absent or hash-mismatched: {name}")

    for dirname in ("correctness", "failures"):
        root = run_root / dirname
        if not root.is_dir():
            publication.fail(f"required supporting evidence directory is missing: {root}")
        for path in sorted(root.rglob("*")):
            if path.is_file() or path.is_symlink():
                add(path)
    if len([name for name in members if name.startswith("correctness/") and name.endswith(".jsonl")]) != 288:
        publication.fail("sanitized archive requires all 288 correctness-gate JSONL files")

    for name in ("cost-report.json", "correctness-report.json", "exclusion-report.json"):
        add(run_root / "reports" / name)
    add(run_root / "provenance/ec2-original-MANIFEST.sha256")

    unexpected_derived = {
        path.relative_to(derived_root).as_posix()
        for path in derived_root.rglob("*")
        if path.is_file()
    } - ({"manifest.json"} | set(outputs))
    if unexpected_derived:
        publication.fail(f"final derived directory contains unregistered files: {sorted(unexpected_derived)}")
    return dict(sorted(members.items()))


def manifest_payload(members: Mapping[str, Path]) -> bytes:
    return "".join(
        f"{publication.sha256_file(path)}  ./{name}\n" for name, path in sorted(members.items())
    ).encode("utf-8")


def write_tar(path: Path, prefix: str, members: Mapping[str, Path], manifest_path: Path) -> None:
    publication.safe_posix_member(prefix, "archive prefix")
    inputs = {f"{prefix}/MANIFEST.sha256": manifest_path}
    inputs.update({f"{prefix}/{name}": source for name, source in members.items()})
    with tarfile.open(path, "w", format=tarfile.USTAR_FORMAT) as bundle:
        for name, source in sorted(inputs.items()):
            info = tarfile.TarInfo(name)
            info.size = source.stat().st_size
            info.mode = 0o644
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mtime = 0
            with source.open("rb") as payload:
                bundle.addfile(info, payload)


def create_archive(
    repo_root: Path, run_root: Path, run_id: str, reports: Mapping[str, Path],
    ec2_manifest_sha256: str, zstd_command: str, mode: str, *,
    compression_level: str = "-19",
) -> dict[str, Any]:
    repo_root, run_root = repo_root.resolve(), run_root.resolve()
    if not publication.RUN_ID_RE.fullmatch(run_id):
        publication.fail("run ID must be a safe lowercase repository path component")
    output = (
        repo_root / "bench/canonical-competitor-results/ac-sparql" / run_id /
        "raw-sanitized.tar.zst"
    ).resolve()
    publication.ensure_inside(repo_root, output, "raw archive output")
    if compression_level not in {f"-{level}" for level in range(1, 20)}:
        publication.fail("zstd compression level must be an explicit -1 through -19")
    if mode == "check" and not output.exists():
        publication.fail(f"archive check found missing output: {output}")
    preserve_ec2_manifest(run_root, ec2_manifest_sha256, mode)
    for name, source in sorted(reports.items()):
        install_report(source.resolve(), run_root / "reports" / name, mode)
    members = collect_members(run_root)
    final_manifest = run_root / "MANIFEST.sha256"
    immutable_file(final_manifest, manifest_payload(members), mode)
    entries = publication.parse_sha256_manifest(final_manifest, run_root)
    executable, _, _ = publication.resolve_zstd(zstd_command)
    if mode == "write":
        output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".raw-archive.", dir=output.parent) as temporary:
        temporary_root = Path(temporary)
        tar_path = temporary_root / "raw-sanitized.tar"
        candidate = temporary_root / "raw-sanitized.tar.zst"
        write_tar(tar_path, run_id, members, final_manifest)
        try:
            completed = subprocess.run(
                [str(executable), "-q", "-f", "-T1", compression_level, str(tar_path), "-o", str(candidate)],
                check=False,
                capture_output=True,
            )
        except OSError as error:
            raise publication.PublicationError(f"cannot run zstd: {error}") from error
        if completed.returncode != 0:
            publication.fail(f"zstd compression failed with exit {completed.returncode}")
        artifact = publication.Artifact(
            candidate.name, candidate, publication.sha256_file(candidate), candidate.stat().st_size
        )
        verification = publication.verify_zstd_tar(
            artifact,
            publication.Artifact(
                final_manifest.name, final_manifest,
                publication.sha256_file(final_manifest), final_manifest.stat().st_size,
            ),
            entries,
            f"{run_id}/MANIFEST.sha256",
            str(executable),
        )
        install_archive(candidate, output, mode)
    installed = publication.Artifact(
        output.name, output, publication.sha256_file(output), output.stat().st_size
    )
    publication.verify_zstd_tar(
        installed,
        publication.Artifact(
            final_manifest.name, final_manifest,
            publication.sha256_file(final_manifest), final_manifest.stat().st_size,
        ),
        entries,
        f"{run_id}/MANIFEST.sha256",
        str(executable),
    )
    return {
        "archive": {
            "path": publication.ensure_inside(repo_root, output, "raw archive output"),
            "sha256": installed.sha256,
            "bytes": installed.bytes,
        },
        "manifest": {
            "path": str(final_manifest),
            "sha256": publication.sha256_file(final_manifest),
            "bytes": final_manifest.stat().st_size,
        },
        "manifest_archive_member": f"{run_id}/MANIFEST.sha256",
        "verification": verification,
    }


def arguments(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, required=True)
    parser.add_argument("--run-root", type=Path, required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--cost-report", type=Path, required=True)
    parser.add_argument("--correctness-report", type=Path, required=True)
    parser.add_argument("--exclusion-report", type=Path, required=True)
    parser.add_argument("--ec2-manifest-sha256", required=True)
    parser.add_argument("--zstd", required=True)
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument("--write", dest="mode", action="store_const", const="write")
    modes.add_argument("--check", dest="mode", action="store_const", const="check")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = arguments(argv)
    result = create_archive(
        args.repo_root,
        args.run_root,
        args.run_id,
        {
            "cost-report.json": args.cost_report,
            "correctness-report.json": args.correctness_report,
            "exclusion-report.json": args.exclusion_report,
        },
        args.ec2_manifest_sha256,
        args.zstd,
        args.mode,
    )
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except publication.PublicationError as error:
        print(f"archive creation refused: {error}", file=__import__("sys").stderr)
        raise SystemExit(2) from error
