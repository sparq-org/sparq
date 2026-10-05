#!/usr/bin/env bash
# sq-toze.3 (GX-2): per-release CycloneDX SBOM + VEX generator.
#
# Produces, into $OUT_DIR (default: ./sbom):
#   - sparq-cli-<version>.sbom.cdx.json     CycloneDX SBOM for the released sparq-cli binary
#   - sparq-server-<version>.sbom.cdx.json  CycloneDX SBOM for the released sparq-server binary
#   - sparq-gui-<version>.sbom.cdx.json     CycloneDX SBOM for the Tauri desktop GUI shell
#                                           (gui/src-tauri; sq-8n1c — the released desktop bundles)
#   - sparq-<version>.vex.cdx.json          the checked-in VEX (supply-chain/vex.cdx.json) with
#                                           the released version + a generated timestamp stamped in
#
# All three are attached to the GitHub Release by .github/workflows/release.yml (and SLSA-
# attested there). Each SBOM enumerates that binary's full dependency tree (NTIA minimum
# elements: supplier, component name, version, unique purl, dependency relationships, author,
# timestamp). The VEX states the exploitability of every advisory cargo-deny is configured to
# ignore (kept 1:1 in sync with deny.toml [advisories].ignore).
#
# cargo-cyclonedx (`--all`) processes every workspace member and writes <crate>.cdx.json
# next to each member's Cargo.toml; we collect the two released-binary crates and discard the
# rest. `--all` is required here: the root is a virtual workspace (no [package]), so a bare
# `cargo cyclonedx` has no package to resolve. We use the **default** feature set (no
# --all-features) so each SBOM matches the actually-shipped binary — the release builds
# (release.yml / Dockerfile) build `-p sparq-cli`/`-p sparq-server` without --all-features,
# and the CI SBOM job (supply-chain.yml) likewise uses `cargo cyclonedx --all`.
#
# Requires: cargo, cargo-cyclonedx, jq, python3. Usage:
#   VERSION=v1.2.3 scripts/gen-sbom-vex.sh            # explicit version
#   scripts/gen-sbom-vex.sh                           # derives version from git describe
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="${VERSION:-${GITHUB_REF_NAME:-$(git describe --tags --always 2>/dev/null || echo dev)}}"
OUT_DIR="${OUT_DIR:-sbom}"
mkdir -p "$OUT_DIR"

# sq-a1fgx: cargo-cyclonedx invokes `cargo metadata`, which may download
# registry crates even though it does not compile them. Keep transient crates.io
# resets from aborting a release while still failing closed on a sustained outage.
retry_cargo_cyclonedx() {
  local attempt=1
  local max_attempts=3

  until cargo cyclonedx "$@"; do
    if [ "$attempt" -ge "$max_attempts" ]; then
      echo "ERROR: cargo cyclonedx failed after $attempt attempts" >&2
      return 1
    fi
    echo "cargo cyclonedx attempt $attempt failed; retrying in 10s..." >&2
    sleep 10
    attempt=$((attempt + 1))
  done
}

echo "==> generating CycloneDX SBOMs (whole workspace, sparq ${VERSION})"
# sq-toze.28 (GS-4 / CDX-3): emit CycloneDX 1.5 natively. cargo-cyclonedx
# 0.5.9 (the pinned toolchain version) accepts --spec-version {1.3,1.4,1.5} and defaults
# to 1.3; pinning 1.5 aligns the SBOM with the VEX (also 1.5) and unlocks the 1.5
# metadata.lifecycles slot (populated, phase=build, by scripts/sbom-normalize.jq).
retry_cargo_cyclonedx --all --format json --spec-version 1.5

# sq-toze.30 (GS-6 / F-6): cargo-cyclonedx 0.5.9 stamps the absolute build dir
# into every workspace/path-dependency bom-ref (path+file:///abs/...#ver) and purl
# (?download_url=file://...), which would leak the CI runner's filesystem layout into the
# PUBLISHED SBOM. Normalize each shipped SBOM through scripts/sbom-normalize.jq, which
# rewrites those refs to the canonical, host-independent pkg:cargo/<name>@<version> form,
# rewriting the dependency graph (component bom-ref + every ref/dependsOn edge) in lock-step
# so internal references stay consistent. The transform is deterministic and idempotent.
NORMALIZE_JQ="$REPO_ROOT/scripts/sbom-normalize.jq"
for crate in sparq-cli sparq-server; do
  src="crates/${crate}/${crate}.cdx.json"
  dst="$OUT_DIR/${crate}-${VERSION}.sbom.cdx.json"
  jq -f "$NORMALIZE_JQ" "$src" > "$dst"
  rm -f "$src"
  # Belt-and-braces: fail loudly if any host-revealing absolute path survived.
  if grep -qE 'path\+file://|download_url=file://|/home/' "$dst"; then
    echo "ERROR: host path leaked into $dst after normalization" >&2
    exit 1
  fi
  echo "    -> $dst (abs-path-normalized)"
done

# Discard the per-member SBOMs we don't ship (keeps the worktree clean for `git status`).
find crates -name '*.cdx.json' -delete

# sq-8n1c: per-release CycloneDX SBOM for the Tauri desktop GUI shell. The release
# ships UNSIGNED desktop bundles (.dmg/.msi/.exe/.deb/.AppImage) built from gui/src-tauri, so
# its dependency tree (Tauri + the webview bindings) must be enumerated for "SBOM rides for
# free" to be literally true. The GUI crate is a STANDALONE crate root (its own empty
# `[workspace]`, excluded from the main workspace), so the `cargo cyclonedx --all` above does
# NOT reach it — generate it separately from inside gui/src-tauri. cargo-cyclonedx resolves the
# dependency graph from Cargo.lock WITHOUT compiling (no webkit/system libs needed), so this
# runs on the same plain ubuntu runner as the rest of this script. Default feature set, to
# match the shipped bundle. Then normalize (abs-path scrub) + leak-check identically to the
# CLI/server SBOMs above.
if [ -f gui/src-tauri/Cargo.toml ]; then
  echo "==> generating CycloneDX SBOM (GUI shell, sparq ${VERSION})"
  ( cd gui/src-tauri && retry_cargo_cyclonedx --format json --spec-version 1.5 )
  gui_src="gui/src-tauri/sparq-gui.cdx.json"
  gui_dst="$OUT_DIR/sparq-gui-${VERSION}.sbom.cdx.json"
  jq -f "$NORMALIZE_JQ" "$gui_src" > "$gui_dst"
  find gui/src-tauri -name '*.cdx.json' -delete
  if grep -qE 'path\+file://|download_url=file://|/home/' "$gui_dst"; then
    echo "ERROR: host path leaked into $gui_dst after normalization" >&2
    exit 1
  fi
  echo "    -> $gui_dst (abs-path-normalized)"
fi

echo "==> stamping VEX (sparq ${VERSION})"
VEX_OUT="$OUT_DIR/sparq-${VERSION}.vex.cdx.json"
python3 - "$VERSION" "$VEX_OUT" <<'PY'
import json, sys, datetime, pathlib
version, out = sys.argv[1], sys.argv[2]
ver = version.lstrip("v")
src = pathlib.Path("supply-chain/vex.cdx.json")
doc = json.loads(src.read_text())
meta = doc.setdefault("metadata", {})
meta["timestamp"] = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
comp = meta.setdefault("component", {})
comp["version"] = ver
comp["bom-ref"] = f"pkg:cargo/sparq@{ver}"
for v in doc.get("vulnerabilities", []):
    for a in v.get("affects", []):
        a["ref"] = comp["bom-ref"]
pathlib.Path(out).write_text(json.dumps(doc, indent=2) + "\n")
print(f"    -> {out}")
PY

echo "==> done. artifacts in $OUT_DIR/:"
ls -1 "$OUT_DIR/"
