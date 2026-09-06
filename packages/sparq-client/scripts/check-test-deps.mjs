#!/usr/bin/env node
// [OPUS-5] sq-owzsl — `pretest` preflight for the @sparq/client suite.
//
// WHY THIS EXISTS
// ---------------
// Issue #3006 reported "2 bzip2 decompression tests fail under local `npm test` but the
// js.yml CI lane is GREEN", and asked whether CI was skipping them (masking a real
// seek-bzip decode gap) or whether it was a local-environment failure.
//
// It is a local-environment failure, and neither half of the premise held:
//
//   * js.yml never ran these tests in the first place — it runs `npm test` in js/,
//     packages/rdfjs-conformance, packages/eyereasoner-compat and packages/solid-server,
//     but not packages/sparq-client. Its green says nothing about this suite either way.
//   * The suite IS covered by CI: the gui.yml `shared TS client typecheck` job runs
//     `npm install && npm run typecheck && npm test` here, and that job carries no
//     .github/advisory-registry.json declaration, so it GATES.
//   * There is no seek-bzip decode gap. seek-bzip 2.0.0 decodes both reference fixtures in
//     test/decompress.test.mjs to byte-identical output against a reference bzip2 decoder.
//
// What actually reproduces the report is an INCOMPLETE INSTALL. `src/decompress.ts` reaches
// its codecs through lazy `import()` calls inside the invocation path (deliberately — neither
// decoder belongs in a surface's initial bundle), so an unresolvable codec package does not
// fail at module load. It fails later, as an ERR_MODULE_NOT_FOUND raised from inside the
// decode calls that need it — which reads like a decoder regression rather than a missing
// dependency. Deleting `node_modules/seek-bzip` and re-running the suite as it stood
// reproduced the reported symptom exactly: the two bzip2 decode tests red, everything else
// green (the extension-selection test accepted any rejection, so it did not join them; that
// masking assertion is now tightened).
//
// So: resolve the codec packages up front and name the install step that was skipped. This
// requires nothing the suite did not already require — every specifier checked here is one
// the tests import anyway.

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const pkgDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");

// import.meta.resolve became a synchronous, unflagged API in Node 20.6. Older runtimes still
// run the suite fine; they just do not get this preflight.
if (typeof import.meta.resolve !== "function") {
  process.exit(0);
}

const manifest = JSON.parse(
  readFileSync(resolve(pkgDir, "package.json"), "utf8"),
);

// The declared runtime dependencies (the lazy codecs) plus `typescript`, which
// test-support/ts-loader.mjs needs to transpile the .ts sources the tests import.
const required = [...Object.keys(manifest.dependencies ?? {}), "typescript"];

// Resolve as if from src/index.ts — the module every test file imports — so the lookup walks
// the same node_modules chain the suite itself will.
const resolveFrom = pathToFileURL(resolve(pkgDir, "src", "index.ts")).href;

const missing = [];
for (const specifier of [...new Set(required)].sort()) {
  try {
    // `buffer` resolves to the Node builtin here even with no install; that is also what the
    // suite gets at runtime, so a pass for it is honest rather than vacuous.
    import.meta.resolve(specifier, resolveFrom);
  } catch {
    missing.push(specifier);
  }
}

if (missing.length === 0) {
  process.exit(0);
}

console.error(
  [
    `pretest: ${missing.length} package(s) the @sparq/client test suite needs cannot be resolved:`,
    "",
    ...missing.map((specifier) => `    ${specifier}`),
    "",
    "This package is a repo-root npm workspace member, so an install run elsewhere in the",
    "workspace (js/, site/) does not populate its dependencies. Install as the gating",
    ".github/workflows/gui.yml `shared TS client typecheck` job does:",
    "",
    "    npm install        # from packages/sparq-client, or from the repo root",
    "",
    "Without this preflight a missing codec surfaces only as two failing bzip2 tests in",
    "test/decompress.test.mjs, because src/decompress.ts imports the codecs lazily (#3006).",
  ].join("\n"),
);
process.exit(1);
