#!/usr/bin/env python3
# [OPUS-4.8] Durable CI replacement for the EPHEMERAL session "watcher b1kzhfxq5"
# (bead sq-84a8). Authored by Opus 4.8 (Fable unavailable; flag for re-review when
# Fable returns).
#
# WHY THIS EXISTS
# ---------------
# The orchestrator's bead<->PR auto-close was, until now, an ephemeral session-scoped
# Monitor/Task ("watcher b1kzhfxq5"). Being session-scoped it does NOT persist, so once
# the session that armed it ended there was no durable automation closing a bead when its
# PR actually merged — the orchestrator had to `bd close` mapped beads by hand every tick.
# This script is the deterministic core of a COMMITTED GitHub Actions workflow
# (.github/workflows/bead-autoclose.yml) that runs on `pull_request: [closed]` (gated on
# merged == true) and so survives across sessions.
#
# WHY A MINIMAL JSONL EDIT (and NOT `bd init --from-jsonl` + `bd close` + `bd export`)
# -----------------------------------------------------------------------------------
# The committed source-of-record is `.beads/issues.jsonl`; the Dolt DB (.beads/embeddeddolt)
# is git-ignored, so a fresh CI checkout has ONLY the JSONL. Rebuilding a Dolt DB in CI then
# `bd export`-ing it round-trips the WHOLE file through this bd version's serializer, which
# (measured locally, bd 1.0.5) rewrites ~368/975 lines cosmetically — most notably flipping
# every dependency row's `created_by` to the CI actor. That is a huge noisy diff that also
# FIGHTS the orchestrator's own canonical `bd export` bookkeeping commits. Instead we mutate
# ONLY the matched bead lines in place (status -> closed + closed_at/updated_at/close_reason),
# leaving every other byte untouched: a minimal, reviewable, conflict-light diff.
#
# SAFETY PROPERTIES (mirrors scripts/bead-close-on-merge.sh's guardrails)
# ----------------------------------------------------------------------
#   * VERIFY-MERGE: with --pr, the merge is verified against the GitHub API
#     (`gh pr view --json mergedAt` must be non-null). A closed-unmerged PR closes NOTHING.
#     The token source (--title / event payload) is NEVER trusted as proof of a merge.
#   * MATCH-ONLY: only beads whose id is an exact sq-XXXX(.NN) token in the PR title (and/or
#     the merge-commit subject) are considered. Unrelated beads are never touched.
#   * OPEN-ONLY: a matched bead is closed only if it is currently open/in_progress/blocked.
#   * IDEMPOTENT: a bead already `closed` is a no-op; a re-run produces no diff.
#   * GRACEFUL SKIP: a PR with no sq-XXXX token (most PRs) exits 0 having changed nothing.
#   * A bare GH issue number (#123) is NOT a bead id and is ignored.
#
# ISSUE-NATIVE CLOSE (issue #2475 — the ruleset-blocked push made the JSONL commit-back a
# guaranteed-lost write)
# --------------------------------------------------------------------------------------
# The default GITHUB_TOKEN cannot push to protected `main` (GH013, empty bypass list — see
# bead-autoclose.yml's history, sq-roe3), and a PR-based write is equally dead from that
# token: GITHUB_TOKEN-created events trigger NO workflows, so the required `ci-summary /
# gate` check would never report and the PR would hang unmergeable forever. The one write
# the token CAN durably land is the issue-native one: the bd->issues migration
# (scripts/bd-to-issues.py) stamps every migrated issue with a `<!-- bd-id:sq-… -->` body
# marker PLUS the migration-owned `bd-migration` label (bodies are forgeable by any issue
# author; labels are not — see _MIGRATION_LABEL), so `--close-issues` resolves each bead
# token to its mapped OPEN marker+label issue and closes it through the API
# (`gh issue close --reason completed`) — mutating the tracker without committing to main
# at all. Guardrail parity with
# reconcile-merged-beads.sh: an epic (`kind:epic`) or human-gated (`needs:*`) issue is
# never auto-closed, only reported. Beads NOT yet migrated are a logged no-op — exactly
# the (non-)outcome the rejected push produced, minus the red-herring failure — and stay
# covered by the orchestrator's manual reconcile-merged-beads.sh sweep.
#
# USAGE
#   scripts/ci-close-merged-beads.py --pr 818 --title "ci(publish): ... (sq-v286.11)"
#   scripts/ci-close-merged-beads.py --pr 818            # title fetched via gh
#   scripts/ci-close-merged-beads.py --title "..." --no-verify   # extract-only, no API
#   scripts/ci-close-merged-beads.py --pr 830 --close-issues --skip-jsonl   # CI issue-native mode
#   scripts/ci-close-merged-beads.py --self-test         # hermetic; no gh, no file writes
#
# Exit 0 on success (whether or not anything changed); non-zero only on a real error.
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

PROG = "ci-close-merged-beads"

# Bead ids are `sq-` + a short slug, optionally a `.NN` molecule suffix (e.g. sq-qhy4,
# sq-toze.37, sq-v286.11). Conservative + word-boundary anchored so `#123` and arbitrary
# substrings never match. Mirrors scripts/bead-close-on-merge.sh's extract regex.
_BEAD_RE = re.compile(r"\bsq-[a-z0-9]+(?:\.[0-9]+)?\b")

# Statuses we are willing to transition to "closed". Anything else is left alone.
_CLOSEABLE = {"open", "in_progress", "blocked"}

# The migration's durable bead<->issue map: bd-to-issues.py stamps `<!-- bd-id:sq-… -->` into
# every migrated issue body (its fetch_bd_map reads the SAME pattern — keep the two in sync).
#
# The capture group is SHAPE-CONSTRAINED to a real bd id (audit-2026-07-25, issue #3797). The old
# `(\S+?)` matched ANY non-space run, so PROSE that merely DOCUMENTS the marker — e.g. the sentence
# "trusts unauthenticated `<!-- bd-id:… -->` markers" inside issues #2534 / #2535 / #3797 — was
# scanned as a bead whose id is the literal ellipsis "…". That phantom id polluted the migration
# map, and here it would have made a nonexistent bead look mapped (and, with three issues carrying
# it, permanently "ambiguous"). Real ids are `sq-` + base36-ish stem + optional dotted subtask path.
_MARKER_RE = re.compile(r"<!--\s*bd-id:(sq-[0-9a-z]+(?:\.\d+)*)\s*-->")

# AUTHENTICATION (PR #2528 review): an issue BODY is written by its author — ANY GitHub user can
# open a decoy issue copying a real `<!-- bd-id:… -->` marker, and a body-only mapping (with
# first-wins on API ordering) could close the decoy and leave the canonical migrated issue open.
# Issue LABELS, by contrast, cannot be attached by unprivileged issue authors (triage/write
# permission required — and no issue template may ever auto-apply this label), so the migration
# stamps every migrated issue with this dedicated label (bd-to-issues.py MIGRATION_LABEL — keep
# the two in sync) and the mapping below trusts ONLY marker+label issues. Zero authenticated
# issues => unmapped no-op; MULTIPLE => ambiguous, FAIL CLOSED (close nothing, exit non-zero so
# the anomaly is visible).
_MIGRATION_LABEL = "bd-migration"

# Never auto-close an epic (container) or a human-gated issue — a merged child PR carrying the
# parent's token must not silently close it. Mirrors reconcile-merged-beads.sh's guardrail.
_GUARDED_LABELS = {"kind:epic"}
_GUARDED_LABEL_PREFIXES = ("needs:",)


def log(msg: str) -> None:
    print(f"[{PROG}] {msg}", file=sys.stderr)


def extract_bead_ids(*texts: str) -> list:
    """Every distinct sq-XXXX / sq-XXXX.NN token across the given texts, order-preserving."""
    seen = {}
    for t in texts:
        for m in _BEAD_RE.findall(t or ""):
            seen.setdefault(m, None)
    return list(seen.keys())


def is_verified_merged(merged_at) -> bool:
    """A merge is verified IFF mergedAt is a non-empty, non-'null' value."""
    return bool(merged_at) and merged_at != "null"


def _now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _dumps(obj) -> str:
    """Serialize one bead row the way `bd export` does — byte-for-byte.

    bd is Go, so `.beads/issues.jsonl` carries Go `encoding/json` conventions, which differ
    from Python's json defaults in BOTH directions (issue #6087):

      * non-ASCII is emitted RAW. Python's `ensure_ascii=True` would `\\uXXXX`-escape it, and
        this repo's bead titles are full of em-dashes — measured on the committed file, 971 of
        its 1262 lines fail to round-trip under `ensure_ascii=True`.
      * `<`, `>` and `&` ARE escaped (Go's HTML-safe default, `SetEscapeHTML(true)`), which
        Python's json never does. 452 committed lines carry such an escape; none carries a
        literal `<`, `>` or `&`.
      * U+2028 / U+2029 are escaped too — Go does this unconditionally, not only in HTML-safe
        mode. (Unexercised by the current corpus, which contains neither character; included
        because a raw one from Python would be a divergence bd could never have produced.)

    The post-pass over the dumped text is exact, not a heuristic: JSON's grammar uses none of
    these characters structurally, and `json.dumps` never emits them inside an escape sequence
    (a `\\uXXXX` escape is a backslash, `u`, and four hex digits), so every occurrence in the
    dump is already a literal inside a string and every one of them must be escaped.

    Getting this wrong is not cosmetic-only in effect: `close_beads_in_jsonl` re-serializes the
    lines it mutates and leaves the rest verbatim, so a non-round-tripping dump rewrites those
    lines in a style the next real `bd export` immediately churns back — the noisy-diff failure
    mode that function exists to avoid.
    """
    return (json.dumps(obj, ensure_ascii=False, separators=(",", ":"))
            .replace("<", "\\u003c")
            .replace(">", "\\u003e")
            .replace("&", "\\u0026")
            .replace("\u2028", "\\u2028")
            .replace("\u2029", "\\u2029"))


def close_beads_in_jsonl(path: str, bead_ids, reason: str, now: str = None):
    """Minimal in-place close. Returns (closed, skipped_already_closed, not_found, changed).

    Only lines whose issue id is in `bead_ids` AND whose status is closeable are mutated;
    every other line is written back BYTE-FOR-BYTE unchanged (we re-serialize only the
    lines we touch, preserving the rest verbatim). The file is rewritten atomically.
    """
    now = now or _now_iso()
    wanted = set(bead_ids)
    closed, skipped_already, changed = [], [], False
    found = set()

    with open(path, "r", encoding="utf-8") as fh:
        lines = fh.readlines()

    out = []
    for line in lines:
        stripped = line.strip()
        if not stripped:
            out.append(line)
            continue
        try:
            obj = json.loads(stripped)
        except json.JSONDecodeError:
            out.append(line)  # never drop a line we cannot parse
            continue
        bid = obj.get("id")
        if obj.get("_type") == "issue" and bid in wanted:
            found.add(bid)
            status = obj.get("status")
            if status == "closed":
                skipped_already.append(bid)
                out.append(line)  # idempotent: untouched
                continue
            if status not in _CLOSEABLE:
                # An unexpected status (e.g. tombstone) — leave it alone, report it.
                log(f"  {bid}: status '{status}' is not closeable — leaving unchanged.")
                out.append(line)
                continue
            obj["status"] = "closed"
            obj["closed_at"] = now
            obj["updated_at"] = now
            if reason:
                obj["close_reason"] = reason
            # Preserve trailing newline if the original line had one. Re-serialize through
            # _dumps, which reproduces bd's Go serializer byte-for-byte (issue #6087), so the
            # mutated line is byte-style consistent with its neighbours and the orchestrator's
            # next `bd export` does not re-churn it.
            nl = "\n" if line.endswith("\n") else ""
            out.append(_dumps(obj) + nl)
            closed.append(bid)
            changed = True
        else:
            out.append(line)

    not_found = [b for b in bead_ids if b not in found]

    if changed:
        d = os.path.dirname(os.path.abspath(path)) or "."
        fd, tmp = tempfile.mkstemp(dir=d, prefix=".issues.", suffix=".jsonl.tmp")
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as tf:
                tf.writelines(out)
            os.replace(tmp, path)
        finally:
            if os.path.exists(tmp):
                os.unlink(tmp)

    return closed, skipped_already, not_found, changed


def _label_names(issue):
    return {lb["name"] if isinstance(lb, dict) else lb for lb in issue.get("labels") or []}


def map_beads_to_open_issues(bead_ids, issues):
    """Pure core of the issue-native close. Returns (closable, guarded, unmapped, ambiguous).

    `issues` is the OPEN-issue snapshot ({number, body, labels}); a bead maps to the issue
    whose body carries its exact `<!-- bd-id:… -->` migration marker AND which is
    AUTHENTICATED by the migration-owned `_MIGRATION_LABEL` (bodies are forgeable by any
    issue author; labels are not). closable/guarded are (bead_id, issue_number) pairs;
    guarded = mapped but epic/human-gated (never auto-closed). unmapped = no OPEN
    authenticated marker-carrying issue (not migrated, not yet label-backfilled, or already
    closed — either way a no-op, which keeps the close idempotent without needing the
    closed-issue set). ambiguous = (bead_id, sorted issue_numbers) pairs where MULTIPLE
    authenticated open issues carry the same marker — fail closed, never guess which to close.
    """
    by_bead = {}
    for it in issues:
        m = _MARKER_RE.search(it.get("body") or "")
        if not m:
            continue
        if _MIGRATION_LABEL not in _label_names(it):
            continue  # unauthenticated marker (forgeable body) — never trusted for mapping
        by_bead.setdefault(m.group(1), []).append(it)
    closable, guarded, unmapped, ambiguous = [], [], [], []
    for bid in bead_ids:
        cands = by_bead.get(bid) or []
        if not cands:
            unmapped.append(bid)
            continue
        if len(cands) > 1:
            ambiguous.append((bid, sorted(c["number"] for c in cands)))
            continue
        it = cands[0]
        labels = _label_names(it)
        if labels & _GUARDED_LABELS or any(
                lb.startswith(_GUARDED_LABEL_PREFIXES) for lb in labels):
            guarded.append((bid, it["number"]))
        else:
            closable.append((bid, it["number"]))
    return closable, guarded, unmapped, ambiguous


def gh_list_open_issues(repo):
    """OPEN-issue snapshot (number/body/labels) — the same shape bd-to-issues.fetch_bd_map
    lists, restricted to open (a closed mapped issue is already the desired end state)."""
    cmd = ["gh", "issue", "list", "--state", "open", "--limit", "10000",
           "--json", "number,body,labels"]
    if repo:
        cmd += ["-R", repo]
    try:
        raw = subprocess.check_output(cmd, text=True)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        raise SystemExit(f"[{PROG}] ERROR: `gh issue list` failed: {e}")
    return json.loads(raw or "[]")


def gh_close_issue(number, comment, repo):
    cmd = ["gh", "issue", "close", str(number), "--reason", "completed", "--comment", comment]
    if repo:
        cmd += ["-R", repo]
    try:
        subprocess.check_output(cmd, text=True)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        raise SystemExit(f"[{PROG}] ERROR: `gh issue close {number}` failed: {e}")


# --------------------------------------------------------------------------------------
# Hermetic self-test (no network, no gh, writes only to a temp file it removes).
# --------------------------------------------------------------------------------------
def self_test() -> int:
    fails = 0

    def check(label, got, want):
        nonlocal fails
        if got == want:
            print(f"  ok   {label}")
        else:
            print(f"  FAIL {label}\n       got:  {got!r}\n       want: {want!r}")
            fails += 1

    log("running --self-test (hermetic; no gh, no persistent writes)")

    check("title token sq-glw2",
          extract_bead_ids("fix(sparq-core): dict spill (sq-glw2)"), ["sq-glw2"])
    check("two tokens incl molecule suffix",
          extract_bead_ids("cert(sbom): purl (sq-tmyw, sq-toze.27)"), ["sq-tmyw", "sq-toze.27"])
    check("dedupe preserves first occurrence",
          extract_bead_ids("closes sq-abcd and again sq-abcd"), ["sq-abcd"])
    check("no token => empty",
          extract_bead_ids("chore(orchestration): no bead here #374"), [])
    check("bare GH issue numbers are not beads",
          extract_bead_ids("closingIssuesReferences: #123 #456"), [])
    check("v286-style suffix matches",
          extract_bead_ids("ci(publish): npm OIDC (sq-v286.11)"), ["sq-v286.11"])
    check("merge-commit subject + title combine, dedupe",
          extract_bead_ids("feat: thing (sq-aaa)", "Merge: feat thing (sq-aaa) (sq-bbb) (#9)"),
          ["sq-aaa", "sq-bbb"])

    check("verified-merge: real timestamp", is_verified_merged("2026-06-19T11:03:11Z"), True)
    check("verified-merge: 'null' string", is_verified_merged("null"), False)
    check("verified-merge: empty", is_verified_merged(""), False)
    check("verified-merge: None", is_verified_merged(None), False)

    # Issue-native mapping (issue #2475): bd-id marker match + epic/needs guard + unmapped no-op,
    # authenticated by the migration-owned label (PR #2528 review — bodies are forgeable).
    open_issues = [
        # Forged decoy FIRST (so any first-wins-on-API-ordering regression is caught): an
        # unprivileged author copied a real marker into a body but CANNOT attach the label.
        {"number": 9, "body": "decoy copying <!-- bd-id:sq-open1 -->", "labels": []},
        {"number": 10, "body": "work item\n\n<!-- bd-id:sq-open1 -->\n",
         "labels": [{"name": "priority:P2"}, {"name": "role:impl"},
                    {"name": _MIGRATION_LABEL}]},
        {"number": 11, "body": "umbrella\n<!--bd-id:sq-epic2-->",
         "labels": [{"name": "kind:epic"}, {"name": _MIGRATION_LABEL}]},
        {"number": 12, "body": "gated\n<!-- bd-id:sq-gated3 -->",
         "labels": ["needs:user", _MIGRATION_LABEL]},
        {"number": 13, "body": "mentions sq-open1 in prose but carries NO marker",
         "labels": [_MIGRATION_LABEL]},
    ]
    closable, guarded, unmapped, ambiguous = map_beads_to_open_issues(
        ["sq-open1", "sq-epic2", "sq-gated3", "sq-ghost9"], open_issues)
    check("authenticated marker maps; forged decoy #9 never shadows it",
          closable, [("sq-open1", 10)])
    check("epic + needs:* mapped issues are guarded, never auto-closed",
          guarded, [("sq-epic2", 11), ("sq-gated3", 12)])
    check("unmigrated bead is unmapped (no-op)", unmapped, ["sq-ghost9"])
    check("single authenticated mapping is not ambiguous", ambiguous, [])
    check("prose mention without a marker does not map",
          map_beads_to_open_issues(["sq-open1"], [open_issues[4]]), ([], [], ["sq-open1"], []))
    # Phantom-marker regression (#3797): a body that DOCUMENTS the marker syntax with an elided id
    # is not a bead mapping. The old `(\S+?)` capture scanned "…" as a bead id on three real issues
    # (#2534/#2535/#3797) and injected that phantom into the migration map. Assert at the SCAN
    # level — a map-level assertion is vacuous here, because an unrequested phantom bead is
    # indistinguishable from no match at all.
    check("prose marker with an elided id does not scan",
          _MARKER_RE.search("fetch_bd_map trusts `<!-- bd-id:… -->` markers"), None)
    check("non-bead marker payload does not scan",
          _MARKER_RE.search("<!-- bd-id:not-a-bead -->"), None)
    check("dotted subtask ids still scan", _MARKER_RE.search(
        "<!-- bd-id:sq-7d3dj.32.2.3 -->").group(1), "sq-7d3dj.32.2.3")
    # …and the phantom, if it were ever REQUESTED as a bead, must not map onto the prose issue.
    prose = [{"number": 30, "body": "fetch_bd_map trusts `<!-- bd-id:… -->` markers",
              "labels": [_MIGRATION_LABEL]}]
    check("phantom bead id never maps onto the prose issue that documents the marker",
          map_beads_to_open_issues(["…"], prose), ([], [], ["…"], []))
    check("unauthenticated marker ALONE does not map (fail closed to no-op)",
          map_beads_to_open_issues(["sq-open1"], [open_issues[0]]), ([], [], ["sq-open1"], []))
    # Duplicate AUTHENTICATED mapping (e.g. a double-created migration): fail closed — never
    # guess which issue is canonical, surface both for manual dedupe.
    dup_issues = [
        {"number": 20, "body": "<!-- bd-id:sq-dup1 -->", "labels": [_MIGRATION_LABEL]},
        {"number": 21, "body": "<!-- bd-id:sq-dup1 -->", "labels": [{"name": _MIGRATION_LABEL}]},
    ]
    check("duplicate authenticated mapping is ambiguous (fail closed, nothing closable)",
          map_beads_to_open_issues(["sq-dup1"], dup_issues),
          ([], [], [], [("sq-dup1", [20, 21])]))

    # Serializer parity with bd's Go `encoding/json` (issue #6087). Each half of _dumps is
    # pinned separately and byte-exactly: `ensure_ascii=True` reds the first, dropping the
    # HTML post-pass reds the second. Both halves are load-bearing — a line this script
    # rewrites in Python's default style is re-churned by the very next real `bd export`.
    check("_dumps leaves non-ASCII RAW (Go emits it unescaped; \\u2014 would churn)",
          _dumps({"t": "red — solid"}), '{"t":"red — solid"}')
    check("_dumps escapes < > & (Go's HTML-safe default; Python's json never does)",
          _dumps({"t": "a<b>c&d"}), '{"t":"a\\u003cb\\u003ec\\u0026d"}')
    check("_dumps escapes U+2028/U+2029 (Go escapes these unconditionally)",
          _dumps({"t": "a\u2028b\u2029c"}), '{"t":"a\\u2028b\\u2029c"}')
    check("_dumps is compact (no whitespace after ',' or ':')",
          _dumps({"a": 1, "b": [2, 3]}), '{"a":1,"b":[2,3]}')

    # The corpus assertion: every committed bead line must survive _dumps(json.loads(line))
    # byte-for-byte. This is the assertion #6087 could not have survived — under the old
    # `ensure_ascii=True` serializer 971 of the 1262 committed lines diverge. Skipped (not
    # failed) when the file is absent, so the self-test stays runnable outside a checkout.
    corpus = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                          ".beads", "issues.jsonl")
    if os.path.exists(corpus):
        with open(corpus, encoding="utf-8") as fh:
            corpus_lines = [ln for ln in fh.read().splitlines() if ln.strip()]
        diverged = [n for n, ln in enumerate(corpus_lines, 1)
                    if _dumps(json.loads(ln)) != ln]
        check(f"all {len(corpus_lines)} committed bead lines round-trip byte-for-byte",
              diverged[:5], [])
        # …and that the corpus actually exercises BOTH halves, so the assertion above cannot
        # go vacuously green if bead text ever stops containing the characters at issue.
        check("corpus exercises the raw-non-ASCII half",
              any(any(ord(c) > 127 for c in ln) for ln in corpus_lines), True)
        check("corpus exercises the HTML-escape half",
              any("\\u003c" in ln or "\\u003e" in ln or "\\u0026" in ln
                  for ln in corpus_lines), True)
    else:
        print(f"  skip corpus round-trip ({corpus} not present)")

    # End-to-end minimal-edit close against a temp JSONL.
    fd, tmp = tempfile.mkstemp(suffix=".jsonl")
    os.close(fd)
    try:
        # Titles carry both characters at issue so the end-to-end path exercises the
        # serializer too: an em-dash (must stay raw) and `<`/`&` (must stay escaped).
        rows = [
            {"_type": "issue", "id": "sq-open1", "title": "open one — <b>bold</b> & bare",
             "status": "in_progress", "priority": 3, "issue_type": "task"},
            {"_type": "issue", "id": "sq-done2", "title": "already closed",
             "status": "closed", "priority": 3, "issue_type": "task"},
            {"_type": "issue", "id": "sq-other3", "title": "unrelated open — <i>italic</i>",
             "status": "open", "priority": 3, "issue_type": "task"},
        ]
        written = [_dumps(r) for r in rows]
        with open(tmp, "w", encoding="utf-8") as f:
            for ln in written:
                f.write(ln + "\n")

        closed, skipped, not_found, changed = close_beads_in_jsonl(
            tmp, ["sq-open1", "sq-done2", "sq-ghost9"], "merged via PR #1 (test)",
            now="2026-06-19T00:00:00Z")
        check("closes only the open matched bead", closed, ["sq-open1"])
        check("already-closed match is a no-op", skipped, ["sq-done2"])
        check("unmatched id reported not_found", not_found, ["sq-ghost9"])
        check("changed flag set", changed, True)

        with open(tmp, encoding="utf-8") as fh:
            raw_after = fh.read().splitlines()
        after = {}
        for line in raw_after:
            d = json.loads(line)
            after[d["id"]] = d

        # The REWRITTEN line must be in bd's own byte style, not Python's default (#6087).
        # Asserted on the bytes, because a decoded-object comparison cannot see the churn.
        mutated = raw_after[0]
        check("rewritten line keeps its em-dash RAW (no \\u2014 churn)",
              "—" in mutated and "\\u2014" not in mutated, True)
        check("rewritten line keeps < > & HTML-escaped (no literal angle bracket)",
              "\\u003cb\\u003ebold\\u003c/b\\u003e \\u0026 bare" in mutated, True)
        check("rewritten line round-trips through bd's serializer",
              _dumps(json.loads(mutated)), mutated)
        # …and the lines this function did NOT touch are byte-for-byte the input.
        check("untouched lines are preserved verbatim", raw_after[1:], written[1:])

        check("sq-open1 now closed", after["sq-open1"]["status"], "closed")
        check("sq-open1 has closed_at", after["sq-open1"].get("closed_at"), "2026-06-19T00:00:00Z")
        check("sq-open1 has close_reason", after["sq-open1"].get("close_reason"), "merged via PR #1 (test)")
        check("UNRELATED sq-other3 untouched", after["sq-other3"]["status"], "open")
        check("sq-other3 gained NO closed_at", "closed_at" not in after["sq-other3"], True)

        # Idempotency: re-run closes nothing and changes nothing.
        c2, s2, _nf2, changed2 = close_beads_in_jsonl(
            tmp, ["sq-open1"], "merged via PR #1 (test)", now="2026-06-19T00:00:00Z")
        check("idempotent re-run closes nothing", c2, [])
        check("idempotent re-run reports already-closed", s2, ["sq-open1"])
        check("idempotent re-run sets changed=False", changed2, False)
    finally:
        os.unlink(tmp)

    print()
    if fails == 0:
        log("self-test PASSED")
        return 0
    log(f"self-test FAILED ({fails} check(s))")
    return 1


def gh_pr_view(pr: str):
    """Return (merged_at, state, title) for the PR via the gh CLI."""
    cmd = ["gh", "pr", "view", pr, "--json", "mergedAt,state,title"]
    try:
        raw = subprocess.check_output(cmd, text=True, stderr=subprocess.DEVNULL)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        raise SystemExit(f"[{PROG}] ERROR: `{' '.join(cmd)}` failed: {e}")
    d = json.loads(raw)
    return (d.get("mergedAt") or "null", d.get("state") or "", d.get("title") or "")


def main() -> int:
    ap = argparse.ArgumentParser(description="Close beads mapped to a MERGED PR (minimal JSONL edit).")
    ap.add_argument("--pr", help="PR number (its merge is verified via the gh API).")
    ap.add_argument("--title", default="", help="PR title (token source; fetched via gh if omitted).")
    ap.add_argument("--subject", default="", help="Extra text (e.g. merge-commit subject) to also scan.")
    ap.add_argument("--jsonl", default=".beads/issues.jsonl", help="Path to the beads JSONL source-of-record.")
    ap.add_argument("--no-verify", action="store_true",
                    help="Skip the gh merge-verification (extract + edit only; for local/manual use).")
    ap.add_argument("--close-issues", action="store_true",
                    help="Issue-native close: resolve each bead to its migrated GitHub issue via the "
                         "`<!-- bd-id:… -->` marker and close it through the API (no push to main).")
    ap.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", ""),
                    help="owner/repo for --close-issues (default: $GITHUB_REPOSITORY / gh's checkout context).")
    ap.add_argument("--skip-jsonl", action="store_true",
                    help="Skip the local JSONL edit (CI mode: a checkout edit cannot be pushed to "
                         "protected main, so it would be a dead write).")
    ap.add_argument("--self-test", action="store_true", help="Run the hermetic self-test and exit.")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    title = args.title
    merged_at = "null"

    if args.pr and not args.no_verify:
        merged_at, state, fetched_title = gh_pr_view(args.pr)
        if not title:
            title = fetched_title
        if not is_verified_merged(merged_at):
            log(f"PR #{args.pr} is NOT verified-merged (state={state}, mergedAt={merged_at}) — closing nothing.")
            return 0
        log(f"verified merge: PR #{args.pr} mergedAt={merged_at} state={state}")
    elif args.no_verify:
        log("--no-verify: skipping gh merge-verification (extract + edit only).")
    elif not title and not args.subject:
        raise SystemExit(f"[{PROG}] ERROR: need --pr, or --title/--subject (with --no-verify).")

    bead_ids = extract_bead_ids(title, args.subject)
    if not bead_ids:
        log("merged PR maps to NO bead (no sq-XXXX token in title/subject) — nothing to close.")
        return 0

    log("candidate bead id(s): " + ", ".join(bead_ids))

    pr_note = f" via PR #{args.pr}" if args.pr else ""
    reason = f"merged{pr_note} (auto-closed by bead-autoclose CI; mergedAt={merged_at})"

    ambiguous = []
    if args.close_issues:
        closable, guarded, unmapped, ambiguous = map_beads_to_open_issues(
            bead_ids, gh_list_open_issues(args.repo))
        comment = (f"> 🤖 SPARQ agent — bead-autoclose CI\n\n"
                   f"Auto-closed: {reason}. Issue-native reconcile (issue #2475): the close is "
                   f"written through the API because a direct commit-back to protected `main` is "
                   f"ruleset-rejected (GH013, sq-roe3).")
        for bid, num in closable:
            gh_close_issue(num, comment, args.repo)
            log(f"  {bid}: CLOSED mapped issue #{num} (issue-native).")
        for bid, num in guarded:
            log(f"  {bid}: mapped issue #{num} is an epic / human-gated (kind:epic|needs:*) — "
                f"NOT auto-closed; left for manual review (reconcile-merged-beads.sh guardrail).")
        for bid in unmapped:
            log(f"  {bid}: no OPEN '{_MIGRATION_LABEL}'-authenticated issue carries its bd-id "
                f"marker (not migrated, label not backfilled, or already closed) — no-op.")
        for bid, nums in ambiguous:
            log(f"  {bid}: ERROR — {len(nums)} authenticated open issues all carry its bd-id "
                f"marker ({', '.join('#' + str(n) for n in nums)}); refusing to guess which is "
                f"canonical (fail closed) — deduplicate the '{_MIGRATION_LABEL}' issues manually.")

    # An ambiguous authenticated mapping is a real tracker anomaly: close nothing for it and
    # exit non-zero so the (post-merge, non-gating) run turns red and gets seen.
    rc = 1 if ambiguous else 0

    if args.skip_jsonl:
        log("skipping the JSONL edit (--skip-jsonl): a CI checkout edit cannot land on protected "
            "main; the orchestrator's bd export / reconcile-merged-beads.sh owns the JSONL.")
        return rc

    if not os.path.exists(args.jsonl):
        raise SystemExit(f"[{PROG}] ERROR: beads JSONL not found at {args.jsonl}")

    closed, skipped, not_found, changed = close_beads_in_jsonl(args.jsonl, bead_ids, reason)

    for b in closed:
        log(f"  {b}: CLOSED.")
    for b in skipped:
        log(f"  {b}: already closed — no-op (idempotent).")
    for b in not_found:
        log(f"  {b}: not present in {args.jsonl} — skipping (no-op).")

    if changed:
        log(f"wrote {len(closed)} status change(s) to {args.jsonl}.")
    else:
        log("no changes to write (nothing matched-and-open).")
    return rc


if __name__ == "__main__":
    sys.exit(main())
