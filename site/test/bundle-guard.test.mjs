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
  firstLoadFromHtml,
  htmlFirstLoad,
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
const SCRIPT = (p) => `<script src="/sparq/_next/static/chunks/${p}" async=""></script>`;

test("firstLoadFromHtml keeps only the page segment after the root layout chunk", () => {
  const html = [
    SCRIPT("main-app-1.js"),
    SCRIPT("6525-shared.js"),
    SCRIPT("app/layout-abc.js"),
    SCRIPT("9287-pagedep.js"),
    SCRIPT("app/surface/data-formats/page-def.js"),
    SCRIPT("webpack-2.js"),
    // Inline RSC payload text is not a script tag and must not be picked up.
    '<script>self.__next_f.push([1,"static/chunks/not-a-tag.js"])</script>',
  ].join("");
  assert.deepEqual(firstLoadFromHtml(html), [
    "static/chunks/9287-pagedep.js",
    "static/chunks/app/surface/data-formats/page-def.js",
    "static/chunks/webpack-2.js",
  ]);
});

test("firstLoadFromHtml flags a heavy chunk statically pulled into the page segment", () => {
  const html = SCRIPT("app/layout-abc.js") + SCRIPT("heavy-0.js") + SCRIPT("app/page-x.js");
  const inputs = healthyInputs();
  inputs.app.pages["/page"] = firstLoadFromHtml(html);
  const { errors } = analyzeBundle(inputs);
  assert.ok(errors.some((e) => e.startsWith("ISOLATION: / ")), errors.join("\n"));
});

test("firstLoadFromHtml with no layout chunk keeps every script chunk", () => {
  const html = SCRIPT("a.js") + SCRIPT("b.js") + SCRIPT("a.js");
  assert.deepEqual(firstLoadFromHtml(html), ["static/chunks/a.js", "static/chunks/b.js"]);
});

test("htmlFirstLoad refuses HTML with no script tag for the route's page entry", () => {
  const stylesheetOnly = '<link rel="stylesheet" href="/sparq/_next/static/css/app.css">';
  assert.match(htmlFirstLoad(stylesheetOnly, "/page").error ?? "", /^FIRST-LOAD: /);
  const otherPage = SCRIPT("app/layout-abc.js") + SCRIPT("app/try/page-x.js");
  assert.match(htmlFirstLoad(otherPage, "/capabilities/page").error ?? "", /^FIRST-LOAD: /);
  const own = SCRIPT("app/layout-abc.js") + SCRIPT("app/capabilities/page-x.js");
  assert.equal(htmlFirstLoad(own, "/capabilities/page").error, null);
});

test("firstLoadFromHtml normalises query-bearing script URLs, so a heavy chunk is still caught", () => {
  const html =
    '<script src="/sparq/_next/static/chunks/app/layout-abc.js?v=1"></script>' +
    "<script src='/sparq/_next/static/chunks/heavy-0.js?dpl=abc#x'></script>" +
    '<script src="/sparq/_next/static/chunks/app/page-x.js?v=1"></script>';
  const { chunks, error } = htmlFirstLoad(html, "/page");
  assert.equal(error, null);
  assert.deepEqual(chunks, ["static/chunks/heavy-0.js", "static/chunks/app/page-x.js"]);
  const inputs = healthyInputs();
  inputs.app.pages["/page"] = chunks;
  const { errors } = analyzeBundle(inputs);
  assert.ok(errors.some((e) => e.startsWith("ISOLATION: / ")), errors.join("\n"));
});
