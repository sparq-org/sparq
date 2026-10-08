#!/usr/bin/env bash
# sq-s1uy (epic sq-dnko / sq-3183) — sparq-fedclient dependency-boundary guard.
#
# THE load-bearing Phase-0 deliverable. The lean core — `sparq-core` and `sparq-engine` —
# must NEVER gain a dependency edge to the OPT-IN streaming federation client
# `sparq-fedclient`. The dependency arrow points ONE WAY *into* the engine: the client
# reuses the engine's SERVICE transport + SSRF guard + local eval, but the engine never
# depends on the client. If it did, the opt-in-feature-architecture constraint (core stays
# lean; the default engine build + the WASM artifact are byte-identical with or without
# the client) would be broken. This was only an invariant stated in the design record
# (research/federation-client-design.md §5) and in Cargo.toml comments; this script makes
# it an enforced CI gate so an accidental edge fails fast instead of silently regressing.
#
# HOW: invert the dependency graph from `sparq-fedclient` (`cargo tree -i`) and assert
# that neither `sparq-core` nor `sparq-engine` appears as a DEPENDENT. `--all-features`
# makes every conditional edge visible, so an edge introduced under ANY feature is caught.
# This mirrors scripts/wasm-deps-guard.sh's `cargo tree` + grep style; tests/boundary.rs
# in the crate is the in-suite `cargo metadata` mirror of the same invariant (either alone
# catches a violation; both together is belt-and-braces).
#
# Run: scripts/fedclient-boundary-guard.sh   (exit 0 = boundary intact, exit 1 = violated)
set -euo pipefail

CLIENT="sparq-fedclient"
# The lean members that must NOT appear among sparq-fedclient's dependents.
FORBIDDEN_DEPENDENTS=(sparq-core sparq-engine)

# `cargo tree -i <pkg>` lists every package that (transitively) DEPENDS ON <pkg>. With
# --all-features every optional/conditional edge is resolved, so a dependent that only
# appears under some feature is still listed. If the boundary is intact, the only
# dependents are sparq-fedclient itself (the root of the inverted tree) — never the core.
#
# Capture stdout AND stderr separately. Because sparq-fedclient depends ON the engine
# (engine -> core), the MOST LIKELY regression — a core/engine -> fedclient edge — forms a
# dependency CYCLE, which cargo rejects with a `cyclic package dependency` error on stderr
# (no tree on stdout). We detect that explicitly and report it as the boundary violation
# it is, surfacing the cycle path; we ALSO keep the inverse-tree grep for any (theoretical)
# non-cyclic dependent.
err_file="$(mktemp)"
trap 'rm -f "$err_file"' EXIT
tree="$(cargo tree -i "$CLIENT" --all-features -e no-dev 2>"$err_file" || true)"
stderr="$(cat "$err_file")"

# A `cyclic package dependency` involving a forbidden dependent IS a boundary violation.
if printf '%s\n' "$stderr" | grep -qi 'cyclic package dependency'; then
  for crate in "${FORBIDDEN_DEPENDENTS[@]}"; do
    if printf '%s\n' "$stderr" | grep -qE "\`${crate} v[0-9]"; then
      echo "::error::BOUNDARY VIOLATION: a dependency CYCLE involving '${crate}' and \
'${CLIENT}' — the lean core gained an edge to the federation client (the dependency arrow \
must point ONE WAY into the engine)."
      printf '%s\n' "$stderr" | head -8
      echo "fedclient-boundary-guard: FAILED — cyclic edge from the lean core to ${CLIENT}."
      exit 1
    fi
  done
  # A cycle NOT involving the lean core is still a hard error worth failing on.
  echo "::error::fedclient-boundary-guard: \`cargo tree -i ${CLIENT}\` reported a dependency cycle:"
  printf '%s\n' "$stderr" | head -8
  exit 1
fi

if [ -z "$tree" ]; then
  echo "::error::fedclient-boundary-guard: \`cargo tree -i ${CLIENT}\` produced no output \
and no recognised error — is ${CLIENT} a registered workspace member?"
  printf '%s\n' "$stderr" | head -8
  exit 1
fi

fail=0
for crate in "${FORBIDDEN_DEPENDENTS[@]}"; do
  # Match a tree line whose package name is exactly $crate (followed by a space+version
  # or end-of-line), ignoring the leading tree-drawing glyphs — same matcher style as
  # wasm-deps-guard.sh.
  if printf '%s\n' "$tree" | grep -qE "[^[:alnum:]_-]${crate}( v[0-9]| |\$)"; then
    echo "::error::BOUNDARY VIOLATION: '${crate}' depends on '${CLIENT}' — the lean core \
must NEVER depend on the federation client (the dependency arrow points ONE WAY into the \
engine)."
    printf '%s\n' "$tree" | grep -E "[^[:alnum:]_-]${crate}( v[0-9]| |\$)" | head -3
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  echo "fedclient-boundary-guard: FAILED — the lean core gained an edge to ${CLIENT} (see above)."
  exit 1
fi
echo "fedclient-boundary-guard: OK — neither [${FORBIDDEN_DEPENDENTS[*]}] depends on ${CLIENT}."
