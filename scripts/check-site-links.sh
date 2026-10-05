#!/usr/bin/env bash
# sq-d8or — anti-drift CI glue: offline link-check over the BUILT docs-site HTML.
#
# WHY: the docs-quality `internal-links` job (bead sq-5fd1) runs lychee --offline over the
# repo's MARKDOWN, but the published GitHub Pages site (the Next.js static export under
# site/out/, plus the in-site rendered papers) is HTML — its relative links + #anchors were
# never link-checked, so a renamed route or a stale cross-link could ship broken to the live
# site. sq-d8or extends the lychee scope to that built HTML. This script is the shared,
# self-testable core (CLI; no Action) so the same check runs in CI (pages.yml, after
# `next build`) and locally (`scripts/check-site-links.sh site/out`).
#
# WHAT: lychee --offline (no network, deterministic) over every *.html under the export,
# validating relative links AND heading anchors (--include-fragments).
#
# sq-uj38w — the org-migration Pages cutover moved the site to the ROOT of the custom
# domain https://sparq.jeswr.org/, so the Pages build is now ROOT-RELATIVE (basePath '',
# trailingSlash:true — site/next.config.ts + the pages.yml "Build static site" step set
# NEXT_PUBLIC_BASE_PATH=''). Every INTERNAL link in the emitted HTML is now an absolute path like
#   /about/   (and the bare root link /).
# `--root-dir <out>` resolves those absolute paths against the export root, and lychee resolves a
# fragment-FREE directory link (/about/) straight to <out>/about/index.html — so NO prefix-strip
# remap is needed any more (the old `/sparq` rules 1 & 2 are gone with the sub-path).
#
# sq-bpoey / sq-uj38w — ONE remap survives, for CROSS-PAGE path+fragment links like
# `/capabilities/#privacy` (homepage theme grid + the removed /surface/* redirect stubs point at
# /capabilities/#<theme>). With `--root-dir`, lychee resolves a *directory* link that carries a
# fragment to the bare directory path (e.g. file://<out>/capabilities) and does NOT fall through to
# that directory's index.html when locating the `#fragment` heading — so it reports "Cannot find
# fragment" for EVERY such link. (Verified empirically against CI lychee 0.23.0.) The remap rewrites
# a fragment-carrying DIRECTORY link to that directory's index.html for the fragment lookup:
#   file://<out>/<dir-path>[/]#<frag> -> file://<out>/<dir-path>/index.html#<frag>
# The final path segment is matched as `[^#/.]+` (no dot) so it targets ONLY directory links and
# NEVER a same-page anchor, which lychee resolves against the CURRENT file as
# file://<out>/<path>/index.html#<frag> — that already ends in `.html`, so the no-dot last-segment
# guard leaves it untouched. (Pre-cutover the discriminator was the `/sparq/` prefix; at root the
# no-extension last segment is the discriminator.) A trailing slash before the fragment is consumed
# by the optional `/?` so both `/dir/#frag` and the slashless `/dir#frag` normalisation are covered.
#
# EXIT: non-zero iff lychee finds a broken internal link/anchor (CI-gating). A self-test of
# the remap + teeth lives in scripts/tests/test_check_site_links.sh.
set -euo pipefail

OUT_DIR="${1:-site/out}"

if ! command -v lychee >/dev/null 2>&1; then
  echo "check-site-links: 'lychee' not found on PATH." >&2
  echo "  Install it (cargo install lychee) or run via the pinned lycheeverse action in CI." >&2
  exit 127
fi

if [ ! -d "$OUT_DIR" ]; then
  echo "check-site-links: export directory '$OUT_DIR' not found." >&2
  echo "  Build it first:  (cd site && npm ci && npm run build)" >&2
  exit 1
fi

# Absolute path so the file:// remap patterns are unambiguous regardless of CWD.
OUT_ABS="$(cd "$OUT_DIR" && pwd)"

echo "check-site-links: lychee --offline over ${OUT_ABS}/**/*.html (root-relative basePath, sq-uj38w)"

# --root-dir lets lychee resolve absolute (/...) links against the export root. TWO --remap
# rules cover the directory-with-fragment cases lychee can't resolve on its own:
#   1. a SUB-DIRECTORY fragment link (/capabilities/#privacy) -> that dir's index.html. The no-dot
#      last-segment `[^#/.]+` targets directory links only, never a same-page anchor (which lychee
#      already resolves against the CURRENT `.html` file).
#   2. the BARE-ROOT fragment link (/#how-it-runs — e.g. the /about RedirectStub back to the home
#      page's #how-it-runs strip) -> the ROOT index.html. This is the root-basePath analogue of the
#      old `/sparq` "rule 2"; without it lychee resolves `/` to the export dir and cannot find the
#      fragment. `/?` makes it match both `/#frag` and the slashless `#frag` normalisation.
# `dev/` (the overlaid benchmark dashboard, a separate first-party artifact written by bench.yml
# onto benchmark-data) is excluded: it is not part of THIS site's source and carries its own links.
# `guide/` (the overlaid mdBook guide, issue #5022) is excluded as an INPUT for the same reason: its
# broken-include gate is scripts/build-guide.sh, and its link-fixup output is not lychee-offline
# shaped. It must still EXIST under the export when this runs (pages.yml overlays it first), so the
# site's own links INTO /guide/ (the app-shell Docs link) are resolved and checked against it.
lychee \
  --offline \
  --include-fragments \
  --no-progress \
  --root-dir "$OUT_ABS" \
  --remap "file://${OUT_ABS}/((?:[^#]*/)?[^#/.]+)/?#(.+) file://${OUT_ABS}/\$1/index.html#\$2" \
  --remap "file://${OUT_ABS}/?#(.+) file://${OUT_ABS}/index.html#\$1" \
  --exclude-path "${OUT_ABS}/dev" \
  --exclude-path "${OUT_ABS}/guide" \
  "${OUT_ABS}/**/*.html"
