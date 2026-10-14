// [FABLE-5] sq-vw3ax.9 — unit tests for the wasm-free-main-bundle guard's pure analyser
// (scripts/check-bundle.mjs `analyzeBundle`). These feed SYNTHETIC manifests — a healthy build, a
// chunk that LEAKED into a route entry, a demo that regressed to a STATIC import, and a broken
// config — and assert the analyser flags exactly the right failure. They need no `next build`, so
// they run in the fast `npm run test:unit` lane (site-e2e); the real `npm run check:bundle` /
// `postbuild` run the SAME analyser against the actual .next/out artifacts.
import { test } from "node:test";
import assert from "node:assert/strict";

import {
  analyzeBundle,
  firstLoadFromClientManifest,
  HEAVY_MODULE_SUFFIXES,
  GUARDED_ROUTES,
} from "../scripts/check-bundle.mjs";

// A minimal healthy fixture: every heavy module is a distinct dynamic split point with its own
// chunk, NONE of which appear in either guarded route's first-load set.
function healthyInputs() {
  const loadable = {};
  let n = 0;
  for (const suffix of HEAVY_MODULE_SUFFIXES) {
    loadable[`src ${suffix}`] = { files: [`static/chunks/heavy-${n++}.js`] };
  }
  const first = ["static/chunks/webpack.js", "static/chunks/main-app.js"];
  const app = {
    pages: {
      "/page": [...first, "static/chunks/app/page.js"],
      "/capabilities/page": [...first, "static/chunks/app/capabilities/page.js"],
      "/surface/data-formats/page": [
        ...first,
        "static/chunks/app/surface/data-formats/page.js",
      ],
    },
  };
  const routes = { basePath: "/sparq" };
  const images = { images: { unoptimized: true } };
  const htmlByLabel = {
    "/": '<script src="/sparq/_next/static/chunks/webpack.js"></script>',
    "/capabilities": '<script src="/sparq/_next/static/chunks/app/capabilities/page.js"></script>',
    "/surface/data-formats":
      '<script src="/sparq/_next/static/chunks/app/surface/data-formats/page.js"></script>',
  };
  return { loadable, app, routes, images, htmlByLabel };
}

test("healthy build passes with zero errors", () => {
  const { errors, summary } = analyzeBundle(healthyInputs());
  assert.deepEqual(errors, []);
  assert.equal(summary.unoptimized, true);
  assert.equal(summary.basePath, "/sparq");
  assert.equal(summary.heavyReport.length, HEAVY_MODULE_SUFFIXES.length);
});

test("ISOLATION: a heavy chunk leaked into a route first-load is flagged", () => {
  const inputs = healthyInputs();
  // Force the zk-car-hire chunk into the /capabilities first-load set (the regression we guard).
  const zkChunk = inputs.loadable["src -> @/components/zk-car-hire"].files[0];
  inputs.app.pages["/capabilities/page"].push(zkChunk);
  const { errors } = analyzeBundle(inputs);
  assert.equal(errors.length, 1);
  assert.match(errors[0], /^ISOLATION: \/capabilities/);
  assert.match(errors[0], new RegExp(zkChunk.replace(/[.]/g, "\\.")));
});

test("PRESENCE: a demo switched to a static import (no split point) is flagged", () => {
  const inputs = healthyInputs();
  // Simulate zk-car-hire becoming a static import: its dynamic entry disappears from the manifest.
  delete inputs.loadable["src -> @/components/zk-car-hire"];
  const { errors } = analyzeBundle(inputs);
  assert.equal(errors.length, 1);
  assert.match(errors[0], /^PRESENCE:/);
  assert.match(errors[0], /@\/components\/zk-car-hire/);
});

test("PRESENCE: the ZK prover deps must stay dynamic", () => {
  const inputs = healthyInputs();
  delete inputs.loadable["src -> @aztec/bb.js"];
  delete inputs.loadable["src -> @noir-lang/noir_js"];
  const { errors } = analyzeBundle(inputs);
  assert.equal(errors.length, 2);
  assert.ok(errors.every((e) => e.startsWith("PRESENCE:")));
});

test("PRESENCE: the hero Graph view must stay a nested split point", () => {
  const inputs = healthyInputs();
  const graphView = "-> @/components/repl-graph-view";
  assert.ok(HEAVY_MODULE_SUFFIXES.includes(graphView));

  delete inputs.loadable[`src ${graphView}`];
  const { errors } = analyzeBundle(inputs);
  assert.equal(errors.length, 1);
  assert.match(errors[0], /^PRESENCE:/);
  assert.match(errors[0], /@\/components\/repl-graph-view/);
});

test("CONFIG: images.unoptimized=false is flagged", () => {
  const inputs = healthyInputs();
  inputs.images.images.unoptimized = false;
  const { errors } = analyzeBundle(inputs);
  assert.equal(errors.length, 1);
  assert.match(errors[0], /images\.unoptimized/);
});

test("CONFIG: adapts to the root-relative (basePath '') build mode", () => {
  const inputs = healthyInputs();
  inputs.routes.basePath = "";
  inputs.htmlByLabel = {
    "/": '<script src="/_next/static/chunks/webpack.js"></script>',
    "/capabilities": '<script src="/_next/static/chunks/app/capabilities/page.js"></script>',
    "/surface/data-formats":
      '<script src="/_next/static/chunks/app/surface/data-formats/page.js"></script>',
  };
  const { errors, summary } = analyzeBundle(inputs);
  assert.deepEqual(errors, []);
  assert.equal(summary.basePath, "");
});

test("CONFIG: HTML missing the resolved basePath prefix is flagged", () => {
  const inputs = healthyInputs();
  // basePath is /sparq but the HTML emits root-relative assets → prefixing broke.
  inputs.htmlByLabel["/"] = '<script src="/_next/static/chunks/webpack.js"></script>';
  const { errors } = analyzeBundle(inputs);
  assert.ok(errors.some((e) => /basePath prefixing is broken/.test(e)));
});

test("CONFIG: an image-optimiser reference in exported HTML is flagged", () => {
  const inputs = healthyInputs();
  inputs.htmlByLabel["/capabilities"] += '<img src="/sparq/_next/image?url=%2Fx.png&w=64&q=75">';
  const { errors } = analyzeBundle(inputs);
  assert.ok(errors.some((e) => /image optimiser/.test(e)));
});

test("GUARDED_ROUTES covers exactly the Home REPL, /capabilities, and the lazy-codec consumer", () => {
  assert.deepEqual(
    GUARDED_ROUTES.map((r) => r.label).sort(),
    ["/", "/capabilities", "/surface/data-formats"],
  );
});

// [FABLE-5] sq-4ssz1 (#1046) — the lazy-codec rule: the zstd decoder must stay a dynamic
// split point, and its chunk must never reach the codec consumer's first-load payload.
test("fzstd is pinned as a heavy module (PRESENCE) and flagged if it leaks into first-load", () => {
  assert.ok(HEAVY_MODULE_SUFFIXES.includes("-> fzstd"));

  // A build where the fzstd split point vanished (static-import regression) fails PRESENCE.
  const noSplit = healthyInputs();
  delete noSplit.loadable["src -> fzstd"];
  const { errors: presenceErrors } = analyzeBundle(noSplit);
  assert.ok(presenceErrors.some((e) => /^PRESENCE: "fzstd"/.test(e)));

  // A build where the codec chunk leaked into /surface/data-formats' first-load fails ISOLATION.
  const leaked = healthyInputs();
  const codecChunk = leaked.loadable["src -> fzstd"].files[0];
  leaked.app.pages["/surface/data-formats/page"].push(codecChunk);
  const { errors: isolationErrors } = analyzeBundle(leaked);
  assert.ok(
    isolationErrors.some(
      (e) => e.startsWith("ISOLATION: /surface/data-formats") && e.includes(codecChunk),
    ),
  );

  // The route's guard is SCOPED to the codec (`heavyModules`): a commons chunk it
  // legitimately shares with another heavy module's file list (the editors also sit in
  // the hero-runner's chunks) must NOT be flagged for this route.
  const shared = healthyInputs();
  const heroChunk = shared.loadable["src -> @/components/home/hero-runner"].files[0];
  shared.app.pages["/surface/data-formats/page"].push(heroChunk);
  const { errors: sharedErrors } = analyzeBundle(shared);
  assert.deepEqual(sharedErrors, []);
});

// Next 16 no longer writes app-build-manifest.json; main() rebuilds the page-entry set from the
// exported HTML. The root layout's group (app-shell code shared with lazy demos) is excluded,
// matching the old manifest's `/page` entry; everything after the layout chunk is kept.
// A client reference manifest in the shape Next 16 writes: each client module's `chunks`
// alternates chunk ids and file paths.
function clientManifest(key, modules) {
  const clientModules = {};
  modules.forEach((files, i) => {
    clientModules[`[project]/m${i}.tsx`] = { id: i, chunks: files.flatMap((f, j) => [`${j}`, f]) };
  });
  return { [key]: { clientModules } };
}

test("firstLoadFromClientManifest unions the page entry's modules and leaves layout modules out", () => {
  const manifest = clientManifest("/capabilities/page", [
    ["static/chunks/9568-a.js", "static/chunks/app/capabilities/page-x.js"],
    ["static/chunks/9568-a.js", "static/chunks/163-b.js", "static/chunks/app/capabilities/page-x.js"],
    ["static/chunks/6525-shared.js", "static/chunks/app/layout-l.js"],
  ]);
  assert.deepEqual(firstLoadFromClientManifest(manifest, "/capabilities/page"), {
    chunks: [
      "static/chunks/9568-a.js",
      "static/chunks/app/capabilities/page-x.js",
      "static/chunks/163-b.js",
    ],
    error: null,
  });
});

test("firstLoadFromClientManifest keeps a heavy chunk the page also shares with the layout", () => {
  // The page and the layout both load heavy-0: the page's set must still list it.
  const manifest = clientManifest("/page", [
    ["static/chunks/heavy-0.js", "static/chunks/app/layout-l.js"],
    ["static/chunks/heavy-0.js", "static/chunks/app/page-x.js"],
  ]);
  const { chunks, error } = firstLoadFromClientManifest(manifest, "/page");
  assert.equal(error, null);
  const inputs = healthyInputs();
  inputs.app.pages["/page"] = chunks;
  const { errors } = analyzeBundle(inputs);
  assert.ok(errors.some((e) => e.startsWith("ISOLATION: / ")), errors.join("\n"));
});

test("firstLoadFromClientManifest refuses a manifest with no module loading the page entry", () => {
  const layoutOnly = clientManifest("/page", [["static/chunks/app/layout-l.js"]]);
  assert.match(firstLoadFromClientManifest(layoutOnly, "/page").error ?? "", /^FIRST-LOAD: /);
  const otherPage = clientManifest("/capabilities/page", [["static/chunks/app/try/page-x.js"]]);
  assert.match(
    firstLoadFromClientManifest(otherPage, "/capabilities/page").error ?? "",
    /^FIRST-LOAD: /,
  );
  assert.match(firstLoadFromClientManifest({}, "/page").error ?? "", /^FIRST-LOAD: /);
  assert.match(firstLoadFromClientManifest(undefined, "/page").error ?? "", /^FIRST-LOAD: /);
});
