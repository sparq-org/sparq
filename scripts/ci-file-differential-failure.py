#!/usr/bin/env python3
# [FABLE-5] Auto-file a nightly differential-fuzz finding (bead sq-0iqzw).
#
# WHAT: the deterministic filing core of .github/workflows/differential.yml's
# failure path. When a nightly shard of the sparq-bench differential fuzzer (the
# in-process Oxigraph oracle) reports a non-adjudicated mismatch, this script:
#   1. PARSES the captured fuzzer log — the `MISMATCH seed=N` lines plus the
#      "FIRST FAILING CASE:" block (seed + message + query + graph), which is a
#      complete deterministic repro (the fuzzer is seeded SplitMix64);
#   2. WRITES the parsed repro to --out-dir (uploaded as a CI artifact);
#   3. FILES a GitHub issue with the repro INLINE (via `gh`), deduped per shard:
#      if an open `[differential-fuzz]` issue for the same shard already exists it
#      comments the fresh window/run instead of opening a duplicate;
#   4. APPENDS a P1 bug bead to .beads/issues.jsonl — the bead-autoclose.yml
#      plumbing pattern IN REVERSE (create instead of close): a minimal one-line
#      append, never a rewrite of other lines, deduped against existing open
#      differential-fuzz beads for the shard. The workflow commits + pushes this
#      BEST-EFFORT (fail-soft on the GH013 protected-main ruleset rejection, like
#      bead-autoclose); the GitHub issue is the reliable channel — the
#      orchestrator's recurrent issue sweep beads from it if the push is declined.
#
# SAFETY / HONESTY PROPERTIES
#   * Never edits or reorders existing JSONL lines (append-only, unlike the close
#     script's targeted in-place edit — creation needs no mutation).
#   * Bead ids are derived (sq-df + hash of date+shard) and collision-checked
#     against every id already in the JSONL; a collision extends the hash.
#   * Idempotent per (shard): re-running the same night's failure neither opens a
#     second issue nor appends a second bead.
#   * `gh` failures degrade to warnings — filing must never mask the red lane
#     (the workflow step runs under `if: failure()`; the lane is already failing).
#
# USAGE
#   scripts/ci-file-differential-failure.py --log fuzz-log.txt --shard equality \
#       --category equality --mode baseline --seed-start N --count M \
#       --out-dir repro/ --jsonl .beads/issues.jsonl --run-url URL
#   scripts/ci-file-differential-failure.py --self-test   # hermetic; no gh, no writes
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

PROG = "ci-file-differential-failure"
MARKER = "[differential-fuzz]"

_MISMATCH_RE = re.compile(r"^MISMATCH seed=(\d+)\s*$", re.MULTILINE)
_ALPHABET = "abcdefghijklmnopqrstuvwxyz0123456789"
# [OPUS-5] sq-c9q4r sibling fix: any run of 3+ backticks in the UNTRUSTED first-failing-case
# block. That block is inlined inside a ``` fence by build_issue_body() and
# write_repro_artifact(), so such a run CLOSES the fence early and everything after it
# renders as markdown in an issue this repo's own CI opens.
_FENCE_RUN_RE = re.compile(r"`{3,}")


def log(msg: str) -> None:
    print(f"[{PROG}] {msg}", file=sys.stderr)


def defuse_code_fences(text: str) -> str:
    """Neutralise ``` runs so untrusted repro text cannot escape the markdown code fence
    it is inlined into.

    [OPUS-5] The twin of scripts/ci-file-demoted-lane-failure.py's helper of the same
    name (sq-c9q4r), applied here to the identical hole this filer carries. The FIRST
    FAILING CASE block is fuzzer-generated SPARQL + RDF text, so a case containing a run
    of three-or-more backticks closes the fence in build_issue_body() /
    write_repro_artifact() early and the remainder renders as markdown in an issue this
    repo's own CI opens (own-repo markdown injection).
    Each such run is rewritten to the same number of apostrophes: ASCII, no invisible
    characters, visually near-identical, and impossible to read as a fence. Nothing
    replayable is lost — the authoritative repro is the SEED (`fuzz <seed> 1 <category>`),
    the inline block is diagnostic display, and the artifact's fuzz-log.txt keeps the raw
    bytes verbatim.
    """
    return _FENCE_RUN_RE.sub(lambda m: "'" * len(m.group(0)), text)


def parse_fuzz_log(text: str) -> dict:
    """Extract the failing-seed list + the FIRST FAILING CASE repro block.

    The case block is fence-defused here (it is only ever inlined into a ``` block);
    the raw log is still written verbatim to the artifact's fuzz-log.txt."""
    seeds = [int(m) for m in _MISMATCH_RE.findall(text)]
    first_case = ""
    marker = "FIRST FAILING CASE:"
    if marker in text:
        first_case = defuse_code_fences(text.split(marker, 1)[1].strip())
    # The summary line (counts) if present — useful context in the issue.
    summary = ""
    for line in text.splitlines():
        if line.startswith("fuzz["):
            summary = line.strip()
    return {"seeds": seeds, "first_case": first_case, "summary": summary}


def derive_bead_id(shard: str, date: str, existing_ids: set, n: int = 5) -> str:
    """Deterministic, collision-checked bead id: sq-df + hash(date+shard).
    Deterministic so a re-run of the same night's failure derives the SAME id
    (the dedupe below then no-ops); collision-extension keeps it unique."""
    digest = hashlib.sha256(f"differential-fuzz:{date}:{shard}".encode()).hexdigest()
    # Map hex to the bead alphabet, take n chars, extend on collision.
    chars = "".join(_ALPHABET[int(c, 16) % len(_ALPHABET)] for c in digest)
    while True:
        bead_id = f"sq-df{chars[:n]}"
        if bead_id not in existing_ids:
            return bead_id
        n += 1


def existing_open_diff_bead(jsonl_path: Path, shard: str) -> str | None:
    """The id of an existing open/in_progress differential-fuzz bead for this
    shard, if any (dedupe: one standing bug bead per shard until it is closed)."""
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


def build_bead_record(bead_id: str, shard: str, parsed: dict, args, now: str) -> dict:
    """A minimal, schema-shaped bead line (mirrors .beads/issues.jsonl records)."""
    n = len(parsed["seeds"])
    first = parsed["seeds"][0] if parsed["seeds"] else "?"
    title = (
        f"[BUG] {MARKER} shard={shard}: {n} differential mismatch(es) vs Oxigraph, "
        f"first seed={first} (window {args.seed_start}+{args.count}, mode={args.mode})"
    )
    replay = (
        f"cargo run -p sparq-bench --release -- fuzz {first} 1 {args.category}"
        if parsed["seeds"]
        else f"cargo run -p sparq-bench --release -- fuzz {args.seed_start} {args.count} {args.category}"
    )
    if args.mode != "baseline":
        replay = f"{args.mode}=1 {replay}"
    description = (
        f"Auto-filed by the nightly differential lane (sq-0iqzw). Run: {args.run_url}\n"
        f"Failing seeds: {parsed['seeds'][:50]}{' …' if n > 50 else ''}\n"
        f"Summary: {parsed['summary']}\n"
        f"Replay: {replay}\n"
        f"Repro (seed + query + graph) is inline in the linked GitHub issue and in the "
        f"differential-repro artifact of the run. Adjudicated divergence classes "
        f"(bench/differential-divergences.json) are already excluded — this is a "
        f"NON-adjudicated wrong-answer candidate. 🤖 SPARQ agent [FABLE-5]"
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
        "created_by": "differential-fuzz CI",
        "updated_at": now,
        "labels": ["differential-fuzz", "ci"],
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
    first = parsed["seeds"][0] if parsed["seeds"] else "?"
    replay = f"cargo run -p sparq-bench --release -- fuzz {first} 1 {args.category}"
    if args.mode != "baseline":
        replay = f"{args.mode}=1 {replay}"
    seeds_line = ", ".join(str(s) for s in parsed["seeds"][:50]) + (" …" if n > 50 else "")
    # [OPUS-5] Defuse AT the fence as well as in parse_fuzz_log(), so the "untrusted text
    # cannot escape this block" invariant holds for every caller rather than only the one
    # that parsed the log. Idempotent (the rewrite emits no backticks), so a case that
    # already went through parse_fuzz_log() is unchanged.
    case = defuse_code_fences(parsed["first_case"]) or "(no FIRST FAILING CASE block captured — see the artifact log)"
    return f"""> 🤖 **SPARQ agent** — auto-filed by the nightly differential-fuzz lane (bead sq-0iqzw). [FABLE-5]

The nightly sparq-vs-Oxigraph differential fuzzer found **{n} non-adjudicated mismatch(es)** in shard `{shard}` (category `{args.category}`, mode `{args.mode}`, seed window {args.seed_start}+{args.count}).

- **Failing seeds:** {seeds_line}
- **Deterministic replay:** `{replay}`
- **Run:** {args.run_url}
- **Bead:** `{bead_id}` (P1, auto-appended; push best-effort)
- **Artifact:** `differential-repro-{shard}` on the run above (full log)

The adjudicated divergence classes in `bench/differential-divergences.json` (sq-eibog cross-family `=`/`!=`; sq-ai2wa bnode-vs-IRI) are already excluded by the comparator, so this is a candidate **wrong-answer bug** — in sparq or (less likely) Oxigraph — until adjudicated.

### First failing case (seed + query + graph)

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
    """Number of an existing open differential-fuzz issue for this shard, if any."""
    try:
        out = gh(
            "issue", "list", "--state", "open",
            "--search", f'in:title "{MARKER} shard={shard}"',
            "--json", "number,title", "--limit", "10",
        )
        for item in json.loads(out or "[]"):
            if MARKER in item.get("title", "") and f"shard={shard}" in item.get("title", ""):
                return str(item["number"])
    except (subprocess.CalledProcessError, json.JSONDecodeError) as e:
        log(f"warning: issue dedupe search failed ({e}) — will attempt creation")
    return None


def file_github_issue(bead_id: str, shard: str, parsed: dict, args) -> None:
    body = build_issue_body(bead_id, shard, parsed, args)
    n = len(parsed["seeds"])
    first = parsed["seeds"][0] if parsed["seeds"] else "?"
    title = (
        f"{MARKER} shard={shard}: {n} differential mismatch(es) vs Oxigraph "
        f"(first seed={first}, mode={args.mode})"
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
            url = gh("issue", "create", "--title", title, "--body-file", body_file)
            log(f"filed GitHub issue: {url}")
    except subprocess.CalledProcessError as e:
        log(f"warning: gh issue filing failed (exit {e.returncode}): {e.stderr.strip() if e.stderr else e}")
        log("the lane is already red and the artifact carries the repro — not fatal.")
    finally:
        os.unlink(body_file)


def write_repro_artifact(out_dir: Path, parsed: dict, log_text: str, args) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "repro.md").write_text(
        f"# differential-fuzz repro — shard={args.shard} mode={args.mode}\n\n"
        f"window: seeds {args.seed_start}+{args.count} category={args.category}\n"
        f"failing seeds: {parsed['seeds']}\n"
        f"summary: {parsed['summary']}\n\n"
        # [OPUS-5] Same fence-defusing as build_issue_body(): repro.md is markdown, and the
        # case block is untrusted fuzzer-generated text. Idempotent.
        f"## first failing case\n\n```\n{defuse_code_fences(parsed['first_case'])}\n```\n",
        encoding="utf-8",
    )
    (out_dir / "fuzz-log.txt").write_text(log_text, encoding="utf-8")


# ── self-test (hermetic: no gh, no repo writes) ──────────────────────────────────
def self_test() -> int:
    sample = (
        "divergence allowlist (x): cross-family-eq-type-error=adjudicated (sq-eibog) "
        "bnode-iri-inequality=adjudicated (sq-ai2wa)\n"
        "MISMATCH seed=4695\nMISMATCH seed=4705\n"
        "fuzz[equality] seeds 4695..6895 : checked=2200 skipped(unsupported)=0 "
        "adjudicated(cross-family-eq)=3 adjudicated(bnode-iri)=0 full_mismatch=2 count_mismatch=0\n"
        "\nFIRST FAILING CASE:\nseed=4695\nsparq query().len()=1 != oxigraph=0\n"
        "--- query ---\nSELECT ?s WHERE { ?s ex:val ?v FILTER(?v = 1) }\n"
        "--- graph ---\nex:n0 ex:val \"1.000000000000000001\"^^xsd:decimal .\n"
    )
    parsed = parse_fuzz_log(sample)
    assert parsed["seeds"] == [4695, 4705], parsed["seeds"]
    assert "seed=4695" in parsed["first_case"]
    assert "--- graph ---" in parsed["first_case"]
    assert parsed["summary"].startswith("fuzz[equality]")
    # No-mismatch log parses to empty (a red lane with no MISMATCH lines — e.g. a
    # panic — still files, with the raw log as the payload).
    empty = parse_fuzz_log("fuzz[all] seeds 0..10 : checked=10 ...")
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
        b1 = derive_bead_id("equality", "2026-07-07", ids)
        b2 = derive_bead_id("equality", "2026-07-07", ids)
        assert b1 == b2 and b1.startswith("sq-df") and len(b1) == len("sq-df") + 5
        b3 = derive_bead_id("equality", "2026-07-07", ids | {b1})
        assert b3 != b1 and b3.startswith("sq-df")
        # Append + dedupe round-trip.
        class A:  # minimal args stand-in
            seed_start, count, mode, category = "4695", "2200", "baseline", "equality"
            run_url = "https://example.invalid/run/1"
        rec = build_bead_record(b1, "equality", parsed, A, "2026-07-07T00:00:00Z")
        assert rec["priority"] == 1 and rec["issue_type"] == "bug" and rec["status"] == "open"
        assert "sq-0iqzw" in rec["description"] and "🤖" in rec["description"]
        append_bead(jsonl, rec)
        # Existing lines untouched; the new line parses; dedupe now finds it.
        lines = jsonl.read_text(encoding="utf-8").splitlines()
        assert len(lines) == 2 and json.loads(lines[0])["id"] == "sq-aaaaa"
        assert json.loads(lines[1])["id"] == b1
        assert existing_open_diff_bead(jsonl, "equality") == b1
        assert existing_open_diff_bead(jsonl, "bgp") is None
        # Issue body carries the inline repro + the self-id.
        body = build_issue_body(b1, "equality", parsed, A)
        assert "🤖" in body and "SPARQ agent" in body
        assert "--- graph ---" in body and "4695" in body
        # Artifact writer round-trip.
        out = Path(td) / "repro"
        write_repro_artifact(out, parsed, sample, argparse.Namespace(
            shard="equality", mode="baseline", seed_start="4695", count="2200",
            category="equality"))
        assert (out / "repro.md").exists() and (out / "fuzz-log.txt").exists()
        # [OPUS-5] sq-c9q4r sibling: MARKDOWN-INJECTION guard. The FIRST FAILING CASE block
        # is fuzzer-generated SPARQL + RDF, so a case carrying a ``` run must not be able to
        # close the fence it is inlined into — this filer opens an issue in THIS repo.
        # Assert on the round-trip through the real parser, not just the helper.
        hostile_log = (
            "MISMATCH seed=7\n"
            "FIRST FAILING CASE:\nseed=7\n--- query ---\n"
            'SELECT ?s WHERE { ?s ex:p """```\n## injected heading\n'
            "[x](https://evil.invalid)\n````\"\"\" }\n"
        )
        hostile = parse_fuzz_log(hostile_log)
        assert "```" not in hostile["first_case"], hostile["first_case"]
        # Runs are neutralised in place, not deleted — length + surrounding text survive.
        assert "'''" in hostile["first_case"] and "''''" in hostile["first_case"]
        assert "seed=7" in hostile["first_case"] and "## injected heading" in hostile["first_case"]
        # The assembled body then contains EXACTLY the two fences it opens/closes itself.
        assert build_issue_body(b1, "equality", hostile, A).count("```") == 2
        # ...and defusing at the fence also covers a caller that never went through the
        # parser (e.g. a hand-built dict), for the body AND the artifact.
        raw = {"seeds": [7], "summary": "", "first_case": hostile_log}
        assert build_issue_body(b1, "equality", raw, A).count("```") == 2
        out2 = Path(td) / "repro-hostile"
        write_repro_artifact(out2, raw, hostile_log, argparse.Namespace(
            shard="equality", mode="baseline", seed_start="7", count="1",
            category="equality"))
        assert (out2 / "repro.md").read_text(encoding="utf-8").count("```") == 2
        # The raw log artifact is NOT markdown, so it keeps the bytes verbatim.
        assert "```" in (out2 / "fuzz-log.txt").read_text(encoding="utf-8")
        # A single/double backtick is NOT a fence and must be left alone (no over-strip).
        assert defuse_code_fences("a `b` c ``d``") == "a `b` c ``d``"
        assert defuse_code_fences(defuse_code_fences(hostile_log)) == defuse_code_fences(hostile_log)
    log("self-test OK")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(prog=PROG)
    ap.add_argument("--log", help="captured fuzzer stdout+stderr")
    ap.add_argument("--shard", help="matrix shard name (e.g. equality, mode-mmap)")
    ap.add_argument("--category", default="all")
    ap.add_argument("--mode", default="baseline")
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
    parsed = parse_fuzz_log(log_text)
    log(f"parsed {len(parsed['seeds'])} failing seed(s) for shard {args.shard}")

    write_repro_artifact(Path(args.out_dir), parsed, log_text, args)

    jsonl = Path(args.jsonl)
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    existing = existing_open_diff_bead(jsonl, args.shard)
    if existing:
        log(f"open differential-fuzz bead {existing} already tracks shard {args.shard} — no new bead")
        bead_id = existing
    else:
        bead_id = derive_bead_id(args.shard, now[:10], all_bead_ids(jsonl))
        append_bead(jsonl, build_bead_record(bead_id, args.shard, parsed, args, now))
        log(f"appended P1 bug bead {bead_id} to {jsonl}")

    file_github_issue(bead_id, args.shard, parsed, args)
    return 0


if __name__ == "__main__":
    sys.exit(main())
