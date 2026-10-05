import path from "node:path";
import type { NextConfig } from "next";

// [OPUS-4.8] sq-ixc3.8 / sq-ixc3.9 — static-export config for the DISTINCT operational GUI
// frontend (NOT the marketing site). This app builds to a backend-free `out/` tree consumed by
// BOTH targets, selected by NEXT_PUBLIC_BASE_PATH (the same single switch the site + the
// @sparq/client wasm loader already key off, kept in lockstep):
//
//   * Tauri 2 desktop webview ("build:tauri"): serves the frontend from the `tauri://` root, so
//     EVERY asset must be ROOT-relative (a `/prefix` 404s there). `tauri.conf.json`'s
//     `beforeBuildCommand` runs `build:tauri`, which sets NEXT_PUBLIC_BASE_PATH='' → basePath ''.
//   * Hosted "Try the GUI live" web target ("build:web"): served under a sub-path on the same
//     GitHub-Pages-style host. The maintainer picked the URL slot (bead sq-vnd0i, Option B):
//     the live GUI is hosted at the `/app` sub-path (the site's "App" nav destination;
//     "/try" stays the lightweight REPL). This app defaults the web build to `/app`,
//     overridable via the env var.
//
// An UNSET var keeps the web default (`/app`); an explicit empty string selects the
// root-relative (Tauri) export. Next requires basePath to be empty or start with `/` (no
// trailing slash), so we honour only `''` or a `/`-leading value and otherwise fall back.
const rawBasePath = process.env.NEXT_PUBLIC_BASE_PATH;
const basePath =
  rawBasePath === undefined
    ? "/app" // unset → hosted-web default (the live-GUI "/app" sub-path)
    : rawBasePath === "" || rawBasePath.startsWith("/")
      ? rawBasePath // '' (Tauri root-relative) or an explicit '/prefix'
      : "/app";

const nextConfig: NextConfig = {
  output: "export",
  // Only emit basePath/assetPrefix when there IS a prefix. An empty basePath must not be set as
  // `assetPrefix: ""` either — leaving both unset is exactly the root-relative behaviour the
  // Tauri webview needs.
  ...(basePath ? { basePath, assetPrefix: basePath } : {}),
  trailingSlash: true,
  // Static export cannot run the Next.js image optimiser.
  images: { unoptimized: true },
  // The @sparq-org/sparq wrapper ships ESM with `.js` import specifiers that resolve to `.ts`/`.tsx`
  // sources; mirror the site's webpack alias so the bundler follows them.
  webpack: (config) => {
    config.resolve.extensionAlias = {
      ".js": [".ts", ".tsx", ".js", ".jsx"],
    };
    // Resolve the shared framework-agnostic client (`packages/sparq-client`) to its TS source —
    // the SAME single-source-of-truth the site consumes, so the GUI is a zero-new-copy consumer
    // of the engine TS surface (research/gui-design.md §4).
    config.resolve.alias = {
      ...config.resolve.alias,
      "@sparq/client": path.resolve(
        __dirname,
        "../../packages/sparq-client/src/index.ts",
      ),
    };
    return config;
  },
};

export default nextConfig;
