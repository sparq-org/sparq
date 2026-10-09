#!/usr/bin/env bash
# sq-9qz6 — wasm dependency-graph guard.
#
# The browser/wasm bundles must stay lean and pure-Rust: the native-only heavy deps
# (parallelism + the compression codecs + the parallel parser crate) must NEVER
# enter a wasm32-target bundle dependency graph. This was only an invariant stated in
# comments; this script makes it an enforced CI gate so an accidental `use` or a
# default-feature change that pulls one of them in fails fast (bundle bloat / broken
# browser build) instead of silently regressing.
#
# Guards every shipped wasm bundle crate's DEFAULT (no-dev) wasm32 graph:
#   - sparq-wasm        (the lean triplestore + SPARQL bundle)
#   - sparq-reason-wasm (the tier-b "W-reason" forward-chaining inference bundle — sq-6qw3)
#   - sparq-rsp-wasm    (the tier-b "W-rsp" windowed RSP-QL stream bundle — sq-nzcb)
#   - sparq-text-wasm   (the tier-b "W-text" BM25 full-text bundle — sq-jbe6)
#   - sparq-shacl-wasm  (the tier-b "W-shacl" SHACL validation bundle — sq-lfmf)
# Opt-in features (sparq-wasm's `shacl`, sparq-reason-wasm's `explain`,
# sparq-shacl-wasm's `shacl-af`) are NOT scanned here: they are off by default so the
# default bundles stay lean, and the wasm32 build + clippy in CI prove the feature-on
# graphs still link. NOTE: `regex` is intentionally NOT forbidden — sparq-reason-wasm
# legitimately carries it (the N3 `string:matches` builtin) and sparq-shacl-wasm carries
# it (the `sh:pattern` constraint), and it is pure-Rust + wasm-portable; the forbidden set
# is the native-ONLY heavy deps.
# sparq-rsp-wasm carries NO regex (sparq-rsp + its engine/core are no-default-features), so
# its graph is among the leanest — guarded by the same forbidden set.
# sparq-text-wasm's `unicode-segmentation` (UAX #29 tokenizer) is likewise pure-Rust +
# wasm-portable and NOT forbidden. sq-jbe6
#
# Run: scripts/wasm-deps-guard.sh   (exit 0 = clean, exit 1 = a forbidden crate is present)
set -euo pipefail

TARGET="wasm32-unknown-unknown"
# The shipped wasm bundle crates whose default wasm32 graph must stay lean.
BUNDLES=(sparq-wasm sparq-reason-wasm sparq-rsp-wasm sparq-text-wasm sparq-shacl-wasm)
# Crates that must be absent from the wasm32 graph (non-dev deps).
FORBIDDEN=(rayon flate2 zstd zstd-safe bzip2 sparq-parse mio tokio)

fail=0
for pkg in "${BUNDLES[@]}"; do
  # Fail LOUD, not silently (#6093): under `set -e` a failing `cargo tree` (unfetched
  # registry, offline runner, manifest/lockfile error) used to abort here with zero output.
  tree_err="$(mktemp)"
  if ! tree="$(cargo tree -p "$pkg" --target "$TARGET" -e no-dev 2>"$tree_err")"; then
    echo "::error::cargo tree failed for ${pkg} — the ${TARGET} dependency graph could not be computed (not a forbidden-crate finding)"
    sed 's/^/  cargo tree: /' "$tree_err" | head -20
    rm -f "$tree_err"
    exit 1
  fi
  rm -f "$tree_err"
  for crate in "${FORBIDDEN[@]}"; do
    # Match a tree line whose package name is exactly $crate (followed by a space+version
    # or end-of-line), ignoring the leading tree-drawing glyphs.
    if printf '%s\n' "$tree" | grep -qE "[^[:alnum:]_-]${crate}( v[0-9]| |\$)"; then
      echo "::error::forbidden crate '${crate}' is in the ${pkg} ${TARGET} dependency graph"
      printf '%s\n' "$tree" | grep -E "[^[:alnum:]_-]${crate}( v[0-9]| |\$)" | head -3
      fail=1
    fi
  done
done

if [ "$fail" -ne 0 ]; then
  echo "wasm-deps-guard: FAILED — a native-only dep leaked into a wasm bundle graph (see above)."
  exit 1
fi
echo "wasm-deps-guard: OK — none of [${FORBIDDEN[*]}] are in the [${BUNDLES[*]}] ${TARGET} graphs."
