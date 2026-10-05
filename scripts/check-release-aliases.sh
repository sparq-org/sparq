#!/usr/bin/env bash
# [FABLE-5] sq-vw3ax.11.3 — fail-closed release alias contract check.
#
# The /download page (site/src/app/download/download-client.tsx) renders direct-download
# buttons against version-STABLE alias assets served at
# https://github.com/<repo>/releases/latest/download/<alias>. release.yml stages alias
# COPIES of the canonical versioned assets (research/site-home-app-download-residuals.md §3);
# this script derives the alias set the site ACTUALLY references and verifies every one of
# them appears in the staged release asset list.
#
# FAIL-CLOSED: any site-referenced alias missing from the asset list exits non-zero, which
# blocks the release — a stable release must never ship silent-404 download buttons. EXTRA
# assets are always allowed (the normative table publishes more than the site references,
# e.g. the arm64-linux GUI/CLI aliases).
#
# The site-referenced set is extracted from the component source, so the SITE is the source
# of truth (renaming an alias there automatically retargets this check). Each class is
# derived TWICE, via structurally independent patterns, and the two derivations must agree
# EXACTLY — so a refactor that breaks one pattern (reformatting a key onto its own line,
# renaming a key, changing the alias prefix, …) fails this check loudly instead of silently
# shrinking the contract. (A mere count floor cannot do this: it approves a partial
# extraction that lost some aliases while keeping the count above the floor.)
#   * GUI aliases:  (a) every `file: "<name>"` key literal (primary + `secondary.file`)
#                   vs (b) every alias-shaped string literal `"sparq-gui-…"` anywhere in the
#                   component (versioned prerelease patterns use `sparq-gui_`, shortName uses
#                   `sparq-gui.`, so (b) matches exactly the alias literals). a == b or die.
#   * CLI aliases:  (a) every `token: "<t>", ext: "<e>"` pair returned by cliTarget(),
#                   composed to `sparq-cli-<t>.<e>` (the component builds
#                   `sparq-cli-${token}.${ext}`) vs (b) the `"<t>":` keys of
#                   CLI_PRERELEASE_PATTERNS — the same token set by construction (the
#                   component looks a pattern up per token). token-sets equal or die.
# Both derivations accept single OR double quotes, so a quote-style reformat neither breaks
# nor shrinks the contract. An alias REMOVED from the component entirely drops out of both
# derivations together — a legitimate site change, and the contract follows it.
# Self-tested (round-trip, teeth, and partial-extraction mutations) by
# scripts/tests/test_check_release_aliases.sh (docs-quality.yml).
#
# Usage: check-release-aliases.sh <asset-list-file> [download-client.tsx]
#   asset-list-file      one staged release asset filename per line (e.g. `ls -1 assets`)
#   download-client.tsx  path to the site component (defaults to the in-repo path)

set -euo pipefail

ASSET_LIST="${1:?usage: check-release-aliases.sh <asset-list-file> [download-client.tsx]}"
CLIENT="${2:-site/src/app/download/download-client.tsx}"

if [ ! -f "$ASSET_LIST" ]; then
  echo "ERROR: asset list file not found: $ASSET_LIST" >&2
  exit 2
fi
if [ ! -f "$CLIENT" ]; then
  echo "ERROR: download client source not found: $CLIENT" >&2
  exit 2
fi

# Q / NQ: either quote character, so a single-vs-double quote-style reformat of the
# component cannot break (or silently shrink) the extraction.
Q="[\"']"
NQ="[^\"']"

# GUI derivation (a): `file:` key string literals. The quote after the colon restricts the
# match to literals (the `file: string;` interface field and doc comments never match).
# `|| true`: a zero-match grep must reach the consistency guard below, not die under pipefail.
gui_from_keys="$(grep -oE "file:[[:space:]]*${Q}${NQ}+${Q}" "$CLIENT" \
  | sed -E "s/^file:[[:space:]]*[\"']([^\"']+)[\"']$/\1/" | sort -u || true)"

# GUI derivation (b): every alias-shaped string literal, independent of which key carries it.
gui_from_shape="$(grep -oE "${Q}sparq-gui-${NQ}+${Q}" "$CLIENT" | tr -d "\"'" | sort -u || true)"

if [ -z "$gui_from_keys" ] || [ "$gui_from_keys" != "$gui_from_shape" ]; then
  echo "ERROR: GUI alias extraction from $CLIENT is broken or inconsistent (fail-closed)." >&2
  echo "  via 'file:' key literals:      [$(printf '%s' "$gui_from_keys" | tr '\n' ' ')]" >&2
  echo "  via 'sparq-gui-*' literals:    [$(printf '%s' "$gui_from_shape" | tr '\n' ' ')]" >&2
  echo "  The component was likely refactored; update the extraction patterns in $0." >&2
  exit 2
fi
gui_aliases="$gui_from_keys"

# CLI derivation (a): cliTarget() returns `{ token: "<t>", ext: "<e>", label: ... }` per
# platform; the component downloads `sparq-cli-<t>.<e>`. Compose the same name per pair.
cli_pairs="$(grep -oE "token:[[:space:]]*${Q}${NQ}+${Q},[[:space:]]*ext:[[:space:]]*${Q}${NQ}+${Q}" "$CLIENT" || true)"
cli_aliases="$(printf '%s' "$cli_pairs" \
  | sed -E "s/^token:[[:space:]]*[\"']([^\"']+)[\"'],[[:space:]]*ext:[[:space:]]*[\"']([^\"']+)[\"']$/sparq-cli-\1.\2/" \
  | sort -u || true)"
cli_tokens_from_pairs="$(printf '%s' "$cli_pairs" \
  | sed -E "s/^token:[[:space:]]*[\"']([^\"']+)[\"'].*$/\1/" | sort -u || true)"

# CLI derivation (b): the CLI_PRERELEASE_PATTERNS map keys — `"<token>": /^sparq-cli-…/`.
cli_tokens_from_patterns="$(grep -oE "${Q}${NQ}+${Q}:[[:space:]]*/\\^sparq-cli-" "$CLIENT" \
  | sed -E "s/^[\"']([^\"']+)[\"'].*$/\1/" | sort -u || true)"

if [ -z "$cli_tokens_from_pairs" ] || [ "$cli_tokens_from_pairs" != "$cli_tokens_from_patterns" ]; then
  echo "ERROR: CLI alias extraction from $CLIENT is broken or inconsistent (fail-closed)." >&2
  echo "  tokens via cliTarget() pairs:            [$(printf '%s' "$cli_tokens_from_pairs" | tr '\n' ' ')]" >&2
  echo "  tokens via CLI_PRERELEASE_PATTERNS keys: [$(printf '%s' "$cli_tokens_from_patterns" | tr '\n' ' ')]" >&2
  echo "  The component was likely refactored; update the extraction patterns in $0." >&2
  exit 2
fi

gui_count=$(printf '%s' "$gui_aliases" | grep -c . || true)
cli_count=$(printf '%s' "$cli_aliases" | grep -c . || true)

echo "site-referenced alias contract ($((gui_count + cli_count)) aliases) from $CLIENT:"
printf '%s\n%s\n' "$gui_aliases" "$cli_aliases" | sed 's/^/  - /'

# RELEASE_OPTIONAL_ALIAS_PREFIXES (space-separated, e.g. "sparq-gui-") names alias
# families whose source lane is optional (release.yml's soft GUI rows). A missing optional
# alias warns (the site button 404s for this release) instead of blocking the whole release.
optional_alias() {
  local prefix
  for prefix in ${RELEASE_OPTIONAL_ALIAS_PREFIXES:-}; do
    case "$1" in "$prefix"*) return 0 ;; esac
  done
  return 1
}

missing=0
while IFS= read -r alias; do
  [ -n "$alias" ] || continue
  if ! grep -Fxq -- "$alias" "$ASSET_LIST"; then
    if optional_alias "$alias"; then
      echo "::warning::optional site-referenced alias '$alias' is not in the staged asset list" >&2
      continue
    fi
    echo "MISSING: site-referenced alias '$alias' is not in the staged asset list" >&2
    missing=$((missing + 1))
  fi
done < <(printf '%s\n%s\n' "$gui_aliases" "$cli_aliases")

if [ "$missing" -gt 0 ]; then
  echo "FAIL: $missing site-referenced alias(es) missing — refusing to release (fail-closed)." >&2
  exit 1
fi

echo "OK: every site-referenced alias is present in the staged asset list."
