#!/usr/bin/env bash
# [OPUS-4.8] sq-toze.27 / [GPT-5.6] sq-epbw4: CycloneDX SBOMs for shipped JS code.
#
# The published `@sparq-org/sparq` package and the shared `@sparq/client` source are both in scope.
# The latter is private as a standalone package, but its runtime code and lazy codec dependencies
# are bundled into the shipped site/GUI artifacts. Each member gets a strict runtime view and a
# full build-time view. The release workflow provenance-attests every emitted `*.sbom.cdx.json`.
#
# Per-member lockfiles are derived deterministically from the committed root workspace lock. The
# transient locks are removed on every exit. cyclonedx-npm stays pinned and schema validation is
# enabled by default; the strict runtime pass never suppresses npm errors.
#
# Usage:
#   VERSION=v1.2.3 scripts/gen-js-sbom.sh
#   scripts/gen-js-sbom.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="${VERSION:-${GITHUB_REF_NAME:-$(git describe --tags --always 2>/dev/null || echo dev)}}"
VERSION="$(printf '%s' "$VERSION" | tr -cs 'A-Za-z0-9._-' '-')"
VERSION="${VERSION:-dev}"
OUT_DIR="${OUT_DIR:-sbom}"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

CDX_NPM="@cyclonedx/cyclonedx-npm@5.0.0"
ROOT_LOCK="$REPO_ROOT/package-lock.json"
if [ ! -f "$ROOT_LOCK" ]; then
  echo "ERROR: repo-root $ROOT_LOCK not found — cannot derive JS member locks for SBOM" >&2
  exit 1
fi

derived_locks=()
cleanup() {
  local lock
  for lock in "${derived_locks[@]}"; do
    rm -f "$lock"
  done
}
trap cleanup EXIT

prepare_lock() {
  local member="$1"
  local lock="$REPO_ROOT/$member/package-lock.json"
  if [ ! -f "$lock" ]; then
    node "$REPO_ROOT/scripts/derive-workspace-member-lock.mjs" "$member" "$lock"
    derived_locks+=("$lock")
  fi
  if [ ! -f "$lock" ]; then
    echo "ERROR: $lock not found and could not be derived" >&2
    exit 1
  fi
}

validate_sbom() {
  local file="$1"
  local expected_name="$2"
  node - "$file" "$expected_name" <<'NODE'
const fs = require("fs");
const file = process.argv[2];
const expectedName = process.argv[3];
const document = JSON.parse(fs.readFileSync(file, "utf8"));
const fail = (message) => { console.error(`ERROR: ${message} in ${file}`); process.exit(1); };
if (document.bomFormat !== "CycloneDX") fail("not CycloneDX");
if (document.specVersion !== "1.5") fail("not CycloneDX 1.5");
if (document.metadata?.component?.name !== expectedName) {
  fail(`unexpected root component ${JSON.stringify(document.metadata?.component?.name)}`);
}
const purls = (document.components ?? []).map((component) => component.purl).filter(Boolean);
if (purls.some((purl) => !purl.startsWith("pkg:npm/"))) fail("contains a non-npm purl");
console.log(`    OK ${file} — ${purls.length} component(s), CycloneDX ${document.specVersion}`);
NODE
}

assert_runtime_dependencies() {
  local file="$1"
  shift
  node - "$file" "$@" <<'NODE'
const fs = require("fs");
const file = process.argv[2];
const required = process.argv.slice(3);
const document = JSON.parse(fs.readFileSync(file, "utf8"));
const purls = (document.components ?? []).map((component) => decodeURIComponent(component.purl ?? ""));
const missing = required.filter((name) => !purls.some((purl) => purl.startsWith(`pkg:npm/${name}@`)));
if (missing.length > 0) {
  console.error(`ERROR: ${file} is missing required runtime component(s): ${missing.join(", ")}`);
  process.exit(1);
}
console.log(`    OK ${file} attests required runtime component(s): ${required.join(", ")}`);
NODE
}

assert_dev_superset() {
  local runtime_file="$1"
  local dev_file="$2"
  node - "$runtime_file" "$dev_file" <<'NODE'
const fs = require("fs");
const purls = (file) => new Set(
  (JSON.parse(fs.readFileSync(file, "utf8")).components ?? [])
    .map((component) => component.purl)
    .filter(Boolean),
);
const runtime = purls(process.argv[2]);
const development = purls(process.argv[3]);
const missing = [...runtime].filter((purl) => !development.has(purl));
if (missing.length > 0) {
  console.error("ERROR: development SBOM is not a superset of its runtime SBOM:", missing);
  process.exit(1);
}
console.log(`    OK runtime SBOM (${runtime.size} purl) is a subset of development (${development.size} purl)`);
NODE
}

generate_member() {
  local member="$1"
  local label="$2"
  local file_prefix="$3"
  local root_name="$4"
  shift 4
  local runtime_dependencies=("$@")
  local member_dir="$REPO_ROOT/$member"
  local runtime_out="$OUT_DIR/${file_prefix}-${VERSION}.sbom.cdx.json"
  local dev_out="$OUT_DIR/${file_prefix}-dev-${VERSION}.sbom.cdx.json"

  prepare_lock "$member"
  echo "==> generating CycloneDX SBOMs for $label (${VERSION})"
  echo "    -> strict runtime tree (--omit dev): $runtime_out"
  ( cd "$member_dir" && npx --yes "$CDX_NPM" \
      --spec-version 1.5 \
      --output-format JSON \
      --package-lock-only \
      --no-workspaces \
      --omit dev \
      --mc-type library \
      --output-file "$runtime_out" )

  # The full build view tolerates npm-ls peer-validation noise but still consumes its complete
  # output. The strict runtime view above does not use this flag. The superset guard below proves
  # the relaxed development pass cannot silently omit a shipped component.
  echo "    -> full build tree (including dev dependencies): $dev_out"
  ( cd "$member_dir" && npx --yes "$CDX_NPM" \
      --spec-version 1.5 \
      --output-format JSON \
      --package-lock-only \
      --no-workspaces \
      --ignore-npm-errors \
      --mc-type library \
      --output-file "$dev_out" )

  local file
  for file in "$runtime_out" "$dev_out"; do
    if [ ! -f "$file" ]; then
      echo "ERROR: expected CycloneDX SBOM was not produced at $file" >&2
      exit 1
    fi
    validate_sbom "$file" "$root_name"
  done
  assert_dev_superset "$runtime_out" "$dev_out"
  if [ "${#runtime_dependencies[@]}" -gt 0 ]; then
    assert_runtime_dependencies "$runtime_out" "${runtime_dependencies[@]}"
  fi
}

generate_member "js" "@sparq-org/sparq" "sparq-js" "sparq" "fzstd"
generate_member \
  "packages/sparq-client" \
  "@sparq/client" \
  "sparq-js-client" \
  "client" \
  "fzstd" \
  "seek-bzip" \
  "buffer"

echo "==> done. JS SBOMs in $OUT_DIR/:"
for file in "$OUT_DIR"/sparq-js*.cdx.json; do
  [ -e "$file" ] && echo "    $file"
done
