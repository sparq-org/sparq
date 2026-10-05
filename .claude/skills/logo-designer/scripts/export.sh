#!/usr/bin/env bash
set -euo pipefail

# Usage: export.sh <input.svg> <output-dir>
# Exports SVG to PNG at standard logo sizes using the best available tool.

INPUT_SVG="${1:?Usage: export.sh <input.svg> <output-dir>}"
OUTPUT_DIR="${2:?Usage: export.sh <input.svg> <output-dir>}"
SIZES=(16 32 48 192 512 1024 2048)
BASENAME="logo"

mkdir -p "$OUTPUT_DIR"

# Copy SVG to output
cp "$INPUT_SVG" "$OUTPUT_DIR/$BASENAME.svg"

# Detect available tool.
# sparq vendoring note: the `npx --yes @aspect-build/resvg` path can DOWNLOAD and
# EXECUTE a remote package, so we never probe it as a tool-detection side effect (a silent
# network/supply-chain action). It is opt-in only: set ALLOW_NPX_RESVG=1 to enable it.
TOOL=""
if command -v resvg &>/dev/null; then
  TOOL="resvg"
elif [ "${ALLOW_NPX_RESVG:-0}" = "1" ] && command -v npx &>/dev/null && \
     npx --yes @aspect-build/resvg --help &>/dev/null 2>&1; then
  TOOL="npx-resvg"
elif command -v node &>/dev/null && node -e "require('sharp')" &>/dev/null 2>&1; then
  TOOL="sharp"
elif command -v inkscape &>/dev/null; then
  TOOL="inkscape"
elif command -v rsvg-convert &>/dev/null; then
  TOOL="rsvg-convert"
else
  echo "ERROR: No SVG-to-PNG converter found."
  echo ""
  echo "Install one of the following:"
  echo "  npm install -g @aspect-build/resvg     (recommended)"
  echo "  brew install inkscape"
  echo "  brew install librsvg"
  echo ""
  echo "Or set ALLOW_NPX_RESVG=1 to allow this script to fetch+run @aspect-build/resvg via npx"
  echo "(downloads and executes a package from the npm registry)."
  exit 1
fi

echo "Using: $TOOL"
echo ""

for SIZE in "${SIZES[@]}"; do
  OUTPUT="$OUTPUT_DIR/${BASENAME}-${SIZE}.png"
  case "$TOOL" in
    resvg)
      resvg "$INPUT_SVG" "$OUTPUT" --width "$SIZE"
      ;;
    npx-resvg)
      npx --yes @aspect-build/resvg "$INPUT_SVG" "$OUTPUT" --width "$SIZE"
      ;;
    sharp)
      # Pass paths as argv (process.argv), NOT interpolated into the JS source:
      # interpolation breaks on paths containing a quote and would allow JS injection from an
      # attacker-controlled path. Resize by width only (height auto) to preserve aspect ratio,
      # matching the other backends (which all scale by --width).
      node -e '
        const sharp = require("sharp");
        const [input, output, width] = [process.argv[1], process.argv[2], parseInt(process.argv[3], 10)];
        sharp(input)
          .resize({ width })
          .png()
          .toFile(output)
          .then(() => process.exit(0))
          .catch(e => { console.error(e); process.exit(1); });
      ' "$INPUT_SVG" "$OUTPUT" "$SIZE"
      ;;
    inkscape)
      inkscape "$INPUT_SVG" --export-type=png --export-filename="$OUTPUT" --export-width="$SIZE"
      ;;
    rsvg-convert)
      rsvg-convert -w "$SIZE" -o "$OUTPUT" "$INPUT_SVG"
      ;;
  esac
  # Backends set the output WIDTH and preserve aspect ratio, so height may differ from width
  # for non-square SVGs — report the width only rather than a misleading SIZExSIZE.
  echo "  Exported: ${BASENAME}-${SIZE}.png (width ${SIZE}px)"
done

echo ""
echo "Done. Files in: $OUTPUT_DIR"
