import { writeFileSync } from "node:fs";
import path from "node:path";
import type { NextConfig } from "next";

// [OPUS-4.8] sq-8thu / sq-uj38w — static-export config for GitHub Pages.
// Pages serves this project site at the ROOT of the custom domain https://sparq.jeswr.org/
// (org-migration cutover, sq-uj38w), so the deployed build is ROOT-RELATIVE (basePath '',
// no asset prefix). `output: "export"` writes a fully static `out/` tree (no Node server)
// that the Pages deploy workflow uploads.
//
// [OPUS-4.8] sq-9vw5 — env-switch the base path so the SAME export tree serves multiple hosts:
//
//   * GitHub Pages @ the custom-domain root (production): the Pages workflow builds with
//     `NEXT_PUBLIC_BASE_PATH=''` (see .github/workflows/pages.yml "Build static site"), so
//     basePath/assetPrefix are unset and every asset/route is root-relative (`/_next/...`).
//   * The Tauri 2 desktop webview: serves the frontend from the `tauri://` root (a local
//     `frontendDist`), so EVERY asset must ALSO be ROOT-relative — a `/prefix` 404s there.
//     The GUI's `gui/src-tauri/tauri.conf.json` `beforeBuildCommand` builds the site with
//     `NEXT_PUBLIC_BASE_PATH=''` too, honoured by this same config.
//
// `NEXT_PUBLIC_BASE_PATH` is the single switch — and the `@sparq/client` wasm loader already
// keys its RUNTIME asset URLs off the SAME env var, so the build-time route prefix and the
// runtime wasm-fetch prefix stay in lockstep. Build modes (also in site/README.md):
//   * Pages / Tauri (root): `NEXT_PUBLIC_BASE_PATH='' npm run build` -> basePath '' (root-relative)
//   * Legacy sub-path      : `npm run build` (no env)                -> basePath '/sparq' (fallback)
//
// An UNSET var keeps the historical `/sparq` sub-path as a LEGACY fallback (pre-cutover behaviour,
// no test/caller change); production (Pages + Tauri) always sets an explicit empty string for the
// root-relative export. Next requires basePath to be empty or start with `/` (no trailing slash),
// so we honour only `''` or a `/`-leading value and otherwise fall back to the legacy default.
const rawBasePath = process.env.NEXT_PUBLIC_BASE_PATH;
const basePath =
  rawBasePath === undefined
    ? "/sparq" // unset -> legacy /sparq sub-path fallback (production sets '' for the custom-domain root)
    : rawBasePath === "" || rawBasePath.startsWith("/")
      ? rawBasePath // '' (root-relative: Pages custom domain + Tauri) or an explicit '/prefix'
      : "/sparq"; // a malformed value falls back to the legacy default

/**
 * Writes `<distDir>/sparq-entry-files.json`: every client entrypoint (e.g. `app/capabilities/page`,
 * `main-app`) mapped to the JS and CSS files webpack emits for it, which is what Next ≤15 kept in
 * app-build-manifest.json.
 */
// The slice of webpack's Compiler this plugin uses (webpack ships inside next, untyped here).
type EntryFilesCompiler = {
  outputPath: string;
  hooks: {
    afterEmit: {
      tap(
        name: string,
        fn: (compilation: {
          entrypoints: Map<string | undefined, { getFiles(): Iterable<string> }>;
        }) => void,
      ): void;
    };
  };
};

class EntryFilesPlugin {
  apply(compiler: EntryFilesCompiler) {
    compiler.hooks.afterEmit.tap("sparq-entry-files", (compilation) => {
      const entries: Record<string, string[]> = {};
      for (const [name, entrypoint] of compilation.entrypoints) {
        if (name) entries[name] = [...entrypoint.getFiles()];
      }
      writeFileSync(
        path.join(compiler.outputPath, "sparq-entry-files.json"),
        JSON.stringify(entries, null, 2),
      );
    });
  }
}

const nextConfig: NextConfig = {
  // Next 16 `next dev` otherwise writes a Next-managed AGENTS.md into this app; the repo keeps
  // one AGENTS.md at its root.
  agentRules: false,
  // Next 16 `next dev` blocks dev resources (HMR socket, chunks) for any origin but
  // `localhost`; the Playwright web server is reached at 127.0.0.1 (playwright.config.ts),
  // and without this the pages never hydrate under test. Dev-only: no effect on the export.
  allowedDevOrigins: ["127.0.0.1"],
  output: "export",
  // Only emit basePath/assetPrefix when there IS a prefix. An empty basePath must not be
  // set as `assetPrefix: ""` either — leaving both unset is exactly the root-relative
  // behaviour the Tauri webview needs.
  ...(basePath ? { basePath, assetPrefix: basePath } : {}),
  trailingSlash: true,
  // Static export cannot run the Next.js image optimiser.
  images: { unoptimized: true },
  // [FABLE-5] sq-qgkwy.1 — tree-shake the MONOLITHIC `radix-ui` barrel. The site imports a
  // handful of primitives (Slot, Dialog, Tooltip, …) via `import { X } from "radix-ui"`, but
  // webpack does not shake the package's namespace re-export barrel: the ENTIRE primitive set
  // (Select, Menu, NavigationMenu, ScrollArea, Toast, Slider, Form, Menubar, …) was bundled
  // into a ~170 KB raw commons chunk shipped on almost every route's first load (measured via
  // source-map attribution — see PR). `optimizePackageImports` rewrites the barrel imports to
  // direct per-primitive imports at compile time, so only primitives actually used are bundled.
  // Purely mechanical (same symbols, same behaviour); lucide-react is already on Next's
  // built-in default list, radix-ui (the monolith) is not.
  //
  // [OPUS-5] sq-w728o — this line is a WORKAROUND for a missing upstream default. Getting
  // `radix-ui` onto Next's built-in list is already proposed in vercel/next.js#76065 (open,
  // unreviewed); see research/nextjs-optimize-package-imports-radix-upstream.md for the
  // measured evidence prepared for that thread. DELETE this line once sparq's Next floor
  // ships the default entry — it is then redundant, not load-bearing.
  experimental: { optimizePackageImports: ["radix-ui"] },
  // [FABLE-5] sq-ymr2e.10 — the visual-regression suite (SPARQ_VR=1, set only by scripts/vr.sh
  // inside the pinned Playwright container) screenshots pages served by `next dev`, and the dev
  // indicator badge would otherwise appear in — and destabilise — every baseline. Scoped to the
  // VR run only; normal `next dev` keeps the indicators.
  ...(process.env.SPARQ_VR ? { devIndicators: false as const } : {}),
  // The @sparq-org/sparq wrapper ships ESM with `.js` import specifiers that resolve
  // to `.ts`/`.tsx` sources in dev; mirror solid-pod-manager's webpack alias so the
  // bundler follows them.
  webpack: (config, { dev, isServer }) => {
    // Next 16 no longer writes app-build-manifest.json, whose per-entry file lists the bundle
    // guard (scripts/check-bundle.mjs) checks. Record the same lists for the client build.
    if (!dev && !isServer) config.plugins.push(new EntryFilesPlugin());
    config.resolve.extensionAlias = {
      ".js": [".ts", ".tsx", ".js", ".jsx"],
    };
    // [OPUS-4.8] sq-2e93 — resolve the shared framework-agnostic client
    // (`packages/sparq-client`) to its TS source. The package is consumed via a path
    // alias (no repo-root workspaces yet — see research/gui-design.md §3), so the
    // bundler needs this alias to follow the import the same way tsconfig `paths` does.
    config.resolve.alias = {
      ...config.resolve.alias,
      "@sparq/client": path.resolve(
        __dirname,
        "../packages/sparq-client/src/index.ts",
      ),
    };
    return config;
  },
};

export default nextConfig;
