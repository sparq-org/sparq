#!/usr/bin/env python3
# Shared `pub `-item diff helper (bead sq-l0a0, branch
# feat-flow-on-refine).
#
# WHY THIS MODULE EXISTS
# ----------------------
# Both halves of the maintenance flow-on system need to answer the SAME question
# — "does this unified diff NET-change a crate's PUBLIC Rust surface?" —
# distinguishing a genuine new/removed `pub ` export from inert
# comment / lint-attribute / pure-relocation edits:
#
#   * the PROACTIVE merge gate G2 (scripts/gate-api-skill.py) BLOCKS a PR that
#     changes a public surface without syncing a SKILL.md;
#   * the REACTIVE engine (scripts/flow-on.py, rule `changed-public-feature-docs`)
#     mints a "sync SKILL.md" follow-on ISSUE after merge for the same case.
#
# G2 originally tripped on ANY src change to a binding crate; PR #258 replaced
# that with a `pub `-item MULTISET diff (added vs removed signatures) so only a
# NET public-API change counts. The reactive flow-on rule was NOT given the same
# treatment, so it kept firing on comment-only / lint-attr-only / relocation
# diffs (dogfooded: PR #250 minted a redundant skill-sync follow-on for a
# comment+lint-attr-only diff — bead sq-l0a0). This module factors the G2 logic
# into ONE place both scripts import, so the two halves can never drift again.
#
# stdlib-only; no I/O — callers feed it unified-diff lines they already have.

from __future__ import annotations

import re

# A line is a PUBLIC-ITEM signature iff, after the leading diff marker
# (+/-) and any indentation, it begins with `pub ` (whitespace-separated) followed
# by one of the exported item keywords. Requiring whitespace after `pub` excludes
# restricted visibilities (`pub(crate)`/`pub(super)`/`pub(in …)`, all written
# `pub(` with no space) — those are NOT the crate's public surface. The
# item-keyword anchor further excludes a `pub` struct *field* (`pub name: T`),
# whose addition/removal is an internal-detail edit, not a new exported path —
# only the eight item forms below count.
PUB_ITEM_RE = re.compile(r"^[+-]\s*pub\s+(?:fn|struct|enum|trait|const|type|mod|use)\b")

# The item KIND drives where its signature ends. The brace-bodied kinds
# (fn/struct/enum/trait) end at the body's opening `{`; the statement kinds
# (use/mod-decl/const/type and unit/tuple struct) end at `;`. We read the kind
# off the first line so a group-import brace in `pub use a::{B, C};` is NOT
# mistaken for a body brace (its real terminator is the trailing `;`).
_KIND_RE = re.compile(r"\bpub\s+(fn|struct|enum|trait|const|type|mod|use)\b")
_BRACE_BODIED = frozenset({"fn", "struct", "enum", "trait"})

# Trailing comma rustfmt INSERTS when it wraps a comma-delimited list
# (params, generics, tuple-struct fields, where-clause bounds). The single-line
# form has no trailing comma before its closing delimiter; the wrapped form does.
# Dropping a comma that immediately precedes a closing delimiter — `)`/`]`/`>`,
# the closing `}` of a braced form, OR the body-opening `{` (rustfmt's wrapped
# `where T: Bound,` lands a comma right before `{`) — makes the two canonical
# forms identical. A comma before `{`/`}` is never meaningful in a real one-line
# signature, so removing it cannot mask a genuine difference.
_TRAILING_COMMA_RE = re.compile(r",(?=[)\]>{}])")


def normalize_signature(text: str) -> str:
    """Canonicalise a `pub `-item signature so a rustfmt LINE-WRAP of it compares
    EQUAL to its single-line form (bead sq-5x2i).

    rustfmt's only *token-level* change when wrapping a signature is the trailing
    comma it inserts before a closing delimiter; everything else is pure
    whitespace (newlines + indentation). So we strip ALL whitespace and drop any
    comma that sits immediately before a closing delimiter. The result is a
    whitespace- and wrap-invariant key used ONLY for the multiset EQUALITY test —
    it is never shown to a user, so collapsing `pub fn` → `pubfn` is harmless."""
    no_ws = "".join(text.split())
    return _TRAILING_COMMA_RE.sub("", no_ws)


def _item_kind(first_line_body: str) -> str:
    """The item keyword (`fn`/`struct`/…/`use`) of a `pub `-item line, or ``."""
    m = _KIND_RE.search(first_line_body)
    return m.group(1) if m else ""


def _signature_complete(acc: str, kind: str) -> bool:
    """True once `acc` (the marker-stripped text gathered so far) contains this
    item KIND's signature terminator at the OUTERMOST `()`/`[]` nesting level.

    A brace-bodied kind (fn/struct/enum/trait) ends at the first depth-0 `{`
    (the body open) or `;` (unit/tuple struct, trait-method decl); a statement
    kind (use/mod/const/type) ends at the first depth-0 `;`. We count only `()`
    and `[]` toward depth so a body `{` or a group-import `{` is never confused
    with a parameter-list delimiter, and a `;` inside a default value's `[...]`
    does not terminate early."""
    depth = 0
    for ch in acc:
        if ch in "([":
            depth += 1
        elif ch in ")]":
            depth = max(0, depth - 1)
        elif depth == 0:
            if ch == ";":
                return True
            if ch == "{" and kind in _BRACE_BODIED:
                return True
    return False


def scan_pub_diff(diff_lines: list[str]) -> tuple[list[str], list[str]]:
    """Split unified-diff lines into (added, removed) public-item signatures.

    A `pub `-item signature that rustfmt LINE-WRAPS spans several diff
    lines on one side: the first matches PUB_ITEM_RE, the continuations
    (`    chunks: &[S],`, `) -> R {`) do not. We REASSEMBLE the wrapped signature
    by appending same-sign (`+`/`-`) continuation lines until this item KIND's
    terminator (`{` body for fn/struct/enum/trait, else `;`) is seen at depth 0,
    then normalise the whole thing (normalize_signature) so
    it collapses to the SAME key as the unwrapped one-liner. This stops a pure
    reformat from reading as a public-surface change (false-positive, tripped
    #469 — bead sq-5x2i).

    Each returned string is the wrap-invariant canonical key, so an identical
    signature that is both added and removed (a pure relocation / re-indentation /
    re-wrap) compares equal across the two lists. File headers (`+++ b/…`,
    `--- a/…`) and the hunk header (`@@ … @@`) are skipped — they are never item
    signatures, and they also break a wrapped run."""
    added: list[str] = []
    removed: list[str] = []
    i = 0
    n = len(diff_lines)
    while i < n:
        line = diff_lines[i]
        if (
            line.startswith("+++")
            or line.startswith("---")
            or not PUB_ITEM_RE.match(line)
        ):
            i += 1
            continue
        sign = line[0]
        acc = line[1:]
        kind = _item_kind(acc)
        i += 1
        # Gather wrapped continuation lines: same sign, not a header/hunk line,
        # until the signature is structurally complete (this item KIND's
        # terminator at depth 0).
        while not _signature_complete(acc, kind) and i < n:
            cont = diff_lines[i]
            if not cont or cont[0] != sign or cont.startswith(("+++", "---")):
                break
            acc += " " + cont[1:]
            i += 1
        (added if sign == "+" else removed).append(normalize_signature(acc))
    return added, removed


def diff_has_net_pub_change(diff_lines: list[str]) -> bool:
    """True iff `diff_lines` (a unified diff) NET-changes a `pub `-exported item.

    A pure RELOCATION — the same signature added once and removed once (net-zero)
    — is NOT a public-API change. We compare added vs removed signatures as
    MULTISETS and report a change only when they differ: a genuinely NEW or
    DELETED export, or a signature whose text actually changed. Comment-only /
    attribute-only / non-`pub` edits yield two empty lists → no change."""
    added, removed = scan_pub_diff(diff_lines)
    return sorted(added) != sorted(removed)
