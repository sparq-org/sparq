#!/usr/bin/env node
// Publish guardrail + git-pin verification for the `@sparq-org/sparq` npm binding.
// [OPUS-4.8] sq-bkag (follow-up to #623 / sq-c4ej).
//
// WHY THIS EXISTS
// ----------------
// `@sparq-org/sparq` is not yet on npm under its settled name, so the only way a
// consumer (e.g. PSS) can depend on it today is to PIN A GIT BUILD:
//
//     "@sparq-org/sparq": "github:sparq-org/sparq#<sha>"   # with package.json `directory: "js"`
//
// A git-pinned install does NOT ship `dist/` or `wasm/` (both are .gitignored).
// npm rebuilds them by running the package's `prepare` lifecycle script on
// install. If `prepare` is missing/broken, or the publish `files` allowlist
// drops the wasm, the consumer silently gets a binding with no engine — an
// `import` that throws at runtime, not at install. That is the exact failure
// mode this guard turns into a hard, pre-publish / pre-pin error.
//
// WHAT IT CHECKS (no network, no Rust toolchain required — inspects the tree
// `npm pack` WOULD ship via `npm pack --dry-run --json`):
//   1. `prepare` is wired so a git-pinned `npm install` builds the binding.
//   2. The publish allowlist (`files`) names `dist`, `wasm` and `wasm-node`.
//   3. The packed tarball actually contains the JS entrypoint (`main`), the
//      type entrypoint (`types`), and the wasm artifact (`wasm/*_bg.wasm`).
//   4. The package self-identifies as `@sparq-org/sparq` (settled name guard).
//   5. The CommonJS engine (`--target nodejs`, sq-2hk) is shipped AND usable:
//      the `./wasm-node` export condition is wired, the glue + wasm bytes are in
//      the tarball, and `wasm-node/package.json` re-scopes that directory to
//      CommonJS (without it the `"type": "module"` at the package root would make
//      Node parse the CJS glue as ESM and `require()` would throw).
//
// USAGE
//   node guardrails/check-package.mjs           # check the packable tree
//   npm run check:package                        # same, via package.json
//
// As a git-pin verification step a CONSUMER runs (after `npm install` resolves
// the git pin) to prove the engine landed:
//   node -e "import('@sparq-org/sparq').then(m=>m.SparqStore.fromString('<a> <b> <c> .','ntriples')).then(s=>{s.free?.();console.log('ok')})"

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { parsePackJson } from './pack-json.mjs';

const pkgDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const EXPECTED_NAME = '@sparq-org/sparq';

const failures = [];
const fail = (msg) => failures.push(msg);

const pkg = JSON.parse(readFileSync(resolve(pkgDir, 'package.json'), 'utf8'));

// (4) settled-name guard — a rename must be deliberate, not silent.
if (pkg.name !== EXPECTED_NAME) {
  fail(`package name is "${pkg.name}", expected the settled name "${EXPECTED_NAME}"`);
}

// (1) git-pin install builds the binding: `prepare` runs on `npm install` of a
//     git dependency. Without it, a git-pinned consumer gets no dist/ or wasm/.
const scripts = pkg.scripts ?? {};
if (!scripts.prepare) {
  fail(
    'missing `scripts.prepare`: a git-pinned `npm install` will not build dist/ or wasm/ ' +
      '(both are .gitignored), so consumers pinning a git build get a binding with no engine',
  );
} else if (!/build/.test(scripts.prepare)) {
  fail(`\`scripts.prepare\` ("${scripts.prepare}") does not invoke a build step`);
}

// (2) publish allowlist names the built outputs.
const files = pkg.files ?? [];
for (const required of ['dist', 'wasm', 'wasm-node']) {
  if (!files.includes(required)) {
    fail(`package.json \`files\` allowlist is missing "${required}" — it would be omitted from the published/packed tarball`);
  }
}

// (5a) the CommonJS engine is reachable: `require('@sparq-org/sparq/wasm-node')` only
//      resolves if the subpath export declares a `require` condition (the root
//      `exports` field makes every undeclared subpath unreachable).
const wasmNodeExport = pkg.exports?.['./wasm-node'];
if (!wasmNodeExport) {
  fail(
    'package.json `exports` has no "./wasm-node" subpath — CommonJS consumers cannot ' +
      "`require('@sparq-org/sparq/wasm-node')` (an `exports` field blocks every undeclared subpath)",
  );
} else if (typeof wasmNodeExport.require !== 'string') {
  fail('the "./wasm-node" export has no `require` condition, so `require()` of it fails to resolve');
}

// (3) the tree `npm pack` would ship actually contains the entrypoints + wasm.
// `npm pack --dry-run --json` runs `prepack` and reports the resulting file
// list WITHOUT writing a tarball or touching the network. Those lifecycle scripts
// share this stdout, so read only the TRAILING JSON value (see pack-json.mjs): a
// stray progress line must not be reported here as "not packable".
let packed;
try {
  const out = execFileSync('npm', ['pack', '--dry-run', '--json'], {
    cwd: pkgDir,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  packed = parsePackJson(out);
} catch (err) {
  fail(`\`npm pack --dry-run\` failed (the package is not packable): ${err.message}`);
}

if (packed) {
  const entries = (packed[0]?.files ?? []).map((f) => f.path);
  const has = (pred) => entries.some(pred);

  const main = (pkg.main ?? '').replace(/^\.\//, '');
  const types = (pkg.types ?? '').replace(/^\.\//, '');
  if (main && !entries.includes(main)) {
    fail(`packed tarball is missing the \`main\` entrypoint "${main}"`);
  }
  if (types && !entries.includes(types)) {
    fail(`packed tarball is missing the \`types\` entrypoint "${types}"`);
  }
  if (!has((p) => /^wasm\/.*_bg\.wasm$/.test(p))) {
    fail('packed tarball is missing the wasm artifact (wasm/*_bg.wasm) — the binding would have no engine');
  }
  if (!has((p) => /^wasm\/.*\.js$/.test(p))) {
    fail('packed tarball is missing the wasm JS glue (wasm/*.js)');
  }

  // (5b) the `--target nodejs` sibling: glue + bytes + the CommonJS marker.
  if (!has((p) => /^wasm-node\/.*_bg\.wasm$/.test(p))) {
    fail('packed tarball is missing the CommonJS wasm artifact (wasm-node/*_bg.wasm) — `require()` consumers would have no engine');
  }
  if (!has((p) => /^wasm-node\/.*\.js$/.test(p))) {
    fail('packed tarball is missing the CommonJS wasm glue (wasm-node/*.js)');
  }
  if (!entries.includes('wasm-node/package.json')) {
    fail(
      'packed tarball is missing wasm-node/package.json — without its `{"type": "commonjs"}` the ' +
        'package-root `"type": "module"` makes Node parse the CommonJS glue as ESM and `require()` throws',
    );
  }
}

if (failures.length) {
  console.error(`check-package: ${failures.length} guardrail failure(s) for ${EXPECTED_NAME}:`);
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}

console.log(`check-package: ${EXPECTED_NAME} is publishable + git-pin-installable (prepare + files + packed dist/wasm verified).`);
