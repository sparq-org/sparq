#!/usr/bin/env python3
"""Re-measure the result evidence members and rewrite the four evidence files.

`verify_result_evidence.py` binds the evidence to sha256 hashes of every zk/compose Noir
source, so a change to those sources needs a fresh measurement. Save the pre-change
evidence, then measure and apply (replace placeholders; stop if any command fails):

    git archive <commit> | tar -x -C run1        # and again into run2
    remeasure_result_evidence.py measure run1 run1.json
    remeasure_result_evidence.py measure run2 run2.json
    remeasure_result_evidence.py apply run1.json run2.json --job job.json \\
        --commit <commit> --observed-at <utc> --platform <host> \\
        --baseline-commit <pre-change-commit> --expect-member <member> ... --expect-source <path> ...

`measure` compiles every member with the pinned nargo, hashes its ACIR bytecode, ABI and
artifact, runs `bb gates -s ultra_honk`, and for the v1 members writes the noir-recursive
verification key. `apply` refuses to write unless the two runs agree (the artifact hash
embeds the checkout path and is not compared), every v1 ACIR, ABI and key equals its
713025dc baseline record, every size equals gate_counts_latest.json, and exactly the expected
members and sources change. It validates all four files before replacing any of them and
attempts every restore independently if a later replacement fails. If any restore fails,
it exits non-zero and lists exactly which files need `git checkout -- <file>` recovery.
An interruption of the process itself (a kill or power loss between renames) can
still leave a mixed set, which
`git checkout -- <the four files>` undoes. It refuses to run while the four files or
gate_counts_latest.json have staged or unstaged edits against HEAD, so that recovery never
discards anything else and the size gate reads the committed snapshot, and unless `--commit`
is HEAD. Git runs without inherited GIT_* environment variables.
`--baseline-commit` names the commit whose evidence is the pre-change baseline (default
HEAD); the files are read from that commit with `git show`, never from the working tree.
Each file keeps its destination `prepared_by`, and `apply` refuses a baseline whose
`prepared_by` differs from it, or that differs from the destination in any field it does
not re-measure or rewrite. The result is always
labelled non-canonical; the job manifest and run outputs are not part of the repository,
so the two-run agreement it records is checked when `apply` runs, not later.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
EVIDENCE = ("result_v1_compatibility", "result_capacity_gates", "result_signed_gates", "result_public_gates")
# Fields each file records per member; the compatibility file is checked separately.
FIELDS = {
    "result_capacity_gates": ("circuit_size", "acir_bytecode_sha256"),
    "result_signed_gates": ("circuit_size", "acir_bytecode_sha256", "abi_sha256"),
    "result_public_gates": ("circuit_size", "acir_bytecode_sha256", "abi_sha256", "gate_log_sha256"),
}
# Top-level fields `apply` writes into every file.
REWRITTEN = ("source_files", "measured_source", "measurement_method", "limitations")
SNAPSHOT = "bench/zk-compose/gate_counts_latest.json"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run(command: list[str], cwd: Path) -> str:
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True, timeout=3600)
    if result.returncode:
        raise SystemExit(f"{command} failed:\n{result.stdout}\n{result.stderr}")
    return result.stdout


def load(root: Path) -> dict[str, dict]:
    return {name: json.loads((root / "bench/zk-compose" / f"{name}.json").read_text()) for name in EVIDENCE}


def measure(source: Path, output: Path) -> None:
    source = source.resolve()
    nargo, bb = os.environ.get("NARGO", "nargo"), os.environ.get("BB", "bb")
    evidence = load(source)
    members = sorted(set().union(*(data["members"] for data in evidence.values())))
    v1 = evidence["result_v1_compatibility"]["members"]
    tool_paths = {tool: Path(run(["which", path], source).strip()).resolve()
                  for tool, path in (("nargo", nargo), ("bb", bb))}
    result = {
        "tools": {"nargo": run([nargo, "--version"], source).strip(), "bb": run([bb, "--version"], source).strip(),
                  "nargo_sha256": digest(tool_paths["nargo"].read_bytes()),
                  "bb_sha256": digest(tool_paths["bb"].read_bytes())},
        "source_files": {
            str(path.relative_to(source)): digest(path.read_bytes())
            for path in sorted((source / "zk/compose").rglob("*.nr"))
            if "target" not in path.relative_to(source).parts
        },
        "members": {},
    }
    compose = source / "zk/compose"
    for member in members:
        run([nargo, "compile", "--package", member, "--force"], compose)
        artifact = compose / "target" / f"{member}.json"
        raw = artifact.read_bytes()
        compiled = json.loads(raw)
        gates = run([bb, "gates", "-s", "ultra_honk", "-b", str(artifact)], compose)
        record = {
            "circuit_size": json.loads(gates)["functions"][0]["circuit_size"],
            "acir_bytecode_sha256": digest(base64.b64decode(compiled["bytecode"])),
            "abi_sha256": digest(json.dumps(compiled["abi"], sort_keys=True, separators=(",", ":")).encode()),
            "artifact_sha256": digest(raw),
            "gate_log_sha256": digest(gates.encode()),
        }
        if member in v1:
            with tempfile.TemporaryDirectory() as key_dir:
                run([bb, "write_vk", "-b", str(artifact), "-o", key_dir, "-t", "noir-recursive"], compose)
                key = (Path(key_dir) / "vk").read_bytes()
            record["vk_sha256"], record["vk_bytes"] = digest(key), len(key)
        result["members"][member] = record
        print(member, record["circuit_size"], flush=True)
    output.write_text(json.dumps(result, indent=1, sort_keys=True) + "\n")


def without_artifacts(run_result: dict) -> dict:
    copy = json.loads(json.dumps(run_result))
    for record in copy["members"].values():
        record.pop("artifact_sha256")
    return copy


def git(*arguments: str) -> subprocess.CompletedProcess:
    # GIT_* variables (GIT_DIR, GIT_INDEX_FILE, ...) could point git at another repository or
    # index than the checkout being rewritten, so they are not inherited.
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    try:
        return subprocess.run(["git", *arguments], cwd=ROOT, capture_output=True, text=True, env=environment)
    except OSError as error:
        raise SystemExit(f"cannot run git: {error}") from error


def load_commit(commit: str) -> dict[str, dict]:
    """Read the evidence files as committed at `commit`, never from the working tree."""
    resolved = git("rev-parse", "--verify", "--quiet", f"{commit}^{{commit}}")
    if resolved.returncode:
        raise SystemExit(f"cannot resolve baseline commit {commit!r}")
    sha = resolved.stdout.strip()
    evidence = {}
    for name in EVIDENCE:
        shown = git("show", f"{sha}:bench/zk-compose/{name}.json")
        if shown.returncode:
            raise SystemExit(f"cannot read {name}.json at baseline commit {sha}:\n{shown.stderr}")
        evidence[name] = json.loads(shown.stdout)
    return evidence


def unrewritten(name: str, data: dict) -> dict:
    """The part of an evidence file that `apply` copies from the baseline unchanged."""
    copy = json.loads(json.dumps(data))
    for key in REWRITTEN:
        copy.pop(key, None)
    for record in copy.get("members", {}).values():
        for field in FIELDS.get(name, ()):
            record.pop(field, None)
    return copy


def apply(args: argparse.Namespace) -> None:
    # The size gate reads the snapshot from the working tree, so it must match HEAD as well.
    paths = [f"bench/zk-compose/{name}.json" for name in EVIDENCE] + [SNAPSHOT]
    # The working tree and the index are checked separately: a staged edit that the working tree
    # reverts is invisible to a working-tree comparison alone.
    for label, command in (("unstaged", ["diff", "--quiet", "HEAD", "--", *paths]),
                           ("staged", ["diff", "--quiet", "--cached", "HEAD", "--", *paths])):
        try:
            clean = git(*command)
        except SystemExit as error:
            raise SystemExit(f"cannot check evidence cleanliness: {error}") from error
        if clean.returncode == 1:
            raise SystemExit(f"the evidence files or the gate snapshot have {label} edits against HEAD; "
                             "commit or discard them first")
        if clean.returncode:
            raise SystemExit(f"cannot check evidence cleanliness against HEAD; git diff failed:\n{clean.stderr}")
    head = git("rev-parse", "--verify", "--quiet", "HEAD^{commit}")
    measured_commit = git("rev-parse", "--verify", "--quiet", f"{args.commit}^{{commit}}")
    if head.returncode or measured_commit.returncode:
        raise SystemExit(f"cannot resolve HEAD and --commit {args.commit!r}")
    commit = measured_commit.stdout.strip()
    if head.stdout.strip() != commit:
        raise SystemExit(f"--commit {args.commit!r} is not HEAD; check out the measured commit first")
    first, second = (json.loads(path.read_text()) for path in (args.first, args.second))
    if without_artifacts(first) != without_artifacts(second):
        raise SystemExit("the two runs disagree")
    measured = first["members"]
    evidence = load_commit(args.baseline_commit)
    destination = load(ROOT)
    for name, data in evidence.items():
        if data.get("prepared_by") != destination[name].get("prepared_by"):
            raise SystemExit(f"{name}: baseline prepared_by differs from the destination file")
        # Every other field is copied from the baseline, so a difference here would silently
        # replace the destination's content.
        kept, current = unrewritten(name, data), unrewritten(name, destination[name])
        if kept != current:
            keys = sorted(key for key in set(kept) | set(current) if kept.get(key) != current.get(key))
            raise SystemExit(f"{name}: baseline differs from the destination outside the re-measured fields: "
                             f"{', '.join(keys)}")
    snapshot = json.loads((ROOT / SNAPSHOT).read_text())["benchmarks"]
    for member, record in evidence["result_v1_compatibility"]["members"].items():
        base, got = record["base"], measured[member]
        if any(got[field] != base[field] for field in ("acir_bytecode_sha256", "abi_sha256", "vk_sha256", "vk_bytes")):
            raise SystemExit(f"{member}: v1 differs from its baseline record")
    changed = set()
    for name, fields in FIELDS.items():
        for member, record in evidence[name]["members"].items():
            new = {field: measured[member][field] for field in fields}
            if new["circuit_size"] != snapshot[member]["circuit_size"]:
                raise SystemExit(f"{member}: measured size differs from gate_counts_latest.json")
            if any(record[field] != new[field] for field in fields):
                changed.add(member)
            record.update(new)
    old = evidence["result_v1_compatibility"]["source_files"]
    new = first["source_files"]
    moved = {path for path in set(old) | set(new) if old.get(path) != new.get(path)}
    if changed != set(args.expect_member) or moved != set(args.expect_source):
        raise SystemExit(f"unexpected change: members {sorted(changed)}, sources {sorted(moved)}")
    tree_lookup = git("rev-parse", "--verify", "--quiet", f"{commit}^{{tree}}")
    if tree_lookup.returncode:
        raise SystemExit(f"cannot resolve the tree of {commit}")
    provenance = {
        "source_commit": commit,
        "source_tree": tree_lookup.stdout.strip(),
        "observed_at_utc": args.observed_at,
        "platform": args.platform,
        "canonical": False,
        "tool_executable_sha256": {"nargo": first["tools"]["nargo_sha256"], "bb": first["tools"]["bb_sha256"]},
        "job_sha256": digest(args.job.read_bytes()),
        "independent_handoff_sha256": digest(args.second.read_bytes()),
    }
    method = ("At measured_source, two independent runs from clean git archives of source_commit compiled every "
              "listed member with the pinned nargo, hashed the ACIR bytecode, ABI and artifact, measured "
              "`bb gates -s ultra_honk` and, for the v1 members, regenerated the noir-recursive verification key "
              "(bench/zk-compose/scripts/remeasure_result_evidence.py). The two runs agreed on every circuit "
              "size, ACIR, ABI, key and gate log hash; every v1 ACIR, ABI and key equalled its baseline record. "
              "The compiled artifact embeds its checkout path, so artifact_sha256 is kept as recorded. The "
              "two-run agreement was checked outside the repository and cannot be reproduced from it alone: "
              "job_sha256 and independent_handoff_sha256 identify the job manifest and the second run's output, "
              "which are retained outside the repository, and that output includes path-dependent artifact "
              "hashes, so a rerun will not reproduce its digest.")
    limitations = ("Non-canonical: measured on a shared work box, not a dedicated machine; future CI replays are "
                   "still required. Static compile, circuit size and key identity only; no timing, soundness or "
                   "external-audit claim. source_files, not source_commit, binds the Noir bytes.")
    tools = first["tools"]
    for name, data in evidence.items():
        recorded = data.get("tool_versions") or {"nargo": data["nargo_version"], "bb": data["bb_version"]}
        if recorded["nargo"] != tools["nargo"] or recorded["bb"] != tools["bb"]:
            raise SystemExit(f"{name}: toolchain differs from the recorded one")
    # Every check has passed. Stage all four files, then replace them one by one. Separate
    # renames are not one atomic update, so if a replacement fails the files already replaced
    # are restored from their original bytes, and staged temporaries are always removed.
    originals, staged, replaced = {}, [], []
    try:
        for name, data in evidence.items():
            data["source_files"] = new
            data["measured_source"] = provenance
            data["measurement_method"] = method
            data["limitations"] = limitations
            target = ROOT / "bench/zk-compose" / f"{name}.json"
            originals[target] = target.read_bytes()
            handle, temporary = tempfile.mkstemp(dir=target.parent, prefix=f".{name}.", suffix=".json")
            staged.append((Path(temporary), target))
            with os.fdopen(handle, "w") as output:
                output.write(json.dumps(data, indent=2) + "\n")
        for temporary, target in staged:
            os.replace(temporary, target)
            replaced.append(target)
    except BaseException as error:
        failed_restores = []
        for target in replaced:
            try:
                target.write_bytes(originals[target])
            except BaseException as restore_error:
                failed_restores.append((target, restore_error))
        if failed_restores:
            recovery = "\n".join(
                f"  {target.relative_to(ROOT)}: {restore_error}\n"
                f"    git checkout -- {target.relative_to(ROOT)}"
                for target, restore_error in failed_restores
            )
            raise SystemExit(f"evidence replacement failed: {error}\n"
                             f"rollback failed; recover these files from the repository root:\n{recovery}") from error
        raise
    finally:
        for temporary, _ in staged:
            temporary.unlink(missing_ok=True)
    print("changed members:", ", ".join(sorted(changed)))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    step = commands.add_parser("measure")
    step.add_argument("source", type=Path, help="clean archive of the commit to measure")
    step.add_argument("output", type=Path)
    step = commands.add_parser("apply")
    step.add_argument("first", type=Path)
    step.add_argument("second", type=Path, help="independent run; its hash is the handoff")
    step.add_argument("--baseline-commit", default="HEAD",
                      help="commit whose bench/zk-compose evidence is the pre-change baseline (default: HEAD)")
    step.add_argument("--job", type=Path, required=True, help="job manifest; its hash is job_sha256")
    step.add_argument("--commit", required=True)
    step.add_argument("--observed-at", required=True)
    step.add_argument("--platform", required=True)
    step.add_argument("--expect-member", action="append", default=[])
    step.add_argument("--expect-source", action="append", default=[])
    args = parser.parse_args()
    if args.command == "measure":
        measure(args.source, args.output)
    else:
        apply(args)


if __name__ == "__main__":
    main()
