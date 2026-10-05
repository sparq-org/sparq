#!/usr/bin/env bash
# [OPUS-5] issue #4571 — hermetic self-test for scripts/verify-release-provenance.sh.
#
# WHY IT EXISTS. The script under test is the EVIDENCE ENGINE for SL-B3-b: its verdict is what
# flips the control from AR to IV. It runs for the first time when a release is being cut, and
# its dangerous failure mode is the silent one — a check that passes vacuously (empty asset set,
# a bundle that covers nothing, an asset silently exempted) would manufacture "verified" out of
# nothing, which is worse than no check at all. So every fail-closed branch is exercised HERE, on
# every PR, with a STUB `slsa-verifier` (the real one needs a signed bundle from a real release).
#
# The stub reads $STUB_DB — one `<bundle>|<asset>` pair per line — and exits 0 iff the pair is
# listed. That is exactly the property the real verifier has (digest-based subject matching) and
# nothing more, so the test pins the script's LOGIC, not the verifier's.
#
# Run: bash scripts/tests/test_verify_release_provenance.sh
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/scripts/verify-release-provenance.sh"
TAG="v9.9.9"
CLI_BUNDLE="sparq-cli-${TAG}.intoto.jsonl"
ARTIFACTS_BUNDLE="sparq-artifacts-${TAG}.intoto.jsonl"

WORK="$(mktemp -d)"
trap 'rm -rf -- "$WORK"' EXIT

failures=0
pass() { echo "  ok   — $1"; }
fail() {
  echo "  FAIL — $1" >&2
  failures=$((failures + 1))
}
# check <ok-label> <fail-label> <cmd...> — an if/else, deliberately not `A && pass || fail`
# (which runs the failure branch whenever `pass` itself fails; shellcheck SC2015).
check() {
  local ok="$1" bad="$2"
  shift 2
  if "$@"; then pass "$ok"; else fail "$bad"; fi
}

# ---- stub slsa-verifier ----------------------------------------------------------------------
mkdir -p "$WORK/bin"
cat >"$WORK/bin/slsa-verifier" <<'STUB'
#!/usr/bin/env bash
# stub: `slsa-verifier verify-artifact <asset> --provenance-path <bundle> --source-uri <uri>`
set -uo pipefail
if [ "${1:-}" = "version" ]; then echo "stub-verifier 0.0.0"; exit 0; fi
asset="${2:-}"; bundle=""; uri=""
shift 2 || true
while [ "$#" -gt 0 ]; do
  case "$1" in
    --provenance-path) bundle="$2"; shift 2 ;;
    --source-uri) uri="$2"; shift 2 ;;
    *) shift ;;
  esac
done
[ -n "$uri" ] || { echo "stub: no --source-uri" >&2; exit 2; }
grep -Fqx "${bundle}|${asset}" "$STUB_DB" || { echo "stub: FAILED: no matching subject" >&2; exit 1; }
echo "stub: PASSED: verified SLSA provenance"
STUB
chmod +x "$WORK/bin/slsa-verifier"
export PATH="$WORK/bin:$PATH"

# ---- fixture builder ---------------------------------------------------------------------------
# make_release <dir> — a well-formed release: two archives (archives bundle), one .deb + one SBOM
# (artifacts bundle), one alias copy of an archive, both bundles, and a matching SHA256SUMS.
make_release() {
  local dir="$1"
  rm -rf -- "$dir"
  mkdir -p "$dir"
  (
    cd "$dir" || exit 1
    echo "archive-x64" >"sparq-cli-${TAG}-x64-baseline.tar.gz"
    echo "archive-arm" >"sparq-cli-${TAG}-arm64-linux.tar.gz"
    cp "sparq-cli-${TAG}-x64-baseline.tar.gz" sparq-cli-x64-linux.tar.gz # version-stable alias
    echo "deb" >"sparq-gui_${TAG}_amd64.deb"
    echo "sbom" >"sparq-cli-${TAG}.sbom.cdx.json"
    echo "dsse-archives" >"$CLI_BUNDLE"
    echo "dsse-artifacts" >"$ARTIFACTS_BUNDLE"
    sha256sum -- * >SHA256SUMS
  )
  {
    echo "${CLI_BUNDLE}|sparq-cli-${TAG}-x64-baseline.tar.gz"
    echo "${CLI_BUNDLE}|sparq-cli-${TAG}-arm64-linux.tar.gz"
    echo "${CLI_BUNDLE}|sparq-cli-x64-linux.tar.gz"
    echo "${ARTIFACTS_BUNDLE}|sparq-gui_${TAG}_amd64.deb"
    echo "${ARTIFACTS_BUNDLE}|sparq-cli-${TAG}.sbom.cdx.json"
  } >"$WORK/stub-db.txt"
  export STUB_DB="$WORK/stub-db.txt"
}

run_script() { # run_script <dir> -> stdout+stderr in $OUT, status in $STATUS
  OUT="$(bash "$SCRIPT" --tag "$TAG" --repo sparq-org/sparq --dir "$1" 2>&1)"
  STATUS=$?
}

expect_pass() { # expect_pass <label> <dir>
  run_script "$2"
  if [ "$STATUS" -eq 0 ]; then pass "$1"; else
    fail "$1 (expected exit 0, got $STATUS)"
    echo "$OUT" | awk '{ print "       " $0 }' >&2
  fi
}

expect_fail() { # expect_fail <label> <dir> <substring the message must contain>
  run_script "$2"
  if [ "$STATUS" -eq 0 ]; then
    fail "$1 (expected non-zero exit, got 0 — a fail-closed branch is not closed)"
  elif ! printf '%s' "$OUT" | grep -qF "$3"; then
    fail "$1 (failed as expected but the message never mentioned '$3')"
    echo "$OUT" | awk '{ print "       " $0 }' >&2
  else
    pass "$1"
  fi
}

echo "== scripts/verify-release-provenance.sh =="

# 1. HAPPY PATH — every asset is a subject of one of the two bundles.
D="$WORK/happy"
make_release "$D"
expect_pass "a well-formed release verifies" "$D"
run_script "$D"
counts_reported() {
  printf '%s' "$OUT" | grep -q "archives bundle .... ${CLI_BUNDLE} — 3 asset(s) verified" &&
    printf '%s' "$OUT" | grep -q "artifacts bundle ... ${ARTIFACTS_BUNDLE} — 2 asset(s) verified"
}
check "the evidence block reports the per-bundle counts (3 archives incl. alias, 2 artifacts)" \
  "the evidence block did not report the expected per-bundle counts" counts_reported

# 2. A MISSING BUNDLE is the "the trusted builder never ran / was not attached" case.
D="$WORK/no-cli-bundle"
make_release "$D"
rm -f "$D/$CLI_BUNDLE"
expect_fail "a missing archives bundle reds" "$D" "$CLI_BUNDLE"

D="$WORK/no-artifacts-bundle"
make_release "$D"
rm -f "$D/$ARTIFACTS_BUNDLE"
expect_fail "a missing artifacts bundle reds" "$D" "$ARTIFACTS_BUNDLE"

# 3. AN EMPTY BUNDLE is not a bundle (a 0-byte upload must not read as "attached").
D="$WORK/empty-bundle"
make_release "$D"
: >"$D/$CLI_BUNDLE"
expect_fail "a zero-byte bundle reds" "$D" "missing or empty"

# 4. SHA256SUMS must EXIST, LIST both bundles, and MATCH the published bytes.
D="$WORK/no-sums"
make_release "$D"
rm -f "$D/SHA256SUMS"
expect_fail "a missing SHA256SUMS reds" "$D" "SHA256SUMS is missing or empty"

D="$WORK/unlisted-bundle"
make_release "$D"
grep -v "  ${CLI_BUNDLE}\$" "$D/SHA256SUMS" >"$D/SHA256SUMS.tmp" && mv "$D/SHA256SUMS.tmp" "$D/SHA256SUMS"
expect_fail "a bundle absent from SHA256SUMS reds" "$D" "not listed in SHA256SUMS"

D="$WORK/tampered"
make_release "$D"
echo "tampered" >"$D/sparq-cli-${TAG}-x64-baseline.tar.gz"
expect_fail "an asset whose digest no longer matches SHA256SUMS reds" "$D" "does not match the published bytes"

# 5. AN UNCOVERED ASSET — the subjects set silently lost a lane. This is the check that a plain
#    "did slsa-verifier exit 0 once" script would miss entirely.
D="$WORK/uncovered"
make_release "$D"
echo "orphan" >"$D/sparq-gui_${TAG}_x64.msi"
(cd "$D" && rm -f SHA256SUMS && sha256sum -- * >SHA256SUMS)
expect_fail "an asset covered by NEITHER bundle reds" "$D" "verify against NEITHER"

# 6. VACUITY — a directory with only the bundles + SHA256SUMS must never read as "verified".
D="$WORK/vacuous"
mkdir -p "$D"
(
  cd "$D" || exit 1
  echo "dsse-archives" >"$CLI_BUNDLE"
  echo "dsse-artifacts" >"$ARTIFACTS_BUNDLE"
  sha256sum -- * >SHA256SUMS
)
expect_fail "an asset-less release reds instead of passing vacuously" "$D" "zero assets were verified"

# 7. A BUNDLE THAT COVERS NOTHING IT PUBLISHED — both bundles must each cover >= 1 asset, so a
#    lane whose subjects all vanished cannot hide behind the other lane's successes.
D="$WORK/one-sided"
make_release "$D"
rm -f "$D/sparq-gui_${TAG}_amd64.deb" "$D/sparq-cli-${TAG}.sbom.cdx.json"
(cd "$D" && rm -f SHA256SUMS && sha256sum -- * >SHA256SUMS)
expect_fail "an artifacts bundle covering none of the published assets reds" "$D" "covering nothing it published"

# 8. ARGUMENT + TOOL PRECONDITIONS.
failed_with() { # failed_with <substring> — the last run must have red WITH that message
  [ "$STATUS" -ne 0 ] && printf '%s' "$OUT" | grep -qF -- "$1"
}

OUT="$(bash "$SCRIPT" --repo sparq-org/sparq --dir "$WORK/happy" 2>&1)"
STATUS=$?
check "a missing --tag reds" "a missing --tag did not red with a usable message" \
  failed_with "--tag is required"

OUT="$(SLSA_VERIFIER=definitely-not-installed bash "$SCRIPT" --tag "$TAG" --dir "$WORK/happy" 2>&1)"
STATUS=$?
check "a missing slsa-verifier reds (never skipped as 'unavailable')" \
  "a missing slsa-verifier did not red" failed_with "not found on PATH"

# 9. The workflow that runs this in CI must actually call the script.
WF="$ROOT/.github/workflows/release-verify.yml"
[ -f "$WF" ] || fail "missing .github/workflows/release-verify.yml"
if [ -f "$WF" ]; then
  check "release-verify.yml invokes the script" \
    "release-verify.yml does not invoke scripts/verify-release-provenance.sh" \
    grep -q "scripts/verify-release-provenance.sh" "$WF"
  # The AUTOMATIC entry point (see check 10). A `release: published` trigger alone would NOT be
  # reached on the normal path, so workflow_call is the load-bearing one.
  check "release-verify.yml is callable by the release pipeline (workflow_call)" \
    "release-verify.yml lost its workflow_call entry point — release.yml can no longer drive it" \
    grep -qE "^  workflow_call:" "$WF"
  check "release-verify.yml keeps the out-of-band release:published trigger" \
    "release-verify.yml lost its release:published trigger (the safety net for a hand-published release)" \
    grep -q "types: \[published\]" "$WF"
  # Trigger-shaped only (a `^  pull_request:` key), so the header prose explaining WHY there is
  # no such trigger does not trip the check. NEGATED: absence is the passing condition.
  no_pr_trigger() { ! grep -qE "^  (pull_request|merge_group):" "$WF"; }
  check "release-verify.yml is PR-invisible (no pull_request/merge_group trigger)" \
    "release-verify.yml gained a PR/merge-queue trigger — it would register a check-run the gate aggregator does not expect" \
    no_pr_trigger
fi

# 10. THE LOAD-BEARING WIRING: the RELEASE-PRODUCING workflow must drive the verifier itself.
#     Asserting a `release: published` trigger (check 9) is NOT enough and must never be mistaken
#     for this: GitHub does not start a workflow run from an event generated by a workflow's own
#     GITHUB_TOKEN, and release.yml creates the Release with exactly that token
#     (softprops/action-gh-release defaults `token: ${{ github.token }}`). So on the normal `v*`
#     path that trigger never fires, and without the caller below "publishing a release yields a
#     citable verdict" would be a false claim with a green self-test over it. Pinned by JOB
#     STRUCTURE, not by substring: some job in release.yml has a job-level `uses:` of this
#     workflow, `needs:` the job that creates the Release (found by the action it runs, not by
#     its id), passes a `tag`, and is neither conditional nor error-swallowing.
RELEASE_WF="$ROOT/.github/workflows/release.yml"
[ -f "$RELEASE_WF" ] || fail "missing .github/workflows/release.yml"
if [ -f "$RELEASE_WF" ]; then
  release_drives_verifier() {
    # Pure stdlib, no PyYAML — same reason test_release_slsa_l3_provenance.py gives: this must
    # run in ANY checkout, including one where the docs-quality setup has not installed it.
    python3 - "$RELEASE_WF" <<'PY'
import re
import sys

lines = open(sys.argv[1], encoding="utf-8").read().splitlines()
try:
    top = next(i for i, l in enumerate(lines) if re.match(r"^jobs:\s*$", l))
except StopIteration:
    sys.exit("release.yml has no top-level `jobs:` mapping — re-point this check")

# Job ids sit at exactly two spaces; every key inside a job is deeper, so this splits cleanly.
blocks, cur = {}, None
for line in lines[top + 1:]:
    m = re.match(r"^  ([A-Za-z0-9_.-]+):\s*$", line)
    if m:
        cur = m.group(1)
        blocks[cur] = []
    elif cur is not None:
        blocks[cur].append(line)

def job_scalar(body, key):
    # Job-LEVEL keys only (four spaces) — never a step's, which is deeper or `- `-prefixed.
    for line in body:
        m = re.match(rf"^    {re.escape(key)}:\s*(.*?)\s*$", line)
        if m:
            return m.group(1)
    return None

def needs_set(body):
    raw = (job_scalar(body, "needs") or "").strip()
    if raw.startswith("[") and raw.endswith("]"):
        raw = raw[1:-1]
    return {p.strip() for p in raw.split(",") if p.strip()}

# The release-PRODUCING job, identified by what it DOES (creates the Release), not by its id.
producers = [
    jid for jid, body in blocks.items()
    if any(re.match(r"^\s*-?\s*uses:\s*softprops/action-gh-release@", l) for l in body)
]
if not producers:
    sys.exit("release.yml has no job creating a GitHub Release — re-point this check")

callers = {
    jid: body for jid, body in blocks.items()
    if job_scalar(body, "uses") == "./.github/workflows/release-verify.yml"
}
if not callers:
    sys.exit(
        "no job in release.yml calls ./.github/workflows/release-verify.yml — a "
        "`release: published` trigger alone NEVER fires for a GITHUB_TOKEN-created Release, "
        "so no normal release would be verified"
    )

wired = [jid for jid, body in callers.items() if needs_set(body) & set(producers)]
if not wired:
    sys.exit(
        "release.yml calls release-verify.yml but not after the Release exists — add "
        "`needs: [%s]` so the published assets are observable" % producers[0]
    )
for jid in wired:
    body = callers[jid]
    if job_scalar(body, "if") is not None:
        sys.exit("release.yml#%s is conditional — the evidence step must run unconditionally" % jid)
    if (job_scalar(body, "continue-on-error") or "false") != "false":
        sys.exit("release.yml#%s swallows failures — it would manufacture the evidence" % jid)
    if not any(re.match(r"^      tag:\s*\S", l) for l in body):
        sys.exit("release.yml#%s passes no `tag` to the verifier" % jid)
PY
  }
  check "release.yml drives the verifier after publishing (not the unreachable release: trigger)" \
    "release.yml no longer runs the provenance verifier after cutting the Release — see the message above" \
    release_drives_verifier
fi

echo
if [ "$failures" -ne 0 ]; then
  echo "FAILED: $failures check(s)" >&2
  exit 1
fi
echo "all checks passed"
