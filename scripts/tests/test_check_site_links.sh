#!/usr/bin/env bash
# Hermetic both-direction self-test for scripts/check-site-links.sh (bead
# sq-d8or — the built-docs-site link gate).
#
# WHY: the gate's value rests on TWO load-bearing behaviours that are easy to break
# silently when the export's basePath / link shape changes:
#   1. CORRECTNESS — the root-relative (custom-domain) base-path remap must resolve a normal
#      internal link (/about/), the bare root link (/), an in-page #anchor,
#      AND a relative sibling link to the right on-disk file, so a CLEAN export PASSES
#      with NO false positives (a false positive would block every Pages deploy).
#   2. TEETH — a genuinely broken internal link MUST fail the gate (exit non-zero), or
#      the gate is decorative.
# This harness pins both directions over a tiny synthetic export that mimics the real
# Next.js static export (basePath:"" root-relative, trailingSlash:true) — so a regression in the
# remap rules or the lychee invocation is caught here, before the gate guards a deploy.
#
# HERMETIC: builds its own throwaway export tree under a mktemp dir; no repo content, no
# network (lychee --offline). Requires the `lychee` binary (CI has it; locally:
# cargo install lychee). If lychee is absent we SKIP with a clear message rather than
# fail, so the harness never reds a contributor box that simply lacks the tool.
#
# Run:  bash scripts/tests/test_check_site_links.sh   (exit 0 = pass, 1 = a case failed)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="${ROOT}/scripts/check-site-links.sh"

[ -f "$GATE" ] || { echo "FATAL: gate script not found at ${GATE}"; exit 2; }

if ! command -v lychee >/dev/null 2>&1; then
  echo "SKIP: 'lychee' not on PATH — install it (cargo install lychee) to run this self-test."
  exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# --------------------------------------------------------------------------- #
# Build a minimal export that mirrors the real one: basePath /sparq, trailing-slash
# dir routes, an in-page anchor, a relative sibling link, and a bare-root link.
# --------------------------------------------------------------------------- #
GOOD="${TMP}/good"
mkdir -p "${GOOD}/about" "${GOOD}/surface/sparql"

# Home page: links the bare root, a route, an anchor on this page, a relative sibling, AND a
# CROSS-PAGE path+fragment link (/about/#section-b) — sq-bpoey: this is the
# shape the redesign's /capabilities/#<theme> anchors take, and the case the gate must resolve
# to the target route's index.html (it reddened pages.yml after PR #1003 before rule 0 existed).
cat > "${GOOD}/index.html" <<'HTML'
<!doctype html><html><head><title>home</title></head><body>
  <a href="/">root</a>
  <a href="/about/">about</a>
  <a href="/about/#section-b">about · section b (cross-page fragment)</a>
  <a href="/surface/sparql/">sparql surface</a>
  <a href="#section-a">jump</a>
  <h2 id="section-a">Section A</h2>
</body></html>
HTML

cat > "${GOOD}/about/index.html" <<'HTML'
<!doctype html><html><head><title>about</title></head><body>
  <a href="/">home</a>
  <a href="/#section-a">home · section a (BARE-ROOT fragment)</a>
  <a href="/surface/sparql/">sparql</a>
  <h2 id="section-b">Section B</h2>
</body></html>
HTML

cat > "${GOOD}/surface/sparql/index.html" <<'HTML'
<!doctype html><html><head><title>sparql</title></head><body>
  <a href="/about/">about</a>
</body></html>
HTML

# --------------------------------------------------------------------------- #
# Case 1 (should-PASS): the clean export must pass with no findings.
# --------------------------------------------------------------------------- #
if bash "$GATE" "$GOOD" >/tmp/site-links-good.log 2>&1; then
  echo "PASS: clean export with root-relative links passes"
else
  echo "FAIL: clean export was rejected (false positive — remap likely broken):"
  sed 's/^/    /' /tmp/site-links-good.log
  exit 1
fi

# --------------------------------------------------------------------------- #
# Case 2 (should-FAIL): a broken internal route link must fail the gate.
# --------------------------------------------------------------------------- #
BAD="${TMP}/bad"
cp -r "$GOOD" "$BAD"
cat >> "${BAD}/index.html" <<'HTML'
<a href="/this-route-does-not-exist/">dead</a>
HTML

if bash "$GATE" "$BAD" >/tmp/site-links-bad.log 2>&1; then
  echo "FAIL: a broken internal link was NOT caught (gate has no teeth):"
  sed 's/^/    /' /tmp/site-links-bad.log
  exit 1
else
  echo "PASS: broken internal link is caught (exit non-zero)"
fi

# --------------------------------------------------------------------------- #
# Case 3 (should-FAIL): a broken in-page #anchor must fail (--include-fragments).
# --------------------------------------------------------------------------- #
ANCHOR="${TMP}/anchor"
cp -r "$GOOD" "$ANCHOR"
cat >> "${ANCHOR}/about/index.html" <<'HTML'
<a href="#no-such-anchor">dangling anchor</a>
HTML

if bash "$GATE" "$ANCHOR" >/tmp/site-links-anchor.log 2>&1; then
  echo "FAIL: a dangling #anchor was NOT caught (--include-fragments not effective):"
  sed 's/^/    /' /tmp/site-links-anchor.log
  exit 1
else
  echo "PASS: dangling #anchor is caught (exit non-zero)"
fi

# --------------------------------------------------------------------------- #
# Case 4 (should-FAIL): a dangling CROSS-PAGE fragment must fail. sq-bpoey —
# rule 0 resolves /<route>#<frag> to <route>/index.html for the fragment lookup; this
# pins that the resolution still has TEETH (a fragment that does not exist on the *target*
# page is caught), not just that valid cross-page fragments pass (Case 1).
# --------------------------------------------------------------------------- #
XFRAG="${TMP}/xfrag"
cp -r "$GOOD" "$XFRAG"
cat >> "${XFRAG}/index.html" <<'HTML'
<a href="/about/#no-such-section">dangling cross-page fragment</a>
HTML

if bash "$GATE" "$XFRAG" >/tmp/site-links-xfrag.log 2>&1; then
  echo "FAIL: a dangling cross-page fragment was NOT caught (rule 0 lost its teeth):"
  sed 's/^/    /' /tmp/site-links-xfrag.log
  exit 1
else
  echo "PASS: dangling cross-page fragment is caught (exit non-zero)"
fi

# --------------------------------------------------------------------------- #
# Case 5 (should-FAIL): a dangling BARE-ROOT fragment must fail. sq-uj38w —
# the bare-root remap (/#frag -> root index.html) is what makes the /about RedirectStub's
# /#how-it-runs link resolve at the custom-domain root; this pins that it still has TEETH
# (a fragment that does NOT exist on the home page is caught). Case 1's about page already
# proves a VALID /#section-a bare-root fragment passes.
# --------------------------------------------------------------------------- #
ROOTFRAG="${TMP}/rootfrag"
cp -r "$GOOD" "$ROOTFRAG"
cat >> "${ROOTFRAG}/about/index.html" <<'HTML'
<a href="/#no-such-home-anchor">dangling bare-root fragment</a>
HTML

if bash "$GATE" "$ROOTFRAG" >/tmp/site-links-rootfrag.log 2>&1; then
  echo "FAIL: a dangling bare-root fragment was NOT caught (bare-root remap lost its teeth):"
  sed 's/^/    /' /tmp/site-links-rootfrag.log
  exit 1
else
  echo "PASS: dangling bare-root fragment is caught (exit non-zero)"
fi

echo "All check-site-links self-tests passed."
