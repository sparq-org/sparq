#!/usr/bin/env node
// sq-hmd7l.15 — jsonld.js adapter for the bench/jsonld suite
// (registered in bench/competitors.json, id: jsonld-js). GATHER-ONLY: jsonld.js
// is NOT a committed dependency — `npm install jsonld` into a scratch dir at
// gather time and point NODE_PATH at it (bench/jsonld/gather.py does this).
//
// Contract (shared with jsonld_adapter.py / the sparq bench_jsonld example):
//   --op expand|flatten|compact|tordf  --input <doc.jsonld> [--context <ctx.jsonld>]
//   [--iters N] [--warmup W] [--out <file>]
// Emits the operation OUTPUT to --out (for the harness's output-equality gate —
// run BEFORE any timing row is trusted) and one JSON envelope line on stdout:
//   {engine, engine_version, op, input, bytes, iters, us_per_op, docs_per_s, mb_per_s}
// Timing task boundary: in-memory JSON text -> operation result (JSON.parse
// inside the loop, serialization outside), matching bench_jsonld's `time`.
// All timings are advisory wall-clock — never canonical numbers.
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const jsonld = require('jsonld');

// Fixtures are self-contained: any remote context fetch is a harness bug.
const documentLoader = (url) => { throw new Error('remote load refused: ' + url); };
const BASE = 'https://w3id.org/sparq/bench/jsonld/'; // same constant as bench_jsonld

const args = process.argv.slice(2);
const opt = (name, dflt) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : dflt;
};
const op = opt('--op');
const input = opt('--input');
const ctxFile = opt('--context');
const iters = Math.max(1, parseInt(opt('--iters', '20'), 10));
const warmup = Math.max(0, parseInt(opt('--warmup', '3'), 10));
const outFile = opt('--out');
if (!op || !input) {
  console.error('usage: jsonld_adapter.mjs --op expand|flatten|compact|tordf --input doc.jsonld [--context ctx.jsonld] [--iters N] [--warmup W] [--out file]');
  process.exit(2);
}

const text = readFileSync(input, 'utf8');
const ctxText = ctxFile ? readFileSync(ctxFile, 'utf8') : null;
const options = { base: BASE, documentLoader };

async function runOnce() {
  const doc = JSON.parse(text); // parse inside the loop (shared task boundary)
  switch (op) {
    case 'expand': return jsonld.expand(doc, options);
    case 'flatten': return jsonld.flatten(doc, null, options);
    case 'compact': return jsonld.compact(doc, JSON.parse(ctxText), options);
    case 'tordf': return jsonld.toRDF(doc, { ...options, format: 'application/n-quads' });
    default: throw new Error('unknown op ' + op);
  }
}

const result = await runOnce(); // correctness output for the equality gate
if (outFile) {
  writeFileSync(outFile, typeof result === 'string' ? result : JSON.stringify(result, null, 2) + '\n');
}
for (let i = 0; i < warmup; i++) await runOnce();
const t0 = process.hrtime.bigint();
for (let i = 0; i < iters; i++) await runOnce();
const usPerOp = Number(process.hrtime.bigint() - t0) / 1000 / iters;
const docsPerS = 1e6 / usPerOp;
console.log(JSON.stringify({
  engine: 'jsonld-js',
  engine_version: require('jsonld/package.json').version,
  node_version: process.version,
  op, input,
  bytes: Buffer.byteLength(text),
  iters,
  us_per_op: Number(usPerOp.toFixed(1)),
  docs_per_s: Number(docsPerS.toFixed(1)),
  mb_per_s: Number((Buffer.byteLength(text) / 1e6 * docsPerS).toFixed(2)),
}));
