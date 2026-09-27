#!/usr/bin/env python3
# [FABLE-5] Auto-file a nightly metamorphic-lane finding (bead sq-3dyje.9).
#
# WHAT: the deterministic filing core of .github/workflows/metamorph.yml's failure
# path — the metamorphic sibling of scripts/ci-file-differential-failure.py (same
# structure, same safety properties, adapted to the single-engine TLP/NoREC
# oracles of crates/sparq-metamorph). When a metamorph-driver window reports a
# non-Pass verdict, this script:
#   1. PARSES the captured driver log — the `VIOLATION seed=N` /
#      `ENGINE-FAILURE seed=N` lines plus the "FIRST FAILING CASE:" block (seed +
#      replay command + all oracle queries + graph), a complete deterministic
#      repro (the generator is seeded SplitMix64 — the seed IS the corpus);
#   2. WRITES the parsed repro to --out-dir (uploaded as a CI artifact);
#   3. FILES a GitHub issue with the repro INLINE (via `gh`), deduped per shard:
#      if an open `[metamorph]` issue for the same shard already exists it
#      comments the fresh window/run instead of opening a duplicate;
#   4. APPENDS a P1 bug bead to .beads/issues.jsonl (append-only, deduped against
#      existing open metamorph beads for the shard). The workflow commits + pushes
#      this BEST-EFFORT (fail-soft on the GH013 protected-main ruleset rejection);
#      the GitHub issue is the reliable channel the orchestrator's issue sweep
#      beads from.
#
# SAFETY / HONESTY PROPERTIES (same as the differential filing script)
#   * Never edits or reorders existing JSONL lines (append-only).
#   * Bead ids are derived (sq-mm + hash of date+shard) and collision-checked.
#   * Idempotent per shard: a re-run neither opens a second issue nor appends a
#     second bead.
#   * `gh` failures degrade to warnings — filing must never mask the red lane.
#
# USAGE
#   scripts/ci-file-metamorph-failure.py --log metamorph-log.txt --shard nightly \
#       --seed-start N --count M --out-dir repro/ --jsonl .beads/issues.jsonl \
#       --run-url URL
#   scripts/ci-file-metamorph-failure.py --self-test   # hermetic; no gh, no writes
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gh_dedupe  # noqa: E402  (sibling module; the sys.path insert above is the seam)

PROG = "ci-file-metamorph-failure"
MARKER = "[metamorph]"
# The label the filer stamps on every issue it opens, and the one its own dedupe lookup
# reads back (#5804).
LABEL = "metamorph"

_FAIL_RE = re.compile(r"^(?:VIOLATION|ENGINE-FAILURE) seed=(\d+) ", re.MULTILINE)
_ALPHABET = "abcdefghijklmnopqrstuvwxyz0123456789"


def log(msg: str) -> None:
    print(f"[{PROG}] {msg}", file=sys.stderr)


def parse_driver_log(text: str) -> dict:
    """Extract the failing-seed list, the FIRST FAILING CASE block, the summary."""
    # De-duplicate while preserving order (a seed may fail both oracles).
    seeds: list[int] = []
    for m in _FAIL_RE.findall(text):
        s = int(m)
        if s not in seeds:
            seeds.append(s)
    first_case = ""
    marker = "FIRST FAILING CASE:"
    if marker in text:
        first_case = text.split(marker, 1)[1].strip()
    summary = ""
    for line in text.splitlines():
        if line.startswith("metamorph seeds"):
            summary = line.strip()
    return {"seeds": seeds, "first_case": first_case, "summary": summary}


def derive_bead_id(shard: str, date: str, existing_ids: set, n: int = 5) -> str:
    """Deterministic, collision-checked bead id: sq-mm + hash(date+shard)."""
    digest = hashlib.sha256(f"metamorph:{date}:{shard}".encode()).hexdigest()
    chars = "".join(_ALPHABET[int(c, 16) % len(_ALPHABET)] for c in digest)
    while True:
        bead_id = f"sq-mm{chars[:n]}"
        if bead_id not in existing_ids:
            return bead_id
        n += 1


def existing_open_metamorph_bead(jsonl_path: Path, shard: str) -> str | None:
    """The id of an existing open metamorph bead for this shard, if any (dedupe:
    one standing bug bead per shard until it is closed)."""
    if not jsonl_path.exists():
        return None
    with open(jsonl_path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            if (
                rec.get("_type") == "issue"
                and rec.get("status") in ("open", "in_progress", "blocked")
                and MARKER in rec.get("title", "")
                and f"shard={shard}" in rec.get("title", "")
            ):
                return rec.get("id")
    return None


def all_bead_ids(jsonl_path: Path) -> set:
    ids = set()
    if not jsonl_path.exists():
        return ids
    with open(jsonl_path, encoding="utf-8") as fh:
        for line in fh:
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(rec, dict) and rec.get("id"):
                ids.add(rec["id"])
    return ids


def replay_command(parsed: dict, args) -> str:
    first = parsed["seeds"][0] if parsed["seeds"] else None
    if first is not None:
        return f"cargo run -p sparq-metamorph --bin metamorph-driver -- {first} 1"
    return (
        f"cargo run -p sparq-metamorph --bin metamorph-driver -- "
        f"{args.seed_start} {args.count}"
    )


def build_bead_record(bead_id: str, shard: str, parsed: dict, args, now: str) -> dict:
    """A minimal, schema-shaped bead line (mirrors .beads/issues.jsonl records)."""
    n = len(parsed["seeds"])
    first = parsed["seeds"][0] if parsed["seeds"] else "?"
    title = (
        f"[BUG] {MARKER} shard={shard}: {n} TLP/NoREC oracle failure(s), "
        f"first seed={first} (window {args.seed_start}+{args.count})"
    )
    description = (
        f"Auto-filed by the nightly metamorphic lane (sq-3dyje.9). Run: {args.run_url}\n"
        f"Failing seeds: {parsed['seeds'][:50]}{' …' if n > 50 else ''}\n"
        f"Summary: {parsed['summary']}\n"
        f"Replay: {replay_command(parsed, args)}\n"
        f"Repro (seed + queries + graph) is inline in the linked GitHub issue and in "
        f"the metamorph-repro artifact of the run. TLP/NoREC are SINGLE-ENGINE "
        f"metamorphic oracles: a VIOLATION is an internal-consistency wrong-result "
        f"signal in sparq itself (no cross-engine adjudication applies); an "
        f"ENGINE-FAILURE is a generated valid query that failed to evaluate. "
        f"🤖 SPARQ agent [FABLE-5]"
    )
    return {
        "_type": "issue",
        "id": bead_id,
        "title": title,
        "description": description,
        "status": "open",
        "priority": 1,
        "issue_type": "bug",
        "created_at": now,
        "created_by": "metamorph CI",
        "updated_at": now,
        "labels": ["metamorph", "ci"],
        "dependency_count": 0,
        "dependent_count": 0,
        "comment_count": 0,
    }


def append_bead(jsonl_path: Path, record: dict) -> None:
    """Append-only, newline-safe: never touches existing lines."""
    with open(jsonl_path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(record, ensure_ascii=False) + "\n")


def build_issue_body(bead_id: str, shard: str, parsed: dict, args) -> str:
    n = len(parsed["seeds"])
    seeds_line = ", ".join(str(s) for s in parsed["seeds"][:50]) + (" …" if n > 50 else "")
    case = parsed["first_case"] or "(no FIRST FAILING CASE block captured — see the artifact log)"
    return f"""> 🤖 **SPARQ agent** — auto-filed by the nightly metamorphic lane (bead sq-3dyje.9). [FABLE-5]

The nightly TLP/NoREC metamorphic driver found **{n} failing seed(s)** in shard `{shard}` (seed window {args.seed_start}+{args.count}).

- **Failing seeds:** {seeds_line}
- **Deterministic replay:** `{replay_command(parsed, args)}`
- **Run:** {args.run_url}
- **Bead:** `{bead_id}` (P1, auto-appended; push best-effort)
- **Artifact:** `metamorph-repro-{shard}` on the run above (full log)

TLP/NoREC are **single-engine** metamorphic oracles (SQLancer's TLP/NoREC re-derived for SPARQL's three-valued EBV semantics): a `VIOLATION` means sparq's own evaluation paths are internally inconsistent — a wrong-result logic-bug signal that needs no cross-engine adjudication. An `ENGINE-FAILURE` means a generated, precondition-respecting query failed to evaluate (an engine-error bug or a harness bug). Both fail the lane; neither is ever skipped.

### First failing case (seed + replay + queries + graph)

```
{case}
```
"""


def gh(*argv: str) -> str:
    """Run gh, returning stdout; raises CalledProcessError on failure."""
    return subprocess.run(
        ["gh", *argv], check=True, capture_output=True, text=True
    ).stdout.strip()


def find_open_issue(shard: str) -> str | None:
    """Number of an existing open metamorph issue for this shard, if any.

    [OPUS-5] #5804: LIST BY LABEL and exact-substring-match the title locally
    (scripts/gh_dedupe.py) rather than trusting `gh --search`. A title of the shape
    `[metamorph] shard=nightly` (metamorph.yml's shards are `smoke` and `nightly`)
    carries exactly the punctuation gh's search tokeniser handles unreliably, and the
    search index also lags — either would MISS an issue this filer already opened and
    mint a duplicate.
    """
    res = gh_dedupe.find_open_issue(
        LABEL,
        (MARKER, f"shard={shard}"),
        legacy_search=f'in:title "{MARKER} shard={shard}"',
        log=log,
        backfill_label=True,
    )
    if res.issue is None and not res.probed:
        log("warning: issue dedupe lookup failed — will attempt creation")
    return res.number


def file_github_issue(bead_id: str, shard: str, parsed: dict, args) -> None:
    body = build_issue_body(bead_id, shard, parsed, args)
    n = len(parsed["seeds"])
    first = parsed["seeds"][0] if parsed["seeds"] else "?"
    title = (
        f"{MARKER} shard={shard}: {n} TLP/NoREC oracle failure(s) "
        f"(first seed={first})"
    )
    # The label is load-bearing for dedupe (find_open_issue reads it back), so upsert
    # it BEFORE the lookup — an unlabelled issue is one the next tick cannot see.
    gh_dedupe.ensure_label(
        LABEL, color="b60205",
        description="TLP/NoREC metamorphic oracle failure (internal inconsistency)",
        log=log,
    )
    existing = find_open_issue(shard)
    with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as tf:
        tf.write(body)
        body_file = tf.name
    try:
        if existing:
            gh("issue", "comment", existing, "--body-file", body_file)
            log(f"commented the fresh window on existing open issue #{existing} (deduped)")
        else:
            url = gh("issue", "create", "--title", title, "--body-file", body_file,
                     "--label", LABEL)
            log(f"filed GitHub issue: {url}")
    except subprocess.CalledProcessError as e:
        log(f"warning: gh issue filing failed (exit {e.returncode}): {e.stderr.strip() if e.stderr else e}")
        log("the lane is already red and the artifact carries the repro — not fatal.")
    finally:
        os.unlink(body_file)


def write_repro_artifact(out_dir: Path, parsed: dict, log_text: str, args) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "repro.md").write_text(
        f"# metamorph repro — shard={args.shard}\n\n"
        f"window: seeds {args.seed_start}+{args.count}\n"
        f"failing seeds: {parsed['seeds']}\n"
        f"summary: {parsed['summary']}\n\n"
        f"## first failing case\n\n```\n{parsed['first_case']}\n```\n",
        encoding="utf-8",
    )
    (out_dir / "metamorph-log.txt").write_text(log_text, encoding="utf-8")


# ── self-test (hermetic: no gh, no repo writes) ──────────────────────────────────
def self_test() -> int:
    sample = (
        "VIOLATION seed=42 oracle=tlp: tlp violation on [sparq]: partition union "
        "differs from base (base=9 true=3 false=3 error=2)\n"
        "VIOLATION seed=42 oracle=norec: norec violation on [sparq]: optimized "
        "cardinality differs from rewrite true-count (optimized=3 rewrite-rows=9 rewrite-true=4)\n"
        "ENGINE-FAILURE seed=57 oracle=tlp: engine failure [Evaluation] on sparq: parse error\n"
        "\nFIRST FAILING CASE:\nseed=42\n"
        "replay: cargo run -p sparq-metamorph --bin metamorph-driver -- 42 1\n"
        "--- queries ---\nSELECT * WHERE { ?s <http://example.org/v> ?v }\n"
        "--- graph ---\n<http://example.org/s0> <http://example.org/v> \"1\"^^<x> .\n"
        "metamorph seeds 0..2000: checked=2000 pass=1998 tlp-violation=1 "
        "norec-violation=1 engine-failure=1\n"
    )
    parsed = parse_driver_log(sample)
    assert parsed["seeds"] == [42, 57], parsed["seeds"]  # deduped, order-preserving
    assert "seed=42" in parsed["first_case"]
    assert "--- graph ---" in parsed["first_case"]
    assert parsed["summary"].startswith("metamorph seeds")
    # No-failure log parses to empty (a red lane with no VIOLATION lines — e.g. a
    # panic — still files, with the raw log as the payload).
    empty = parse_driver_log("metamorph seeds 0..10: checked=10 pass=10 ...")
    assert empty["seeds"] == [] and empty["first_case"] == ""

    with tempfile.TemporaryDirectory() as td:
        jsonl = Path(td) / "issues.jsonl"
        jsonl.write_text(
            json.dumps({"_type": "issue", "id": "sq-aaaaa", "title": "x", "status": "open"}) + "\n",
            encoding="utf-8",
        )
        ids = all_bead_ids(jsonl)
        assert ids == {"sq-aaaaa"}
        # Deterministic id derivation + collision extension.
        b1 = derive_bead_id("nightly", "2026-07-11", ids)
        b2 = derive_bead_id("nightly", "2026-07-11", ids)
        assert b1 == b2 and b1.startswith("sq-mm") and len(b1) == len("sq-mm") + 5
        b3 = derive_bead_id("nightly", "2026-07-11", ids | {b1})
        assert b3 != b1 and b3.startswith("sq-mm")

        class A:  # minimal args stand-in
            shard, seed_start, count = "nightly", "42", "2000"
            run_url = "https://example.invalid/run/1"

        assert replay_command(parsed, A).endswith("metamorph-driver -- 42 1")
        assert replay_command(empty, A).endswith("metamorph-driver -- 42 2000")
        rec = build_bead_record(b1, "nightly", parsed, A, "2026-07-11T00:00:00Z")
        assert rec["priority"] == 1 and rec["issue_type"] == "bug" and rec["status"] == "open"
        assert "sq-3dyje.9" in rec["description"] and "🤖" in rec["description"]
        append_bead(jsonl, rec)
        # Existing lines untouched; the new line parses; dedupe now finds it.
        lines = jsonl.read_text(encoding="utf-8").splitlines()
        assert len(lines) == 2 and json.loads(lines[0])["id"] == "sq-aaaaa"
        assert json.loads(lines[1])["id"] == b1
        assert existing_open_metamorph_bead(jsonl, "nightly") == b1
        assert existing_open_metamorph_bead(jsonl, "smoke") is None
        # Issue body carries the inline repro + the self-id.
        body = build_issue_body(b1, "nightly", parsed, A)
        assert "🤖" in body and "SPARQ agent" in body
        assert "--- graph ---" in body and "42" in body
        assert "single-engine" in body  # honest oracle framing, not "vs Oxigraph"
        # Artifact writer round-trip.
        out = Path(td) / "repro"
        write_repro_artifact(out, parsed, sample, argparse.Namespace(
            shard="nightly", seed_start="42", count="2000"))
        assert (out / "repro.md").exists() and (out / "metamorph-log.txt").exists()
    log("self-test OK")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(prog=PROG)
    ap.add_argument("--log", help="captured driver stdout+stderr")
    ap.add_argument("--shard", help="workflow shard name (smoke | nightly)")
    ap.add_argument("--seed-start", default="unknown")
    ap.add_argument("--count", default="unknown")
    ap.add_argument("--out-dir", help="repro artifact directory")
    ap.add_argument("--jsonl", default=".beads/issues.jsonl")
    ap.add_argument("--run-url", default="")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()
    if not args.log or not args.shard or not args.out_dir:
        ap.error("--log, --shard and --out-dir are required (or use --self-test)")

    log_text = Path(args.log).read_text(encoding="utf-8", errors="replace")
    parsed = parse_driver_log(log_text)
    log(f"parsed {len(parsed['seeds'])} failing seed(s) for shard {args.shard}")

    write_repro_artifact(Path(args.out_dir), parsed, log_text, args)

    jsonl = Path(args.jsonl)
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    existing = existing_open_metamorph_bead(jsonl, args.shard)
    if existing:
        log(f"open metamorph bead {existing} already tracks shard {args.shard} — no new bead")
        bead_id = existing
    else:
        bead_id = derive_bead_id(args.shard, now[:10], all_bead_ids(jsonl))
        append_bead(jsonl, build_bead_record(bead_id, args.shard, parsed, args, now))
        log(f"appended P1 bug bead {bead_id} to {jsonl}")

    file_github_issue(bead_id, args.shard, parsed, args)
    return 0


if __name__ == "__main__":
    sys.exit(main())
