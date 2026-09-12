#!/usr/bin/env python3
"""[OPUS-5] issue #5804 — open-issue dedupe that does not TRUST gh's search index.

Every auto-filing CI lane in this repo carries the same non-spam invariant: before it
opens an issue for a (marker, key) pair it must FIND the one it already opened, and
comment on that instead. Five call sites implemented that lookup as

    gh issue list --state open --search 'in:title "<marker> <key>"' --limit 10

and that mechanism has two failure modes this repo has already written up twice, both
of which make the lookup MISS — so the lane re-files an issue it already filed and the
non-spam invariant fails OPEN, quietly:

  * gh's search TOKENISER handles the punctuation these keys carry (`[marker]`,
    `lane=`, `shard=`, `cluster=a+b`) unreliably. That is why
    ``scripts/ci_selection_alarm.py:open_issue_exists`` deliberately refuses
    ``--search`` for exactly this job and lists by label instead.
  * the search INDEX lags, so an issue filed minutes ago can be invisible to the next
    tick. That is why ``scripts/triage-area.py:open_issues`` refuses it too — "it
    reads a lagging index and has caused three wrong conclusions in this repo".

This module is that refusal, factored out so the five sites cannot drift.
:func:`find_open_issue` reads ``gh issue list --label <the lane's own label>`` — a
database read, not an index read — and matches the rows by EXACT SUBSTRING locally,
where there is no tokeniser to get it wrong. The lane's own label is what keeps that
listing small enough to scan.

``legacy_search=`` is a SUPPLEMENT, consulted ONLY after the label listing has already
failed to match. It can therefore only ever turn a MISS into a HIT — it never decides
that an issue is absent. It exists for two bounded reasons:

  1. MIGRATION. Issues filed before a lane started labelling its issues carry no label,
     so the primary listing cannot see them. Callers that can write pass
     ``backfill_label=`` so any issue found this way is labelled on the spot; the shim
     is therefore self-liquidating rather than permanent.
  2. TRUNCATION. If a label's open set ever exceeds ``LIST_LIMIT``, the listing is a
     prefix and the match may be past the cut. That is logged, not silent.

Every lookup is FAIL-SOFT in the same shape the callers already use: a gh failure or a
malformed payload yields "could not probe" (``DedupeResult.probed is False``) rather
than an exception, and each caller keeps its own fail-open/fail-closed policy.
"""

from __future__ import annotations

import json
import subprocess
from typing import Callable, NamedTuple

# How many open issues to pull for the label listing. Matches the sibling this module
# generalises (ci_selection_alarm.open_issue_exists): a per-lane label keeps the open
# set tiny, so one page is enough — and a truncated page is reported, never assumed
# complete.
LIST_LIMIT = 100
# The legacy `--search` supplement stays at the width the five call sites used.
SEARCH_LIMIT = 10
_FIELDS = "number,title,body"
_TIMEOUT_S = 120

Runner = Callable[[list], "subprocess.CompletedProcess"]


def _default_runner(argv: list) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["gh", *argv], capture_output=True, text=True, timeout=_TIMEOUT_S
    )


def _gh_json(argv: list, runner: Runner | None) -> list | None:
    """Run a ``gh ... --json`` command; return the decoded rows, or None if the lookup
    could not be performed (gh missing/failed, or an undecodable payload). None means
    "no answer", NOT "no issues" — the distinction is what lets a caller fail open."""
    try:
        cp = (runner or _default_runner)(argv)
    except (subprocess.SubprocessError, OSError):
        return None
    if getattr(cp, "returncode", 1) != 0:
        return None
    try:
        rows = json.loads((cp.stdout or "").strip() or "[]")
    except json.JSONDecodeError:
        return None
    return rows if isinstance(rows, list) else None


def _repo_args(repo: str | None) -> list:
    return ["--repo", repo] if repo else []


def list_open_issues(
    label: str,
    *,
    repo: str | None = None,
    limit: int = LIST_LIMIT,
    runner: Runner | None = None,
) -> list | None:
    """Open issues carrying ``label``, or None if the listing could not be performed."""
    return _gh_json(
        ["issue", "list", *_repo_args(repo), "--state", "open", "--label", label,
         "--json", _FIELDS, "--limit", str(limit)],
        runner,
    )


def search_open_issues(
    query: str,
    *,
    repo: str | None = None,
    limit: int = SEARCH_LIMIT,
    runner: Runner | None = None,
) -> list | None:
    """The LEGACY index-backed lookup. Never call this as a primary — see the module
    docstring; :func:`find_open_issue` only consults it after a label listing missed."""
    return _gh_json(
        ["issue", "list", *_repo_args(repo), "--state", "open", "--search", query,
         "--json", _FIELDS, "--limit", str(limit)],
        runner,
    )


def match_marker(rows: list | None, *needles: str, field: str = "title") -> dict | None:
    """First row whose ``field`` contains EVERY needle as a literal substring.

    Plain `in` on the string gh returned — no tokenisation, no escaping, so a needle
    made of `[`, `=`, `+`, `/` or spaces matches exactly what it says.
    """
    for row in rows or []:
        if not isinstance(row, dict):
            continue
        hay = str(row.get(field) or "")
        if all(n in hay for n in needles):
            return row
    return None


class DedupeResult(NamedTuple):
    """issue: the matched open issue, or None.
    source: "label" | "legacy-search" | None — which lookup found it.
    probed: True iff at least one lookup returned a listing. ``issue is None and not
    probed`` means "could not tell", which is NOT the same as "there is none"."""

    issue: dict | None
    source: str | None
    probed: bool

    @property
    def number(self) -> str | None:
        return str(self.issue["number"]) if self.issue else None


def find_open_issue(
    label: str,
    needles,
    *,
    repo: str | None = None,
    legacy_search: str | None = None,
    field: str = "title",
    limit: int = LIST_LIMIT,
    runner: Runner | None = None,
    log: Callable[[str], None] | None = None,
    backfill_label: bool = False,
) -> DedupeResult:
    """Find the open issue this lane already filed for ``needles``.

    ``needles`` is the marker plus the lane's key (e.g. ``("[demoted-lane]",
    "lane=fuzz-randomized")``); every one must appear literally in ``field``.
    """
    if isinstance(needles, str):
        needles = (needles,)
    needles = tuple(needles)
    note = log or (lambda _m: None)
    probed = False

    rows = list_open_issues(label, repo=repo, limit=limit, runner=runner)
    if rows is not None:
        probed = True
        hit = match_marker(rows, *needles, field=field)
        if hit:
            return DedupeResult(hit, "label", True)
        if len(rows) >= limit:
            note(f"warning: open `{label}` issues hit the {limit}-row listing cap — "
                 "the label listing may be truncated; consulting the legacy search too")
    else:
        note(f"warning: could not list open `{label}` issues (missing label, or gh "
             "failed) — falling back to the legacy search")

    if legacy_search:
        srows = search_open_issues(legacy_search, repo=repo, runner=runner)
        if srows is not None:
            probed = True
            hit = match_marker(srows, *needles, field=field)
            if hit:
                # Pre-label or past-the-cap. Label it so the primary lookup sees it
                # next time and this supplement stops being load-bearing.
                if backfill_label:
                    add_label(str(hit["number"]), label, repo=repo, runner=runner,
                              log=note)
                return DedupeResult(hit, "legacy-search", True)

    return DedupeResult(None, None, probed)


def ensure_label(
    name: str,
    *,
    color: str | None = None,
    description: str | None = None,
    repo: str | None = None,
    runner: Runner | None = None,
    log: Callable[[str], None] | None = None,
) -> bool:
    """Upsert a label (``--force`` updates rather than erroring). Fail-soft.

    Load-bearing for dedupe, not cosmetics: the label is what the primary lookup reads,
    so a lane that files an issue without it files an issue its own next tick cannot
    see.
    """
    argv = ["label", "create", name, *_repo_args(repo), "--force"]
    if color:
        argv += ["--color", color]
    if description:
        argv += ["--description", description]
    try:
        cp = (runner or _default_runner)(argv)
    except (subprocess.SubprocessError, OSError) as e:
        (log or (lambda _m: None))(f"warning: label upsert {name} failed ({e})")
        return False
    ok = getattr(cp, "returncode", 1) == 0
    if not ok:
        (log or (lambda _m: None))(f"warning: label upsert {name} failed (non-fatal)")
    return ok


def add_label(
    number: str,
    label: str,
    *,
    repo: str | None = None,
    runner: Runner | None = None,
    log: Callable[[str], None] | None = None,
) -> bool:
    """Add ``label`` to an existing issue (dedupe backfill). Fail-soft."""
    try:
        cp = (runner or _default_runner)(
            ["issue", "edit", str(number), *_repo_args(repo), "--add-label", label]
        )
    except (subprocess.SubprocessError, OSError) as e:
        (log or (lambda _m: None))(f"warning: could not label issue #{number} ({e})")
        return False
    ok = getattr(cp, "returncode", 1) == 0
    if ok:
        (log or (lambda _m: None))(
            f"backfilled `{label}` onto issue #{number} so the label lookup finds it next time"
        )
    return ok
