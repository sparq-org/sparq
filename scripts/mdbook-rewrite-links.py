#!/usr/bin/env python3
# [OPUS-4.8] mdBook link-rewriting preprocessor (bead sq-g4h0c).
#
# WHY THIS EXISTS — the relative-vs-portable link conflict
# --------------------------------------------------------
# The docs guide single-sources its prose from the canonical files (README.md and
# skills/<surface>/SKILL.md) via mdBook `{{#include}}` (bead sq-im8u). Those source
# files link the per-surface guides with REPO-RELATIVE paths (e.g. skills/cli/SKILL.md)
# because the lychee internal-links gate (docs-quality.yml) requires every relative
# link to resolve on disk. But when mdBook pulls that content into a chapter, it
# rewrites a relative link RELATIVE TO THE INCLUDING PAGE'S MOUNT — so a copy included
# into book/src/getting-started/capabilities.md resolves `skills/cli/SKILL.md` to the
# non-existent `getting-started/skills/cli/SKILL.html` and 404s under GitHub Pages.
#
# This preprocessor closes that gap: at book-build time it rewrites repo-relative
# markdown links in the (already-assembled, post-`{{#include}}`) chapter text to the
# mount-portable canonical form
#     https://github.com/sparq-org/sparq/blob/main/<path>
# so the SAME content renders correctly under BOTH mounts (raw GitHub view of the
# source file AND the mdBook site) WITHOUT the source files having to carry absolute
# URLs that the lychee gate cannot offline-resolve. Single source of truth stays in
# README.md / SKILL.md with their lychee-friendly relative links; the book gets
# portable links for free. Resolves bead sq-g4h0c, option (a).
#
# CONTRACT (mdBook preprocessor protocol — https://rust-lang.github.io/mdBook/):
#   * `mdbook-rewrite-links.py supports <renderer>` — exit 0 (we support every
#     renderer; we only touch chapter markdown, never renderer internals).
#   * otherwise: read a JSON `[context, book]` array on stdin, walk every Chapter's
#     `content`, rewrite links, and write the mutated `book` object to stdout.
#
# SCOPE OF THE REWRITE (deliberately narrow — leave everything else untouched):
#   * ONLY inline-markdown link targets `[text](target)` and `[ref]: target`
#     reference definitions whose target is a REPO-RELATIVE path (no scheme, not
#     starting with `/`, `#`, `mailto:`, etc.) that points at a tracked repo file —
#     i.e. begins with one of the allow-listed top-level prefixes below, or is a
#     bare root-level `*.md` (README.md, AGENTS.md, SECURITY.md,
#     inference-conformance-report.md, ...).
#   * Anchor-only (`#frag`), absolute-URL (`http(s)://`, `//`), root-absolute (`/x`),
#     and `mailto:`/`tel:` targets are left exactly as-is.
#   * A trailing `#fragment` on a rewritten path is preserved.
#   * Links INSIDE fenced/indented code blocks and inline-code spans are NOT rewritten
#     (a `[x](y)` printed inside ``` ``` ``` is code, not a link).
#
# Idempotent (re-running over already-absolute links is a no-op) and dependency-free
# (stdlib only — runs in CI with the same python3 the other docs gates use).

import json
import re
import sys

GITHUB_BLOB_BASE = "https://github.com/sparq-org/sparq/blob/main/"

# Repo-relative targets we rewrite: a path under one of these tracked top-level dirs,
# or a bare root-level *.md file. Kept explicit (not "any relative path") so we never
# rewrite an intra-book chapter link like `./getting-started/install.md`.
_DIR_PREFIXES = ("skills/", "crates/", "docs/", "research/", "bench/", "vendor/", "compliance/", "assets/")
# Root-level markdown, ANY case (issue #5021): the class was `[A-Z]`, which matched
# README.md / AGENTS.md but NOT a lowercase root-level doc such as
# inference-conformance-report.md — that link survived the preprocessor unrewritten and
# then 404'd under the book mount (README.md had to carry it as an absolute URL to work
# around it; research/docs-site-single-sourcing-anti-drift.md §8). Case is not a property
# of "is this a root-level repo doc", so it must not decide the rewrite.
# NOTE the standing convention this relies on: an intra-book link to a SIBLING chapter must
# be written `./chapter.md`, not bare `chapter.md` — `./` is what marks it as book-internal
# (see _is_repo_relative). Every book/src chapter already writes them that way.
_ROOT_MD = re.compile(r"[A-Za-z][A-Za-z0-9_.-]*\.md\Z")  # README.md, inference-conformance-report.md, ...

# An mdBook chapter is plain markdown. Match the link TARGET inside:
#   inline:    [text](TARGET)            and [text](TARGET "title")
#   reference: [label]: TARGET           (at line start, optional title)
# We only capture the target run (no spaces, no `)` ) and rewrite it in place.
_INLINE = re.compile(r"(\]\()([^)\s]+)([)\s])")
_REFDEF = re.compile(r"(?m)^(\s*\[[^\]]+\]:\s+)(\S+)(.*)$")


def _is_repo_relative(target: str) -> bool:
    # Strip a trailing #fragment for the prefix test; fragments are re-attached later.
    path = target.split("#", 1)[0]
    if not path:
        return False  # pure `#anchor`
    # Reject anything that already has a scheme / is protocol-relative / root-absolute.
    if "://" in target or target.startswith(("/", "#", "mailto:", "tel:", "//")):
        return False
    if path.startswith("./") or path.startswith("../"):
        # Intra-book navigation between chapters — mdBook resolves these correctly.
        return False
    if path.startswith(_DIR_PREFIXES):
        return True
    return bool(_ROOT_MD.match(path))


def _rewrite_target(target: str) -> str:
    if not _is_repo_relative(target):
        return target
    if "#" in target:
        path, frag = target.split("#", 1)
        return f"{GITHUB_BLOB_BASE}{path}#{frag}"
    return f"{GITHUB_BLOB_BASE}{target}"


# Mask fenced code blocks (```...``` / ~~~...~~~) and inline code spans (`...`) so we
# never rewrite a link that is actually printed as code, then restore them verbatim.
_FENCE = re.compile(r"(?ms)^(`{3,}|~{3,}).*?^\1[ \t]*$")
_INLINE_CODE = re.compile(r"`[^`\n]*`")


def _mask(text: str):
    store = []

    def stash(m):
        store.append(m.group(0))
        return f"\x00{len(store) - 1}\x00"

    masked = _FENCE.sub(stash, text)
    masked = _INLINE_CODE.sub(stash, masked)
    return masked, store


def _unmask(text: str, store) -> str:
    return re.sub(r"\x00(\d+)\x00", lambda m: store[int(m.group(1))], text)


def rewrite_content(content: str) -> str:
    masked, store = _mask(content)
    masked = _INLINE.sub(lambda m: m.group(1) + _rewrite_target(m.group(2)) + m.group(3), masked)
    masked = _REFDEF.sub(lambda m: m.group(1) + _rewrite_target(m.group(2)) + m.group(3), masked)
    return _unmask(masked, store)


def _walk(section) -> None:
    # A book item is either {"Chapter": {...}} or {"Separator": ...} / {"PartTitle": ...}.
    chapter = section.get("Chapter") if isinstance(section, dict) else None
    if chapter is None:
        return
    if isinstance(chapter.get("content"), str):
        chapter["content"] = rewrite_content(chapter["content"])
    for sub in chapter.get("sub_items", []) or []:
        _walk(sub)


# --------------------------------------------------------------------------------------
# Hermetic self-test (repo convention: `--self-test` over inline fixtures, no network, no
# files, no subprocess). Runs in docs-quality.yml's HARD quick-gates bucket on every PR —
# docs.yml only fires on its own path filter, and a preprocessor regression is silent
# (mdBook exits 0 and just emits a dead link). Both directions: what MUST be rewritten and
# what MUST be left alone.
# --------------------------------------------------------------------------------------
B = GITHUB_BLOB_BASE

_REWRITE_CASES = [
    # (input, expected) — REWRITTEN
    # Issue #5021 — the lowercase root-level doc. Narrowing _ROOT_MD back to `[A-Z]`
    # reds exactly this case.
    ("see [report](inference-conformance-report.md)", f"see [report]({B}inference-conformance-report.md)"),
    ("[ref]: inference-conformance-report.md", f"[ref]: {B}inference-conformance-report.md"),
    ("[a](README.md)", f"[a]({B}README.md)"),
    ("[a](AGENTS.md#hygiene)", f"[a]({B}AGENTS.md#hygiene)"),
    ("[a](skills/cli/SKILL.md)", f"[a]({B}skills/cli/SKILL.md)"),
    ("[a](research/roadmap.md)", f"[a]({B}research/roadmap.md)"),
    # UNTOUCHED
    ("[a](./getting-started/install.md)", "[a](./getting-started/install.md)"),
    ("[a](./capabilities.md)", "[a](./capabilities.md)"),
    ("[a](../introduction.md)", "[a](../introduction.md)"),
    ("[a](#status)", "[a](#status)"),
    ("[a](/README.md)", "[a](/README.md)"),
    ("[a](https://example.org/x.md)", "[a](https://example.org/x.md)"),
    ("[a](mailto:x@example.org)", "[a](mailto:x@example.org)"),
    ("[a](deny.toml)", "[a](deny.toml)"),  # root-level, but not markdown → out of scope
    ("`[a](README.md)`", "`[a](README.md)`"),  # inline code span
    ("```\n[a](README.md)\n```", "```\n[a](README.md)\n```"),  # fenced block
    # Idempotent: an already-absolute link is a no-op on a second pass.
    (f"[a]({B}inference-conformance-report.md)", f"[a]({B}inference-conformance-report.md)"),
]


def _self_test() -> int:
    failures = []
    for src, want in _REWRITE_CASES:
        got = rewrite_content(src)
        if got != want:
            failures.append(f"  rewrite_content({src!r})\n    got  {got!r}\n    want {want!r}")

    # End-to-end: the [context, book] walk mutates nested chapters in place.
    book = {
        "sections": [
            {
                "Chapter": {
                    "content": "[a](inference-conformance-report.md)",
                    "sub_items": [{"Chapter": {"content": "[b](README.md)", "sub_items": []}}],
                }
            },
            {"Separator": None},
        ]
    }
    for section in book["sections"]:
        _walk(section)
    top = book["sections"][0]["Chapter"]
    if top["content"] != f"[a]({B}inference-conformance-report.md)":
        failures.append(f"  chapter walk: got {top['content']!r}")
    if top["sub_items"][0]["Chapter"]["content"] != f"[b]({B}README.md)":
        failures.append(f"  sub-chapter walk: got {top['sub_items'][0]['Chapter']['content']!r}")

    if failures:
        print("mdbook-rewrite-links self-test: FAIL", file=sys.stderr)
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(f"mdbook-rewrite-links self-test: PASS ({len(_REWRITE_CASES)} link cases + chapter walk)")
    return 0


def main() -> int:
    # `supports <renderer>` handshake: we support all renderers.
    if len(sys.argv) > 1 and sys.argv[1] == "supports":
        return 0
    if len(sys.argv) > 1 and sys.argv[1] == "--self-test":
        return _self_test()

    context_and_book = json.load(sys.stdin)
    # stdin is the 2-element array [context, book]; we mutate and emit `book`.
    book = context_and_book[1]
    for section in book.get("sections", []):
        _walk(section)
    json.dump(book, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
