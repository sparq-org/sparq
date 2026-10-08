// #2422 — build-time resource hints for the in-tab engine, resolved through the SAME
// content-hash manifest the runtime loader uses.
//
// The root layout preloads the engine glue + wasm so their downloads overlap the JS bundle
// (sq-qgkwy.2). The runtime loader (`@sparq/client` loadSparq → resolveWasmAsset) requests the
// CONTENT-HASHED names from public/wasm/wasm-manifest.json (sq-b66fc). The hints used to name the
// UNHASHED files, so on a hosted build with a manifest the preload never matched the loader's
// request: the browser downloaded the ~MB engine wasm twice (once for the unused preload, once for
// the hashed copy) and warned about an unused preload. Resolving the hints with the shared
// `resolveWasmAssetWithManifest` keeps the two URLs identical by construction.
//
// Server-only (reads the manifest from disk while the static export renders the layout); the
// manifest is written by scripts/sync-wasm.mjs in `prebuild`, before `next build` runs.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { resolveWasmAssetWithManifest, type WasmManifest } from "@sparq/client";

/** Reads public/wasm/wasm-manifest.json, or `null` when absent/unreadable (dev, no sync). */
export function readWasmManifest(
  publicDir: string = join(process.cwd(), "public"),
): WasmManifest | null {
  try {
    return JSON.parse(readFileSync(join(publicDir, "wasm", "wasm-manifest.json"), "utf8"));
  } catch {
    return null;
  }
}

/** The glue + wasm URLs the loader will request, for the layout's preload hints. */
export function enginePreloadHrefs(
  manifest: WasmManifest | null,
  basePath: string,
): { glue: string; wasm: string } {
  return {
    glue: resolveWasmAssetWithManifest("sparq_wasm.js", manifest, basePath),
    wasm: resolveWasmAssetWithManifest("sparq_wasm_bg.wasm", manifest, basePath),
  };
}
