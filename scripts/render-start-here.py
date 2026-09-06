#!/usr/bin/env python3
# [OPUS-5] render-start-here.py — render the maintainer front door (#1135) from
# orchestration/start-here.toml plus LIVE GitHub state. Issue sparq-org/sparq#4145.
"""render-start-here.py — regenerate issue #1135 ("START HERE — what needs @jeswr").

THE BUG THIS EXISTS TO KILL. #1135 was titled "auto-maintained" and never was. On 2026-07-26 it
was 13 days stale and SEVEN of its entries were already resolved (#1848/#992/#1917 closed,
#1084/#1973 closed-unmerged, #1791/#1155 merged) while still being shown to the maintainer as work
needing them. A front door that lists resolved work is worse than no front door: it burns the one
resource the project cannot scale. So the de-staling must be STRUCTURAL — a closed or merged entry
drops out because the renderer reads live state, not because someone remembered.

SPLIT OF RESPONSIBILITY
  orchestration/start-here.toml  the JUDGEMENT: which asks exist and how they are worded.
  this script                    the STATE: what is still open, which PR has rotted to CONFLICTING,
                                 how many issues carry needs:user / needs:area / needs:ec2, how many
                                 PRs are held.
The renderer may only DROP, ANNOTATE and ORDER entries from the TOML. It NEVER invents an ask —
a new 🔴/🟡 item arrives as a PR to the TOML.

FAIL-CLOSED RULES (the reason this script is longer than it looks)
  1. An entry is dropped ONLY on positive evidence of resolution (state == closed, or a PR with
     merged == true). Missing, malformed or unreadable state KEEPS the entry and marks it
     "state unknown". Quietly dropping a P0 because of a transient 403 is the worst available
     failure, so the unknown path is biased all the way to keep.
  2. A per-ref failure still publishes (keeping is loss-free) but the process exits NON-ZERO so the
     workflow reds and a human sees the degradation. The exit status is STICKY: a later success can
     never clear an earlier failure (this repo has been bitten repeatedly by exit-zero swallowing
     an earned failure).
  3. An AGGREGATE failure (a label count or the held-PR list) ABORTS the whole render without
     touching the issue: those numbers are rendered as prose with no per-item marker to hang an
     "unknown" on, so publishing them wrong is not an option.
  4. GitHub's issue-body limit is 65 536 characters. If the body exceeds it, the 🟢 "in flight"
     section is truncated first, then the removed-as-resolved list, then the process notes. The
     🔴 / 🟡 sections are NEVER truncated; if the body still does not fit, the render aborts.
  5. The issue is edited ONLY when the rendered text differs from the current body — no no-op
     churn, no notification spam.

LIVE COUNTS USE THE LIST API, NOT THE SEARCH INDEX. GitHub's search index lags label writes by
minutes to hours, so `search/issues?q=label:...` under-reports (measured 2026-07-26: search said 34
issues carried needs:ec2 while the list API said 35). Counts here come from `gh issue list --label`.

USAGE
  python3 scripts/render-start-here.py --dry-run   # print the markdown, touch nothing
  python3 scripts/render-start-here.py --self-test # hermetic assertions, no network
  python3 scripts/render-start-here.py             # render + `gh issue edit` iff the body differs
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
import textwrap
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - py<3.11
    import tomli as tomllib  # type: ignore[no-redef]

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CONFIG = REPO_ROOT / "orchestration" / "start-here.toml"

# GitHub's hard limit on an issue body. Rule 4 above.
BODY_LIMIT = 65_536
WRAP = 100

BUCKETS = ("blocked", "decision", "gated-pr")
REF_RE = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+#[1-9][0-9]*$")
# Placeholder names contain DIGITS (needs_ec2). An earlier `[a-z_]+` class silently failed to
# recognise `{needs_ec2}` at all, so parse-time validation passed and the maintainer was shown a
# literal "{needs_ec2} issues sit on needs:ec2". Hence both the digit-tolerant class here AND the
# LEFTOVER_PLACEHOLDER_RE sweep of the finished body below: a name this regex cannot see is still
# caught before publication.
PLACEHOLDER_RE = re.compile(r"\{([a-z_][a-z0-9_]*)\}")
LEFTOVER_PLACEHOLDER_RE = re.compile(r"\{[A-Za-z_][A-Za-z0-9_]*\}")

# Exit codes. 0 clean; 2 aborted without publishing; 3 published but degraded.
EXIT_OK = 0
EXIT_ABORT = 2
EXIT_DEGRADED = 3

# The ask shown for a PR that has rotted to CONFLICTING. Rendered INSTEAD of the TOML's
# clean-state ask, so the front door can never say "arm it" about a PR that cannot be armed.
REBASE_ASK = "needs rebase before it can arm — say the word and the fleet rebases"
UNKNOWN_MARK = "⚠️ **state unknown** — live state could not be read, so this entry is KEPT"
TRUNCATED_MARK = (
    "- *(this section was truncated to fit GitHub's 65 536-character issue-body limit; "
    "the 🔴 and 🟡 sections above are never truncated)*"
)


class RenderAborted(Exception):
    """Raised when publishing would be lossy or impossible. Nothing is written."""


class Status:
    """Sticky exit status: severity only ever escalates.

    A later successful step must never downgrade an earlier earned failure — this repo has shipped
    that bug repeatedly (a later transient discarding a hard failure). `code` is therefore a
    monotone maximum and there is deliberately no way to lower it.
    """

    def __init__(self) -> None:
        self._code = EXIT_OK
        self.notes: list[str] = []

    @property
    def code(self) -> int:
        return self._code

    def degrade(self, note: str, code: int = EXIT_DEGRADED) -> None:
        self.notes.append(note)
        self._code = max(self._code, code)

    @property
    def degraded(self) -> bool:
        return self._code != EXIT_OK


# --------------------------------------------------------------------------------------------
# Live state
# --------------------------------------------------------------------------------------------


@dataclass
class RefState:
    """Live state of one issue/PR ref. `error` set => nothing about it is trusted."""

    ref: str
    is_pr: bool = False
    state: str = ""  # "open" / "closed" / "" when unknown
    merged: bool = False
    draft: bool = False
    mergeable: str = ""  # MERGEABLE / CONFLICTING / UNKNOWN
    merge_state_status: str = ""  # CLEAN / BLOCKED / DIRTY / BEHIND / UNSTABLE / UNKNOWN
    error: str | None = None

    @property
    def unknown(self) -> bool:
        return self.error is not None or not self.state

    def is_resolved(self) -> bool:
        """True ONLY on positive evidence of resolution. Unknown is never resolved (rule 1).

        THE `unknown` GUARD IS DEFENCE IN DEPTH, AND IT NEEDS A CONSTRUCTED FIXTURE TO PIN.
        No `live_ref_state` exit produces `error` set alongside a resolved-looking `state`
        today: every error path leaves `state` at ''/'open', and the merged/closed paths
        early-return before `error` can be set. So a fixture like
        `RefState(error="HTTP 403")` does NOT discriminate this branch — it returns False via
        `state.lower() != "closed"` even with the branch deleted. The branch only becomes
        load-bearing when a ref LOOKS resolved and the read failed, which is the shape
        `_self_test` and `test_a_resolved_looking_ref_whose_read_failed_is_not_resolved`
        assert on. Keep both: the branch is what stops a future caller (a cached payload, a
        partial GraphQL read, a second data source) from turning a 403 into "resolved, drop
        the maintainer's P0".
        """
        if self.unknown:
            return False
        return self.merged or self.state.lower() == "closed"

    def resolution_word(self) -> str:
        return "merged" if self.merged else "closed"


@dataclass
class Counts:
    needs_user: int
    needs_area: int
    needs_ec2: int
    held_prs: int
    held_prs_conflicting: int

    def as_dict(self) -> dict[str, int]:
        return {
            "needs_user": self.needs_user,
            "needs_area": self.needs_area,
            "needs_ec2": self.needs_ec2,
            "held_prs": self.held_prs,
            "held_prs_conflicting": self.held_prs_conflicting,
        }


def _gh(args: list[str]) -> str:
    proc = subprocess.run(
        ["gh", *args], capture_output=True, text=True, check=False, timeout=120
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"gh {' '.join(args)} -> exit {proc.returncode}: "
            f"{(proc.stderr or proc.stdout).strip()[:400]}"
        )
    return proc.stdout


def live_ref_state(ref: str) -> RefState:
    """Read one ref. Any failure is reported as `error`, never as "not closed"."""
    repo, _, number = ref.partition("#")
    try:
        raw = _gh(["api", f"repos/{repo}/issues/{number}"])
        payload = json.loads(raw)
    except Exception as exc:  # noqa: BLE001 - every failure mode is the same: unknown
        return RefState(ref=ref, error=str(exc))
    state = payload.get("state")
    if not isinstance(state, str) or state.lower() not in ("open", "closed"):
        # Malformed payload is UNKNOWN, not "open" and not "closed" (rule 1).
        return RefState(ref=ref, error=f"no usable state field in payload for {ref}")
    pull = payload.get("pull_request")
    if not isinstance(pull, dict):
        return RefState(ref=ref, is_pr=False, state=state)
    merged = bool(pull.get("merged_at"))
    out = RefState(ref=ref, is_pr=True, state=state, merged=merged)
    if merged or state.lower() == "closed":
        return out  # resolved: the mergeability detail below is irrelevant
    try:
        detail = json.loads(
            _gh(
                [
                    "pr",
                    "view",
                    number,
                    "--repo",
                    repo,
                    "--json",
                    "isDraft,mergeable,mergeStateStatus",
                ]
            )
        )
    except Exception as exc:  # noqa: BLE001
        out.error = str(exc)
        return out
    out.draft = bool(detail.get("isDraft"))
    out.mergeable = str(detail.get("mergeable") or "")
    out.merge_state_status = str(detail.get("mergeStateStatus") or "")
    if not out.mergeable:
        out.error = f"no mergeable field for {ref}"
    return out


def live_counts(repo: str) -> Counts:
    """Label populations + the held-PR list, from the LIST API (never the lagging search index)."""

    def label_count(label: str) -> int:
        raw = _gh(
            [
                "issue",
                "list",
                "--repo",
                repo,
                "--label",
                label,
                "--state",
                "open",
                "--limit",
                "1000",
                "--json",
                "number",
            ]
        )
        return len(json.loads(raw))

    held = json.loads(
        _gh(
            [
                "pr",
                "list",
                "--repo",
                repo,
                "--state",
                "open",
                "--label",
                "review:needs-user",
                "--limit",
                "300",
                "--json",
                "number,mergeable",
            ]
        )
    )
    return Counts(
        needs_user=label_count("needs:user"),
        needs_area=label_count("needs:area"),
        needs_ec2=label_count("needs:ec2"),
        held_prs=len(held),
        held_prs_conflicting=sum(1 for p in held if p.get("mergeable") == "CONFLICTING"),
    )


# --------------------------------------------------------------------------------------------
# Config
# --------------------------------------------------------------------------------------------


@dataclass
class Entry:
    refs: list[str]
    bucket: str
    ask: str
    short: str
    what: str = ""
    untracked: bool = False


@dataclass
class Config:
    repo: str
    issue: int
    text: dict[str, str]
    entries: list[Entry] = field(default_factory=list)
    process: list[str] = field(default_factory=list)
    flight: list[str] = field(default_factory=list)

    def bucket(self, name: str) -> list[Entry]:
        return [e for e in self.entries if e.bucket == name]


REQUIRED_TEXT = ("header", "preamble", "blocked_intro", "gated_intro", "process_intro")


def parse_config(doc: dict) -> Config:
    """Parse + VALIDATE. A malformed curated file must fail loudly, never render half a door."""
    errs: list[str] = []
    meta = doc.get("meta") or {}
    repo = str(meta.get("repo") or "")
    issue = meta.get("issue")
    if not REF_RE.match(f"{repo}#1"):
        errs.append("meta.repo must be 'owner/repo'")
    if not isinstance(issue, int) or issue <= 0:
        errs.append("meta.issue must be a positive integer")
    text = doc.get("text") or {}
    for key in REQUIRED_TEXT:
        if not str(text.get(key) or "").strip():
            errs.append(f"text.{key} is required")

    entries: list[Entry] = []
    for i, raw in enumerate(doc.get("entry") or []):
        where = f"entry[{i}]"
        bucket = str(raw.get("bucket") or "")
        if bucket not in BUCKETS:
            errs.append(f"{where}: bucket must be one of {BUCKETS}, got {bucket!r}")
        ask = _one_line(raw.get("ask") or "")
        short = _one_line(raw.get("short") or "")
        if not ask:
            errs.append(f"{where}: ask is required")
        if not short:
            errs.append(f"{where}: short is required (it labels the entry when it is dropped)")
        untracked = bool(raw.get("untracked"))
        ref_field = raw.get("ref")
        refs: list[str] = []
        if isinstance(ref_field, str):
            refs = [ref_field.strip()] if ref_field.strip() else []
        elif isinstance(ref_field, list):
            refs = [str(r).strip() for r in ref_field if str(r).strip()]
        elif ref_field is not None:
            errs.append(f"{where}: ref must be a string or a list of strings")
        for ref in refs:
            if not REF_RE.match(ref):
                errs.append(f"{where}: ref {ref!r} must look like 'owner/repo#123'")
        # An entry with no ref can never be auto-dropped. That is the SAFE direction, so it is
        # allowed — but only when declared, so a typo'd/empty ref cannot silently become
        # undroppable-and-unnoticed.
        if not refs and not untracked:
            errs.append(f"{where}: no ref — set `untracked = true` to declare that on purpose")
        if refs and untracked:
            errs.append(f"{where}: `untracked = true` contradicts ref {refs}")
        if bucket == "gated-pr":
            if len(refs) != 1:
                errs.append(f"{where}: a gated-pr entry needs exactly one PR ref")
            if not str(raw.get("what") or "").strip():
                errs.append(f"{where}: a gated-pr entry needs `what`")
        if bucket == "decision" and len(refs) != 1:
            errs.append(f"{where}: a decision entry needs exactly one ref (it renders the Item cell)")
        for cell_field in ("ask", "what") if bucket in ("decision", "gated-pr") else ():
            if "|" in _one_line(raw.get(cell_field) or ""):
                errs.append(f"{where}: {cell_field} renders into a table cell — escape the '|'")
        entries.append(
            Entry(
                refs=refs,
                bucket=bucket,
                ask=ask,
                short=short,
                what=_one_line(raw.get("what") or ""),
                untracked=untracked,
            )
        )

    process = [_one_line(p.get("text") or "") for p in (doc.get("process") or [])]
    flight = [_one_line(f.get("text") or "") for f in (doc.get("flight") or [])]
    for i, t in enumerate(process):
        if not t:
            errs.append(f"process[{i}]: text is required")
    for i, t in enumerate(flight):
        if not t:
            errs.append(f"flight[{i}]: text is required")
    if not entries:
        errs.append("no [[entry]] blocks — refusing to render an empty front door")

    # Placeholders are checked against the live-count key set at PARSE time so a typo fails the
    # PR's self-test rather than shipping a literal "{needs_user}" to the maintainer.
    known = set(Counts(0, 0, 0, 0, 0).as_dict()) | {"date", "dropped_sentence"}
    for label, values in (("text", list(text.values())), ("process", process), ("flight", flight)):
        for value in values:
            for name in PLACEHOLDER_RE.findall(str(value)):
                if name not in known:
                    errs.append(f"{label}: unknown placeholder {{{name}}}")
    if errs:
        raise RenderAborted("invalid orchestration/start-here.toml:\n  - " + "\n  - ".join(errs))
    return Config(
        repo=repo,
        issue=int(issue),
        text={k: _one_line(v) if k != "header" else str(v).strip() for k, v in text.items()},
        entries=entries,
        process=process,
        flight=flight,
    )


def load_config(path: Path) -> Config:
    with path.open("rb") as fh:
        return parse_config(tomllib.load(fh))


def _one_line(value: object) -> str:
    """Collapse a curated multi-line TOML value to one logical line (the renderer owns wrapping)."""
    return " ".join(str(value).split())


# --------------------------------------------------------------------------------------------
# Rendering
# --------------------------------------------------------------------------------------------


def _ref_url(ref: str, is_pr: bool) -> str:
    repo, _, number = ref.partition("#")
    kind = "pull" if is_pr else "issues"
    return f"https://github.com/{repo}/{kind}/{number}"


def _ref_label(ref: str) -> str:
    repo, _, number = ref.partition("#")
    return f"#{number}" if repo == "sparq-org/sparq" else f"{repo}#{number}"


def _substitute(text: str, values: dict[str, object]) -> str:
    def repl(match: re.Match[str]) -> str:
        name = match.group(1)
        if name not in values:
            raise RenderAborted(f"unknown placeholder {{{name}}} in curated text")
        return str(values[name])

    return PLACEHOLDER_RE.sub(repl, text)


def _fill(text: str, width: int = WRAP, initial: str = "", subsequent: str = "") -> str:
    """Wrap for readability WITHOUT ever breaking a token.

    `break_on_hyphens`/`break_long_words` must stay off: the default settings split
    `https://github.com/sparq-org/...` after the hyphen, and a newline inside a markdown link
    destination silently un-links it — which on this issue means the maintainer loses the link to
    the very thing they are being asked about.
    """
    return textwrap.fill(
        text,
        width=width,
        initial_indent=initial,
        subsequent_indent=subsequent,
        break_on_hyphens=False,
        break_long_words=False,
    )


def _bullet(text: str, marker: str = "- ") -> str:
    return _fill(text, initial=marker, subsequent=" " * len(marker))


def _quote(text: str) -> str:
    return _fill(text, width=WRAP - 2, initial="> ", subsequent="> ")


def _pr_state_cell(state: RefState) -> str:
    bits: list[str] = []
    if state.unknown:
        return "⚠️ state unknown"
    if state.draft:
        bits.append("draft")
    if state.mergeable == "CONFLICTING":
        bits.append("⚠️ CONFLICTING")
    elif state.merge_state_status in ("BLOCKED", "BEHIND", "DIRTY"):
        bits.append(state.merge_state_status)
    elif state.mergeable == "MERGEABLE":
        bits.append("MERGEABLE")
    else:
        # mergeable == UNKNOWN (GitHub still computing, or we could not read it). Saying
        # "MERGEABLE" here would invite the maintainer to arm a PR that cannot merge.
        bits.append("⚠️ state unknown")
    return ", ".join(bits)


@dataclass
class Rendered:
    body: str
    dropped: list[tuple[Entry, RefState]]
    unknown: list[tuple[Entry, RefState]]


def render(
    config: Config,
    states: dict[str, RefState],
    counts: Counts,
    *,
    date: str,
    limit: int = BODY_LIMIT,
) -> Rendered:
    """Pure render. `states` must cover every ref in the config (missing => unknown => kept)."""
    kept: list[tuple[Entry, list[RefState]]] = []
    dropped: list[tuple[Entry, RefState]] = []
    unknown: list[tuple[Entry, RefState]] = []
    for entry in config.entries:
        entry_states = [
            states.get(ref, RefState(ref=ref, error="no state was fetched for this ref"))
            for ref in entry.refs
        ]
        for st in entry_states:
            if st.unknown:
                unknown.append((entry, st))
        if entry_states and all(st.is_resolved() for st in entry_states):
            dropped.append((entry, entry_states[0]))
            continue
        kept.append((entry, entry_states))

    values: dict[str, object] = dict(counts.as_dict())
    values["date"] = date
    values["dropped_sentence"] = _dropped_sentence(len(dropped))

    head = [config.text["header"], "", _quote(_substitute(config.text["preamble"], values))]
    red = _render_blocked(config, kept)
    decisions = _render_decisions(config, kept, states)
    gated = _render_gated(config, kept, states)
    process = _render_process(config, values)
    flight_items = [_bullet(_substitute(t, values)) for t in config.flight]
    removed = _render_removed(dropped)
    footer = _render_footer(date, unknown)

    def assemble(flight_kept: list[str], removed_text: str, process_text: str) -> str:
        flight = ["## 🟢 In flight — no action needed", ""]
        flight.extend(flight_kept)
        # Say so whenever ANY 🟢 item was sacrificed, not only when the section is emptied: a
        # silently shortened list reads as a complete one.
        if len(flight_kept) < len(flight_items):
            flight.append(TRUNCATED_MARK)
        chunks = ["\n".join(head), red, decisions, gated, process_text, "\n".join(flight)]
        if removed_text:
            chunks.append(removed_text)
        chunks.append(footer)
        return "\n\n".join(c for c in chunks if c).strip() + "\n"

    # Truncation ladder (rule 4): sacrifice 🟢 items, then the whole 🟢 section, then the
    # removed-as-resolved list, then the process notes. 🔴 / 🟡 are never touched.
    flight_kept = list(flight_items)
    body = assemble(flight_kept, removed, process)
    while len(body) > limit and flight_kept:
        flight_kept.pop()
        body = assemble(flight_kept, removed, process)
    if len(body) > limit and removed:
        removed = _removed_placeholder(len(dropped))
        body = assemble(flight_kept, removed, process)
    if len(body) > limit:
        body = assemble(flight_kept, removed, _render_process(config, values, truncate=True))
    if len(body) > limit:
        raise RenderAborted(
            f"rendered body is {len(body)} chars, over GitHub's {limit} limit, and only the "
            "🔴/🟡 sections are left — refusing to truncate the sections that need a human. "
            "Shorten orchestration/start-here.toml."
        )
    # Nothing that looks like a placeholder may survive into the published body. This caught the
    # real `{needs_ec2}` escape (a name the substitution regex could not even see) and it catches
    # every future one regardless of which regex missed it.
    leftover = LEFTOVER_PLACEHOLDER_RE.search(body)
    if leftover:
        raise RenderAborted(
            f"unsubstituted placeholder {leftover.group(0)} survived into the rendered body — "
            "refusing to publish it. Fix the placeholder name in orchestration/start-here.toml "
            "(known names: " + ", ".join(sorted(values)) + ")."
        )
    return Rendered(body=body, dropped=dropped, unknown=unknown)


def _dropped_sentence(count: int) -> str:
    if count == 0:
        return "Nothing needed dropping this pass."
    plural = "entry was" if count == 1 else "entries were"
    return (
        f"{count} {plural} already resolved and {'has' if count == 1 else 'have'} been dropped "
        "automatically (see *Removed as resolved* at the bottom)."
    )


def _render_blocked(config: Config, kept: list[tuple[Entry, list[RefState]]]) -> str:
    out = ["## 🔴 Only you can unblock", "", _wrap_para(config.text["blocked_intro"]), ""]
    n = 0
    for entry, states in kept:
        if entry.bucket != "blocked":
            continue
        n += 1
        marker = f"{n}. "
        out.append(_fill(_annotate(entry.ask, entry, states), initial=marker,
                         subsequent=" " * len(marker)))
    if n == 0:
        out.append("*(nothing — every human-only blocker is resolved.)*")
    return "\n".join(out)


def _render_decisions(
    config: Config, kept: list[tuple[Entry, list[RefState]]], states: dict[str, RefState]
) -> str:
    rows = []
    for entry, entry_states in kept:
        if entry.bucket != "decision":
            continue
        ref = entry.refs[0]
        st = states.get(ref)
        link = f"**[{_ref_label(ref)}]({_ref_url(ref, bool(st and st.is_pr))})**"
        rows.append(f"| {link} | {_annotate(entry.ask, entry, entry_states)} |")
    if not rows:
        return ""
    return "\n".join(["## 🟡 Decisions — one line from you each", "", "| Item | Ask |", "|---|---|", *rows])


def _render_gated(
    config: Config, kept: list[tuple[Entry, list[RefState]]], states: dict[str, RefState]
) -> str:
    rows = []
    for entry, entry_states in kept:
        if entry.bucket != "gated-pr":
            continue
        ref = entry.refs[0]
        st = states.get(ref) or RefState(ref=ref, error="no state was fetched for this ref")
        link = f"**[{_ref_label(ref)}]({_ref_url(ref, True)})**"
        # A CONFLICTING PR cannot be armed, so the live state REPLACES the curated "arm it" ask.
        ask = REBASE_ASK if st.mergeable == "CONFLICTING" else _annotate(entry.ask, entry, entry_states)
        rows.append(f"| {link} | {_pr_state_cell(st)} | {entry.what} | {ask} |")
    if not rows:
        return ""
    return "\n".join(
        [
            "## 🟡 Maintainer-gated PRs — a word from you arms each",
            "",
            _wrap_para(config.text["gated_intro"]),
            "",
            "| PR | State | What | Ask |",
            "|---|---|---|---|",
            *rows,
        ]
    )


def _render_process(config: Config, values: dict[str, object], *, truncate: bool = False) -> str:
    out = ["## ⚠️ Process problems worth your steer", "", _wrap_para(config.text["process_intro"]), ""]
    if truncate:
        out.append(TRUNCATED_MARK)
    else:
        out.extend(_bullet(_substitute(t, values)) for t in config.process)
    return "\n".join(out)


def _render_removed(dropped: list[tuple[Entry, RefState]]) -> str:
    if not dropped:
        return ""
    items = [
        f"`{_ref_label(st.ref)}` {entry.short} (**{st.resolution_word()}**)"
        for entry, st in dropped
    ]
    return "\n".join(
        [
            "---",
            "",
            "### Removed as resolved (dropped automatically by this render)",
            "",
            _fill(" · ".join(items)),
        ]
    )


def _removed_placeholder(count: int) -> str:
    return "\n".join(
        [
            "---",
            "",
            "### Removed as resolved (dropped automatically by this render)",
            "",
            f"*{count} resolved entries were dropped; the list was truncated to fit the "
            "issue-body limit.*",
        ]
    )


def _render_footer(date: str, unknown: list[tuple[Entry, RefState]]) -> str:
    lines = [
        _wrap_para(
            "*Rendered by the SPARQ agent 🤖 from `orchestration/start-here.toml` via "
            "`scripts/render-start-here.py` — last refresh "
            f"**{date}**. Closed and merged items drop out by construction; the PR table reads "
            "live conflict state. Add or reword an ask by PR to that TOML.*"
        )
    ]
    if unknown:
        refs = ", ".join(sorted({_ref_label(st.ref) for _e, st in unknown}))
        lines += [
            "",
            _wrap_para(
                f"*⚠️ This refresh ran DEGRADED: live state for {refs} could not be read, so "
                "those entries were kept unverified rather than dropped. The refresh job is red "
                "until the next clean run.*"
            ),
        ]
    return "\n".join(lines)


def _annotate(ask: str, entry: Entry, states: list[RefState]) -> str:
    """Add the only annotations the renderer is allowed to add: unknown, and partly-resolved."""
    out = ask
    resolved = [st for st in states if st.is_resolved()]
    if resolved and len(resolved) < len(states):
        done = ", ".join(f"{_ref_label(st.ref)} {st.resolution_word()}" for st in resolved)
        out += f" *(partly resolved: {done} — the rest of this ask still stands.)*"
    if any(st.unknown for st in states):
        out += f" {UNKNOWN_MARK}."
    return out


def _wrap_para(text: str) -> str:
    return _fill(text)


# --------------------------------------------------------------------------------------------
# Live run
# --------------------------------------------------------------------------------------------


def collect_states(config: Config, status: Status, fetch=live_ref_state) -> dict[str, RefState]:
    states: dict[str, RefState] = {}
    for entry in config.entries:
        for ref in entry.refs:
            if ref in states:
                continue
            st = fetch(ref)
            states[ref] = st
            if st.unknown:
                status.degrade(f"{ref}: {st.error or 'no state'} (kept, never dropped)")
    return states


def current_body(repo: str, issue: int, runner=None) -> str:
    # Resolved at CALL time, not bound as a default: the tests patch the module's `_gh` to
    # intercept every subprocess, and a default-bound reference would slip past that seam.
    runner = runner or _gh
    raw = runner(["issue", "view", str(issue), "--repo", repo, "--json", "body"])
    return str(json.loads(raw).get("body") or "")


def publish(repo: str, issue: int, body: str, runner=None) -> None:
    # `--body-file` rather than `--body`: the body carries newlines, backticks and emoji, and an
    # argv round-trip through a shell is where those get mangled.
    runner = runner or _gh
    with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False, encoding="utf-8") as fh:
        fh.write(body)
        path = fh.name
    try:
        runner(["issue", "edit", str(issue), "--repo", repo, "--body-file", path])
    finally:
        Path(path).unlink(missing_ok=True)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    ap.add_argument("--dry-run", action="store_true", help="print the markdown; touch nothing")
    ap.add_argument("--self-test", action="store_true", help="hermetic assertions; no network")
    ap.add_argument("--issue", type=int, default=None)
    ap.add_argument("--repo", default=None)
    ap.add_argument(
        "--if-ref",
        default=None,
        metavar="owner/repo#N",
        help=(
            "COST GUARD for the on-close triggers: do nothing unless this ref is one the front "
            "door actually lists. At the 60-merges/hour target a full render per PR close would "
            "spend ~2400 API calls/hour against GITHUB_TOKEN's 1000/hour/repository budget, and "
            "the render that mattered would be the one that got rate-limited."
        ),
    )
    args = ap.parse_args(argv)

    if args.self_test:
        return _self_test(args.config)

    status = Status()
    try:
        config = load_config(args.config)
        if args.if_ref:
            known = {ref for entry in config.entries for ref in entry.refs}
            if args.if_ref not in known:
                print(
                    f"{args.if_ref} is not listed on the front door — nothing to re-render "
                    "(no API calls made).",
                    file=sys.stderr,
                )
                return EXIT_OK
        repo = args.repo or config.repo
        issue = args.issue or config.issue
        counts = live_counts(repo)  # rule 3: a failure here aborts before anything is written
        states = collect_states(config, status)
        rendered = render(
            config,
            states,
            counts,
            date=datetime.now(timezone.utc).strftime("%Y-%m-%d"),
        )
    except RenderAborted as exc:
        print(f"ABORT: {exc}", file=sys.stderr)
        return EXIT_ABORT
    except Exception as exc:  # noqa: BLE001 - aggregate/live failure: abort, publish nothing
        print(f"ABORT: could not read live state: {exc}", file=sys.stderr)
        return EXIT_ABORT

    if args.dry_run:
        print(rendered.body, end="")
        _report(rendered, status, published=False)
        return status.code

    try:
        existing = current_body(repo, issue)
    except Exception as exc:  # noqa: BLE001
        print(f"ABORT: could not read the current body of {repo}#{issue}: {exc}", file=sys.stderr)
        return EXIT_ABORT

    if existing.strip() == rendered.body.strip():
        print(f"{repo}#{issue}: already up to date ({len(rendered.body)} chars) — no edit.", file=sys.stderr)
        _report(rendered, status, published=False)
        return status.code

    try:
        publish(repo, issue, rendered.body)
    except Exception as exc:  # noqa: BLE001
        print(f"ABORT: gh issue edit failed: {exc}", file=sys.stderr)
        return EXIT_ABORT
    print(f"{repo}#{issue}: updated ({len(rendered.body)} chars).", file=sys.stderr)
    _report(rendered, status, published=True)
    return status.code


def _report(rendered: Rendered, status: Status, *, published: bool) -> None:
    for entry, st in rendered.dropped:
        print(f"  dropped {st.ref} ({st.resolution_word()}): {entry.short}", file=sys.stderr)
    for note in status.notes:
        print(f"  DEGRADED {note}", file=sys.stderr)
    if status.degraded:
        print(
            "  exiting non-zero: some refs could not be verified. Nothing was dropped on their "
            f"account{' and the body was still published' if published else ''}.",
            file=sys.stderr,
        )


# --------------------------------------------------------------------------------------------
# Hermetic self-test (no network). The exhaustive suite is scripts/tests/test_start_here_render.py.
# --------------------------------------------------------------------------------------------


def _self_test(config_path: Path) -> int:
    config = load_config(config_path)  # the SHIPPED file must validate
    assert config.repo and config.issue, "meta must resolve"
    assert config.bucket("blocked"), "the shipped TOML must carry the 🔴 asks"

    open_i = RefState(ref="sparq-org/sparq#1", state="open")
    closed_i = RefState(ref="sparq-org/sparq#2", state="closed")
    merged_pr = RefState(ref="sparq-org/sparq#3", is_pr=True, state="closed", merged=True)
    conflicting = RefState(
        ref="sparq-org/sparq#4",
        is_pr=True,
        state="open",
        mergeable="CONFLICTING",
        merge_state_status="DIRTY",
    )
    broken = RefState(ref="sparq-org/sparq#5", error="HTTP 403")
    # A ref that LOOKS resolved but whose read failed. This — not `broken` — is what pins
    # `if self.unknown: return False` in is_resolved(); see that method's docstring. `broken`
    # has no state at all, so it returns False via the empty-state fallback whether the guard
    # is there or not, and an assertion on it alone would pass for the wrong reason on the most
    # important invariant in the file.
    half_closed = RefState(ref="sparq-org/sparq#6", state="closed", error="HTTP 403")
    half_merged = RefState(
        ref="sparq-org/sparq#7", is_pr=True, state="open", merged=True, error="HTTP 403"
    )
    assert not open_i.is_resolved() and closed_i.is_resolved() and merged_pr.is_resolved()
    assert not broken.is_resolved(), "a ref with NO readable state must never count as resolved"
    assert not half_closed.is_resolved(), (
        "a ref that reads `closed` from a FAILED read must never count as resolved — "
        "dropping it would delete an ask on the strength of an error"
    )
    assert not half_merged.is_resolved(), (
        "a ref that reads `merged` from a FAILED read must never count as resolved"
    )
    assert broken.unknown and RefState(ref="sparq-org/sparq#8").unknown, (
        "both clauses of `unknown` must hold: an error, and a state that never arrived"
    )
    assert _pr_state_cell(conflicting).endswith("CONFLICTING")
    assert _pr_state_cell(broken) == "⚠️ state unknown"

    cfg = Config(
        repo="sparq-org/sparq",
        issue=1135,
        text={k: f"{k} text" for k in REQUIRED_TEXT},
        entries=[
            Entry(refs=[open_i.ref], bucket="blocked", ask="keep me", short="keep"),
            Entry(refs=[closed_i.ref], bucket="blocked", ask="drop me", short="closed one"),
            Entry(refs=[merged_pr.ref], bucket="decision", ask="drop me too", short="merged one"),
            Entry(
                refs=[conflicting.ref],
                bucket="gated-pr",
                ask='"arm it"',
                short="rotted",
                what="a rotted PR",
            ),
            Entry(refs=[broken.ref], bucket="blocked", ask="unverifiable", short="unknown one"),
        ],
        process=["{needs_user} on needs:user"],
        flight=["in flight"],
    )
    states = {s.ref: s for s in (open_i, closed_i, merged_pr, conflicting, broken)}
    out = render(cfg, states, Counts(1, 2, 3, 4, 5), date="2026-01-01")
    assert "keep me" in out.body and "unverifiable" in out.body
    assert "drop me" not in out.body and "drop me too" not in out.body
    assert REBASE_ASK in out.body and '"arm it"' not in out.body
    assert "1 on needs:user" in out.body
    assert len(out.dropped) == 2 and len(out.unknown) == 1

    status = Status()
    status.degrade("boom")
    status.notes.append("a later success")
    assert status.code == EXIT_DEGRADED, "the exit status must be sticky"
    print("render-start-here.py --self-test: OK")
    return EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
