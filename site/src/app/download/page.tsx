import type { Metadata } from "next";

import { DownloadClient } from "./download-client";
import { PackageInstalls } from "./package-installs";

// [OPUS-4.8] sq-gl3cf / sq-ixc3 / sq-vw3ax.11 — the /download route.
//
// Server component: owns the route metadata; the interactive body lives in
// ./download-client.tsx (client-side OS detection + one-click DIRECT per-asset
// downloads via GitHub's version-stable `releases/latest/download/<alias>` endpoint,
// enriched with version/size/sha256 from an unauthenticated api.github.com fetch). It is
// honest that the desktop bundles are UNSIGNED developer builds (signing/notarization is
// the separate needs:user bead sq-v286.8).
export const metadata: Metadata = {
  title: "Download",
  description:
    "Install sparq through npm, crates.io, or PyPI, or download desktop and CLI/server builds for macOS, Windows, and Linux. Desktop bundles are unsigned developer builds.",
};

export default function DownloadPage() {
  return <DownloadClient packages={<PackageInstalls />} />;
}
