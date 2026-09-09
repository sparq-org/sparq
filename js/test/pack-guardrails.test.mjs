// Guards the `npm pack --dry-run --json` stdout channel. [SONNET-4.6] issue #5328.
//
// `guardrails/check-package.mjs` captures that command's stdout and parses it as JSON,
// while npm runs the `prepare`/`prepack` lifecycle scripts with their stdout inherited
// into the same stream. Two independent guards, so neither half can rot silently:
// the lifecycle scripts must not write to stdout, and the reader must survive it if one
// ever does. These are source/unit checks — no build artifacts, no npm invocation.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { parsePackJson } from '../guardrails/pack-json.mjs';

const pkgDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const manifest = JSON.parse(readFileSync(resolve(pkgDir, 'package.json'), 'utf8'));

// wasm-pack forwards its trailing arguments to Cargo, but an explicit `--` makes
// Cargo treat options such as `--features` as rustc arguments. [GPT-5.6]
test('wasm-pack build scripts keep Cargo options before the argument separator', () => {
  const wasmBuildScripts = Object.entries(manifest.scripts ?? {}).filter(([, body]) =>
    body.includes('wasm-pack build'),
  );
  assert.ok(wasmBuildScripts.length > 0, 'expected at least one wasm-pack build script');

  for (const [name, body] of wasmBuildScripts) {
    assert.doesNotMatch(
      body,
      /\s--\s--(?:features|config|profile)\b/,
      `${name} places a Cargo option after the argument separator`,
    );
  }
});

/** Every `node <script>` reachable from the `prepare`/`prepack` lifecycles. */
const lifecycleNodeScripts = () => {
  const scripts = manifest.scripts ?? {};
  const seen = new Set();
  const found = new Set();
  const visit = (name) => {
    if (seen.has(name) || !scripts[name]) return;
    seen.add(name);
    const body = scripts[name];
    for (const [, ref] of body.matchAll(/\bnpm run ([\w:._-]+)/g)) visit(ref);
    for (const [, file] of body.matchAll(/\bnode\s+((?:[\w.-]+\/)*[\w.-]+\.(?:mjs|cjs|js))\b/g)) {
      found.add(file);
    }
  };
  visit('prepare');
  visit('prepack');
  return [...found].sort();
};

test('no `npm pack` lifecycle script writes to stdout', () => {
  const chain = lifecycleNodeScripts();
  // Pin the derivation itself: an empty/short list would make the assertion below vacuous.
  assert.deepEqual(chain, ['guardrails/prepare-build.mjs', 'scripts/copy-wasm-node.mjs']);

  for (const file of chain) {
    const source = readFileSync(resolve(pkgDir, file), 'utf8');
    const offenders = [...source.matchAll(/^.*(?:console\.log|process\.stdout\.write)\s*\(.*$/gm)];
    assert.deepEqual(
      offenders.map((m) => m[0].trim()),
      [],
      `${file} runs inside \`npm pack --dry-run --json\`, whose stdout check-package.mjs ` +
        'parses as JSON — log progress to stderr instead',
    );
  }
});

test('parsePackJson reads the trailing JSON value even when stdout is polluted', () => {
  const manifestJson = JSON.stringify([{ id: 'x@0.0.0', files: [{ path: 'dist/index.js' }] }], null, 2);

  // The clean case CI sees today.
  assert.deepEqual(parsePackJson(manifestJson), [{ id: 'x@0.0.0', files: [{ path: 'dist/index.js' }] }]);
  assert.deepEqual(parsePackJson(`${manifestJson}\n`), parsePackJson(manifestJson));

  // A lifecycle script printing progress lines ahead of the JSON — including one that
  // itself starts with a bracket, so a naive "first bracket wins" scan would be fooled.
  const noisy = ['prepare: building @sparq-org/sparq ...', '[build] wasm-pack done', manifestJson].join('\n');
  assert.deepEqual(parsePackJson(noisy), parsePackJson(manifestJson));

  // Compact (non-pretty-printed) output behind noise still resolves.
  assert.deepEqual(parsePackJson(`noise\n${JSON.stringify([{ id: 'x' }])}`), [{ id: 'x' }]);

  // Genuine garbage must still fail — a broken pack is a real failure to report.
  assert.throws(() => parsePackJson('npm ERR! code ELIFECYCLE\n'), SyntaxError);
  assert.throws(() => parsePackJson(''), SyntaxError);
});
