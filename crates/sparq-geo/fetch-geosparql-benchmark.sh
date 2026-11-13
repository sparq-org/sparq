#!/usr/bin/env bash
# Fetch external GPL-2.0 fixtures; never vendor them. The default revision is
# calibrated to the ratchet. GEOSPARQL_BENCHMARK_PIN=HEAD selects upstream HEAD;
# any other revision overrides the pin. GEOSPARQL_BENCHMARK_DIR uses an existing
# checkout verbatim, without fetching or modifying it (default: tests/fixtures/...).
set -euo pipefail
root="$(cd "$(dirname "$0")" && pwd)"
repo="${GEOSPARQL_BENCHMARK_DIR:-$root/tests/fixtures/geosparql-benchmark}"
pin="${GEOSPARQL_BENCHMARK_PIN:-879e0746e74c1327f74e57366104bcf1a6d63fb1}"
# Even a clean `git status` can refresh the index; directory overrides are read-only.
fixture_status() { git --no-optional-locks -C "$repo" status --porcelain "$@"; }
if [[ -z "${GEOSPARQL_BENCHMARK_DIR:-}" ]]; then
  mkdir -p "$(dirname "$repo")"
  if [[ ! -d "$repo/.git" ]]; then
    git clone https://github.com/OpenLinkSoftware/GeoSPARQLBenchmark.git "$repo"
  else
    # Refuse edited fixtures rather than silently replacing a developer's files.
    if [[ -n "$(fixture_status)" ]]; then
      echo "benchmark checkout has local changes: $repo" >&2
      exit 1
    fi
  fi
  git -C "$repo" fetch origin "$pin"
  git -C "$repo" checkout --detach FETCH_HEAD
fi
if [[ -n "$(fixture_status -- src/main/resources)" ]]; then
  echo "benchmark resources have local changes: $repo" >&2
  exit 1
fi
for dir in gsb_dataset gsb_queries gsb_answers; do
  [[ -d "$repo/src/main/resources/$dir" ]] || { echo "missing benchmark $dir: $repo" >&2; exit 1; }
done
git -C "$repo" rev-parse HEAD >&2
(cd "$repo" && pwd -P)
