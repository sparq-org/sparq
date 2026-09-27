#!/usr/bin/env bash
# [OPUS-5] sq-6vshe.15 item 3 (issue #5164) — the sccache A/B's cache-key namespace.
#
# Derives `SCCACHE_GHA_VERSION`, which sccache uses as the prefix/version of every
# key it writes to the GitHub Actions cache backend, and exports it to $GITHUB_ENV.
#
# WHY IT IS COMPUTED RATHER THAN CONSTANT — research/ci-structural-speedup.md §6
# (bead sq-6vshe.5) states the key schema this experiment must not violate:
# `{rustc version, target triple, feature-family, Cargo.lock hash}`, and "never fall
# back across rustc or family (cross-family hits are how caches thrash at the 10 GB
# GHA cap; cross-rustc hits are how they rot)". So all four components are hashed in:
#
#   * `rustc -vV` supplies BOTH the exact toolchain (release + commit-hash, so a
#     runner-image toolchain bump re-namespaces automatically) and the `host:` target
#     triple, in one command;
#   * $ARCHIVE_FEATURES is the feature-family — ci.yml's build-archive job compiles a
#     specific opt-in set (#363) and a differently-featured build is a different
#     compilation;
#   * the Cargo.lock digest pins the resolved dependency graph.
#
# Correctness note on what this does and does not buy. sccache is already SAFE against
# a stale hit by construction — its per-object key is a hash of the whole compiler
# invocation (compiler binary, arguments, preprocessed source), so a mismatch is a
# MISS, never a wrong artifact. This namespace is therefore not a correctness backstop
# for the compiler; it is the THRASH and ISOLATION control the bead asks for: distinct
# toolchain/lockfile/feature populations get disjoint namespaces instead of competing
# for one LRU-evicted budget, and the `sccache-ab-` literal keeps this whole experiment
# in a namespace that can never alias a future production sccache key.
#
# Usage:  ./scripts/sccache-ab-namespace.sh          # appends to $GITHUB_ENV if set
# Always prints the computed value on stdout, so it is inspectable in the job log and
# runnable outside Actions.
set -euo pipefail

features="${ARCHIVE_FEATURES:-}"
if [[ -z "$features" ]]; then
  echo "sccache-ab-namespace: ARCHIVE_FEATURES is unset — refusing to compute a" \
       "namespace that would alias across feature sets (sq-6vshe.5 key schema)." >&2
  exit 1
fi

if [[ ! -f Cargo.lock ]]; then
  echo "sccache-ab-namespace: no Cargo.lock in $(pwd) — refusing to compute a" \
       "namespace that would alias across dependency graphs (sq-6vshe.5 key schema)." >&2
  exit 1
fi

# `rustc -vV` carries the toolchain AND the host triple; both are key components.
#
# CAPTURED AND CHECKED SEPARATELY, not piped inline. Piping it straight into the
# digest is FAIL-OPEN and was a live defect here: `set -euo pipefail` does not abort
# a failing command inside a `{ ... } | sha256sum` group in a command substitution, so
# a broken/absent toolchain contributed an EMPTY string and the script still printed a
# confident namespace — silently aliasing across every rustc version, which is the one
# thing sq-6vshe.5 says must never happen ("cross-rustc hits are how they rot").
# Observed while authoring this: on a box where `rustc -vV` exits 1 with empty stdout,
# the unchecked version emitted a namespace anyway.
rustc_version=""
if ! rustc_version="$(rustc -vV)" || [[ -z "$rustc_version" ]]; then
  echo "sccache-ab-namespace: 'rustc -vV' failed or printed nothing — refusing to" \
       "compute a namespace that would alias across toolchains (sq-6vshe.5 key schema)." >&2
  exit 1
fi

lock_digest=""
if ! lock_digest="$(sha256sum Cargo.lock)" || [[ -z "$lock_digest" ]]; then
  echo "sccache-ab-namespace: could not digest Cargo.lock — refusing to compute a" \
       "namespace that would alias across dependency graphs (sq-6vshe.5 key schema)." >&2
  exit 1
fi

digest="$(
  printf '%s\nfeatures=%s\n%s\n' "$rustc_version" "$features" "$lock_digest" |
    sha256sum | cut -c1-16
)"

namespace="sccache-ab-${digest}"
echo "$namespace"
if [[ -n "${GITHUB_ENV:-}" ]]; then
  echo "SCCACHE_GHA_VERSION=${namespace}" >> "$GITHUB_ENV"
fi
