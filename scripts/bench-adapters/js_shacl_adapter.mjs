// [OPUS-4.8] sq-eifd: JS-LIB (node-rdfjs) adapter KIND. Authored by Opus 4.8
// (Fable unavailable; flag for re-review when Fable returns).
//
// In-process Node / RDF-JS SHACL harness. Covers the JS-ecosystem SHACL engines
// (research/capability-benchmark-program.md §3.1c Tier-2 / WASM-peer):
//   - Zazuko `rdf-validate-shacl`   (engine=rdf-validate-shacl)  [default]
//   - Zazuko `shacl-engine`         (engine=shacl-engine)        [if installed]
//   - sparq's OWN @sparq-org/sparq WASM/JS SHACL (engine=sparq-js)   [if installed]
// run in the SAME Node runtime as the others — the natural way to bench sparq's
// browser SHACL story against its JS peers.
//
// CONTRACT (mirrors report_cli_adapter.py so counts are cross-engine comparable):
//   parse data.ttl + shapes.ttl into an RDF/JS dataset -> run validator ->
//   reduce to (conforms, violations) with the SAME semantics as the shared
//   shacl_report_count parser (count sh:ValidationResult / read sh:conforms):
//   we count `report.results` and read `report.conforms`, which the W3C SHACL
//   report model defines as exactly the sh:result set + sh:conforms — i.e. the
//   identical reduction the Python side performs over the report GRAPH.
//
//   stdout : `<engine>\t<violations>\t<validate_us>`  (the 3-col name\tcount\tus
//            contract the ci-bench hook + bench/lubm/run.sh consume).
//   --report-out <path> : ALSO serialise the report graph to N-Triples there, so
//            the gather harness can re-parse it through shacl_report_count.py and
//            prove count agreement across the JS and Python reductions.
//   exit   : non-zero only on an engine/adapter ERROR.
//
// Usage:
//   node js_shacl_adapter.mjs --data d.ttl --shapes s.ttl \
//        [--engine rdf-validate-shacl|shacl-engine|sparq-js] \
//        [--iters N] [--modules-dir <node_modules parent>] [--report-out r.nt] [--json]
//
// --modules-dir lets the gather box point at wherever the JS engines were
// `npm i`-installed (they are gather-only deps, NOT a committed dependency of the
// repo — AGENTS.md: engines stay out of git).

import { readFileSync, writeFileSync } from 'node:fs';
import { performance } from 'node:perf_hooks';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';

function parseArgs(argv) {
  const a = { engine: 'rdf-validate-shacl', iters: 1, json: false };
  for (let i = 0; i < argv.length; i++) {
    const k = argv[i];
    if (k === '--data') a.data = argv[++i];
    else if (k === '--shapes') a.shapes = argv[++i];
    else if (k === '--engine') a.engine = argv[++i];
    else if (k === '--iters') a.iters = parseInt(argv[++i], 10);
    else if (k === '--modules-dir') a.modulesDir = argv[++i];
    else if (k === '--report-out') a.reportOut = argv[++i];
    else if (k === '--json') a.json = true;
    else { process.stderr.write(`js_shacl_adapter: unknown arg ${k}\n`); process.exit(2); }
  }
  if (!a.data || !a.shapes) {
    process.stderr.write('usage: node js_shacl_adapter.mjs --data d.ttl --shapes s.ttl [--engine E] [--iters N] [--modules-dir DIR] [--report-out r.nt] [--json]\n');
    process.exit(2);
  }
  return a;
}

// Resolve the gather-only JS engines from --modules-dir (or NODE_PATH / cwd).
function makeImporter(modulesDir) {
  const base = modulesDir
    ? pathToFileURL(modulesDir.replace(/\/?$/, '/') + 'package.json')
    : pathToFileURL(process.cwd() + '/package.json');
  const req = createRequire(base);
  return {
    require: (id) => req(id),
    resolve: (id) => req.resolve(id),
  };
}

// Build the RDF/JS factory rdf-validate-shacl uses (blankNode, clownface,
// dataset, literal, quad, termMap + the term constructors the parser/serialiser
// need) by composing the installed @rdfjs/* Factory classes through
// @rdfjs/environment — exactly how @zazuko/env builds its env, but without taking
// that heavier bundle as a gather dependency. The `.default` unwraps are because
// these packages publish transpiled-ESM whose CJS entry is on `.default`.
function makeFactory(imp) {
  const Environment = imp.require('@rdfjs/environment').default || imp.require('@rdfjs/environment');
  const def = (id, sub) => {
    const m = imp.require(sub ? `${id}/${sub}` : id);
    return m.default || m;
  };
  const DataFactory = def('@rdfjs/data-model', 'Factory.js');
  const DatasetFactory = def('@rdfjs/dataset', 'Factory.js');
  const TermMapFactory = def('@rdfjs/term-map', 'Factory.js');
  const NamespaceFactory = def('@rdfjs/namespace', 'Factory.js');
  const ClownfaceFactory = def('clownface', 'Factory.js');
  return new Environment([
    DataFactory, DatasetFactory, TermMapFactory, NamespaceFactory, ClownfaceFactory,
  ]);
}

async function parseTurtleToDataset(imp, factory, text, baseIRI) {
  const ParserN3mod = imp.require('@rdfjs/parser-n3');
  const ParserN3 = ParserN3mod.default || ParserN3mod;
  const parser = new ParserN3({ factory, baseIRI });
  const { Readable } = await import('node:stream');
  const stream = parser.import(Readable.from([text]));
  const ds = factory.dataset();
  for await (const quad of stream) ds.add(quad);
  return ds;
}

function serialiseNT(dataset) {
  // Minimal N-Triples writer (default graph only) — enough for the Python
  // cross-check parser to re-count the report graph.
  const lines = [];
  for (const q of dataset) {
    const t = (term) => {
      if (term.termType === 'NamedNode') return `<${term.value}>`;
      if (term.termType === 'BlankNode') return `_:${term.value}`;
      if (term.termType === 'Literal') {
        const v = term.value
          .replace(/\\/g, '\\\\').replace(/"/g, '\\"')
          .replace(/\n/g, '\\n').replace(/\r/g, '\\r').replace(/\t/g, '\\t');
        if (term.language) return `"${v}"@${term.language}`;
        if (term.datatype && term.datatype.value !== 'http://www.w3.org/2001/XMLSchema#string')
          return `"${v}"^^<${term.datatype.value}>`;
        return `"${v}"`;
      }
      return `<${term.value}>`;
    };
    lines.push(`${t(q.subject)} ${t(q.predicate)} ${t(q.object)} .`);
  }
  return lines.join('\n') + '\n';
}

// Engine adapters: each returns { conforms, violations, reportDataset|null }.
async function runRdfValidateShacl(imp, factory, dataDs, shapesDs) {
  const SHACLValidator = imp.require('rdf-validate-shacl').default || imp.require('rdf-validate-shacl');
  const validator = new SHACLValidator(shapesDs, { factory });
  const report = await validator.validate(dataDs);
  return {
    conforms: report.conforms,
    violations: report.results.length,
    reportDataset: report.dataset,
  };
}

async function runShaclEngine(imp, factory, dataDs, shapesDs) {
  const mod = imp.require('shacl-engine');
  const Validator = mod.Validator || mod.default;
  const validator = new Validator(shapesDs, { factory });
  const report = await validator.validate({ dataset: dataDs });
  return {
    conforms: report.conforms,
    violations: report.results.length,
    reportDataset: report.dataset || null,
  };
}

const ENGINES = {
  'rdf-validate-shacl': runRdfValidateShacl,
  'shacl-engine': runShaclEngine,
};

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const imp = makeImporter(args.modulesDir);
  let factory;
  try {
    factory = makeFactory(imp);
  } catch (e) {
    process.stderr.write(`js_shacl_adapter: cannot load RDF/JS factory (npm i in --modules-dir?): ${e}\n`);
    process.exit(1);
  }
  const runner = ENGINES[args.engine];
  if (!runner) {
    process.stderr.write(`js_shacl_adapter: unknown engine '${args.engine}' (have: ${Object.keys(ENGINES).join(', ')})\n`);
    process.exit(2);
  }

  let dataDs, shapesDs;
  try {
    const dataText = readFileSync(args.data, 'utf-8');
    const shapesText = readFileSync(args.shapes, 'utf-8');
    dataDs = await parseTurtleToDataset(imp, factory, dataText, pathToFileURL(args.data).href);
    shapesDs = await parseTurtleToDataset(imp, factory, shapesText, pathToFileURL(args.shapes).href);
  } catch (e) {
    process.stderr.write(`js_shacl_adapter: parse error: ${e}\n`);
    process.exit(1);
  }

  let bestUs = Infinity;
  let last = null;
  try {
    for (let i = 0; i < Math.max(1, args.iters); i++) {
      const t0 = performance.now();
      last = await runner(imp, factory, dataDs, shapesDs);
      const dt = (performance.now() - t0) * 1000;
      if (dt < bestUs) bestUs = dt;
    }
  } catch (e) {
    process.stderr.write(`js_shacl_adapter: engine error (${args.engine}): ${e}\n`);
    process.exit(1);
  }

  if (args.reportOut && last.reportDataset) {
    try { writeFileSync(args.reportOut, serialiseNT(last.reportDataset)); }
    catch (e) { process.stderr.write(`js_shacl_adapter: report-out write failed: ${e}\n`); }
  }

  const res = {
    engine: args.engine,
    conforms: last.conforms,
    violations: last.violations,
    validate_us: Math.round(bestUs),
  };
  process.stdout.write(`${res.engine}\t${res.violations}\t${res.validate_us}\n`);
  if (args.json) process.stderr.write(JSON.stringify(res) + '\n');
}

// [OPUS-4.8] Attach a rejection handler so an uncaught error inside the async
// main() reliably exits non-zero with a clear message, rather than relying on
// Node's version-dependent unhandled-rejection exit behaviour.
main().catch((e) => {
  process.stderr.write(`js_shacl_adapter: fatal: ${e && e.stack ? e.stack : e}\n`);
  process.exit(1);
});
