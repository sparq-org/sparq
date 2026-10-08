#!/usr/bin/env python3
# sq-toze.5 (gap GX-4): GATE the machine-readable OpenSSF Best-Practices (CII)
# badge self-certification.
#
# WHY: compliance/openssf/best-practices-self-cert.json is the import-ready, structured form
# of the drafted badge answers (one object per bestpractices.dev criterion: status +
# justification + repo-relative evidence path(s)). The drafted prose in
# compliance/openssf/evidence.md §Badge is human-readable but cannot be machine-checked; the
# JSON can. The risk a self-certification carries is SILENT DRIFT — an answer keeps citing a
# file that was renamed/deleted, or a future edit introduces an invalid status/level token, so
# the "evidence-grounded" claim quietly rots. This checker is the backstop that keeps the
# self-cert honest at PR time:
#   * every cited `evidence` path resolves to a real artifact in the repo;
#   * every `status` is one of the four legal tokens; every `family` is a known badge family;
#   * the JSON parses and carries the expected shape (schema tag + non-empty criteria);
#   * criterion ids are unique (no duplicate-answer ambiguity);
#   * if `project.filed` is true, a non-empty `badge_url` MUST be present (so the file can
#     never claim the external GX-4 filing happened without recording the resulting badge).
# It does NOT (and cannot) verify the SEMANTIC truth of a justification — that is the auditor's
# job and is anchored in evidence.md. It verifies the answers stay STRUCTURALLY grounded.
#
# GX-4 honesty: filing the questionnaire on bestpractices.dev is an EXTERNAL human-owned step.
# This file + this gate are the in-repo deliverable; they do not claim the badge is filed
# (project.filed is false until a maintainer files it and records the badge_url).
#
# Exit codes: 0 = self-cert is structurally valid and every evidence path resolves;
#             1 = a finding (missing evidence path / bad token / shape error / filed-without-url);
#             2 = usage / parse error.
#
# Run:  python3 scripts/check-bestpractices-evidence.py
#       python3 scripts/check-bestpractices-evidence.py path/to/self-cert.json
# (stdlib only — json/pathlib on the runner's Python 3.12.)

from __future__ import annotations

import json
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SELF_CERT = REPO_ROOT / "compliance" / "openssf" / "best-practices-self-cert.json"

VALID_STATUS = {"Met", "Met w/justification", "Unmet", "N/A"}
VALID_FAMILY = {"basics", "change_control", "reporting", "quality", "security", "analysis"}
EXPECTED_SCHEMA = "sparq-openssf-bestpractices-selfcert/v1"


class CheckError(Exception):
    """Parse / usage failure (distinct from a validation finding)."""


def evaluate(doc: dict, repo_root: Path) -> tuple[bool, list[str]]:
    """Pure validation of a parsed self-cert document against `repo_root`.

    Returns (ok, lines). `ok` is True iff there are no findings.
    """
    findings: list[str] = []
    notes: list[str] = []

    schema = doc.get("schema")
    if schema != EXPECTED_SCHEMA:
        findings.append(f"schema: expected {EXPECTED_SCHEMA!r}, got {schema!r}")

    project = doc.get("project")
    if not isinstance(project, dict):
        findings.append("project: missing or not an object")
        project = {}

    # filed=true must record the resulting badge so the external GX-4 step can never be
    # silently claimed-as-done without evidence of the actual badge entry.
    if project.get("filed") is True:
        if not project.get("badge_url"):
            findings.append(
                "project.filed is true but project.badge_url is missing/empty — "
                "a filed badge MUST record its bestpractices.dev URL"
            )
    else:
        notes.append("project.filed is false — GX-4 external filing not yet done (expected; honest)")

    criteria = doc.get("criteria")
    if not isinstance(criteria, list) or not criteria:
        findings.append("criteria: missing or empty list")
        criteria = []

    seen_ids: set[str] = set()
    distinct_paths: set[str] = set()
    for i, c in enumerate(criteria):
        where = f"criteria[{i}]"
        if not isinstance(c, dict):
            findings.append(f"{where}: not an object")
            continue
        cid = c.get("id")
        if not cid or not isinstance(cid, str):
            findings.append(f"{where}: missing/blank id")
            cid = f"<criteria[{i}]>"
        else:
            where = cid
            if cid in seen_ids:
                findings.append(f"{cid}: duplicate criterion id")
            seen_ids.add(cid)

        if c.get("status") not in VALID_STATUS:
            findings.append(f"{where}: invalid status {c.get('status')!r} (must be one of {sorted(VALID_STATUS)})")
        if c.get("family") not in VALID_FAMILY:
            findings.append(f"{where}: invalid family {c.get('family')!r} (must be one of {sorted(VALID_FAMILY)})")
        if not (c.get("justification") or "").strip():
            findings.append(f"{where}: missing/blank justification")

        evidence = c.get("evidence")
        if not isinstance(evidence, list) or not evidence:
            findings.append(f"{where}: missing/empty evidence list")
            continue
        for e in evidence:
            if not isinstance(e, str) or not e.strip():
                findings.append(f"{where}: non-string/blank evidence entry {e!r}")
                continue
            distinct_paths.add(e)
            if not (repo_root / e).exists():
                findings.append(f"{where}: evidence path does not resolve in repo: {e}")

    ok = not findings
    summary: list[str] = []
    summary.append(
        f"{'OK' if ok else 'FAIL'}: {len(criteria)} criteria, "
        f"{len(distinct_paths)} distinct evidence paths, {len(findings)} finding(s)"
    )
    summary.extend(notes)
    summary.extend(findings)
    return ok, summary


def load(path: Path) -> dict:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise CheckError(f"self-cert file not found: {path}") from exc
    except json.JSONDecodeError as exc:
        raise CheckError(f"self-cert file is not valid JSON ({path}): {exc}") from exc


def main(argv: list[str]) -> int:
    args = argv[1:]
    if args and args[0] in {"-h", "--help"}:
        print(__doc__)
        return 0
    target = Path(args[0]) if args else DEFAULT_SELF_CERT
    try:
        doc = load(target)
    except CheckError as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 2
    ok, lines = evaluate(doc, REPO_ROOT)
    for line in lines:
        print(line)
    if not ok:
        print("::error::OpenSSF Best-Practices self-cert validation FAILED", file=sys.stderr)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
