#!/usr/bin/env bash
# [OPUS-4.8] bead sq-mraf — end-to-end proof of the build-BOUNDARY honesty assertion in
# site/scripts/build-papers.mjs. Authored by Opus 4.8 (Fable unavailable; flag for re-review
# when Fable returns).
#
# WHY: the data-layer runHonestyGate() validates the evidence-record envelope but never reads
# PROSE (the .typ paper text or the human `note` fields). sq-mraf adds a build-boundary
# assertion that RE-RUNS the two shared CI honesty gates over the exact paper sources + the
# evidence file BEFORE any artifact is written, so the factory can never serve an un-scanned
# or violating paper. Both gates + this boundary consume the SINGLE shared forbidden-phrase
# list (scripts/honesty-phrases.json) — one source of truth, zero drift.
#
# This harness builds a self-contained throwaway "site" mirror (papers.ts + a paper .typ +
# paper-evidence.json + the real gate scripts + the shared list + the evidence-BINDING verifier
# and a shrink-only allowlist that tolerates the synthetic fixture record) and drives the REAL
# build-papers.mjs against it, asserting:
#   - CLEAN sources                       => the boundary scan PASSES (and the build proceeds;
#                                            typst may be absent — we only assert the scan, so
#                                            we stop the build at the placeholder branch).
#   - a planted '12× faster' in the .typ  => the boundary scan FAILS (non-zero exit) and NO
#                                            PDF/HTML artifact is written (fail-closed).
#   - a planted 'verifier is sound' note  => the boundary scan FAILS (privacy half, via the
#                                            git-tracked evidence file on the gate's surface).
#
# HONEST SCOPE: this pins the COARSE phrase/number boundary only — a subtle semantic overclaim
# is NOT caught here (Stage-5 human review; skills/academic-paper/SKILL.md).
#
# Run:  bash scripts/tests/test_build_papers_honesty_boundary.sh   (exit 0 = all cases pass)
# Hermetic: a self-contained `git init` in a tmpdir; no network. Requires node + python3 + bash
# + git (all CI baseline). typst is NOT required (the boundary scan runs before compilation).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUILDER="${ROOT}/site/scripts/build-papers.mjs"
PERF_GATE="${ROOT}/scripts/check-no-perf-numbers.py"
PRIV_GATE="${ROOT}/scripts/check-privacy-claims.sh"
SHARED="${ROOT}/scripts/honesty-phrases.json"
# [OPUS-4.8] sq-gum8.13: build-papers.mjs now runs the fail-closed evidence-BINDING verifier
# (scripts/verify-paper-evidence.py) at the build boundary, resolving each canonical record's
# `binding` against its committed source. The throwaway repo must therefore mirror that too —
# copy the verifier and seed a shrink-only allowlist that TOLERATES this test's synthetic
# fixture record (whose `source` is a fake path that cannot resolve in the throwaway repo),
# exactly as the real repo's allowlist tolerates a real un-migrated record. This keeps BOTH
# fail-closed gates real: the verifier still runs (it is not disabled), and the honesty-boundary
# assertions (the actual thing under test) are untouched.
VERIFIER="${ROOT}/scripts/verify-paper-evidence.py"
# [OPUS-5] sq-gum8.16: build-papers.mjs also imports the canonical-timing derivation module and
# byte-compares its output against the committed site/src/data/paper-timing.generated.json. The
# throwaway repo must mirror that dependency too, or the builder would fail to even load its
# imports — which would make the should-FAIL cases below pass for the WRONG reason and silently
# void this whole harness.
TIMING_SYNC="${ROOT}/site/scripts/sync-canonical-timing.mjs"
# ...and the publication-boundary source gate, which the builder also imports (it asserts no
# paper source reaches a measured timing except through the provenance-rendering accessors).
TIMING_GATE="${ROOT}/site/scripts/timing-source-gate.mjs"

for f in "$BUILDER" "$PERF_GATE" "$PRIV_GATE" "$SHARED" "$VERIFIER" "$TIMING_SYNC" "$TIMING_GATE"; do
  [ -f "$f" ] || { echo "FATAL: required file not found: $f"; exit 2; }
done
command -v node >/dev/null 2>&1 || { echo "FATAL: node not found"; exit 2; }

pass=0
fail=0
note() { printf '  %s\n' "$1"; }
expect() {  # expect <want-code> <label> <got-code>
  if [ "$3" -eq "$1" ]; then pass=$((pass + 1)); note "PASS  [$2] exit=$3 (wanted $1)";
  else fail=$((fail + 1)); note "FAIL  [$2] exit=$3 (wanted $1)"; fi
}

TMP="$(mktemp -d)"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT

# ---- build a throwaway "site" repo mirroring the real layout build-papers.mjs expects ----
REPO="${TMP}/repo"
mkdir -p "${REPO}/scripts" "${REPO}/site/scripts" "${REPO}/site/papers/_lib" \
         "${REPO}/site/src/data"
cp "$BUILDER"    "${REPO}/site/scripts/build-papers.mjs"
cp "$PERF_GATE"  "${REPO}/scripts/check-no-perf-numbers.py"
cp "$PRIV_GATE"  "${REPO}/scripts/check-privacy-claims.sh"
cp "$SHARED"     "${REPO}/scripts/honesty-phrases.json"
# [OPUS-4.8] sq-gum8.13: the builder now invokes this verifier at the build boundary.
cp "$VERIFIER"   "${REPO}/scripts/verify-paper-evidence.py"
# [OPUS-5] sq-gum8.16: the builder imports this module and re-derives the measured-timing file.
cp "$TIMING_SYNC" "${REPO}/site/scripts/sync-canonical-timing.mjs"
cp "$TIMING_GATE" "${REPO}/site/scripts/timing-source-gate.mjs"
# ...and, when typst IS available, compiles the timing-lib self-check fixture. Mirror both so
# this harness behaves identically with and without typst on PATH. (It also means the
# zero-record branch of the fixture gets exercised: this throwaway repo derives no timings.)
cp "${ROOT}/site/papers/_lib/timing.typ" "${ROOT}/site/papers/_lib/timing-selfcheck.typ" \
   "${REPO}/site/papers/_lib/"

# Minimal papers.ts the builder's regex registry parser understands (slug + source).
cat > "${REPO}/site/src/data/papers.ts" <<'TS'
export const papers = [{ slug: "demo", source: "demo.typ" }];
TS

# A minimal bench.typ stub so the paper imports resolve if a compile is attempted (it won't be
# reached in the should-FAIL cases; in the clean case typst is likely absent → placeholder).
cat > "${REPO}/site/papers/_lib/bench.typ" <<'TYP'
#let evidence = json(bytes(sys.inputs.data))
#let headline(key) = str(evidence.records.at(key).value)
#let ev(key) = str(evidence.records.at(key).value)
#let provenance(key) = []
TYP

write_clean_sources() {
  cat > "${REPO}/site/papers/demo.typ" <<'TYP'
#import "_lib/bench.typ": headline, ev, provenance
= Demo
The recall floor is #headline("x.foo") over 20000 vectors.
TYP
  cat > "${REPO}/site/src/data/paper-evidence.json" <<'JSON'
{ "records": { "x.foo": {
  "value": 0.9, "environment": "canonical", "source": "crates/x/tests/y.rs::z",
  "note": "Asserted recall floor on a 20000-vector synthetic set; machine-independent." } } }
JSON
  # [OPUS-4.8] sq-gum8.13: the fixture record above is canonical + UNBOUND (no `binding`), and
  # its `source` is a fake path that does not resolve in this throwaway repo. The build-boundary
  # evidence-BINDING verifier is fail-closed on exactly that. This record is incidental
  # scaffolding — the thing under test is the .typ PERF / evidence-note PRIVACY phrase gate, not
  # the binding. So we tolerate it via the verifier's OWN intended mechanism: seed the shrink-only
  # allowlist with the key (seed_count matches), exactly as the real repo's allowlist tolerates a
  # real not-yet-migrated record. This does NOT weaken the verifier (it still runs, resolves, and
  # would still fail-closed on a drifted/renamed/grown-allowlist record — that behaviour is proven
  # non-vacuously by `verify-paper-evidence.py --self-test`, a separate CI check).
  cat > "${REPO}/scripts/paper-evidence-binding-allowlist.json" <<'JSON'
{ "seed_count": 1, "keys": ["x.foo"] }
JSON
  # [OPUS-5] sq-gum8.16: derive the measured-timing file the same way `prebuild` does, rather
  # than hand-writing an expected serialisation here (which would drift the moment the schema
  # changes). This throwaway repo has no bench/canonical-competitor-results/, so the honest
  # result is an EMPTY derivation — and the builder's byte-compare must accept exactly that.
  ( cd "$REPO" && node site/scripts/sync-canonical-timing.mjs >/dev/null )
}

# Run the builder inside the throwaway repo; echo its exit code.
run_builder() {
  (
    cd "$REPO"
    git add -A >/dev/null 2>&1
    set +e
    node site/scripts/build-papers.mjs >/dev/null 2>&1
    echo $?
  )
}

git -C "$REPO" init -q
git -C "$REPO" config user.email t@t
git -C "$REPO" config user.name t

# ---- 1. clean sources: boundary scan PASSES (build returns 0 — placeholder or real) ----
write_clean_sources
code="$(run_builder)"
expect 0 "clean .typ + clean evidence note => build-boundary scan passes" "$code"

# ---- 2. planted '12× faster' in the .typ: boundary scan FAILS, NO artifact written ----
# Clear any artifacts a prior (clean) build wrote, so a leftover does not mask a regression:
# the boundary scan runs BEFORE the output dirs are (re)created, so a violating build must
# leave NO fresh artifact.
rm -rf "${REPO}/site/public/papers" "${REPO}/site/src/generated"
write_clean_sources
printf '\nOur engine is 12× faster than the baseline.\n' >> "${REPO}/site/papers/demo.typ"
code="$(run_builder)"
expect 1 "planted '12× faster' in .typ => build aborts (fail-closed)" "$code"
if [ -e "${REPO}/site/public/papers/demo.pdf" ] || [ -e "${REPO}/site/src/generated/papers/demo.html" ]; then
  fail=$((fail + 1)); note "FAIL  [no artifact written on a violating build] artifact present"
else
  pass=$((pass + 1)); note "PASS  [no artifact written on a violating build]"
fi

# ---- 3. planted 'verifier is sound' in an evidence note: boundary scan FAILS ----
write_clean_sources
cat > "${REPO}/site/src/data/paper-evidence.json" <<'JSON'
{ "records": { "x.foo": {
  "value": 0.9, "environment": "canonical", "source": "crates/x/tests/y.rs::z",
  "note": "The verifier is sound and the construction is privacy-preserving." } } }
JSON
code="$(run_builder)"
expect 1 "planted 'verifier is sound' in an evidence note => build aborts" "$code"

echo ""
echo "test_build_papers_honesty_boundary: ${pass} passed, ${fail} failed."
[ "$fail" -eq 0 ] || exit 1
echo "test_build_papers_honesty_boundary: OK — the factory refuses to serve an un-scanned/violating paper."
