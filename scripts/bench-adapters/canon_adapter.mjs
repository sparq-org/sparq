#!/usr/bin/env node
// sq-hmd7l.16 — rdf-canonize (digitalbazaar, JavaScript) adapter for
// the bench/canon RDFC-1.0 comparison panel. rdf-canonize is the reference
// INDEPENDENT implementation (sparq-canon delegates its algorithm to the Rust
// rdf-canon crate, so the JS column is the real cross-implementation check).
//
// Speaks the same CLI + exit-code contract as sparq's runner
// (crates/sparq-canon/examples/canon_bench.rs):
//   node canon_adapter.mjs --engine rdf-canonize --input <f.nq> [--sha384]
//   stdout: canonical N-Quads (byte-exact parity artifact)
//   stderr: parse_us= / canon_us= advisory timings, version=, algorithm=
//   exit:   0 ok | 2 rejected (work-factor/deep-iteration guard) | 3 parse | 1 usage/IO
//
// The module is a GATHER-TIME dependency (npm install rdf-canonize), never
// committed: resolution root is $CANON_NODE_MODULES (a dir whose node_modules
// contains rdf-canonize) or the current working directory.
//
// DoS posture: rdf-canonize's default guard (maxWorkFactor=1, i.e. O(n) deep
// iterations) is used AS SHIPPED — that default IS the engine's measured
// DoS-resistance. $CANON_JS_MAX_WORK_FACTOR overrides it for exploration only.
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { hrtime, exit, env, argv, stdout, stderr } from 'node:process';
import { pathToFileURL } from 'node:url';

function usage() {
  stderr.write('usage: canon_adapter.mjs --engine rdf-canonize --input <f.nq> [--sha384]\n');
  exit(1);
}

const args = argv.slice(2);
let engine = null;
let input = null;
let sha384 = false;
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--engine') engine = args[++i];
  else if (args[i] === '--input') input = args[++i];
  else if (args[i] === '--sha384') sha384 = true;
  else usage();
}
if (engine !== 'rdf-canonize' || !input) usage();

const root = env.CANON_NODE_MODULES || process.cwd();
const req = createRequire(pathToFileURL(root + '/'));
let canonize;
try {
  canonize = req('rdf-canonize');
} catch (e) {
  stderr.write(`canon_adapter.mjs: cannot resolve rdf-canonize from ${root} — ` +
    'npm install rdf-canonize there (gather-time dep, never committed): ' + e.message + '\n');
  exit(1);
}
stderr.write('version=' + req('rdf-canonize/package.json').version + '\n');

let text;
try {
  text = readFileSync(input, 'utf8');
} catch (e) {
  stderr.write('canon_adapter.mjs: read ' + input + ': ' + e.message + '\n');
  exit(1);
}

let dataset;
const t0 = hrtime.bigint();
try {
  dataset = canonize.NQuads.parse(text);
} catch (e) {
  stderr.write('canon_adapter.mjs: parse ' + input + ': ' + e.message + '\n');
  exit(3);
}
stderr.write('parse_us=' + Number(hrtime.bigint() - t0) / 1000 + '\n');

const options = { messageDigestAlgorithm: sha384 ? 'sha384' : 'sha256' };
if (env.CANON_JS_MAX_WORK_FACTOR !== undefined) {
  options.maxWorkFactor = Number(env.CANON_JS_MAX_WORK_FACTOR);
}

// RDFC-1.0 is the finalized name of URDNA2015; older rdf-canonize releases only
// know the old name. Try the standard name first, fall back on the alias, and
// record which one ran.
async function run(algorithm) {
  const t = hrtime.bigint();
  const out = await canonize.canonize(dataset, { algorithm, ...options });
  const us = Number(hrtime.bigint() - t) / 1000;
  stderr.write('algorithm=' + algorithm + '\n' + 'canon_us=' + us + '\n');
  return out;
}

let out;
try {
  try {
    out = await run('RDFC-1.0');
  } catch (e) {
    if (/unknown|unsupported|invalid.*algorithm/i.test(e.message)) {
      out = await run('URDNA2015');
    } else {
      throw e;
    }
  }
} catch (e) {
  // Guard trip (work factor / deep iterations) or any canonicalization
  // rejection: the fail-closed DoS-resistance outcome.
  stderr.write('canon_adapter.mjs: rejected (fail-closed): ' + e.message + '\n');
  exit(2);
}
stdout.write(out);
exit(0);
