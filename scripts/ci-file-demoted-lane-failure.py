#!/usr/bin/env python3
# [FABLE-5] Auto-file a DEMOTED-LANE full-form failure (bead sq-6vshe.6).
#
# CONTEXT — the demotion auto-bead protocol (research/ci-structural-speedup.md §7).
# The heavy-lane placement change (sq-6vshe.6) demotes the per-PR variant of certain
# heavy CI lanes to a cheap deterministic slice and RELOCATES the expensive full form
# to a push-to-main / nightly / merge-queue run:
#   * fuzz     : per-PR = deterministic corpus-replay (`-runs=0`); full form =
#                randomized cargo-fuzz on push-to-main + nightly.
#   * (future) : any other lane demoted off per-PR under this program.
# The load-bearing safety property is that a lane demoted off per-PR CANNOT SILENTLY
# ROT: if its FULL form fails wherever it was relocated to, that failure must file a
# standing P1 bead + a GitHub issue naming the lane + run, so a finding the cheap
# per-PR slice could not have caught is still tracked to closure. This is the exact
# mirror of the fmx4u §6.1 nightly-failure protocol and the differential.yml nightly
# filer (scripts/ci-file-differential-failure.py) — this script is the GENERIC form of
# the same create-a-bead-on-a-relocated-failure plumbing, parameterised by lane.
#
# WHAT (on a full-form failure):
#   1. FILES a GitHub issue (via `gh`) naming the lane, its full + per-PR forms, and the
#      run URL, with a tail of the captured log inline — deduped per lane (an existing
#      open `[demoted-lane]` issue for the lane is commented, not re-opened).
#   2. APPENDS a P1 bug bead to --jsonl (.beads/issues.jsonl) — append-only, never a
#      rewrite of existing lines, deduped against existing open beads for the lane. The
#      workflow commits + pushes this BEST-EFFORT (fail-soft on the GH013 protected-main
#      ruleset rejection); the GitHub issue is the reliable channel.
#
# SAFETY / HONESTY PROPERTIES (identical contract to the differential filer):
#   * Never edits or reorders existing JSONL lines (append-only).
#   * Bead ids are derived (sq-<prefix> + hash of date+lane) and collision-checked
#     against every id already in the JSONL; a collision extends the hash.
#   * Idempotent per (lane, date): re-running the same failure neither opens a second
#     issue nor appends a second bead.
#   * `gh` failures degrade to warnings — filing must never mask the red lane (the
#     workflow step runs under `if: failure()`; the lane is already failing).
#
# USAGE
#   scripts/ci-file-demoted-lane-failure.py --lane fuzz-randomized \
#       --full-form "randomized cargo-fuzz (schedule)" \
#       --per-pr-form "deterministic corpus-replay (-runs=0)" \
#       --bead-prefix fz --log run-log.txt --jsonl .beads/issues.jsonl --run-url URL
#   scripts/ci-file-demoted-lane-failure.py --self-test   # hermetic; no gh, no writes
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

PROG = "ci-file-demoted-lane-failure"
MARKER = "[demoted-lane]"
_ALPHABET = "abcdefghijklmnopqrstuvwxyz0123456789"
# How much of the captured log to inline in the issue (the tail is where the crash /
# reproducer lines are).
_LOG_TAIL_CHARS = 4000
# [OPUS-5] sq-c9q4r: any run of 3+ backticks in the UNTRUSTED log tail. The tail is
# inlined inside a ``` fence in build_issue_body(), so such a run CLOSES the fence
# early and everything after it renders as markdown in an own-repo issue.
_FENCE_RUN_RE = re.compile(r"`{3,}")


def log(msg: str) -> None:
    print(f"[{PROG}] {msg}", file=sys.stderr)


def _sanitize_lane(lane: str) -> str:
    """Restrict the lane token to a safe title/search slug (letters/digits/-/_)."""
    return re.sub(r"[^A-Za-z0-9._-]", "-", lane).strip("-") or "lane"


def derive_bead_id(prefix: str, lane: str, date: str, existing_ids: set, n: int = 5) -> str:
    """Deterministic, collision-checked bead id: sq-<prefix> + hash(date+lane).
    Deterministic so a re-run of the same failure derives the SAME id (the dedupe
    then no-ops); collision-extension keeps it unique."""
    prefix = re.sub(r"[^a-z0-9]", "", prefix.lower())[:4] or "dl"
    digest = hashlib.sha256(f"demoted-lane:{date}:{lane}".encode()).hexdigest()
    chars = "".join(_ALPHABET[int(c, 16) % len(_ALPHABET)] for c in digest)
    while True:
        bead_id = f"sq-{prefix}{chars[:n]}"
        if bead_id not in existing_ids:
            return bead_id
        n += 1


def _open_issue_bead_for_lane(jsonl_path: Path, lane: str) -> str | None:
    """Id of an existing open/in_progress/blocked demoted-lane bead for this lane."""
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
                and f"lane={lane}" in rec.get("title", "")
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


def build_bead_record(bead_id: str, lane: str, args, now: str) -> dict:
    title = f"[BUG] {MARKER} lane={lane}: full-form CI run failed (demotion safety-net)"
    description = (
        f"Auto-filed by the sq-6vshe.6 demotion auto-bead protocol. "
        f"The per-PR variant of this lane is the cheap deterministic slice "
        f"({args.per_pr_form}); its FULL form ({args.full_form}) FAILED. "
        f"This is a finding the per-PR slice could not have caught, so it is tracked "
        f"here to closure so the demoted lane cannot silently rot.\n"
        f"Run: {args.run_url}\n"
        f"Reproduce locally with the lane's full form (see the workflow that filed "
        f"this) and the run log inline in the linked GitHub issue. "
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
        "created_by": "demoted-lane CI",
        "updated_at": now,
        "labels": ["demoted-lane", "ci"],
        "dependency_count": 0,
        "dependent_count": 0,
        "comment_count": 0,
    }


def append_bead(jsonl_path: Path, record: dict) -> None:
    """Append-only, newline-safe: never touches existing lines."""
    with open(jsonl_path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(record, ensure_ascii=False) + "\n")


def build_issue_body(bead_id: str, lane: str, args, log_tail: str) -> str:
    # [OPUS-5] sq-c9q4r: defuse AT the fence as well as in read_log_tail(), so the
    # "untrusted text cannot escape this block" invariant holds for every caller
    # rather than only the one that read the file. Idempotent (the rewrite emits no
    # backticks), so a tail that already went through read_log_tail() is unchanged.
    tail = defuse_code_fences(log_tail) or "(no log captured — see the run's job log + any uploaded artifact)"
    return f"""> 🤖 **SPARQ agent** — auto-filed by the demoted-lane safety net (bead sq-6vshe.6). [FABLE-5]

The **full form** of a CI lane that was demoted off the per-PR critical path **failed**. The per-PR variant runs only the cheap deterministic slice, so this is a finding the per-PR run could not have caught — tracked here so the demoted lane cannot silently rot.

- **Lane:** `{lane}`
- **Full form (failed):** {args.full_form}
- **Per-PR form (unaffected):** {args.per_pr_form}
- **Run:** {args.run_url}
- **Bead:** `{bead_id}` (P1, auto-appended; push best-effort)

### Captured log (tail)

```
{tail}
```
"""


def defuse_code_fences(text: str) -> str:
    """Neutralise ``` runs so untrusted log text cannot escape the markdown code fence
    it is inlined into.

    [OPUS-5] sq-c9q4r. The log tail is arbitrary bytes from a fuzz target's stdout —
    including the failing INPUT libFuzzer echoes back — so a log containing a run of
    three-or-more backticks closes build_issue_body()'s fence early and the remainder
    renders as markdown in an issue this repo's own CI opens (own-repo markdown
    injection; the same shape as the differential filer's inline repro block).
    Each such run is rewritten to the same number of apostrophes: ASCII, no invisible
    characters, visually near-identical, and impossible to read as a fence. A 3+
    backtick run carries no diagnostic meaning in a crash log, so nothing is lost.
    """
    return _FENCE_RUN_RE.sub(lambda m: "'" * len(m.group(0)), text)


def read_log_tail(path: str | None) -> str:
    """Tail of the captured run log, fence-defused (it is inlined into a ``` block)."""
    if not path:
        return ""
    p = Path(path)
    if not p.exists():
        return ""
    text = p.read_text(encoding="utf-8", errors="replace")
    if len(text) > _LOG_TAIL_CHARS:
        text = "…(truncated)…\n" + text[-_LOG_TAIL_CHARS:]
    return defuse_code_fences(text)


def gh(*argv: str) -> str:
    return subprocess.run(["gh", *argv], check=True, capture_output=True, text=True).stdout.strip()


def find_open_issue(lane: str) -> str | None:
    try:
        out = gh(
            "issue", "list", "--state", "open",
            "--search", f'in:title "{MARKER} lane={lane}"',
            "--json", "number,title", "--limit", "10",
        )
        for item in json.loads(out or "[]"):
            if MARKER in item.get("title", "") and f"lane={lane}" in item.get("title", ""):
                return str(item["number"])
    except (subprocess.CalledProcessError, json.JSONDecodeError) as e:
        log(f"warning: issue dedupe search failed ({e}) — will attempt creation")
    return None


def file_github_issue(bead_id: str, lane: str, args, log_tail: str) -> None:
    body = build_issue_body(bead_id, lane, args, log_tail)
    title = f"{MARKER} lane={lane}: full-form CI run failed"
    existing = find_open_issue(lane)
    with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as tf:
        tf.write(body)
        body_file = tf.name
    try:
        if existing:
            gh("issue", "comment", existing, "--body-file", body_file)
            log(f"commented the fresh failure on existing open issue #{existing} (deduped)")
        else:
            url = gh("issue", "create", "--title", title, "--body-file", body_file)
            log(f"filed GitHub issue: {url}")
    except subprocess.CalledProcessError as e:
        log(f"warning: gh issue filing failed (exit {e.returncode}): {e.stderr.strip() if e.stderr else e}")
        log("the lane is already red — not fatal.")
    finally:
        os.unlink(body_file)


# ── self-test (hermetic: no gh, no repo writes) ──────────────────────────────────
def self_test() -> int:
    class A:  # minimal args stand-in
        full_form = "randomized cargo-fuzz (schedule)"
        per_pr_form = "deterministic corpus-replay (-runs=0)"
        run_url = "https://example.invalid/run/1"

    assert _sanitize_lane("fuzz/randomized!") == "fuzz-randomized"
    assert _sanitize_lane("") == "lane"

    with tempfile.TemporaryDirectory() as td:
        jsonl = Path(td) / "issues.jsonl"
        jsonl.write_text(
            json.dumps({"_type": "issue", "id": "sq-aaaaa", "title": "x", "status": "open"}) + "\n",
            encoding="utf-8",
        )
        ids = all_bead_ids(jsonl)
        assert ids == {"sq-aaaaa"}
        # Deterministic id derivation + collision extension.
        b1 = derive_bead_id("fz", "fuzz-randomized", "2026-07-08", ids)
        b2 = derive_bead_id("fz", "fuzz-randomized", "2026-07-08", ids)
        assert b1 == b2 and b1.startswith("sq-fz") and len(b1) == len("sq-fz") + 5, b1
        b3 = derive_bead_id("fz", "fuzz-randomized", "2026-07-08", ids | {b1})
        assert b3 != b1 and b3.startswith("sq-fz")
        # A different lane derives a DIFFERENT id (no cross-lane dedupe collision).
        bx = derive_bead_id("fz", "other-lane", "2026-07-08", ids)
        assert bx != b1
        # Append + dedupe round-trip.
        rec = build_bead_record(b1, "fuzz-randomized", A, "2026-07-08T00:00:00Z")
        assert rec["priority"] == 1 and rec["issue_type"] == "bug" and rec["status"] == "open"
        assert "sq-6vshe.6" in rec["description"] and "🤖" in rec["description"]
        assert "demoted-lane" in rec["labels"]
        append_bead(jsonl, rec)
        lines = jsonl.read_text(encoding="utf-8").splitlines()
        assert len(lines) == 2 and json.loads(lines[0])["id"] == "sq-aaaaa"
        assert json.loads(lines[1])["id"] == b1
        assert _open_issue_bead_for_lane(jsonl, "fuzz-randomized") == b1
        assert _open_issue_bead_for_lane(jsonl, "bench-full") is None
        # A CLOSED bead for the lane is NOT deduped against (a new failure re-files).
        closed = build_bead_record("sq-fzclosed", "closed-lane", A, "2026-07-08T00:00:00Z")
        closed["status"] = "closed"
        append_bead(jsonl, closed)
        assert _open_issue_bead_for_lane(jsonl, "closed-lane") is None
        # Issue body carries the inline log tail + the self-id.
        body = build_issue_body(b1, "fuzz-randomized", A, "PANIC: known crasher\n")
        assert "🤖" in body and "SPARQ agent" in body
        assert "known crasher" in body and "fuzz-randomized" in body
        # Log tail truncation.
        big = "x" * (_LOG_TAIL_CHARS + 500)
        logf = Path(td) / "big.log"
        logf.write_text(big, encoding="utf-8")
        tail = read_log_tail(str(logf))
        assert tail.startswith("…(truncated)…") and len(tail) <= _LOG_TAIL_CHARS + 40
        assert read_log_tail(None) == "" and read_log_tail(str(Path(td) / "missing")) == ""
        # [OPUS-5] sq-c9q4r: MARKDOWN-INJECTION guard. A log carrying a ``` run must not
        # be able to close the issue body's fence — the tail is arbitrary fuzz-target
        # output (incl. the echoed failing input), and this filer opens an issue in THIS
        # repo. Assert on the round-trip through the real reader, not just the helper.
        hostile = "PANIC\n```\n## injected heading\n[x](https://evil.invalid)\n````\n"
        hostile_log = Path(td) / "hostile.log"
        hostile_log.write_text(hostile, encoding="utf-8")
        hostile_tail = read_log_tail(str(hostile_log))
        assert "```" not in hostile_tail, hostile_tail
        # Runs are neutralised in place, not deleted — length + surrounding text survive.
        assert "'''" in hostile_tail and "''''" in hostile_tail and "PANIC" in hostile_tail
        assert "## injected heading" in hostile_tail
        # The assembled body then contains EXACTLY the two fences it opens/closes itself.
        hostile_body = build_issue_body(b1, "fuzz-randomized", A, hostile_tail)
        assert hostile_body.count("```") == 2, hostile_body.count("```")
        # ...and defusing at the fence covers a caller that never went through the reader.
        assert build_issue_body(b1, "fuzz-randomized", A, hostile).count("```") == 2
        # A single/double backtick is NOT a fence and must be left alone (no over-strip).
        assert defuse_code_fences("a `b` c ``d``") == "a `b` c ``d``"
        assert defuse_code_fences(defuse_code_fences(hostile)) == defuse_code_fences(hostile)
    log("self-test OK")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(prog=PROG)
    ap.add_argument("--lane", help="short lane slug (e.g. fuzz-randomized)")
    ap.add_argument("--full-form", default="the lane's full form")
    ap.add_argument("--per-pr-form", default="the cheap per-PR slice")
    ap.add_argument("--bead-prefix", default="dl", help="bead-id infix (e.g. fz)")
    ap.add_argument("--log", help="captured run log (its tail is inlined)")
    ap.add_argument("--jsonl", default=".beads/issues.jsonl")
    ap.add_argument("--run-url", default="")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()
    if not args.lane:
        ap.error("--lane is required (or use --self-test)")

    lane = _sanitize_lane(args.lane)
    log_tail = read_log_tail(args.log)

    jsonl = Path(args.jsonl)
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    existing = _open_issue_bead_for_lane(jsonl, lane)
    if existing:
        log(f"open demoted-lane bead {existing} already tracks lane {lane} — no new bead")
        bead_id = existing
    else:
        bead_id = derive_bead_id(args.bead_prefix, lane, now[:10], all_bead_ids(jsonl))
        append_bead(jsonl, build_bead_record(bead_id, lane, args, now))
        log(f"appended P1 bug bead {bead_id} to {jsonl}")

    file_github_issue(bead_id, lane, args, log_tail)
    return 0


if __name__ == "__main__":
    sys.exit(main())
