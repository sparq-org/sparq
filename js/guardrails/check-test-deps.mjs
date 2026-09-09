#!/usr/bin/env node
// `pretest` preflight for the gating `js` CI lane (.github/workflows/js.yml). Issue #2491.
//
// WHY THIS EXISTS
// ---------------
// The `js` suite imports four packages that are deliberately NOT declared in
// js/package.json — `@rdfjs-test/conformance` (the packages/rdfjs-conformance
// WORKSPACE member) plus `@solid/acl-check` / `@solidlab/policy-engine` / `rdflib`
// (used by test/solid-differential.test.mjs; the first two are repo-ROOT
// devDependencies, rdflib is hoisted transitively via @solid/acl-check). They live
// outside this manifest on purpose: the js-sbom lane derives the published
// @sparq-org/sparq SBOM from js/package.json, and test-only deps there would pollute the
// runtime component list (sq-f04e / sq-pl1p).
//
// The consequence is that `npm ci` run INSIDE js/ installs only this member's own
// closure, and those four imports then die with a bare `ERR_MODULE_NOT_FOUND` partway
// through an otherwise-green run — noise that looks like a regression in whatever the
// agent was actually changing. CI never hits this because js.yml installs
// from the repo ROOT (`working-directory: .`, `npm ci`), which hoists all four into the
// workspace node_modules.
//
// So: resolve them up front and, when they are missing, say exactly which install step
// was skipped instead of letting the suite fail confusingly two thirds of the way in.
// This gates nothing that `npm test` did not already require — every import checked here
// is one the suite performs anyway.

import { readdirSync, readFileSync, statSync, existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const pkgDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const testDir = resolve(pkgDir, 'test');

// `npm run build` must have run: every test file imports ../dist/index.js, so a missing
// dist/ fails all 178 with the same shape of unhelpful resolution error.
const distEntry = resolve(pkgDir, 'dist', 'index.js');
if (!existsSync(distEntry)) {
  console.error(
    [
      'pretest: dist/ is not built — the test suite imports ../dist/index.js.',
      '',
      '    npm run build   # wasm-pack build ../crates/sparq-wasm + tsc',
      '',
      'See js/README.md, "Reproducing the `js` CI gate locally".',
    ].join('\n'),
  );
  process.exit(1);
}

// import.meta.resolve became a synchronous, unflagged API in Node 20.6. Older runtimes
// still run the suite fine; they just do not get this preflight.
if (typeof import.meta.resolve !== 'function') {
  process.exit(0);
}

const walk = (dir) =>
  readdirSync(dir).flatMap((entry) => {
    const path = resolve(dir, entry);
    return statSync(path).isDirectory() ? walk(path) : [path];
  });

// The npm package-name grammar, optionally followed by a subpath. Anchoring on this
// keeps template-literal fragments and other regex noise out of the resolve list — a
// false positive here would red the gate for no reason.
const PACKAGE_SPECIFIER = /^(?:@[a-z0-9~-][a-z0-9._~-]*\/)?[a-z0-9~-][a-z0-9._~-]*(?:\/.+)?$/;

const IMPORT_PATTERNS = [
  /\b(?:import|export)\b[^;'"]*?\bfrom\s*['"]([^'"]+)['"]/g, // import x from 'y'
  /^[ \t]*import\s+['"]([^'"]+)['"]/gm, //                      import 'y'
  /\bimport\s*\(\s*['"]([^'"]+)['"]\s*\)/g, //                  await import('y')
];

const specifiers = new Set();
for (const file of walk(testDir).filter((f) => /\.(?:mjs|cjs|js)$/.test(f))) {
  const source = readFileSync(file, 'utf8');
  for (const pattern of IMPORT_PATTERNS) {
    for (const match of source.matchAll(pattern)) {
      const specifier = match[1];
      if (specifier.startsWith('node:') || !PACKAGE_SPECIFIER.test(specifier)) continue;
      specifiers.add(specifier);
    }
  }
}

// Resolve as if from dist/index.js — the module every test file imports — so the lookup
// walks the same node_modules chain the suite itself will.
const resolveFrom = pathToFileURL(distEntry).href;

const missing = [];
for (const specifier of [...specifiers].sort()) {
  try {
    import.meta.resolve(specifier, resolveFrom);
  } catch {
    missing.push(specifier);
  }
}

if (missing.length === 0) {
  process.exit(0);
}

// Partition the report honestly: a specifier this manifest DOES declare is missing only
// because no install has run, which is a different diagnosis from the workspace-external
// ones an install inside js/ can never satisfy.
const manifest = JSON.parse(readFileSync(resolve(pkgDir, 'package.json'), 'utf8'));
const declared = new Set([
  ...Object.keys(manifest.dependencies ?? {}),
  ...Object.keys(manifest.devDependencies ?? {}),
]);
const packageOf = (specifier) =>
  specifier.startsWith('@') ? specifier.split('/').slice(0, 2).join('/') : specifier.split('/')[0];
const external = missing.filter((specifier) => !declared.has(packageOf(specifier)));
const own = missing.filter((specifier) => declared.has(packageOf(specifier)));

console.error(
  [
    `pretest: ${missing.length} package(s) the js test suite imports cannot be resolved.`,
    '',
    ...(own.length
      ? [
          'Declared in js/package.json but not installed:',
          ...own.map((specifier) => `    ${specifier}`),
          '',
        ]
      : []),
    ...(external.length
      ? [
          'Test-only, NOT declared in js/package.json — they come from the repo-root',
          'workspace install — so an install run inside js/ can never fetch these:',
          ...external.map((specifier) => `    ${specifier}`),
          '',
        ]
      : []),
    'Install from the REPO ROOT, exactly as .github/workflows/js.yml does:',
    '',
    '    cd .. && npm ci        # hoists the root devDeps + links packages/rdfjs-conformance',
    '',
    'To run a subset without a root install (this preflight does not apply):',
    '',
    '    node --test test/store.test.mjs',
    '',
    'See js/README.md, "Reproducing the `js` CI gate locally".',
  ].join('\n'),
);
process.exit(1);
