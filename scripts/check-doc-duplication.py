#!/usr/bin/env python3
# [OPUS-5] Docs anti-drift: content-duplication detection across the published doc
# surface (issue #5019; specification: research/docs-site-single-sourcing-anti-drift.md
# §4, bead sq-w9sr). 🤖 SPARQ agent.
#
# WHY. The docs mandate is a site that cannot silently drift from the code and from the
# canonical prose. The single-sourcing MECHANISM already exists for the guide (mdBook
# `{{#include}}` / `{{#rustdoc_include}}`), but nothing DETECTED a hand-copy — and a
# hand-copy is precisely the thing that drifts, because only one of the two copies gets
# edited. The design record measured the surface and found the dominant duplicate kind
# is a code example copied between a `SKILL.md` and the crate `README.md` documenting
# the same API.
#
# The property that makes this gate cheap: injected content is NOT physically present
# twice in the source tree (an `{{#include}}`d region exists once, in the canonical
# file), so the scanner sees only hand-copies. It needs no knowledge of the injection
# mechanism, and its finding count is a direct measure of un-single-sourced content.
#
# TWO TIERS, matching the repo's advisory→gating probation convention:
#
#   * EXACT tier (default invocation) — GATING. An identical normalised block in two or
#     more files on the surface fails the build. Deterministic; the only false positives
#     are genuine boilerplate, which the escape hatches below cover.
#   * NEAR tier (`--near`) — ADVISORY, run with `--advisory` inside docs-quality.yml's
#     `docs-quality quick-advisory (advisory)` job. 8-gram Jaccard at or above
#     NEAR_THRESHOLD is REPORTED, never failed, while the measured backlog (research
#     record §3) is open. It promotes to HARD the way every other step in that job
#     promotes: by MOVING the step into `quick-gates` (advisory-registry criteria) — the
#     research record's "delete its registry entry" phrasing predates the sq-6vshe.20
#     consolidation that made this a step in a shared advisory job rather than its own.
#     Landing it gating on day one would mean either failing `main` immediately or
#     picking a threshold high enough to be decorative.
#
# NORMALISATION (research record §4). Strip YAML frontmatter, `BEGIN-INJECT`/`END-INJECT`
# regions (§5 — injected content is generated, not hand-copied), HTML comments, HTML
# tags, and link-reference definitions; split on blank lines; strip markdown link
# TARGETS while keeping link text; drop bare/auto-linked URLs; lowercase; collapse
# whitespace. The link-target strip is load-bearing: the same paragraph copied between
# two mounts usually differs ONLY in whether its links are repo-relative or absolute —
# which is exactly the difference the mdBook link-fixup preprocessor exists to erase. A
# comparator that keeps targets scores such a pair as merely "similar" and lets it
# through.
#
# ESCAPE HATCHES, following the house pattern (`terminology-allow` /
# `privacy-claims-allow` / `perf-neutrality-allow`):
#   * inline `<!-- doc-duplication-allow: <why> -->`, which exempts the NEXT content
#     block below it. The reason is mandatory — a bare marker is itself a finding, so
#     the opt-out stays auditable. A marker with no block below it is dead config and is
#     also a finding.
#   * scripts/doc-duplication-allowlist.json — entries naming the exact FILES and the
#     exact normalised-block DIGESTS they exempt (mirroring banned-terminology.json's
#     data-driven discipline). An entry that suppressed nothing is stale config and
#     fails, so the allowlist cannot quietly outlive the duplication it excused.
# Both hatches are deliberately narrow. Broad path exclusions to make a violation pass
# defeat the gate; fix the wording, or inject the block, instead.
#
# MEASURED at the chosen knobs on the surface as of this commit (reproduce with
# `python3 scripts/check-doc-duplication.py --stats`): 1 exact-tier group — the
# `sparq-vc` path-dependency stanza, allowlisted with a reason and a named follow-up —
# and 7 near-tier pairs. Those 7 are exactly the set the design record's §3 ad-hoc scan
# reported as remaining (its five surviving table rows, with the three-way mutual
# `*-wasm` row expanding to the 3 pairs it stands for), and the two the record names as
# largest — README.md ↔ crates/sparq-cli/README.md and skills/shacl-validation/SKILL.md
# ↔ crates/sparq-shacl/README.md — rank first and second here. So this committed
# comparator reproduces the record's measured baseline rather than redefining it.
#
# Usage:
#   check-doc-duplication.py                 # EXACT tier over the surface (GATING)
#   check-doc-duplication.py --near          # NEAR tier (8-gram Jaccard)
#   check-doc-duplication.py --advisory      # never exit non-zero on findings
#   check-doc-duplication.py --stats         # surface/block census, exit 0
#   check-doc-duplication.py --self-test     # hermetic both-direction fixtures
#
# Exit 0 = clean; exit 1 = findings (unless --advisory). stdlib-only, no network.

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from collections import defaultdict

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

ALLOWLIST_PATH = "scripts/doc-duplication-allowlist.json"

# --- The published doc surface (research record §4). Root markdown, the docs/ tree, the
#     mdBook SOURCES (book/src — never book/book, the rendered output), the skills tree,
#     and one README per crate.
#
#     Deliberately EXCLUDED: research/** (design records legitimately restate context,
#     and the markdownlint config already treats research/ as a separate tier),
#     .claude/** and AGENTS-worker-core.md (the agent-contract replication is INTENTIONAL
#     and is governed by the contract's own single-source rule), and any vendored subtree.
SURFACE_DIR_PREFIXES = ("docs/", "book/src/", "skills/")
CRATE_README_RE = re.compile(r"^crates/[^/]+/README\.md$")
EXCLUDED_PATHS = frozenset({"AGENTS-worker-core.md"})
# Only the INFIX spellings are kept: a root-level `vendor/x.md` is already off-surface
# because it is neither root-level markdown nor under a surface directory, so a
# `startswith` list alongside this one would be unreachable config.
EXCLUDED_SUBSTRINGS = ("/vendor/", "/node_modules/", "/target/")

# --- Knobs. Both were chosen on a MEASURED PLATEAU rather than at a boundary, which is
#     the check against "tuned until today's tree passes": sweeping WORD_FLOOR over
#     15 / 20 / 25 leaves BOTH tiers' verdicts identical (1 exact group, 7 near pairs),
#     so 20 sits mid-plateau and no small edit to the corpus flips a verdict.
#
#     WORD_FLOOR is the prose-length floor. Below roughly 15 words the surface stops
#     being content and becomes STRUCTURE — and specifically structure that another
#     gate MANDATES: check-readme-template.py requires every crate README to carry the
#     same `## 🚀 Quickstart` / `## ✨ Features` / `## 📚 Learn more` / License headings,
#     and skills carry a shared `## See also`. Measured: at floor 0 this surface reports
#     44 exact groups, of which 43 are sub-sentence structure (only the one real
#     duplicate survives a floor of 15) and the LARGEST are exactly those mandated
#     headings. A gate without a floor would therefore contradict the template gate
#     outright — satisfying one would red the other. The floor is what keeps the two
#     consistent, not a leniency knob.
#
#     (The crate-README badge stanza needs no floor at all — it is pure HTML markup, so
#     normalisation reduces it to ZERO content words and it never reaches the
#     comparator. That is structural, not threshold-dependent.)
WORD_FLOOR = 20
#     NGRAM/NEAR_THRESHOLD reproduce the record's §3 near-duplicate scan: 8-gram Jaccard
#     at 0.5 yields exactly the record's remaining pair set (see the MEASURED note).
NGRAM = 8
NEAR_THRESHOLD = 0.5

# --- Regexes. Every "strip" that happens BEFORE block splitting must preserve line
#     structure, or reported line numbers drift off the real content.
FRONTMATTER_RE = re.compile(r"\A---\n.*?\n---\n", re.S)
HTML_COMMENT_RE = re.compile(r"<!--.*?-->", re.S)
INJECT_REGION_RE = re.compile(
    r"<!--\s*BEGIN-INJECT:.*?-->.*?<!--\s*END-INJECT\s*-->", re.S)
REF_DEF_RE = re.compile(r"^[ \t]*\[[^\]]+\]:[ \t]*\S+.*$", re.M)
ALLOW_MARKER_RE = re.compile(r"<!--\s*doc-duplication-allow:(.*?)-->", re.S)

# Link-target stripping (applied to a single block, AFTER splitting).
INLINE_LINK_RE = re.compile(r"!?\[([^\[\]]*)\]\([^()]*\)")
REF_LINK_RE = re.compile(r"!?\[([^\[\]]*)\]\[[^\[\]]*\]")
HTML_TAG_RE = re.compile(r"<[^<>]+>")
URL_RE = re.compile(r"(?:https?|mailto):\S+")


# ---------------------------------------------------------------------------
# Normalisation
# ---------------------------------------------------------------------------

def _blank_out(match: re.Match[str]) -> str:
    """Replace a matched region with the same number of newlines it spanned, so every
    later line number still points at the real source line."""
    return "\n" * match.group(0).count("\n")


def strip_line_preserving(text: str) -> str:
    """Remove frontmatter, INJECT regions, HTML comments and link-reference definitions
    without moving any surviving line."""
    text = FRONTMATTER_RE.sub(_blank_out, text)
    text = INJECT_REGION_RE.sub(_blank_out, text)
    text = HTML_COMMENT_RE.sub(_blank_out, text)
    text = REF_DEF_RE.sub("", text)
    return text


def normalise_block(chunk: str) -> str:
    """Normalise one block to its comparable content form.

    Link TEXT survives; link TARGETS, bare URLs and HTML markup do not.
    """
    prev = None
    # Nested constructs (`[![alt](img)](href)`) need more than one pass; iterate to a
    # fixpoint rather than guessing a pass count.
    while prev != chunk:
        prev = chunk
        chunk = INLINE_LINK_RE.sub(r"\1", chunk)
        chunk = REF_LINK_RE.sub(r"\1", chunk)
    chunk = HTML_TAG_RE.sub(" ", chunk)
    chunk = URL_RE.sub(" ", chunk)
    return " ".join(chunk.lower().split())


def digest(normalised: str) -> str:
    """Short, stable identity of a normalised block — what an allowlist entry names."""
    return hashlib.sha256(normalised.encode("utf-8")).hexdigest()[:16]


class Block:
    __slots__ = ("path", "line", "text", "words", "grams", "digest", "exempt")

    def __init__(self, path: str, line: int, text: str) -> None:
        self.path = path
        self.line = line
        self.text = text
        self.words = text.split()
        self.grams = {
            tuple(self.words[i:i + NGRAM])
            for i in range(max(1, len(self.words) - NGRAM + 1))
        }
        self.digest = digest(text)
        self.exempt = False

    def __repr__(self) -> str:  # pragma: no cover - debugging aid
        return f"<Block {self.path}:{self.line} {self.digest}>"


def parse_document(path: str, raw: str) -> tuple[list[Block], list[str]]:
    """Split one document into comparable blocks.

    Returns (blocks, marker_findings). A `doc-duplication-allow` marker exempts the next
    content block below it; a marker with an empty reason, or with no block below it, is
    itself a finding (an unauditable or dead opt-out).
    """
    findings: list[str] = []

    # Markers are read from the RAW text: normalisation strips HTML comments, so by the
    # time blocks exist the marker is gone. Record the line each marker ENDS on.
    markers: list[tuple[int, str]] = []
    for m in ALLOW_MARKER_RE.finditer(raw):
        end_line = raw.count("\n", 0, m.end()) + 1
        reason = m.group(1).strip()
        if not reason:
            findings.append(
                f"{path}:{end_line}: empty doc-duplication-allow marker "
                f"(a reason is required, e.g. "
                f"`<!-- doc-duplication-allow: shared install stanza, tracked in #123 -->`)")
            continue
        markers.append((end_line, reason))

    stripped = strip_line_preserving(raw)

    blocks: list[Block] = []
    current: list[str] = []
    start = 0
    for lineno, line in enumerate(stripped.split("\n"), start=1):
        if line.strip():
            if not current:
                start = lineno
            current.append(line)
            continue
        if current:
            normalised = normalise_block("\n".join(current))
            if len(normalised.split()) >= WORD_FLOOR:
                blocks.append(Block(path, start, normalised))
            current = []
    if current:
        normalised = normalise_block("\n".join(current))
        if len(normalised.split()) >= WORD_FLOOR:
            blocks.append(Block(path, start, normalised))

    # Attach each marker to the first block that starts below it.
    for end_line, reason in markers:
        target = next((b for b in blocks if b.line > end_line), None)
        if target is None:
            findings.append(
                f"{path}:{end_line}: doc-duplication-allow marker exempts nothing "
                f"(no content block follows it) — delete it. Reason given: {reason!r}")
            continue
        target.exempt = True

    return blocks, findings


# ---------------------------------------------------------------------------
# Allowlist
# ---------------------------------------------------------------------------

class Allowlist:
    """File+digest pairs that are permitted to repeat, each carrying a `why`.

    An entry suppresses a finding only when EVERY file in the finding is listed AND
    EVERY block digest in the finding is listed — so an entry cannot silently excuse a
    second, unrelated duplication between the same two files.
    """

    def __init__(self, entries: list[dict]) -> None:
        self.entries = entries
        self.used: set[int] = set()

    @staticmethod
    def from_obj(obj: dict) -> tuple["Allowlist", list[str]]:
        problems: list[str] = []
        entries = obj.get("allow", [])
        if not isinstance(entries, list):
            return Allowlist([]), [f"{ALLOWLIST_PATH}: `allow` must be a list"]
        for i, e in enumerate(entries):
            if not isinstance(e, dict):
                problems.append(f"{ALLOWLIST_PATH}: entry {i} is not an object")
                continue
            for field in ("files", "digests", "why"):
                if not e.get(field):
                    problems.append(
                        f"{ALLOWLIST_PATH}: entry {i} is missing a non-empty `{field}`")
        return Allowlist(entries), problems

    def covers(self, files: set[str], digests: set[str]) -> str | None:
        for i, e in enumerate(self.entries):
            if not isinstance(e, dict):
                continue
            if files <= set(e.get("files") or ()) and digests <= set(e.get("digests") or ()):
                self.used.add(i)
                return str(e.get("why", ""))
        return None

    def stale(self) -> list[str]:
        return [
            f"{ALLOWLIST_PATH}: entry {i} ({', '.join(e.get('files') or [])}) suppressed "
            f"nothing — the duplication it excused is gone, so delete the entry."
            for i, e in enumerate(self.entries)
            if isinstance(e, dict) and i not in self.used
        ]


# ---------------------------------------------------------------------------
# The two tiers
# ---------------------------------------------------------------------------

def exact_groups(blocks: list[Block]) -> list[list[Block]]:
    """Blocks whose normalised text is identical across two or more FILES."""
    by_text: dict[str, list[Block]] = defaultdict(list)
    for b in blocks:
        if not b.exempt:
            by_text[b.text].append(b)
    groups = [g for g in by_text.values() if len({b.path for b in g}) > 1]
    groups.sort(key=lambda g: (-len(g), g[0].path, g[0].line))
    return groups


def near_pairs(blocks: list[Block]) -> list[tuple[float, Block, Block]]:
    """Cross-file block pairs whose 8-gram Jaccard similarity is >= NEAR_THRESHOLD.

    Exact duplicates (similarity 1.0) are reported by the EXACT tier and excluded here,
    so the two tiers never double-report the same finding.

    Candidates come from an inverted 8-gram index: only blocks sharing at least one
    8-gram can clear any positive threshold. An 8-gram is highly discriminative on
    prose, so the buckets stay tiny — measured on this surface, 2420 blocks produce
    ~273k distinct 8-grams, the largest bucket holds 9 blocks, and the whole tier costs
    ~3.8k pair comparisons.
    """
    live = [b for b in blocks if not b.exempt and b.grams]
    index: dict[tuple, list[int]] = defaultdict(list)
    for i, b in enumerate(live):
        for g in b.grams:
            index[g].append(i)

    candidates: set[tuple[int, int]] = set()
    for bucket in index.values():
        if len(bucket) < 2:
            continue
        for a in range(len(bucket)):
            for c in range(a + 1, len(bucket)):
                candidates.add((bucket[a], bucket[c]))

    out: list[tuple[float, Block, Block]] = []
    for i, j in candidates:
        x, y = live[i], live[j]
        if x.path == y.path or x.text == y.text:
            continue
        sim = len(x.grams & y.grams) / len(x.grams | y.grams)
        if sim >= NEAR_THRESHOLD:
            first, second = sorted((x, y), key=lambda b: (b.path, b.line))
            out.append((sim, first, second))
    out.sort(key=lambda t: (-t[0], t[1].path, t[1].line))
    return out


def scan(documents: list[tuple[str, str]], allowlist: Allowlist,
         near: bool) -> tuple[list[str], list[Block]]:
    """Run one tier over `documents` and return (findings, blocks).

    Findings are ready-to-print strings. Marker-hygiene findings are reported by BOTH
    tiers: an unauditable opt-out is a defect regardless of which tier is running.

    BOTH tiers are always computed, even though only the requested one is reported,
    because allowlist usage — and therefore the stale-entry verdict — must not depend on
    which invocation is looking. An entry excusing an exact repeat would otherwise be
    reported as stale by every `--near` run.
    """
    findings: list[str] = []
    blocks: list[Block] = []
    for path, raw in documents:
        doc_blocks, doc_findings = parse_document(path, raw)
        blocks.extend(doc_blocks)
        findings.extend(doc_findings)

    exact_reported: list[str] = []
    for group in exact_groups(blocks):
        files = {b.path for b in group}
        if allowlist.covers(files, {group[0].digest}) is not None:
            continue
        where = ", ".join(f"{b.path}:{b.line}" for b in group)
        exact_reported.append(
            f"{group[0].path}:{group[0].line}: identical block repeated in "
            f"{len(files)} files [{where}]\n"
            f"    digest: {group[0].digest}\n"
            f"    {group[0].text[:110]}…")

    near_reported: list[str] = []
    for sim, a, b in near_pairs(blocks):
        if allowlist.covers({a.path, b.path}, {a.digest, b.digest}) is not None:
            continue
        near_reported.append(
            f"{a.path}:{a.line}: near-duplicate ({sim:.0%} 8-gram Jaccard) of "
            f"{b.path}:{b.line}\n"
            f"    digests: {a.digest} / {b.digest}\n"
            f"    {a.text[:110]}…")

    findings.extend(near_reported if near else exact_reported)
    findings.extend(allowlist.stale())
    return findings, blocks


# ---------------------------------------------------------------------------
# Surface discovery
# ---------------------------------------------------------------------------

def in_surface(path: str) -> bool:
    if path in EXCLUDED_PATHS:
        return False
    if any(s in path for s in EXCLUDED_SUBSTRINGS):
        return False
    if "/" not in path:
        return True
    if path.startswith(SURFACE_DIR_PREFIXES):
        return True
    return bool(CRATE_README_RE.match(path))


def surface_documents(root: str) -> list[tuple[str, str]]:
    """Every git-tracked markdown file on the published doc surface, as (path, text)."""
    listing = subprocess.run(
        ["git", "-C", root, "ls-files", "*.md"],
        capture_output=True, text=True, check=True,
    ).stdout.splitlines()
    docs: list[tuple[str, str]] = []
    for path in sorted(p for p in listing if in_surface(p)):
        with open(os.path.join(root, path), encoding="utf-8") as fh:
            docs.append((path, fh.read()))
    return docs


def load_allowlist(root: str) -> tuple[Allowlist, list[str]]:
    full = os.path.join(root, ALLOWLIST_PATH)
    if not os.path.exists(full):
        return Allowlist([]), [f"{ALLOWLIST_PATH}: missing — the gate's allowlist must exist"]
    with open(full, encoding="utf-8") as fh:
        return Allowlist.from_obj(json.load(fh))


# ---------------------------------------------------------------------------
# Self-test (hermetic, both directions — research record §4)
# ---------------------------------------------------------------------------

FILLER = " ".join(f"word{i}" for i in range(30))
OTHER_FILLER = " ".join(f"token{i}" for i in range(30))
# Deliberately longer than NGRAM words, so a pair sharing it has a NON-ZERO similarity.
SHARED_SPAN = ("a shared opening span of some length that runs on for a while so its "
               "eight grams count")


def _case(name: str, documents, near, allow_obj, expect_hits: bool) -> str | None:
    allowlist, problems = Allowlist.from_obj(allow_obj)
    findings, _ = scan(documents, allowlist, near=near)
    findings = problems + findings
    if bool(findings) != expect_hits:
        return (f"  ✗ {name}: expected {'a finding' if expect_hits else 'silence'}, got "
                f"{findings if findings else 'nothing'}")
    return None


def self_test() -> int:
    empty: dict = {"allow": []}
    dup = f"This paragraph is copied by hand. {FILLER}"
    distinct_a = f"This paragraph is unique to the first document. {FILLER}"
    distinct_b = f"An entirely different paragraph lives here. {OTHER_FILLER}"
    dup_digest = digest(normalise_block(dup))

    # A near pair: same opening, divergent tail — below exact, above NEAR_THRESHOLD.
    near_a = f"shared opening sentence for the near pair. {FILLER}"
    near_b = f"shared opening sentence for the near pair. {FILLER} plus a short tail."

    failures = [f for f in [
        # --- EXACT tier, must FIRE ---
        _case("exact: a hand-copied block across two files is caught",
              [("a.md", dup), ("b.md", dup)], False, empty, True),
        _case("exact: link TARGETS are stripped, so repo-relative vs absolute still matches",
              [("a.md", f"See [the guide](../book/src/x.md) for details. {FILLER}"),
               ("b.md", f"See [the guide](https://example.org/book/x.html) for details. {FILLER}")],
              False, empty, True),
        # The reference DEFINITION is glued to the paragraph (no blank line) on purpose:
        # that is the only placement where it lands inside a content block, and so the
        # only placement where failing to strip it would mask a copy.
        _case("exact: REFERENCE-style links — both the use and its definition are targets",
              [("a.md", f"See [the guide][rel] for details. {FILLER}\n[rel]: ../book/src/x.md"),
               ("b.md", f"See [the guide][abs] for details. {FILLER}\n[abs]: https://example.org/x")],
              False, empty, True),
        _case("exact: a NESTED badge link ([![alt](img)](href)) reduces to its text",
              [("a.md", f"[![build](../img/ci.svg)](../actions) {FILLER}"),
               ("b.md", f"[![build](https://x.test/ci.svg)](https://x.test/actions) {FILLER}")],
              False, empty, True),
        _case("exact: a BARE url is a target too — prose differing only in it still matches",
              [("a.md", f"Full details live at https://example.org/a/x. {FILLER}"),
               ("b.md", f"Full details live at https://example.com/b/y. {FILLER}")],
              False, empty, True),
        _case("exact: case differences do not hide a copy",
              [("a.md", dup), ("b.md", dup.upper())], False, empty, True),
        _case("exact: re-wrapping / re-indenting does not hide a copy",
              [("a.md", dup),
               ("b.md", dup.replace(". ", ".\n   ").replace(" ", "  ", 3))],
              False, empty, True),
        _case("exact: a bare doc-duplication-allow marker is itself a finding",
              [("a.md", f"<!-- doc-duplication-allow: -->\n{dup}"), ("b.md", dup)],
              False, empty, True),
        _case("exact: a marker with no block below it is dead config",
              [("a.md", f"{distinct_a}\n\n<!-- doc-duplication-allow: nothing follows -->\n")],
              False, empty, True),
        _case("exact: an allowlist entry that suppressed nothing is stale",
              [("a.md", distinct_a), ("b.md", distinct_b)], False,
              {"allow": [{"files": ["a.md", "b.md"], "digests": ["deadbeefdeadbeef"],
                          "why": "obsolete"}]}, True),
        _case("exact: an allowlist entry naming the wrong digest does NOT suppress",
              [("a.md", dup), ("b.md", dup)], False,
              {"allow": [{"files": ["a.md", "b.md"], "digests": ["0000000000000000"],
                          "why": "wrong digest"}]}, True),
        _case("exact: an entry naming the right digest but the wrong FILES does NOT suppress",
              [("a.md", dup), ("b.md", dup)], False,
              {"allow": [{"files": ["a.md", "c.md"], "digests": [dup_digest],
                          "why": "excuses a different pair"}]}, True),
        _case("exact: an allowlist entry missing `why` is rejected",
              [("a.md", dup), ("b.md", dup)], False,
              {"allow": [{"files": ["a.md", "b.md"], "digests": [dup_digest]}]}, True),
        # --- EXACT tier, must stay QUIET ---
        _case("exact: distinct prose is not reported",
              [("a.md", distinct_a), ("b.md", distinct_b)], False, empty, False),
        _case("exact: a short block (below the prose floor) is not content",
              [("a.md", "Install it."), ("b.md", "Install it.")], False, empty, False),
        _case("exact: a repeat WITHIN one file is not cross-file duplication",
              [("a.md", f"{dup}\n\n{dup}")], False, empty, False),
        _case("exact: identical YAML frontmatter is metadata, not duplicated content",
              [("a.md", f"---\nname: a\ndescription: {FILLER}\n---\n\n{distinct_a}"),
               ("b.md", f"---\nname: a\ndescription: {FILLER}\n---\n\n{distinct_b}")],
              False, empty, False),
        _case("exact: an inline marker exempts the block below it",
              [("a.md", f"<!-- doc-duplication-allow: shared install stanza -->\n{dup}"),
               ("b.md", dup)], False, empty, False),
        _case("exact: an allowlist entry naming both files and the digest suppresses it",
              [("a.md", dup), ("b.md", dup)], False,
              {"allow": [{"files": ["a.md", "b.md"], "digests": [dup_digest],
                          "why": "tracked follow-up"}]}, False),
        _case("exact: a BEGIN-INJECT region is generated, not hand-copied",
              [("a.md", f"<!-- BEGIN-INJECT: c.md#x -->\n{dup}\n<!-- END-INJECT -->"),
               ("b.md", f"<!-- BEGIN-INJECT: c.md#x -->\n{dup}\n<!-- END-INJECT -->")],
              False, empty, False),
        _case("exact: an INJECT region and a HAND-copy of the same text still differ",
              [("a.md", f"<!-- BEGIN-INJECT: c.md#x -->\n{dup}\n<!-- END-INJECT -->"),
               ("b.md", dup), ("c.md", dup)], False, empty, True),
        # --- NEAR tier, must FIRE ---
        _case("near: a divergent-tail copy clears the Jaccard threshold",
              [("a.md", near_a), ("b.md", near_b)], True, empty, True),
        # --- NEAR tier, must stay QUIET ---
        _case("near: unrelated prose is below the Jaccard threshold",
              [("a.md", distinct_a), ("b.md", distinct_b)], True, empty, False),
        # Pins NEAR_THRESHOLD from BELOW as well as above: a genuinely shared span that
        # is nonetheless swamped by divergent bodies is a quotation, not a copy. This
        # pair scores ~0.18 — safely inside (0, NEAR_THRESHOLD), so it stays quiet at
        # 0.5 and would be REPORTED if the threshold were loosened much. It must share a
        # span LONGER than NGRAM words, or it would score a vacuous 0.0 and merely
        # restate the unrelated-prose case above.
        _case("near: a shared span swamped by divergent bodies is a quotation, not a copy",
              [("a.md", f"{SHARED_SPAN}. {FILLER}"),
               ("b.md", f"{SHARED_SPAN}. " + " ".join(f"extra{i}" for i in range(20)))],
              True, empty, False),
        # Pins NGRAM: an n-gram comparator must see word ORDER. A bag-of-words
        # comparator (NGRAM=1) would score this pair as identical.
        _case("near: the same words in a different ORDER are not a copy",
              [("a.md", dup), ("b.md", " ".join(reversed(dup.split())))],
              True, empty, False),
        _case("near: an EXACT duplicate is left to the exact tier, not double-reported",
              [("a.md", dup), ("b.md", dup)], True, empty, False),
        _case("near: an entry excusing an EXACT repeat is not called stale by this tier",
              [("a.md", dup), ("b.md", dup)], True,
              {"allow": [{"files": ["a.md", "b.md"], "digests": [dup_digest],
                          "why": "tracked follow-up"}]}, False),
        _case("near: two similar blocks in the SAME file are not cross-file drift",
              [("a.md", f"{near_a}\n\n{near_b}")], True, empty, False),
        _case("near: the inline marker exempts a block from this tier too",
              [("a.md", f"<!-- doc-duplication-allow: intentional -->\n{near_a}"),
               ("b.md", near_b)], True, empty, False),
    ] if f]

    # The SURFACE boundary is a property in its own right: an off-surface tree becoming
    # scannable would red `main` on duplication the record deliberately sanctions
    # (research/ restates context; the agent contract is replicated on purpose), and an
    # on-surface tree silently dropping out would make the gate vacuous for it. A
    # findings-based fixture cannot pin this — one extra in-surface file has nothing to
    # pair with — so assert the predicate directly.
    for path, expected in [
        ("README.md", True), ("AGENTS.md", True),
        ("docs/branch-protection.md", True), ("book/src/introduction.md", True),
        ("skills/cli/SKILL.md", True), ("crates/sparq-engine/README.md", True),
        ("research/x.md", False), (".claude/agents/y.md", False),
        ("AGENTS-worker-core.md", False), ("bench/CATALOG.md", False),
        ("crates/sparq-engine/docs/notes.md", False), ("book/book/print.md", False),
        ("vendor/x.md", False), ("crates/sparq-engine/vendor/x.md", False),
        # The infix exclusions, exercised where they are actually REACHABLE — i.e.
        # inside a surface directory, which is the only place they can bite.
        ("docs/vendor/oxigraph/README.md", False),
        ("skills/cli/node_modules/pkg/README.md", False),
        ("book/src/target/generated.md", False),
    ]:
        if in_surface(path) is not expected:
            failures.append(f"  ✗ surface: in_surface({path!r}) should be {expected}")

    # The crate-README badge stanza is pure HTML markup, and the header claims it
    # reaches the comparator as ZERO content words — which is what stops 60-odd
    # identical badge blocks being reported as duplication. Assert that directly rather
    # than relying on the prose floor to hide it.
    badge = ('<p>\n'
             '  <a href="https://crates.io/crates/sparq-engine">'
             '<img src="https://img.shields.io/crates/v/sparq-engine.svg" alt="crates.io"></a>\n'
             '  <a href="https://docs.rs/sparq-engine">'
             '<img src="https://docs.rs/sparq-engine/badge.svg" alt="docs.rs"></a>\n'
             '</p>')
    if normalise_block(badge) != "":
        failures.append(f"  ✗ badge stanza: expected zero content words, got "
                        f"{normalise_block(badge)!r}")

    # Line numbers must survive every line-preserving strip, or a finding points at the
    # wrong place. (The frontmatter + comment + INJECT strips are the risk.)
    doc = ("---\ntitle: x\n---\n\n<!-- a comment\nspanning lines -->\n\n"
           "<!-- BEGIN-INJECT: c.md#x -->\ninjected\n<!-- END-INJECT -->\n\n"
           f"{distinct_a}\n")
    blocks, _ = parse_document("a.md", doc)
    expected_line = doc.split("\n").index(distinct_a) + 1
    if not blocks or blocks[0].line != expected_line:
        failures.append(f"  ✗ line numbers: expected block at line {expected_line}, "
                        f"got {[b.line for b in blocks]}")

    print("check-doc-duplication self-test")
    if failures:
        print("\n".join(failures))
        print(f"\n{len(failures)} self-test failure(s).")
        return 1
    print("  all cases pass (exact + near, both directions, incl. line-number fidelity)")
    return 0


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def write_summary(tier: str, findings: list[str], blocks: list[Block],
                  docs: int, advisory: bool) -> None:
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if not path:
        return
    label = f"doc-duplication ({tier} tier{', advisory' if advisory else ''})"
    with open(path, "a", encoding="utf-8") as fh:
        if not findings:
            fh.write(f"### {label}: clean ✅\n\n"
                     f"{len(blocks)} content block(s) across {docs} doc(s) on the "
                     f"published surface; no un-single-sourced duplication.\n\n")
            return
        fh.write(f"### {label}: {len(findings)} finding(s)"
                 f"{' — reported, not blocking' if advisory else ' ❌'}\n\n")
        fh.write("Single-source the block instead of copying it: `{{#include}}` it "
                 "(guide pages), move a code example into `crates/<crate>/examples/` "
                 "and inject it, or — for genuine boilerplate — annotate it "
                 "`<!-- doc-duplication-allow: <why> -->`. See "
                 "`research/docs-site-single-sourcing-anti-drift.md` §4–§6.\n\n")
        fh.write("```\n")
        for f in findings[:40]:
            fh.write(f + "\n")
        fh.write("```\n")
        if len(findings) > 40:
            fh.write(f"\n…and {len(findings) - 40} more (see the job log).\n")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--near", action="store_true",
                    help="run the NEAR tier (8-gram Jaccard) instead of the EXACT tier")
    ap.add_argument("--advisory", action="store_true",
                    help="report findings but always exit 0")
    ap.add_argument("--stats", action="store_true",
                    help="print the surface/block census and exit 0")
    ap.add_argument("--self-test", action="store_true",
                    help="run the hermetic both-direction fixtures and exit")
    ap.add_argument("--root", default=REPO_ROOT, help="repo root to scan")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    documents = surface_documents(args.root)
    allowlist, problems = load_allowlist(args.root)

    if args.stats:
        blocks: list[Block] = []
        for path, raw in documents:
            blocks.extend(parse_document(path, raw)[0])
        print(f"surface: {len(documents)} doc(s), {len(blocks)} content block(s) "
              f"(>= {WORD_FLOOR} normalised words)")
        print(f"exact groups: {len(exact_groups(blocks))}; "
              f"near pairs (>= {NEAR_THRESHOLD:.0%} {NGRAM}-gram Jaccard): "
              f"{len(near_pairs(blocks))}")
        return 0

    tier = "near" if args.near else "exact"
    findings, blocks = scan(documents, allowlist, near=args.near)
    findings = problems + findings

    for f in findings:
        print(f)
    print(f"\ndoc-duplication ({tier} tier): {len(findings)} finding(s) across "
          f"{len(blocks)} content block(s) in {len(documents)} doc(s).")

    write_summary(tier, findings, blocks, len(documents), args.advisory)

    if findings and not args.advisory:
        print("::error::documentation content is duplicated across the published doc "
              "surface — single-source it (research/docs-site-single-sourcing-anti-drift.md "
              "§4), or justify the repeat with an inline "
              "`<!-- doc-duplication-allow: <why> -->` marker or an entry in "
              f"{ALLOWLIST_PATH}.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
