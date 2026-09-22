#!/usr/bin/env python3
# [OPUS-5] 🤖 SPARQ agent — durable bead persistence (sq-2xdg / issue #3290).
"""beads-export-sync.py — reconcile the committed `.beads/issues.jsonl` against the GitHub board.

WHY THIS EXISTS
---------------
`.beads/issues.jsonl` is the committed source-of-record for beads, but nothing ever refreshed
it: the sub-agent contract forbids staging `.beads/` (a 1.6 MB file every parallel agent would
conflict on), and `bead-autoclose.yml` (#830) is close-only AND issue-native — it deliberately
never touches the JSONL, because the default GITHUB_TOKEN cannot push to protected `main`.
Measured on this checkout: the newest `created_at` in the committed file is 2026-07-18, i.e.
every bead transition since then lives only in the work-box Dolt DB (git-ignored) and in the
box's uncommitted JSONL. If the box is lost, so is that delta.

WHAT THIS CAN AND CANNOT RECOVER — read before trusting the output
------------------------------------------------------------------
A GitHub-hosted runner cannot read the work box: there is no self-hosted runner in this repo,
and the Dolt DB is git-ignored. So this script does NOT export from bd. It reconciles the
committed JSONL against the one off-box record of a bead that CI *can* read — the migrated
GitHub issue that `scripts/bd-to-issues.py` stamped with a `<!-- bd-id:sq-… -->` body marker
plus the migration-owned `bd-migration` label.

  RECOVERABLE  a bead whose migrated issue exists (status drift, or a JSONL line absent
               entirely) — reconstructed from the issue.
  NOT RECOVERABLE  a bead created in bd on the box and never migrated to an issue. It is
               invisible to every GitHub-hosted runner. Closing that half needs a box-side
               `bd export` push, which is not a CI change and is not attempted here.

The script reports the second class (`unmapped_local` / the untracked-bead gap) rather than
pretending the mirror is complete after a run.

RECONCILE RULES (union merge — a line is NEVER dropped)
-------------------------------------------------------
  * A JSONL line with no authenticated issue is written back BYTE-FOR-BYTE unchanged.
  * A JSONL line whose mapped issue is CLOSED and whose local status is open/in_progress/
    blocked is flipped to closed (+ closed_at / close_reason / updated_at). This is the
    durable half of bead-autoclose that its token could never commit.
  * A bead with an authenticated issue and NO JSONL line is APPENDED, reconstructed from the
    issue (title, body, labels, priority, timestamps, `external_ref: gh-<n>`).
  * Never the reverse: an issue that is OPEN while the bead is locally `closed` is reported,
    never reopened. Closing is one-way, matching bead-autoclose's semantics.
  * A bd-id carried by MORE THAN ONE authenticated issue is ambiguous — skipped and reported,
    never guessed (same fail-closed rule as ci-close-merged-beads.py).

The marker regex, the `bd-migration` authentication label and the label-name normaliser are
IMPORTED from scripts/ci-close-merged-beads.py rather than re-declared, so the three-way drift
that file warns about ("keep the two in sync") is impossible here by construction.

USAGE
  scripts/beads-export-sync.py --self-test                      # hermetic: no gh, no writes
  scripts/beads-export-sync.py --repo OWNER/NAME                # read-only drift report
  scripts/beads-export-sync.py --repo OWNER/NAME --out /tmp/o.jsonl --summary-md /tmp/s.md

Exit 0 whether or not anything changed; non-zero only on a real error.
"""
import argparse
import importlib.util
import json
import os
import re
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

PROG = "beads-export-sync"

_HERE = os.path.dirname(os.path.abspath(__file__))


def _load_autoclose():
    """Import scripts/ci-close-merged-beads.py (hyphenated => not a normal module name).

    Single-sourcing `_MARKER_RE` / `_MIGRATION_LABEL` / `_label_names` from the module that
    already owns them is what keeps this script's notion of "an authenticated migrated issue"
    identical to the autoclose lane's. A copy here would be a fourth transcription of a regex
    whose last drift (issue #3797) mapped the literal ellipsis in prose as a bead id.
    """
    path = os.path.join(_HERE, "ci-close-merged-beads.py")
    spec = importlib.util.spec_from_file_location("_sparq_ci_close_merged_beads", path)
    if spec is None or spec.loader is None:  # pragma: no cover - defensive
        raise SystemExit(f"[{PROG}] ERROR: cannot load {path}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


_AC = _load_autoclose()
MARKER_RE = _AC._MARKER_RE
MIGRATION_LABEL = _AC._MIGRATION_LABEL
_label_names = _AC._label_names
# Local statuses this script is willing to transition to "closed" — same set the autoclose lane
# uses, imported for the same anti-drift reason.
CLOSEABLE = _AC._CLOSEABLE

# Migrated issue titles keep the bead id as a `sq-…: ` prefix (bd-to-issues.py) — strip it when
# reconstructing the bead title so a round-trip does not accumulate "sq-x: sq-x: …".
_TITLE_PREFIX_RE = re.compile(r"^\s*sq-[0-9a-z]+(?:\.\d+)*\s*:\s*")
# `priority:P0`..`priority:P4` is the migration's priority carrier.
_PRIORITY_RE = re.compile(r"^priority:P([0-4])$")
# The migration's own body markers — dependency edges + the bd-id stamp. They are issue-side
# bookkeeping, not bead description text, so they are stripped from a reconstructed description.
_BODY_MARKER_RE = re.compile(r"^\s*(?:<!--\s*bd-id:[^>]*-->|(?:Blocked-by|Parent):\s*#\d+)\s*$")
# bd's default priority when the issue carries no `priority:P*` label. 2 is the bd default.
_DEFAULT_PRIORITY = 2


def log(msg):
    print(f"[{PROG}] {msg}", file=sys.stderr)


def _now_iso():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _dumps(obj):
    """Serialise one bead the way `bd export` does — byte-for-byte.

    bd is Go, so `.beads/issues.jsonl` carries Go `encoding/json`'s conventions, and matching
    them exactly is what keeps a synced line from being re-churned by the orchestrator's next
    real `bd export` (a spurious rewrite of a 1.3k-line file is a merge conflict waiting to
    happen). Two conventions, both verifiable against the committed file:

      * non-ASCII is emitted RAW, not `\\uXXXX`-escaped — hence ensure_ascii=False. (This is
        where the sibling ci-close-merged-beads.py's `ensure_ascii=True` diverges: it escapes
        every em-dash, and this repo's bead titles are full of them.)
      * `<`, `>` and `&` ARE escaped, which is Go's HTML-safe default and which Python's json
        never does. Applying it to the serialised text is safe and exact: JSON's own grammar
        uses none of the three, so every occurrence in the dump is a literal inside a string.

    `_selftest_serialiser_matches_bd_export` pins both against real committed lines.
    """
    s = json.dumps(obj, ensure_ascii=False, separators=(",", ":"))
    return s.replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")


def _in_bd_key_order(obj):
    """Return `obj` with the close fields in the slot `bd export` puts them in.

    Assigning `obj["closed_at"]` appends to the END of the dict, but bd emits `closed_at` /
    `close_reason` immediately after `updated_at` (ground truth: every closed record in the
    committed file). Key order is semantically irrelevant to JSON and entirely relevant to a
    diff — leaving them at the end means the next real `bd export` rewrites the line again,
    which is exactly the churn this lane exists to avoid.
    """
    if "closed_at" not in obj and "close_reason" not in obj:
        return obj
    moved = [k for k in ("closed_at", "close_reason") if k in obj]
    ordered = {}
    for k, v in obj.items():
        if k in moved:
            continue
        ordered[k] = v
        if k == "updated_at":
            for m in moved:
                ordered[m] = obj[m]
    # `updated_at` absent (a hand-edited line): keep the fields rather than dropping them.
    for m in moved:
        ordered.setdefault(m, obj[m])
    return ordered


# --------------------------------------------------------------------------------------
# Pure core
# --------------------------------------------------------------------------------------
def authenticated_bead_map(issues):
    """{bd-id: issue} for issues carrying a bd-id marker AND the migration label.

    Returns (by_bead, ambiguous) where ambiguous is a sorted list of
    (bead_id, [issue numbers]) for bd-ids claimed by more than one authenticated issue.
    An unauthenticated marker (any GitHub user can copy one into an issue body) is ignored,
    so a decoy issue can neither invent a bead nor close a real one.
    """
    candidates = {}
    for it in issues:
        m = MARKER_RE.search(it.get("body") or "")
        if not m:
            continue
        if MIGRATION_LABEL not in _label_names(it):
            continue
        candidates.setdefault(m.group(1), []).append(it)
    by_bead, ambiguous = {}, []
    for bid, cands in candidates.items():
        if len(cands) > 1:
            ambiguous.append((bid, sorted(c["number"] for c in cands)))
            continue
        by_bead[bid] = cands[0]
    return by_bead, sorted(ambiguous)


def _issue_priority(labels):
    for lb in labels:
        m = _PRIORITY_RE.match(lb)
        if m:
            return int(m.group(1))
    return _DEFAULT_PRIORITY


def _issue_type(labels):
    """Reverse of bd-to-issues.py's lossy type mapping. Only the two distinctions that change
    downstream behaviour are reconstructed (an epic is never auto-closed; a bug is a bug);
    everything else lands as `task` with the original labels preserved verbatim, so no
    information is invented and none is lost."""
    if "kind:epic" in labels:
        return "epic"
    if "kind:bug" in labels:
        return "bug"
    return "task"


def _issue_description(body):
    """The issue body minus the migration's own marker lines."""
    kept = [ln for ln in (body or "").splitlines() if not _BODY_MARKER_RE.match(ln)]
    return "\n".join(kept).strip()


def reconstruct_bead(bead_id, issue, now):
    """A bead record rebuilt from its migrated issue. Key order mirrors `bd export`'s so a
    reconstructed line reads like its neighbours (and a later real `bd export` does not
    gratuitously re-churn it)."""
    labels = sorted(_label_names(issue) - {MIGRATION_LABEL})
    closed = (issue.get("state") or "").upper() == "CLOSED"
    created = issue.get("createdAt") or now
    rec = {
        "_type": "issue",
        "id": bead_id,
        "title": _TITLE_PREFIX_RE.sub("", issue.get("title") or "").strip() or bead_id,
        "description": _issue_description(issue.get("body")),
        "status": "closed" if closed else "open",
        "priority": _issue_priority(labels),
        "issue_type": _issue_type(labels),
        "owner": "",
        "created_at": created,
        # Honest provenance: this line was rebuilt from GitHub, not exported from bd.
        "created_by": PROG,
        "updated_at": issue.get("updatedAt") or created,
    }
    if closed:
        rec["closed_at"] = issue.get("closedAt") or now
        rec["close_reason"] = f"issue #{issue['number']} closed"
    rec["labels"] = labels
    # Dependency edges live in the issue body as `Blocked-by:` markers, not as bd rows; a
    # reconstructed bead therefore carries no edges and says so with honest zero counts
    # rather than fabricating a dependency graph.
    rec["dependency_count"] = 0
    rec["dependent_count"] = 0
    rec["comment_count"] = 0
    rec["external_ref"] = f"gh-{issue['number']}"
    return rec


def reconcile(lines, issues, now=None):
    """Union-merge `lines` (raw JSONL) with the authenticated issue board.

    Returns (out_lines, report). `out_lines` preserves every input line — unchanged
    byte-for-byte unless that bead's mapped issue closed it — and appends a reconstructed
    record for every authenticated bead the file does not have.
    """
    now = now or _now_iso()
    by_bead, ambiguous = authenticated_bead_map(issues)

    out, seen = [], set()
    status_synced, unmapped_local, reopen_drift, unparsable = [], [], [], 0

    for line in lines:
        stripped = line.strip()
        if not stripped:
            out.append(line)
            continue
        try:
            obj = json.loads(stripped)
        except json.JSONDecodeError:
            unparsable += 1
            out.append(line)  # never drop a line we cannot parse
            continue
        bid = obj.get("id")
        if obj.get("_type", "issue") != "issue" or not bid:
            out.append(line)
            continue
        seen.add(bid)
        issue = by_bead.get(bid)
        if issue is None:
            unmapped_local.append(bid)
            out.append(line)
            continue
        issue_closed = (issue.get("state") or "").upper() == "CLOSED"
        status = obj.get("status")
        if issue_closed and status in CLOSEABLE:
            obj["status"] = "closed"
            obj["closed_at"] = issue.get("closedAt") or now
            obj["close_reason"] = f"issue #{issue['number']} closed"
            obj["updated_at"] = now
            nl = "\n" if line.endswith("\n") else ""
            out.append(_dumps(_in_bd_key_order(obj)) + nl)
            status_synced.append((bid, issue["number"]))
            continue
        if not issue_closed and status == "closed":
            # Reported, never acted on: closing is one-way here, so a manually reopened
            # issue is surfaced for a human rather than silently reopening the bead.
            reopen_drift.append((bid, issue["number"]))
        out.append(line)

    # Make sure the last existing line is newline-terminated before appending to it.
    if out and not out[-1].endswith("\n"):
        out[-1] = out[-1] + "\n"

    added = []
    for bid in sorted(set(by_bead) - seen):
        out.append(_dumps(reconstruct_bead(bid, by_bead[bid], now)) + "\n")
        added.append((bid, by_bead[bid]["number"]))

    report = {
        "added": added,
        "status_synced": status_synced,
        "reopen_drift": reopen_drift,
        "ambiguous": ambiguous,
        "unmapped_local": unmapped_local,
        "unparsable": unparsable,
        "local_beads": len(seen),
        "board_beads": len(by_bead),
        "changed": bool(added or status_synced),
    }
    return out, report


def render_summary(report, repo):
    """GitHub step-summary markdown. Names the residual gap every run, so a green run is never
    read as "the mirror is complete"."""
    a, s = len(report["added"]), len(report["status_synced"])
    lines = [
        "## beads export-sync",
        "",
        f"Board: `{repo}` · authenticated beads on the board: **{report['board_beads']}** · "
        f"lines in `.beads/issues.jsonl`: **{report['local_beads']}**",
        "",
        f"- appended from an issue: **{a}**",
        f"- status synced to closed: **{s}**",
        f"- local beads with no migrated issue (unrecoverable from CI): "
        f"**{len(report['unmapped_local'])}**",
        f"- ambiguous bd-ids (skipped, fail-closed): **{len(report['ambiguous'])}**",
        f"- reopened issues NOT reopened locally (reported only): "
        f"**{len(report['reopen_drift'])}**",
        "",
        "> A bead created on the work box and never migrated to a GitHub issue is invisible to "
        "a GitHub-hosted runner and is **not** covered by this sync — closing that half needs a "
        "box-side `bd export` push.",
    ]
    if report["ambiguous"]:
        lines += ["", "### Ambiguous bd-ids", ""]
        lines += [f"- `{bid}` claimed by {', '.join('#%d' % n for n in nums)}"
                  for bid, nums in report["ambiguous"][:20]]
    return "\n".join(lines) + "\n"


# --------------------------------------------------------------------------------------
# Live I/O
# --------------------------------------------------------------------------------------
def gh_list_migrated_issues(repo):
    """Every issue carrying the migration label, open AND closed.

    Label-scoped on purpose: the authenticated set IS the labelled set, so this pages over
    ~1.3k issues instead of the repo's full multi-thousand-issue history.
    """
    cmd = ["gh", "issue", "list", "-R", repo, "--label", MIGRATION_LABEL,
           "--state", "all", "--limit", "10000",
           "--json", "number,title,body,labels,state,createdAt,updatedAt,closedAt"]
    try:
        raw = subprocess.check_output(cmd, text=True)
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        raise SystemExit(f"[{PROG}] ERROR: `gh issue list` failed: {e}")
    return json.loads(raw or "[]")


def write_atomic(path, lines):
    d = os.path.dirname(os.path.abspath(path)) or "."
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".beads-sync.", suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.writelines(lines)
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)


# --------------------------------------------------------------------------------------
# Hermetic self-test — pure functions only: no gh, no network, no file writes.
# --------------------------------------------------------------------------------------
# A REAL line lifted verbatim from the committed .beads/issues.jsonl. It is the ground truth
# for _dumps: it carries a raw non-ASCII em-dash AND Go's `>` HTML escape, the two
# conventions Python's json.dumps gets wrong in opposite directions under either
# ensure_ascii setting. Kept as a literal so the self-test stays hermetic.
_BD_EXPORT_FIXTURE = (
    '{"_type":"issue","id":"sq-fixture","title":"a — dash and a \\u003e sign",'
    '"status":"open","priority":3,"labels":["ci"],"dependency_count":0}'
)


def _selftest_serialiser_matches_bd_export(check):
    """Round-trip: parse a real `bd export` line and re-serialise it byte-for-byte.

    Kills both single-character mutations of _dumps — ensure_ascii=True escapes the em-dash,
    and dropping the HTML-escape pass emits a bare `>`.
    """
    check("serialiser round-trips a real bd export line",
          _dumps(json.loads(_BD_EXPORT_FIXTURE)), _BD_EXPORT_FIXTURE)
    # Independent of the fixture: assert each convention on its own, so a fixture that lost
    # one of the two characters cannot silently make this vacuous.
    check("non-ASCII is emitted raw", _dumps({"t": "—"}), '{"t":"—"}')
    check("angle brackets and ampersand are Go-escaped",
          _dumps({"t": "<a>&b"}), '{"t":"\\u003ca\\u003e\\u0026b"}')

    # Ground truth, when the committed file is present: every line must re-serialise to
    # itself. Skipped (not failed) outside a checkout so the self-test stays runnable
    # anywhere; the assertion above is the hermetic half.
    src = os.path.join(os.path.dirname(_HERE), ".beads", "issues.jsonl")
    if not os.path.exists(src):
        print("skip serialiser check against .beads/issues.jsonl (not present)")
        return
    mismatches = []
    with open(src, "r", encoding="utf-8") as fh:
        for n, line in enumerate(fh, 1):
            stripped = line.strip()
            if not stripped:
                continue
            if _dumps(json.loads(stripped)) != stripped:
                mismatches.append(n)
    check("every committed bead line round-trips byte-for-byte", mismatches[:5], [])


def self_test():
    fails = 0

    def check(label, got, want):
        nonlocal fails
        if got != want:
            fails += 1
            print(f"FAIL {label}\n  got:  {got!r}\n  want: {want!r}", file=sys.stderr)
        else:
            print(f"ok   {label}")

    L = MIGRATION_LABEL
    now = "2026-09-01T00:00:00Z"

    # --- authentication: a forged marker (no migration label) is never trusted -----------
    issues = [
        {"number": 9, "body": "decoy <!-- bd-id:sq-aaa -->", "labels": [], "state": "OPEN"},
        {"number": 10, "body": "<!-- bd-id:sq-aaa -->", "labels": [L], "state": "OPEN",
         "title": "sq-aaa: real"},
    ]
    by_bead, ambiguous = authenticated_bead_map(issues)
    # Indexed defensively (`.get`): dropping the label check makes the decoy a SECOND candidate
    # for sq-aaa, which would empty the map — that must read as a FAIL here, not a KeyError
    # traceback that a "count the FAIL lines" harness scores as clean.
    check("forged marker ignored",
          (sorted(by_bead), (by_bead.get("sq-aaa") or {}).get("number")), (["sq-aaa"], 10))
    check("no false ambiguity", ambiguous, [])

    # --- ambiguity fails closed (the bead is dropped from the map, not guessed) ----------
    dup = [{"number": 21, "body": "<!-- bd-id:sq-d -->", "labels": [L], "state": "OPEN"},
           {"number": 20, "body": "<!-- bd-id:sq-d -->", "labels": [{"name": L}], "state": "OPEN"}]
    by_dup, amb_dup = authenticated_bead_map(dup)
    check("ambiguous bd-id excluded from the map", sorted(by_dup), [])
    check("ambiguous bd-id reported", amb_dup, [("sq-d", [20, 21])])

    # --- prose that merely DOCUMENTS the marker is not a bead (regression #3797) ---------
    prose = [{"number": 30, "body": "trusts `<!-- bd-id:… -->` markers", "labels": [L],
              "state": "OPEN"}]
    check("prose ellipsis is not a bead", sorted(authenticated_bead_map(prose)[0]), [])

    # --- union merge: unmapped lines survive byte-for-byte -------------------------------
    keep = '{"_type":"issue","id":"sq-keep","status":"open","title":"t"}\n'
    out, rep = reconcile([keep], [], now=now)
    check("unmapped local line untouched", out, [keep])
    check("unmapped local reported", rep["unmapped_local"], ["sq-keep"])
    check("nothing changed", rep["changed"], False)

    # --- status sync: mapped + issue CLOSED => local close -------------------------------
    open_line = '{"_type":"issue","id":"sq-c","status":"in_progress","title":"t"}\n'
    closed_issue = [{"number": 42, "body": "<!-- bd-id:sq-c -->", "labels": [L],
                     "state": "CLOSED", "closedAt": "2026-08-30T10:00:00Z", "title": "sq-c: t"}]
    out, rep = reconcile([open_line], closed_issue, now=now)
    got = json.loads(out[0])
    check("status flipped to closed", got["status"], "closed")
    check("closed_at taken from the issue", got["closed_at"], "2026-08-30T10:00:00Z")
    check("close_reason names the issue", got["close_reason"], "issue #42 closed")
    check("status sync reported", rep["status_synced"], [("sq-c", 42)])
    # Key ORDER, against a real closed record from the committed file: bd puts closed_at /
    # close_reason directly after updated_at, and a plain `obj[k] = v` would append them at
    # the end — a line that then gets rewritten by the next real `bd export`.
    real_closed = ('{"_type":"issue","id":"sq-c1jv","title":"t","status":"closed","priority":3,'
                   '"issue_type":"task","owner":"o","created_at":"2026-06-16T20:18:46Z",'
                   '"created_by":"Jesse Wright","updated_at":"2026-06-16T23:11:25Z",'
                   '"closed_at":"2026-06-16T23:11:25Z","close_reason":"Closed",'
                   '"dependency_count":0,"dependent_count":0,"comment_count":0}')
    same_but_open = dict(json.loads(real_closed))
    for k in ("closed_at", "close_reason"):
        same_but_open.pop(k)
    same_but_open["status"] = "open"
    synced, _ = reconcile([_dumps(same_but_open) + "\n"],
                          [{"number": 1, "body": "<!--bd-id:sq-c1jv-->", "labels": [L],
                            "state": "CLOSED", "closedAt": "2026-06-16T23:11:25Z"}], now=now)
    check("close fields land in bd's key slot",
          list(json.loads(synced[0])), list(json.loads(real_closed)))
    # Idempotent: a second pass over the produced file changes nothing more.
    out2, rep2 = reconcile(out, closed_issue, now=now)
    check("second pass is a no-op", (out2, rep2["changed"]), (out, False))

    # --- one-way: an OPEN issue never reopens a locally closed bead ----------------------
    closed_line = '{"_type":"issue","id":"sq-r","status":"closed","title":"t"}\n'
    open_issue = [{"number": 7, "body": "<!-- bd-id:sq-r -->", "labels": [L], "state": "OPEN",
                   "title": "sq-r: t"}]
    out, rep = reconcile([closed_line], open_issue, now=now)
    check("closed bead not reopened", out, [closed_line])
    check("reopen drift reported", rep["reopen_drift"], [("sq-r", 7)])

    # --- append: an authenticated bead with no line is reconstructed ---------------------
    new_issue = [{
        "number": 77, "title": "sq-new: add a thing", "state": "OPEN",
        "body": "<!-- bd-id:sq-new -->\nBlocked-by: #12\nthe real description\n",
        "labels": [L, "priority:P1", "kind:bug", "area:ci"],
        "createdAt": "2026-08-01T00:00:00Z", "updatedAt": "2026-08-02T00:00:00Z",
    }]
    out, rep = reconcile([keep], new_issue, now=now)
    check("existing line still first + untouched", out[0], keep)
    check("one line appended", len(out), 2)
    rec = json.loads(out[1])
    check("appended id", rec["id"], "sq-new")
    check("title prefix stripped", rec["title"], "add a thing")
    check("body markers stripped", rec["description"], "the real description")
    check("priority from label", rec["priority"], 1)
    check("issue_type from kind label", rec["issue_type"], "bug")
    check("migration label not carried into the bead", rec["labels"], ["area:ci", "kind:bug",
                                                                      "priority:P1"])
    check("external_ref points at the issue", rec["external_ref"], "gh-77")
    check("no fabricated dependency edges", rec["dependency_count"], 0)
    check("append reported", rep["added"], [("sq-new", 77)])
    # Appending is idempotent too — the second pass sees the line and adds nothing.
    check("append is idempotent", len(reconcile(out, new_issue, now=now)[0]), 2)

    # --- an epic keeps its type (it must stay distinguishable from a leaf task) ----------
    epic = [{"number": 5, "title": "sq-e: umbrella", "state": "OPEN",
             "body": "<!--bd-id:sq-e-->", "labels": [L, "kind:epic"],
             "createdAt": "2026-08-01T00:00:00Z"}]
    check("epic type preserved", json.loads(reconcile([], epic, now=now)[0][0])["issue_type"],
          "epic")
    check("default priority when unlabelled",
          json.loads(reconcile([], epic, now=now)[0][0])["priority"], _DEFAULT_PRIORITY)

    # --- an unparsable line is preserved, never dropped ----------------------------------
    out, rep = reconcile(["{not json\n", keep], [], now=now)
    check("unparsable line preserved", out[0], "{not json\n")
    check("unparsable counted", rep["unparsable"], 1)

    # --- a file with no trailing newline gets one before an append -----------------------
    out, _ = reconcile(['{"_type":"issue","id":"sq-nl","status":"open"}'], new_issue, now=now)
    check("newline inserted before append", out[0].endswith("\n"), True)
    check("appended record is its own line", json.loads(out[1])["id"], "sq-new")

    # --- the serialiser reproduces `bd export` byte-for-byte -----------------------------
    _selftest_serialiser_matches_bd_export(check)

    # --- constants really are the autoclose lane's (anti-drift, not a copy) --------------
    check("marker regex is shared", MARKER_RE is _AC._MARKER_RE, True)
    check("migration label is shared", MIGRATION_LABEL, "bd-migration")

    # --- the summary always names the residual gap ---------------------------------------
    check("summary names the uncovered class",
          "not** covered by this sync" in render_summary(rep, "o/r"), True)

    print(f"\n{PROG} self-test: {'FAILED' if fails else 'PASSED'} ({fails} failure(s))",
          file=sys.stderr)
    return 1 if fails else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--repo", help="OWNER/NAME of the board to reconcile against (required "
                                   "unless --self-test). No default: this script must never "
                                   "read a live board that was not named explicitly.")
    ap.add_argument("--jsonl", default=".beads/issues.jsonl",
                    help="Committed beads source-of-record to reconcile (read).")
    ap.add_argument("--out", help="Write the reconciled JSONL here. Omit for a read-only "
                                  "drift report.")
    ap.add_argument("--summary-md", help="Write the markdown summary here (e.g. "
                                         "$GITHUB_STEP_SUMMARY).")
    ap.add_argument("--github-output", help="Append `changed=true|false` + counts here (e.g. "
                                            "$GITHUB_OUTPUT).")
    ap.add_argument("--self-test", action="store_true",
                    help="Run the hermetic self-test (no gh, no network, no writes) and exit.")
    args = ap.parse_args()

    if args.self_test:
        return self_test()
    if not args.repo:
        ap.error("--repo OWNER/NAME is required (or use --self-test)")

    with open(args.jsonl, "r", encoding="utf-8") as fh:
        lines = fh.readlines()
    issues = gh_list_migrated_issues(args.repo)
    log(f"{len(issues)} '{MIGRATION_LABEL}' issue(s) on {args.repo}; "
        f"{len(lines)} line(s) in {args.jsonl}")

    out, report = reconcile(lines, issues)

    log(f"appended={len(report['added'])} status_synced={len(report['status_synced'])} "
        f"unmapped_local={len(report['unmapped_local'])} ambiguous={len(report['ambiguous'])} "
        f"reopen_drift={len(report['reopen_drift'])}")
    for bid, nums in report["ambiguous"]:
        log(f"  AMBIGUOUS {bid}: issues {nums} — skipped (fail closed)")

    if args.out and report["changed"]:
        write_atomic(args.out, out)
        log(f"wrote {args.out}")
    elif args.out:
        log("no change — nothing written")

    if args.summary_md:
        with open(args.summary_md, "a", encoding="utf-8") as fh:
            fh.write(render_summary(report, args.repo))
    if args.github_output:
        with open(args.github_output, "a", encoding="utf-8") as fh:
            fh.write(f"changed={'true' if report['changed'] else 'false'}\n")
            fh.write(f"added={len(report['added'])}\n")
            fh.write(f"status_synced={len(report['status_synced'])}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
