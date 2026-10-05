#!/usr/bin/env bash
# [OPUS-5] issue #5022 — shared mdBook guide builder (build + broken-include teeth).
#
# WHY THIS IS A SCRIPT AND NOT AN INLINE `run:` BLOCK: the guide is now built in TWO
# workflows — docs.yml VALIDATES it on PRs that touch the guide or any embedded source,
# and pages.yml BUILDS the very same guide into the published Pages artifact under
# /guide/ (the mount decided in issue #5022; see research/docs-site-single-sourcing-anti-drift.md
# §7 option (a)). If those two copies of the build command drift, the guide that PRs
# validate stops being the guide that ships. One script, called by both, removes that
# drift class; scripts/check-guide-publish-wiring.py gates that both lanes keep calling it.
#
# THE TEETH: mdBook EXITS 0 even on a broken {{#include}} / missing anchor
# (rust-lang/mdBook#1094) — it logs `[ERROR] ... ` and renders the page with the literal
# include directive still in it. The guide's whole single-sourcing design
# ({{#include}} of README.md / skills/*/SKILL.md anchors, {{#rustdoc_include}} of a
# compiled example) rests on those includes resolving, so a silent [ERROR] would publish a
# guide full of raw directives. This script therefore captures the build log and FAILS on
# any `[ERROR]` line in addition to mdbook's own exit code.
#
# It does NOT run `mdbook test` (the ```rust fence compile-check): that needs a Rust
# toolchain and belongs to the validate lane (docs.yml), not the publish path.
#
# EXIT: 0 iff mdbook succeeded, logged no [ERROR] line, and produced a rendered index.html.
#
# Usage:
#   scripts/build-guide.sh              # build ./book -> ./book/book
#   scripts/build-guide.sh <book-dir>   # build <book-dir> -> <book-dir>/book
#
# A hermetic self-test of the teeth lives in scripts/tests/test_build_guide.sh.
set -euo pipefail

BOOK_DIR="${1:-book}"

if ! command -v mdbook >/dev/null 2>&1; then
  echo "build-guide: 'mdbook' not found on PATH." >&2
  echo "  Install it (cargo install mdbook --locked) or run via the pinned" >&2
  echo "  taiki-e/install-action step CI uses (tool: mdbook@<version>)." >&2
  exit 127
fi

if [ ! -f "$BOOK_DIR/book.toml" ]; then
  echo "build-guide: '$BOOK_DIR/book.toml' not found — not an mdBook source dir." >&2
  exit 1
fi

# `build-dir = "book"` in book.toml, so the render lands at <book-dir>/book.
RENDER_DIR="$BOOK_DIR/book"
log="$(mktemp)"
trap 'rm -f "$log"' EXIT

echo "build-guide: mdbook build $BOOK_DIR"
# pipefail (set above) propagates mdbook's own non-zero exit through the tee.
mdbook build "$BOOK_DIR" 2>&1 | tee "$log"

if grep -q '\[ERROR\]' "$log"; then
  echo "::error::mdbook build emitted [ERROR] (broken {{#include}} / missing anchor) — see log above" >&2
  echo "build-guide: FAIL — mdbook exits 0 on a broken include, so the log is the only signal." >&2
  exit 1
fi

if [ ! -f "$RENDER_DIR/index.html" ]; then
  echo "::error::mdbook build produced no $RENDER_DIR/index.html" >&2
  exit 1
fi

echo "build-guide: OK — clean build (no [ERROR] lines), rendered to $RENDER_DIR/"
