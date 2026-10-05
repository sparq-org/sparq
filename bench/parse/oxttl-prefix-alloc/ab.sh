#!/usr/bin/env bash
# [OPUS-5] sq-98w7z.3 — one-command A/B of the two oxttl legs, plus an optional
# per-commit attribution probe.
#
#   ./ab.sh                      # generate the corpus if absent, then A/B both legs
#   ./ab.sh --corpus my.ttl      # A/B against a real corpus (e.g. the sq-wrn61 slice)
#   ./ab.sh --attribute <sha>    # extra leg pinned to an arbitrary oxigraph commit
#   ./ab.sh --self-test          # mutation-check the equivalence gate; builds nothing
#
# Both legs are built in every configuration the README, the registry entry and
# research/oxttl-prefixed-name-alloc-2026-07.md quote — default, mimalloc, rdf-12,
# count-alloc and rdf-12+count-alloc, ten builds in all — so one command really
# does reproduce every recorded row.
#
# EVERY leg is then held to the differential invariant the README and the research
# record call load-bearing: the same triple count AND the same digest as the
# released reference leg. That comparison is what makes this an A/B rather than two
# tables printed side by side — each binary only checks its own repeated parses
# against its OWN warm-up, so without a cross-leg check two parsers that
# deterministically disagree would both print happily and this command would still
# exit 0. A divergence prints both sides and exits nonzero.
#
# Every timing this prints is NON-CANONICAL: it is whatever box you ran it on.
# The allocation counts come from no clock, so they do not move with box speed or
# load: repeated runs of the SAME binary over the same corpus return the same
# counts. That is not box-independence — the shim counts every allocation the whole
# binary makes, so toolchain, target, dependency resolution and features can all
# move them. Record the configuration this script prints alongside any count.
set -euo pipefail

cd "$(dirname "$0")"
HERE="$PWD"
CORPUS="$HERE/data/wd-shape-60mb.ttl"
ITERS=5
ATTRIBUTE=()
SELF_TEST=0
RESULTS="$HERE/.ab-results"

while [ $# -gt 0 ]; do
  case "$1" in
    --corpus) CORPUS="$2"; shift 2 ;;
    --iters) ITERS="$2"; shift 2 ;;
    --attribute) ATTRIBUTE+=("$2"); shift 2 ;;
    --self-test) SELF_TEST=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# ---------------------------------------------------------------------------
# The equivalence gate.
#
# Every leg writes a --json result; the FIRST one to report (released, default
# features) becomes the reference, and every later leg must match it on both
# triple count and digest. The count alone is a vacuous guard here — the research
# record's mutation table shows both deliberate corruptions moving the digest
# while leaving the count untouched — so BOTH are compared and a mismatch in
# either fails the run.
#
# Legs are compared across feature sets as well as across revisions, which is the
# claim the docs actually make (all recorded builds returned one digest and one
# count). A corpus where `rdf-12` legitimately parses differently would surface
# here as a reported mismatch naming both configurations, which is the right
# outcome for a differential harness: a human looks, rather than the divergence
# passing silently.
# ---------------------------------------------------------------------------

REF_LABEL=""
REF_TRIPLES=""
REF_DIGEST=""
MISMATCHES=0

# Read one scalar out of the harness's one-field-per-line --json document.
# Deliberately not a JSON parser: the emitter in src/main.rs is a fixed format!
# with one field per line, and this harness must not grow a jq dependency.
json_field() {
  [ -f "$1" ] || return 0
  sed -n 's/^[[:space:]]*"'"$2"'": "\{0,1\}\([^",]*\)"\{0,1\},\{0,1\}[[:space:]]*$/\1/p' "$1"
}

check_equivalence() {
  local label="$1" file="$2" triples digest
  triples="$(json_field "$file" triples)"
  digest="$(json_field "$file" digest)"
  if [ -z "$triples" ] || [ -z "$digest" ]; then
    echo "EQUIVALENCE: $label — no triples/digest in $file" >&2
    MISMATCHES=$((MISMATCHES + 1))
    return 0
  fi
  if [ -z "$REF_LABEL" ]; then
    REF_LABEL="$label"; REF_TRIPLES="$triples"; REF_DIGEST="$digest"
    echo "equivalence reference: $label — $triples triples, digest $digest"
    return 0
  fi
  if [ "$triples" != "$REF_TRIPLES" ] || [ "$digest" != "$REF_DIGEST" ]; then
    echo "EQUIVALENCE MISMATCH — the two legs did NOT produce the same triple stream" >&2
    echo "  reference $REF_LABEL: $REF_TRIPLES triples, digest $REF_DIGEST" >&2
    echo "  leg       $label: $triples triples, digest $digest" >&2
    MISMATCHES=$((MISMATCHES + 1))
    return 0
  fi
  echo "equivalence ok: $label — $triples triples, digest $digest"
}

# Mutation check for the gate above: it has to go RED when a leg disagrees, or it
# is decoration. Runs no cargo build, so it is cheap enough to run every time.
self_test() {
  local tmp="$HERE/.ab-selftest" fails=0 name expected
  rm -rf "$tmp"; mkdir -p "$tmp"
  stub() { printf '{\n  "triples": %s,\n  "digest": "%s"\n}\n' "$2" "$3" >"$tmp/$1.json"; }
  stub ref         1921676 0x96c046175f277757
  stub same        1921676 0x96c046175f277757
  stub bad_digest  1921676 0x96c046175f277758   # one nibble — the count is untouched
  stub bad_triples 1921675 0x96c046175f277757
  printf '{\n  "note": "a leg that emitted no result fields"\n}\n' >"$tmp/no_fields.json"

  # "<stub> <expected mismatches>" — `same` must pass, every corruption must fail.
  for probe in "same 0" "bad_digest 1" "bad_triples 1" "no_fields 1"; do
    read -r name expected <<<"$probe"
    REF_LABEL=""; REF_TRIPLES=""; REF_DIGEST=""; MISMATCHES=0
    check_equivalence reference "$tmp/ref.json" >/dev/null
    check_equivalence "$name" "$tmp/$name.json" >/dev/null 2>&1
    if [ "$MISMATCHES" != "$expected" ]; then
      echo "self-test FAILED: $name — expected $expected mismatch(es), got $MISMATCHES" >&2
      fails=$((fails + 1))
    else
      echo "self-test ok: $name — $MISMATCHES mismatch(es), as expected"
    fi
  done

  rm -rf "$tmp"
  REF_LABEL=""; REF_TRIPLES=""; REF_DIGEST=""; MISMATCHES=0
  if [ "$fails" -gt 0 ]; then
    echo "self-test FAILED — the equivalence gate does not detect a diverged leg" >&2
    return 1
  fi
  echo "self-test passed: the gate goes red on a mutated digest, a mutated triple count, and a missing field."
}

if [ "$SELF_TEST" = 1 ]; then
  self_test
  exit 0
fi

# The gate decides whether this whole run means anything, so prove it still bites
# BEFORE spending ten release builds on the results it exists to check.
self_test
echo

# The repo pins a toolchain this harness does not need and that a sandbox may not
# be able to install; honour an explicit override if the caller set one.
CARGO=(cargo)
if [ -n "${HARNESS_TOOLCHAIN:-}" ]; then CARGO=(cargo "+${HARNESS_TOOLCHAIN}"); fi

echo "# oxttl prefixed-name A/B — $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo
echo "Box: $(uname -srm) / $(nproc) cores. NON-CANONICAL — timings are box-specific."
echo

if [ ! -f "$CORPUS" ]; then
  echo "corpus $CORPUS absent — generating" >&2
  mkdir -p "$(dirname "$CORPUS")"
  ( cd released && "${CARGO[@]}" build --release >/dev/null )
  ./released/target/release/oxttl-prefix-alloc-released gen 60 "$CORPUS"
fi

# "features|target-dir|mode" — one entry per configuration the docs claim this
# command reproduces. Each gets its OWN target dir, so no build ever overwrites
# another's binary (a count-alloc build must never clobber a timed one). `time`
# legs print the wall-clock table; `count` legs drop ONLY that table — the
# counting shim's atomics perturb the clock — and keep the configuration block
# (corpus, oxttl features, allocator, iterations, digest) that identifies which
# build each allocation row came from.
LEGS=(
  "|target|time"                                 # system allocator, default features
  "mimalloc|target-mi|time"                      # the allocator sparq-cli ingest ships with
  "rdf-12|target-rdf12|time"                     # the feature set the sparq workspace enables
  "count-alloc|target-count|count"               # allocation counts, default features
  "rdf-12,count-alloc|target-rdf12-count|count"  # ... and under rdf-12, to check they match
)

rm -rf "$RESULTS"; mkdir -p "$RESULTS"

for leg in released upstream; do
  bin="oxttl-prefix-alloc-$leg"
  for spec in "${LEGS[@]}"; do
    IFS='|' read -r feats dir mode <<<"$spec"
    args=(build --release --target-dir "$dir")
    if [ -n "$feats" ]; then args+=(--features "$feats"); fi
    ( cd "$leg" && "${CARGO[@]}" "${args[@]}" >/dev/null )
    label="$leg [${feats:-default}]"
    out="$RESULTS/$leg-$dir.json"
    if [ "$mode" = count ]; then
      # Name the exact cargo features, then suppress the wall-clock table (its
      # header plus the separator and the single data row) while keeping every
      # other line, so the two count configurations are told apart in the
      # captured stdout by more than their identical build name.
      echo "### cargo features: ${feats}"
      "./$leg/$dir/release/$bin" bench "$CORPUS" --iters 3 --json "$out" \
        | awk '/^\| build \| oxttl \| s \(min\)/ { skip = 3 } skip { skip--; next } { print }'
    else
      "./$leg/$dir/release/$bin" bench "$CORPUS" --iters "$ITERS" --json "$out"
    fi
    echo
    check_equivalence "$label" "$out"
    echo
  done
done

# Per-commit attribution: "which upstream commit actually bought the delta?".
# Generates a throwaway wrapper manifest per sha under .attrib/ (gitignored) from
# the upstream manifest, so the measured source stays the single shared main.rs.
for sha in ${ATTRIBUTE+"${ATTRIBUTE[@]}"}; do
  d="$HERE/.attrib/$sha"
  mkdir -p "$d"
  sed -e "s/9af7d59/$sha/g" \
      -e "s/oxttl-prefix-alloc-upstream/ox-$sha/g" \
      -e "s|path = \"../src/main.rs\"|path = \"$HERE/src/main.rs\"|" \
      upstream/Cargo.toml > "$d/Cargo.toml"
  ( cd "$d" && "${CARGO[@]}" build --release >/dev/null )
  "$d/target/release/ox-$sha" bench "$CORPUS" --iters "$ITERS" --json "$RESULTS/attrib-$sha.json"
  echo
  # An attribution leg is a leg: it answers to the same invariant.
  check_equivalence "attribution $sha" "$RESULTS/attrib-$sha.json"
  echo
done

# The A/B verdict. Streaming the tables above is not the result; agreeing on the
# triple stream is the precondition for the timings above meaning anything at all,
# so a mismatch is a nonzero exit, not a footnote.
if [ "$MISMATCHES" -gt 0 ]; then
  echo "FAILED: $MISMATCHES leg(s) diverged from the released reference ($REF_LABEL)." >&2
  echo "The legs did not parse the corpus identically, so the timings above are NOT" >&2
  echo "comparable. Per-leg results: $RESULTS" >&2
  exit 1
fi
echo "EQUIVALENCE OK — every leg matched $REF_LABEL: $REF_TRIPLES triples, digest $REF_DIGEST."
