// [OPUS-5] sq-bhx (#3398): smoke test for the NON-NODE arm of `init()` in js/src/wasm.ts.
//
// The Node arm is covered incidentally by the rest of the suite (anything touching the ESM
// wrapper ends up awaiting `init()` under Node). The `else` arm — call `initWasm()` with no
// argument and let the wasm-pack glue resolve + `fetch` the module-relative
// `sparq_wasm_bg.wasm` — had no executable coverage at all, only a comment saying what it
// was for.
//
// Each case runs helpers/browser-env-loader.mjs in a fresh child process; see that file for
// what the simulated environment does and, importantly, does not reproduce (no real browser,
// no real network — this is the loader branch, not the browser wasm engines).
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { RESULT_MARKER } from './helpers/browser-env-loader.mjs';

const HARNESS = fileURLToPath(new URL('./helpers/browser-env-loader.mjs', import.meta.url));

// What `new URL('sparq_wasm_bg.wasm', import.meta.url)` must come out as when the glue in
// js/wasm/sparq_wasm.js resolves it — i.e. the asset sitting next to its own module.
const WASM_ASSET = new URL('../wasm/sparq_wasm_bg.wasm', import.meta.url).href;

function runInBrowserEnv(mode) {
  const child = spawnSync(process.execPath, [HARNESS, mode], { encoding: 'utf8' });
  assert.equal(
    child.status,
    0,
    `browser-env harness (${mode}) exited ${child.status}\n--- stdout ---\n${child.stdout}\n--- stderr ---\n${child.stderr}`,
  );
  const line = child.stdout.split('\n').find((l) => l.startsWith(RESULT_MARKER));
  assert.ok(line, `harness produced no result line\n--- stdout ---\n${child.stdout}`);
  return JSON.parse(line.slice(RESULT_MARKER.length));
}

test('with no `process` global, init() fetches the wasm relative to the glue module', () => {
  const result = runInBrowserEnv('ok');

  // A single module-relative fetch is the whole observable signature of this branch: the
  // Node arm reads the file off disk and never calls fetch, so an empty list here would
  // mean the fork went the wrong way.
  assert.deepEqual(result.fetched, [WASM_ASSET]);

  // Asserted in the same case rather than a sibling one on purpose: a query answering
  // correctly does not by itself pin the branch (it would answer just as well off the Node
  // arm). What it adds, once the fetch above is established, is that those fetched bytes are
  // what got instantiated — the branch reached a genuinely working engine, not just a stub.
  assert.equal(result.size, 1);
  assert.deepEqual(result.objects, ['http://e/c']);
});

test('a failed fetch is retryable — init() does not memoise the rejection', () => {
  // src/wasm.ts drops its memoised `ready` on failure specifically so a transient error is
  // recoverable. That only matters on this branch (the Node arm reads a local file), and it
  // is invisible unless the first attempt actually fails.
  const result = runInBrowserEnv('retry');

  assert.match(result.firstInitError, /simulated transient network failure/);
  // Two attempts, same asset: the second init() re-entered the loader instead of handing
  // back the rejected promise.
  assert.deepEqual(result.fetched, [WASM_ASSET, WASM_ASSET]);
  assert.deepEqual(result.objects, ['http://e/c']);
});
