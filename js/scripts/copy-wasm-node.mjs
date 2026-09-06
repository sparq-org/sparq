#!/usr/bin/env node
// [OPUS-5] sq-2hk (#3396) — copy the `--target nodejs` wasm-pack output into the
// package's `wasm-node/` dir, next to the `--target web` `wasm/` tree.
//
// WHY A SECOND TARGET
// -------------------
// `wasm/` is built with `--target web`: an ESM module whose default export is an
// async `init` that must run before any `Store.*` static. That is exactly right for
// browsers/Deno and for the package's own ESM entry, but it cannot be `require()`d —
// a CommonJS consumer has no way to `import` it synchronously. The `--target nodejs`
// glue is plain CommonJS and instantiates the module eagerly at `require()` time
// (`fs.readFileSync(__dirname + '/sparq_wasm_bg.wasm')`), so `require()` hands back a
// ready `Store` with no init step at all.
//
// WHY THE MARKER package.json
// ---------------------------
// `js/package.json` declares `"type": "module"`, which makes every `.js` file under it
// ESM — including the CommonJS glue we just copied, which would then die with
// `ReferenceError: module is not defined`. A nested `wasm-node/package.json` carrying
// `{"type": "commonjs"}` scopes the copied tree back to CJS. It is generated here (not
// copied from wasm-pack's own `pkg-node/package.json`, which carries publish metadata
// for a package we do not publish separately).
//
// Node, not shell: `npm run build` is invoked by the `prepare` lifecycle on a git-pinned
// install, which also runs on Windows (cf. sq-gl3cf in guardrails/prepare-build.mjs), so
// this step must not depend on `rm -rf`/`cp`.
import { cpSync, existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const pkgDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
// A DEDICATED wasm-pack out-dir (`pkg-node`, see `build:wasm:node`), never the crate's
// default `pkg/` — that one holds the `--target web` build `_copy:wasm` syncs into
// `wasm/`, and sharing an out-dir would let each build silently clobber the other.
const src = resolve(pkgDir, '..', 'crates', 'sparq-wasm', 'pkg-node');
const dst = resolve(pkgDir, 'wasm-node');

// The `--target nodejs` glue is a single self-contained CommonJS file plus the wasm
// bytes it reads at require() time; there is no separate `_bg.js` (that is the
// web/bundler shape).
const REQUIRED = ['sparq_wasm.js', 'sparq_wasm.d.ts', 'sparq_wasm_bg.wasm'];
// Emitted by newer wasm-bindgen only, and unused by the CJS glue — ship it when present
// so the type surface matches `wasm/`, but never fail the build on its absence.
const OPTIONAL = ['sparq_wasm_bg.wasm.d.ts'];

const missing = REQUIRED.filter((f) => !existsSync(resolve(src, f)));
if (missing.length) {
  console.error(
    `copy-wasm-node: ${src} is missing ${missing.join(', ')} — run \`npm run build:wasm:node\` ` +
      '(wasm-pack build ../crates/sparq-wasm --target nodejs --out-dir pkg-node) first.',
  );
  process.exit(1);
}

rmSync(dst, { recursive: true, force: true });
mkdirSync(dst, { recursive: true });
const copied = [...REQUIRED, ...OPTIONAL.filter((f) => existsSync(resolve(src, f)))];
for (const f of copied) cpSync(resolve(src, f), resolve(dst, f));
writeFileSync(resolve(dst, 'package.json'), `${JSON.stringify({ type: 'commonjs' }, null, 2)}\n`);

// STDERR, not stdout. `build` runs from the `prepack` lifecycle, and `prepack` runs inside
// `npm pack --dry-run --json` — whose STDOUT is a machine-read data channel: guardrails/
// check-package.mjs captures it and `JSON.parse`s it. A progress line on stdout lands ahead
// of that JSON and makes the parse throw, which the guardrail reports as the package being
// unpackable. Every other step in the chain (wasm-pack, tsc, the `cp`/`rm` copies) already
// keeps stdout clean for exactly this reason.
console.error(`copy-wasm-node: copied ${copied.length} artifacts + a commonjs marker into ${dst}`);
