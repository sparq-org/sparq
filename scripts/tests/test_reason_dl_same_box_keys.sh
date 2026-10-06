#!/usr/bin/env bash
# Hermetic self-test for scripts/bench/reason-dl-same-box.sh per-input
# keying (PR #3479 review): the corpus walk admits subdirectories, so two corpus
# files may share a basename (a/foo.owl vs b/foo.owl), AND the lossy sanitized stem can
# alias distinct paths (a/foo.owl and a_foo.owl both sanitize to "a_foo") — so a purely
# content-SHA'd, stem-based key collides for byte-identical inputs. Every derived artifact —
# converted NT, raw per-engine output, envelope filename — must be keyed by the exact
# corpus-relative path (path-SHA) + a content-SHA prefix, so:
#   1. each same-basename ontology is CONVERTED INDEPENDENTLY (no [ -f "$NT" ]
#      cache hit on a stale basename-derived path), and
#   2. each produces a DISTINCT envelope whose ontology_sha256 is the SHA-256 of
#      ITS OWN source file (no later row overwriting an earlier one).
#
# HERMETIC: no network, no JVM, no cargo build — `cargo` is PATH-shadowed by a
# no-op, riot is a logging stub under a scratch JENA_HOME, ore_bench is a stub
# under a scratch repo copy's target/, and corpus/GATHER/OUT_DIR are all mktemp
# sandboxed. Only python3 + coreutils (the script's own baseline deps) are real.
#
# Run:  bash scripts/tests/test_reason_dl_same_box_keys.sh   (exit 0 = all pass)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="${ROOT}/scripts/bench/reason-dl-same-box.sh"
[ -f "$SRC" ] || { echo "FATAL: script not found at ${SRC}"; exit 2; }

pass=0
fail=0
note_pass() { pass=$((pass + 1)); }
note_fail() { fail=$((fail + 1)); printf 'CASE FAILED: %s\n' "$1"; }

# --------------------------------------------------------------------------- #
# Sandbox: a scratch repo copy (so SPARQ_BIN=$ROOT/target/... resolves to a stub),
# a stub cargo on PATH, a logging riot stub under a scratch JENA_HOME, and a
# corpus with TWO same-basename, different-content ontologies in subdirectories.
# --------------------------------------------------------------------------- #
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT
REPO="${SANDBOX}/repo"
BIN="${SANDBOX}/bin"
CORPUS="${SANDBOX}/corpus"
mkdir -p "${REPO}/scripts/bench" "${REPO}/target/release/examples" "$BIN" \
  "${SANDBOX}/jena/bin" "${CORPUS}/a" "${CORPUS}/b" "${CORPUS}/x"
cp "$SRC" "${REPO}/scripts/bench/reason-dl-same-box.sh"

# No-op cargo stub — the full-mode `cargo build` must not touch the real workspace.
printf '#!/usr/bin/env bash\nexit 0\n' > "${BIN}/cargo"
chmod +x "${BIN}/cargo"

# ore_bench stub at the path the copied script derives from its own ROOT: emits a
# definitive verdict + a tiny internal breakdown (so the wall >= extract+check
# boundary assertion trivially holds).
cat > "${REPO}/target/release/examples/ore_bench" <<'EOF'
#!/usr/bin/env bash
echo "profile=ALCH verdict=consistent extract_s=0.000001 check_s=0.000001"
EOF
chmod +x "${REPO}/target/release/examples/ore_bench"

# riot stub: logs every source it converts, then "converts" by copying it through.
RIOT_LOG="${SANDBOX}/riot.log"
: >"$RIOT_LOG"
cat > "${SANDBOX}/jena/bin/riot" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$2" >> "${RIOT_LOG}"
cat "\$2"
EOF
chmod +x "${SANDBOX}/jena/bin/riot"

# Two same-basename ontologies with DIFFERENT content in different subdirectories.
printf '<urn:a> <urn:p> <urn:oa> .\n' > "${CORPUS}/a/foo.owl"
printf '<urn:b> <urn:p> <urn:ob> .\n' > "${CORPUS}/b/foo.owl"
SHA_A="$(sha256sum "${CORPUS}/a/foo.owl" | cut -d' ' -f1)"
SHA_B="$(sha256sum "${CORPUS}/b/foo.owl" | cut -d' ' -f1)"

# Two DISTINCT paths whose LOSSY sanitized stems COLLIDE (x/bar.owl and x_bar.owl both
# sanitize to "x_bar") AND whose contents are BYTE-IDENTICAL (so the content-SHA suffix
# alone cannot tell them apart). Only a path-bound key keeps them distinct — this is the
# regression the sanitized-stem key silently dropped.
printf '<urn:same> <urn:p> <urn:o> .\n' > "${CORPUS}/x/bar.owl"
printf '<urn:same> <urn:p> <urn:o> .\n' > "${CORPUS}/x_bar.owl"
SHA_XSUB="$(sha256sum "${CORPUS}/x/bar.owl" | cut -d' ' -f1)"
SHA_XTOP="$(sha256sum "${CORPUS}/x_bar.owl" | cut -d' ' -f1)"
[ "$SHA_XSUB" = "$SHA_XTOP" ] || { echo "FATAL: collision fixtures must have identical content"; exit 2; }

OUT_DIR="${SANDBOX}/out"
GATHER="${SANDBOX}/gather"
if ! PATH="${BIN}:${PATH}" \
    ORE_CORPUS_DIR="$CORPUS" ONLY="sparq" OUT_DIR="$OUT_DIR" GATHER="$GATHER" \
    JENA_HOME="${SANDBOX}/jena" TIMEOUT_S=30 \
    bash "${REPO}/scripts/bench/reason-dl-same-box.sh" >"${SANDBOX}/run.log" 2>&1; then
  echo "FATAL: wrapper run failed — log follows"
  cat "${SANDBOX}/run.log"
  exit 1
fi

# --------------------------------------------------------------------------- #
# 1. INDEPENDENT CONVERSION — riot invoked once per input, on BOTH files (the
#    second same-basename file must not reuse the first file's cached NT).
# --------------------------------------------------------------------------- #
if [ "$(wc -l < "$RIOT_LOG")" -eq 4 ]; then note_pass; else
  note_fail "riot invoked $(wc -l < "$RIOT_LOG") time(s), wanted 4 (cache collision reused a conversion)"; fi
if grep -qx "${CORPUS}/a/foo.owl" "$RIOT_LOG" && grep -qx "${CORPUS}/b/foo.owl" "$RIOT_LOG" \
   && grep -qx "${CORPUS}/x/bar.owl" "$RIOT_LOG" && grep -qx "${CORPUS}/x_bar.owl" "$RIOT_LOG"; then
  note_pass
else
  note_fail "riot did not convert all four inputs incl. the stem-collision pair (log: $(tr '\n' ' ' < "$RIOT_LOG"))"
fi
if [ "$(find "$GATHER" -name '*.nt' | wc -l)" -eq 4 ]; then note_pass; else
  note_fail "expected 4 distinct converted .nt files in GATHER, found $(find "$GATHER" -name '*.nt' | wc -l)"; fi

# --------------------------------------------------------------------------- #
# 2. DISTINCT ENVELOPES — one per input (a name collision would overwrite,
#    leaving only one file), each tied to ITS OWN source SHA-256.
# --------------------------------------------------------------------------- #
ENV_COUNT="$(find "$OUT_DIR" -name '*.json' | wc -l)"
if [ "$ENV_COUNT" -eq 4 ]; then note_pass; else
  note_fail "expected 4 envelopes, found ${ENV_COUNT} (name collision overwrote a row?)"; fi

check_envelope() {  # check_envelope <corpus-relative ontology> <expected sha256>
  OUT_DIR="$OUT_DIR" WANT_ONT="$1" WANT_SHA="$2" python3 - <<'PYEOF'
import glob, json, os, sys
want_ont = os.environ["WANT_ONT"]
want_sha = os.environ["WANT_SHA"]
for path in glob.glob(os.path.join(os.environ["OUT_DIR"], "*.json")):
    with open(path) as fh:
        env = json.load(fh)
    if env.get("ontology") == want_ont:
        sys.exit(0 if env.get("ontology_sha256") == want_sha else 1)
sys.exit(2)
PYEOF
}
# The last two pairs are the sanitize-collision case: distinct relative paths but
# byte-identical content, so each envelope must be identified by its OWN relative path
# (not merged) even though they share ontology_sha256.
for pair in "a/foo.owl:$SHA_A" "b/foo.owl:$SHA_B" "x/bar.owl:$SHA_XSUB" "x_bar.owl:$SHA_XTOP"; do
  ont="${pair%%:*}"; sha="${pair##*:}"
  set +e; check_envelope "$ont" "$sha"; rc=$?; set -e
  case "$rc" in
    0) note_pass ;;
    1) note_fail "envelope for ${ont} carries the WRONG ontology_sha256 (mixed rows)" ;;
    *) note_fail "no envelope identifies ontology ${ont} (basename-ambiguous or overwritten)" ;;
  esac
done

# --------------------------------------------------------------------------- #
# 3. STATIC — the collision-resistant key survives refactors: derived artifacts
#    must not regress to bare-basename paths.
# --------------------------------------------------------------------------- #
if grep -q 'OWL_SHA:0:12' "$SRC"; then note_pass; else
  note_fail "per-input key no longer embeds a content-SHA prefix in the source"; fi
if grep -q '\$GATHER/\$ONT' "$SRC"; then
  note_fail "source regressed to basename-derived \$GATHER/\$ONT.* paths"
else
  note_pass
fi

# --------------------------------------------------------------------------- #
echo ""
echo "test_reason_dl_same_box_keys: ${pass} passed, ${fail} failed."
[ "$fail" -eq 0 ] || exit 1
echo "test_reason_dl_same_box_keys: OK — same-basename inputs convert independently and emit distinct SHA-bound envelopes."
