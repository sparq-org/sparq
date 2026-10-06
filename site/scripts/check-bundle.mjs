// [FABLE-5] sq-vw3ax.9 — repeatable, CI-runnable BUNDLE GUARD for the wasm-free main bundle.
//
// Proves the two "route + prove in under one screen" surfaces — the Home REPL (`/`) and the
// consolidated /capabilities gallery — never ship the ~2.5 MB engine wasm, the in-tab ZK prover
// (@aztec/bb.js + @noir-lang/noir_js, ~MB), or any of the 8 heavy demo chunks in their FIRST-LOAD
// payload. Each of those must arrive ONLY on disclosure-expand (see components/capabilities/
// lazy-demo.tsx + components/home/hero-runner-lazy.tsx). This is the §7-risk-2 requirement of
// research/website-redesign.md: "Verify with a bundle check that no demo chunk loads on route
// entry." The e2e/capabilities-lazy.spec.ts test proves the SAME invariant in a live browser; this
// is its cheap, deterministic, build-artifact-only counterpart — no browser, so CI-runnable as:
//
//     npm run build && npm run check:bundle            # (also runs automatically as `postbuild`)
//
// It reads ONLY the build's own manifests, so there is no wall-clock / timing threshold and no
// third-party-internal string fingerprint to drift (the honesty copy legitimately mentions
// "UltraHonk"/"@aztec/bb.js", so a content grep would false-positive — we match CHUNK FILES).
//
// TWO invariants, plus a config-preservation guard:
//   1. PRESENCE — every heavy module is STILL a `next/dynamic` split point (has an entry in
//      react-loadable-manifest.json). This catches the sneaky regression where a demo is switched
//      back to a STATIC import: its code would fold into the route-entry chunk AND vanish from the
//      loadable manifest, so a pure "is this chunk in the entry set" test would FALSELY pass.
//   2. ISOLATION — none of those heavy modules' code-split chunk files appear in the FIRST-LOAD
//      set (app-build-manifest.json `pages`) of the guarded routes.
//   3. CONFIG — images:{unoptimized:true} and basePath prefixing are load-bearing for GitHub
//      Pages / Tauri (bead item 4). Read straight from the build's OWN resolved artifacts
//      (routes-manifest.basePath, images-manifest), so the check adapts to BOTH build modes
//      (the default `/sparq` sub-path and the `NEXT_PUBLIC_BASE_PATH=''` root-relative export).

import { readFileSync, existsSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * Heavy modules that MUST stay code-split OUT of the guarded routes' first-load payload. Keyed by
 * the react-loadable-manifest KEY SUFFIX (the "-> <import specifier>" tail of each dynamic-import
 * split point), which is stable across builds — the hashed chunk FILENAMES are not, so we never
 * hard-code them. These are exactly the modules dynamic()-imported by lazy-demo.tsx /
 * hero-runner-lazy.tsx, and the two ~MB ZK prover deps dynamic-imported by lib/zk-prover.ts.
 */
export const HEAVY_MODULE_SUFFIXES = [
  // The 8 /capabilities gallery demos (lazy-demo.tsx).
  "-> @/components/geosparql-walkthrough",
  "-> @/components/text-playground",
  "-> @/components/vector-walkthrough",
  "-> @/components/genai-walkthrough",
  "-> @/components/http-server-walkthrough",
  "-> @/components/rsp-playground",
  "-> @/components/zk-car-hire",
  "-> @/components/mpc-demo",
  // The Home hero's in-browser SPARQL runner — pulls the editors + @sparq/client engine wasm glue.
  "-> @/components/home/hero-runner",
  // [GPT-5] #3606 — the hero's optional Graph view must remain a nested split point so its SVG
  // renderer and result-graph derivation load only when the visitor invokes that view.
  "-> @/components/repl-graph-view",
  // The in-tab ZK prover (~MB each), dynamic-imported inside lib/zk-prover.ts.
  "-> @aztec/bb.js",
  "-> @noir-lang/noir_js",
  // [GPT-5.6] #1046 — the zstd decoder, dynamic-imported inside @sparq/client's decompress.ts and
  // reached through the site's bytesToRdf pipeline. The maintainer's standing rule is that
  // rarely-used codecs are fetched ONLY when a user actually decodes a file of that
  // type, so it must STAY a lazy split point (never fold into a route-entry bundle).
  "-> fzstd",
];

/**
 * The guarded surfaces. `manifestKey` is the app-build-manifest.json `pages` key;
 * `html` is the exported document under out/ (trailingSlash:true → <route>/index.html).
 * `/` + `/capabilities` are the original sq-vw3ax.9 "route + prove in under one screen"
 * surfaces, guarded against EVERY heavy module; `/surface/data-formats` joins them
 * ([FABLE-5] sq-4ssz1) guarded against the lazy zstd codec ONLY (`heavyModules`) — the
 * page legitimately shares editor commons chunks with the hero-runner's dynamic file
 * list, so the blunt all-modules check would false-positive there. What must never
 * reach its first-load payload is the codec decoder itself.
 */
export const GUARDED_ROUTES = [
  { manifestKey: "/page", label: "/", html: "index.html" },
  { manifestKey: "/capabilities/page", label: "/capabilities", html: "capabilities/index.html" },
  {
    manifestKey: "/surface/data-formats/page",
    label: "/surface/data-formats",
    html: "surface/data-formats/index.html",
    heavyModules: ["-> fzstd"],
  },
];

/**
 * Pure analyser: takes the already-parsed manifests + the exported HTML per guarded route, and
 * returns `{ errors, summary }`. Pure (no fs) so it is unit-testable with synthetic manifests
 * (see test/bundle-guard.test.mjs). The CLI `main()` below does the file I/O and calls this.
 *
 * @param {object} p
 * @param {Record<string, {files?: string[]} | string[]>} p.loadable  react-loadable-manifest.json
 * @param {{ pages: Record<string, string[]> }} p.app                 app-build-manifest.json
 * @param {{ basePath?: string }} p.routes                            routes-manifest.json
 * @param {{ images?: { unoptimized?: boolean } }} p.images           images-manifest.json
 * @param {Record<string, string | undefined>} p.htmlByLabel          exported HTML keyed by route label
 */
export function analyzeBundle({ loadable, app, routes, images, htmlByLabel }) {
  const errors = [];

  // Normalise the loadable manifest into [{ key, files }] (Next has used both shapes historically).
  const entries = Object.entries(loadable ?? {}).map(([key, v]) => ({
    key,
    files: Array.isArray(v) ? v : Array.isArray(v?.files) ? v.files : [],
  }));

  // 1. PRESENCE — each heavy module must still be a dynamic split point; collect its chunk
  //    files, both as one union set and per module (a route can scope its guard — see below).
  const heavyFiles = new Set();
  const heavyFilesByModule = new Map();
  const heavyReport = [];
  for (const suffix of HEAVY_MODULE_SUFFIXES) {
    const name = suffix.replace(/^-> /, "");
    const matches = entries.filter((e) => e.key.endsWith(suffix));
    if (matches.length === 0) {
      errors.push(
        `PRESENCE: "${name}" is no longer a next/dynamic split point (absent from ` +
          `react-loadable-manifest.json). A static import would fold its heavy code into the ` +
          `route-entry bundle — keep it wrapped in next/dynamic (see lazy-demo.tsx / ` +
          `hero-runner-lazy.tsx).`,
      );
      continue;
    }
    const files = new Set();
    for (const m of matches) for (const f of m.files) {
      heavyFiles.add(f);
      files.add(f);
    }
    heavyFilesByModule.set(suffix, files);
    heavyReport.push(`${name} → ${files.size} chunk(s)`);
  }

  // 2. ISOLATION — no heavy chunk file may appear in a guarded route's first-load set.
  //    A route with `heavyModules` is checked against just those modules' chunks (used when
  //    the route legitimately shares commons chunks with an unrelated heavy module's file
  //    list, but must never ship a specific module — e.g. the lazy zstd codec).
  const routeReport = [];
  for (const { manifestKey, label, heavyModules } of GUARDED_ROUTES) {
    const first = app?.pages?.[manifestKey];
    if (!Array.isArray(first)) {
      errors.push(
        `ISOLATION: no first-load entry for guarded route ${label} (${manifestKey}) in ` +
          `app-build-manifest.json — did the build write it?`,
      );
      continue;
    }
    let guardSet = heavyFiles;
    if (Array.isArray(heavyModules)) {
      guardSet = new Set();
      for (const suffix of heavyModules) {
        for (const f of heavyFilesByModule.get(suffix) ?? []) guardSet.add(f);
      }
    }
    const leaked = first.filter((f) => guardSet.has(f));
    if (leaked.length > 0) {
      errors.push(
        `ISOLATION: ${label} first-load bundle ships heavy code-split chunk(s) that must be ` +
          `lazy-loaded on expand: ${leaked.join(", ")}`,
      );
    }
    routeReport.push(`${label}: ${first.length} first-load JS files, ${leaked.length} heavy`);
  }

  // 3. CONFIG PRESERVATION — images.unoptimized + basePath prefixing (load-bearing for Pages/Tauri).
  const unoptimized = images?.images?.unoptimized === true;
  if (!unoptimized) {
    errors.push(
      `CONFIG: images.unoptimized must stay true (a static export cannot run the Next image ` +
        `optimiser) — images-manifest reports ${JSON.stringify(images?.images?.unoptimized)}.`,
    );
  }
  const basePath = routes?.basePath ?? "";
  for (const { label, html } of GUARDED_ROUTES) {
    const doc = htmlByLabel?.[label];
    if (doc == null) {
      errors.push(
        `CONFIG: exported HTML for ${label} not found (out/${html}) — was the static export written?`,
      );
      continue;
    }
    const prefix = `${basePath}/_next/`;
    if (!doc.includes(prefix)) {
      errors.push(
        `CONFIG: ${label} exported HTML references no assets under the build's resolved basePath ` +
          `("${prefix}") — basePath prefixing is broken (load-bearing for Pages/Tauri).`,
      );
    }
    if (doc.includes("/_next/image?")) {
      errors.push(
        `CONFIG: ${label} exported HTML references the Next image optimiser (/_next/image?), which ` +
          `a static export can't serve — images.unoptimized must be true.`,
      );
    }
  }

  return { errors, summary: { basePath, unoptimized, heavyReport, routeReport } };
}

/**
 * [FABLE-5] sq-qgkwy.1 — 4. DATA ISOLATION: the ~1.3 MB benchmark snapshot
 * (src/data/benchmarks.generated.json) is SERVER-ONLY. It renders into the prerendered
 * HTML/RSC payload; no emitted client JS chunk may embed it. A "use client" module (or
 * anything transitively imported by one — the subtle case: metric-table.tsx has no
 * directive yet is client-bundled via suite-group) importing a VALUE from
 * src/data/benchmarks.ts folds the whole snapshot into a chunk — that double-shipped
 * /benchmarks/[type]'s data as a ~762 KB raw first-load page chunk on top of the already
 * prerendered HTML. Client code imports display helpers from src/lib/fmt-num.ts instead.
 *
 * Detection is by FINGERPRINT, not size threshold: `latest.commit` from the snapshot is a
 * full commit sha — long, unique, and by construction present in any chunk that embeds the
 * JSON. Pure (fs-free) for unit-testing; main() supplies the chunk texts.
 *
 * @param {string} fingerprint          distinctive string from the server-only snapshot
 * @param {Array<{file: string, text: string}>} chunks  emitted client JS chunks
 * @returns {string[]} offending chunk file names
 */
export function findSnapshotLeaks(fingerprint, chunks) {
  if (!fingerprint || fingerprint.length < 8) {
    throw new Error(
      `DATA-ISOLATION: unusable snapshot fingerprint ${JSON.stringify(fingerprint)} — ` +
        `did benchmarks.generated.json lose latest.commit?`,
    );
  }
  return chunks.filter((c) => c.text.includes(fingerprint)).map((c) => c.file);
}

/** Read + JSON-parse a build manifest, or return null if it does not exist. */
function readJson(path) {
  return existsSync(path) ? JSON.parse(readFileSync(path, "utf8")) : null;
}

/** Every emitted client JS chunk under .next/static/chunks, recursively. */
function readClientChunks(dir, prefix = "") {
  const chunks = [];
  if (!existsSync(dir)) return chunks;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) chunks.push(...readClientChunks(join(dir, entry.name), rel));
    else if (entry.name.endsWith(".js"))
      chunks.push({ file: rel, text: readFileSync(join(dir, entry.name), "utf8") });
  }
  return chunks;
}

function main() {
  const siteRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
  const nextDir = join(siteRoot, ".next");
  const outDir = join(siteRoot, "out");

  const app = readJson(join(nextDir, "app-build-manifest.json"));
  const loadable = readJson(join(nextDir, "react-loadable-manifest.json"));
  const routes = readJson(join(nextDir, "routes-manifest.json"));
  const images = readJson(join(nextDir, "images-manifest.json"));

  if (!app || !loadable || !routes || !images) {
    console.error(
      "[check-bundle] Build manifests missing under site/.next — run `npm run build` first.",
    );
    process.exit(1);
  }

  const htmlByLabel = {};
  for (const { label, html } of GUARDED_ROUTES) {
    const p = join(outDir, html);
    htmlByLabel[label] = existsSync(p) ? readFileSync(p, "utf8") : undefined;
  }

  const { errors, summary } = analyzeBundle({ loadable, app, routes, images, htmlByLabel });

  // 4. DATA ISOLATION — the server-only benchmark snapshot must not appear in ANY client chunk.
  const snapshot = readJson(join(siteRoot, "src", "data", "benchmarks.generated.json"));
  const fingerprint = snapshot?.latest?.commit;
  let leakReport = "skipped (no snapshot)";
  if (snapshot) {
    const leaked = findSnapshotLeaks(
      fingerprint,
      readClientChunks(join(nextDir, "static", "chunks")),
    );
    for (const file of leaked) {
      errors.push(
        `DATA-ISOLATION: client chunk ${file} embeds the server-only benchmark snapshot ` +
          `(fingerprint: latest.commit ${fingerprint}). A client-bundled module imports a ` +
          `VALUE from src/data/benchmarks.ts — import display helpers from ` +
          `"@/lib/fmt-num" instead (see that file's header).`,
      );
    }
    leakReport = leaked.length === 0 ? "no client chunk embeds it" : `${leaked.length} LEAK(S)`;
  }

  console.log("[check-bundle] wasm-free main-bundle guard (sq-vw3ax.9)");
  console.log(`  basePath (resolved): "${summary.basePath}"   images.unoptimized: ${summary.unoptimized}`);
  console.log("  heavy modules kept code-split (lazy on expand):");
  for (const line of summary.heavyReport) console.log(`    - ${line}`);
  console.log("  guarded route first-load payloads:");
  for (const line of summary.routeReport) console.log(`    - ${line}`);
  console.log(`  server-only benchmark snapshot: ${leakReport}`);

  if (errors.length > 0) {
    console.error(`\n[check-bundle] FAIL — ${errors.length} problem(s):`);
    for (const e of errors) console.error(`  ✗ ${e}`);
    process.exit(1);
  }

  console.log(
    "\n[check-bundle] PASS — no engine wasm / ZK prover / demo / lazy-codec chunk in any " +
      "guarded route's first-load bundle; server-only benchmark snapshot in no client chunk; " +
      "basePath + images.unoptimized preserved.",
  );
}

// Run only as a CLI (not when imported by the unit test).
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main();
}
