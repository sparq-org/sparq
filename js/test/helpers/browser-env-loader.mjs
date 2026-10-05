// [OPUS-5] sq-bhx (#3398): the browser-side half of the `init()` fork in js/src/wasm.ts.
//
// `init()` branches on `process?.versions?.node`. Every other test that goes through the
// ESM wrapper takes the Node arm (read the `.wasm` off disk, hand the bytes to
// `initWasm({module_or_path})`), so the `else` arm — no argument, let the wasm-pack glue resolve
// `new URL('sparq_wasm_bg.wasm', import.meta.url)` and `fetch` it — was only ever checked
// by reading it.
//
// This module is that arm's harness. It is NOT a browser: there is no DOM and no real
// network. What it reproduces is the two environmental facts the branch keys on and
// consumes — there is no `process` global, and `fetch` resolves the module-relative asset
// URL and answers with a `Response` carrying an `application/wasm` content type, the shape
// a browser serves and the one the wasm-pack glue needs in order to instantiate from the
// response rather than from a buffer. A real-browser run (Playwright) would additionally
// cover what is deliberately NOT claimed here: real HTTP, CORS/MIME behaviour, and the
// browser wasm engines themselves.
//
// It runs as a CHILD PROCESS (spawned by ../wasm-browser-path.test.mjs) for two reasons:
// deleting the `process` global is not something to do inside the shared test process, and
// both `init()` and the glue underneath it memoise their instantiation, so each scenario
// needs a virgin module registry.
//
// Being under test/, `node --test` also sweeps this file up and runs it — hence the entry
// guard at the bottom, which keeps it inert (a zero-test file) in that role, the same way
// helpers/zstd-fixtures.mjs is a pure exporter.
import { readFile } from 'node:fs/promises';

const DIST = new URL('../../dist/index.js', import.meta.url).href;

/** The scenarios the parent test can ask for. Doubles as the entry guard, see the bottom. */
const MODES = new Set(['ok', 'retry']);

/** Parsed back out of stdout by the parent test; keeps the payload separable from any
 * other chatter the loaded module tree prints. */
const RESULT_MARKER = '__BROWSER_ENV_RESULT__ ';

async function main(mode) {
  // Bind everything we need off `process` while it still exists.
  const write = process.stdout.write.bind(process.stdout);

  /** Every URL the module asked for, in order — a Node-arm `init()` would leave this empty. */
  const fetched = [];
  let failNextFetch = mode === 'retry';

  globalThis.fetch = async (input) => {
    const url = new URL(String(input));
    fetched.push(url.href);
    if (failNextFetch) {
      // The transient failure src/wasm.ts's `ready.catch(() => { ready = undefined })` exists for.
      failNextFetch = false;
      throw new TypeError('simulated transient network failure');
    }
    if (url.protocol !== 'file:') {
      throw new TypeError(`harness only serves the local build artifact, got ${url.href}`);
    }
    return new Response(await readFile(url), {
      headers: { 'content-type': 'application/wasm' },
    });
  };

  // The line that makes this the browser arm: no `process` global at all, so the
  // `typeof process !== 'undefined'` guard in src/wasm.ts is false. Node's own internals
  // hold their own reference and are unaffected; only user-code lookups go through globalThis.
  delete globalThis.process;
  if (typeof process !== 'undefined') {
    throw new Error('precondition failed: the `process` global survived deletion');
  }

  const { SparqStore, init } = await import(DIST);

  let firstInitError;
  if (mode === 'retry') {
    try {
      await init();
    } catch (error) {
      firstInitError = String(error?.message ?? error);
    }
    if (firstInitError === undefined) {
      throw new Error('expected the first init() to reject when the wasm fetch fails');
    }
  }

  await init();

  // `fetched` above pins WHICH route the loader took; running a real query pins that the
  // route ended at a working engine rather than at a module that merely instantiated.
  const store = await SparqStore.fromString('<http://e/a> <http://e/b> <http://e/c> .', 'ntriples');
  const bindings = JSON.parse(store.queryJson('SELECT ?o WHERE { ?s ?p ?o }')).results.bindings;

  write(
    RESULT_MARKER +
      JSON.stringify({
        fetched,
        firstInitError,
        size: store.size,
        objects: bindings.map((row) => row.o.value),
      }) +
      '\n',
  );
}

export { RESULT_MARKER };

// `node --test` sweeps every .mjs under test/ and runs it as its own entry point, so
// comparing argv[1] to import.meta.url does NOT distinguish that sweep from a deliberate
// spawn. Gate on the mode argument instead — only the parent test passes one, so the sweep
// (and any plain import) leaves this file inert.
if (MODES.has(process.argv[2])) {
  await main(process.argv[2]);
}
