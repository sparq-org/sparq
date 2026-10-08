// [OPUS-4.8] sq-17nw — unit tests for the REPL dataset helpers that make the live
// SPARQL REPL PRESERVE named graphs (GRAPH) on upload instead of folding everything
// into the default graph. These cover the pure, framework-free decision + serialisation
// logic; the engine-level named-graph behaviour is already proven by the Rust tests and
// js/test/store.test.mjs (loadDataset). Run via `npm run test:unit` (a node:test process
// with the in-repo TypeScript ESM loader).
import { test } from "node:test";
import assert from "node:assert/strict";

import {
  isDatasetFormat,
  ALL_QUADS_QUERY,
  rowsToNQuads,
  guessFormat,
  formatFromContentType,
  classifyQueryForm,
  isGraphForm,
  modeSupportsForm,
  NAMED_GRAPH_STATS_QUERY,
  DEFAULT_GRAPH_COUNT_QUERY,
  parseGraphStats,
} from "../src/lib/repl-dataset.ts";
import { parseNTriples } from "@sparq/client";

test("isDatasetFormat is true exactly for the quad-bearing formats", () => {
  // Quad formats carry named graphs and must be loaded with loadDataset.
  assert.equal(isDatasetFormat("nquads"), true);
  assert.equal(isDatasetFormat("trig"), true);
  // [OPUS-4.8] sq-dvyi: JSON-LD `@graph` carries named graphs too.
  assert.equal(isDatasetFormat("jsonld"), true);
  // Triple-only formats have no named graphs; load() is correct (and cheaper).
  assert.equal(isDatasetFormat("turtle"), false);
  assert.equal(isDatasetFormat("ntriples"), false);
  // Unknown / empty defaults to the non-dataset path.
  assert.equal(isDatasetFormat("nt"), false);
  assert.equal(isDatasetFormat(""), false);
});

// [OPUS-4.8] sq-dvyi: the REPL upload/URL format detection now recognises JSON-LD.
test("guessFormat maps file/URL extensions to the engine format", () => {
  assert.equal(guessFormat("data.ttl"), "turtle");
  assert.equal(guessFormat("data.nt"), "ntriples");
  assert.equal(guessFormat("data.nq"), "nquads");
  assert.equal(guessFormat("data.trig"), "trig");
  // JSON-LD: both the .jsonld and .json-ld spellings, and through a URL query string.
  assert.equal(guessFormat("data.jsonld"), "jsonld");
  assert.equal(guessFormat("https://host/dir/data.jsonld?v=2#frag"), "jsonld");
  assert.equal(guessFormat("data.json-ld"), "jsonld");
  // Unknown extension falls back to Turtle.
  assert.equal(guessFormat("data.bin"), "turtle");
});

test("formatFromContentType maps a served media type (ignoring charset) to a format", () => {
  assert.equal(formatFromContentType("text/turtle"), "turtle");
  assert.equal(formatFromContentType("application/n-triples"), "ntriples");
  assert.equal(formatFromContentType("application/n-quads"), "nquads");
  assert.equal(formatFromContentType("application/trig"), "trig");
  // JSON-LD, including a charset parameter and mixed case.
  assert.equal(formatFromContentType("application/ld+json"), "jsonld");
  assert.equal(formatFromContentType("application/ld+json; charset=utf-8"), "jsonld");
  assert.equal(formatFromContentType("APPLICATION/LD+JSON"), "jsonld");
  // Unknown / absent content types yield undefined so the caller falls back to the ext.
  assert.equal(formatFromContentType("application/octet-stream"), undefined);
  assert.equal(formatFromContentType(null), undefined);
});

test("ALL_QUADS_QUERY spans the default graph AND every named graph", () => {
  // A UNION of the default-graph pattern and the GRAPH ?g pattern: the only way to
  // enumerate the WHOLE dataset (the default-graph-only `{ ?s ?p ?o }` misses named
  // graphs, which is exactly the folding bug this bead fixes).
  assert.match(ALL_QUADS_QUERY, /GRAPH \?g/);
  assert.match(ALL_QUADS_QUERY, /UNION/);
});

test("rowsToNQuads emits a triple line for default-graph rows (no ?g)", () => {
  const rows = [
    {
      s: { type: "uri", value: "http://ex/a" },
      p: { type: "uri", value: "http://ex/p" },
      o: { type: "literal", value: "default" },
    },
  ];
  assert.equal(
    rowsToNQuads(rows),
    '<http://ex/a> <http://ex/p> "default" .',
  );
});

// #6044: SPARQL 1.2 JSON nests an RDF 1.2 triple term under `value`. The old local
// writer had no "triple" case and wrote the string "undefined" into the snapshot.
test("rowsToNQuads writes an RDF 1.2 triple term as <<( s p o )>>", () => {
  const rows = [
    {
      s: { type: "bnode", value: "r1" },
      p: { type: "uri", value: "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies" },
      o: {
        type: "triple",
        value: {
          subject: { type: "uri", value: "http://ex/alice" },
          predicate: { type: "uri", value: "http://ex/says" },
          object: { type: "literal", value: 'hi "there"', "xml:lang": "en" },
        },
      },
      g: { type: "uri", value: "http://ex/g1" },
    },
  ];
  const out = rowsToNQuads(rows);
  assert.equal(
    out,
    '_:r1 <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> ' +
      '<<( <http://ex/alice> <http://ex/says> "hi \\"there\\""@en )>> <http://ex/g1> .',
  );
  assert.ok(!out.includes("undefined"));
});

// RDF 1.2 base direction (`its:dir` in SPARQL 1.2 JSON) is part of a literal's identity:
// a snapshot must write `"hi"@en--ltr`, at the top level and inside a triple term.
test("rowsToNQuads round-trips a directional literal, top-level and in a triple term", () => {
  const a = { type: "uri", value: "http://ex/a" };
  const p = { type: "uri", value: "http://ex/p" };
  const hi = (dir) => ({ type: "literal", value: "hi", "xml:lang": "en", "its:dir": dir });
  const tt = (o) => ({ type: "triple", value: { subject: a, predicate: p, object: o } });
  const rows = [
    { s: a, p, o: hi("ltr") },
    { s: a, p, o: tt(hi("rtl")) },
    { s: a, p, o: tt(hi(undefined)) },
  ];
  const nq = rowsToNQuads(rows);
  assert.equal(
    nq,
    '<http://ex/a> <http://ex/p> "hi"@en--ltr .\n' +
      '<http://ex/a> <http://ex/p> <<( <http://ex/a> <http://ex/p> "hi"@en--rtl )>> .\n' +
      '<http://ex/a> <http://ex/p> <<( <http://ex/a> <http://ex/p> "hi"@en )>> .',
  );
  const { statements } = parseNTriples(nq);
  assert.deepEqual(
    statements.map((st) => (st.o.kind === "triple" ? st.o.o.lang : st.o.lang)),
    ["en--ltr", "en--rtl", "en"],
  );
});

test("rowsToNQuads emits a quad line (with the graph) for named-graph rows", () => {
  const rows = [
    {
      s: { type: "uri", value: "http://ex/a" },
      p: { type: "uri", value: "http://ex/p" },
      o: { type: "literal", value: "in-g1" },
      g: { type: "uri", value: "http://ex/g1" },
    },
  ];
  assert.equal(
    rowsToNQuads(rows),
    '<http://ex/a> <http://ex/p> "in-g1" <http://ex/g1> .',
  );
});

test("rowsToNQuads round-trips a mixed dataset, preserving the named graph", () => {
  const rows = [
    {
      s: { type: "uri", value: "http://ex/d" },
      p: { type: "uri", value: "http://ex/p" },
      o: { type: "literal", value: "default" },
    },
    {
      s: { type: "uri", value: "http://ex/a" },
      p: { type: "uri", value: "http://ex/p" },
      o: { type: "literal", value: "in-g1" },
      g: { type: "uri", value: "http://ex/g1" },
    },
  ];
  const nq = rowsToNQuads(rows);
  // The default-graph triple stays a triple; the named-graph triple keeps <g1>.
  assert.equal(
    nq,
    '<http://ex/d> <http://ex/p> "default" .\n' +
      '<http://ex/a> <http://ex/p> "in-g1" <http://ex/g1> .',
  );
});

test("rowsToNQuads encodes term kinds (IRI / bnode / literal w/ datatype & lang)", () => {
  const rows = [
    {
      s: { type: "bnode", value: "b0" },
      p: { type: "uri", value: "http://ex/n" },
      o: { type: "literal", value: "42", datatype: "http://www.w3.org/2001/XMLSchema#integer" },
    },
    {
      s: { type: "uri", value: "http://ex/x" },
      p: { type: "uri", value: "http://ex/label" },
      o: { type: "literal", value: "hi", "xml:lang": "en" },
    },
  ];
  assert.equal(
    rowsToNQuads(rows),
    '_:b0 <http://ex/n> "42"^^<http://www.w3.org/2001/XMLSchema#integer> .\n' +
      '<http://ex/x> <http://ex/label> "hi"@en .',
  );
});

test("rowsToNQuads escapes backslash, quote, CR and LF in literals", () => {
  // Mirrors the engine's own N-Triples escaping (js/src/sparql.ts escapeLiteral):
  // backslash, double-quote, LF and CR. A raw tab is legal inside a quoted literal
  // and is left as-is, so the re-parse is byte-faithful.
  const rows = [
    {
      s: { type: "uri", value: "http://ex/x" },
      p: { type: "uri", value: "http://ex/p" },
      o: { type: "literal", value: 'a "q"\nlf\r cr \\ bs' },
    },
  ];
  assert.equal(
    rowsToNQuads(rows),
    '<http://ex/x> <http://ex/p> "a \\"q\\"\\nlf\\r cr \\\\ bs" .',
  );
});

// [OPUS-4.8] sq-vfbm — query-form classification: the REPL routes each form to a
// different wasm export (query / queryQuads / updateInPlace), so the classifier must
// read the leading significant keyword past the prologue and comments.
test("classifyQueryForm recognises the four query forms", () => {
  assert.equal(classifyQueryForm("SELECT ?s WHERE { ?s ?p ?o }"), "select");
  assert.equal(classifyQueryForm("ASK { ?s ?p ?o }"), "ask");
  assert.equal(
    classifyQueryForm("CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }"),
    "construct",
  );
  assert.equal(classifyQueryForm("DESCRIBE <http://ex/a>"), "describe");
  // Case-insensitive.
  assert.equal(classifyQueryForm("select ?s where { ?s ?p ?o }"), "select");
  assert.equal(classifyQueryForm("describe ?x"), "describe");
});

test("classifyQueryForm recognises every SPARQL Update keyword", () => {
  for (const kw of [
    "INSERT DATA { <a> <b> <c> }",
    "DELETE WHERE { ?s ?p ?o }",
    "LOAD <http://ex/g>",
    "CLEAR GRAPH <http://ex/g>",
    "CREATE GRAPH <http://ex/g>",
    "DROP GRAPH <http://ex/g>",
    "ADD <http://ex/a> TO <http://ex/b>",
    "MOVE <http://ex/a> TO <http://ex/b>",
    "COPY <http://ex/a> TO <http://ex/b>",
    "WITH <http://ex/g> DELETE { ?s ?p ?o } WHERE { ?s ?p ?o }",
  ]) {
    assert.equal(classifyQueryForm(kw), "update", kw);
  }
});

test("classifyQueryForm reads past the prologue (PREFIX / BASE)", () => {
  const q = `PREFIX ex: <http://example.org/>
PREFIX foaf: <http://xmlns.com/foaf/0.1/>
SELECT ?name WHERE { ex:alice foaf:name ?name }`;
  assert.equal(classifyQueryForm(q), "select");

  const u = `PREFIX ex: <http://example.org/>
INSERT DATA { ex:alice ex:knows ex:erin }`;
  assert.equal(classifyQueryForm(u), "update");

  const c = `BASE <http://example.org/>
CONSTRUCT { ?a <friendOf> ?b } WHERE { ?a <knows> ?b }`;
  assert.equal(classifyQueryForm(c), "construct");
});

test("classifyQueryForm skips leading comments", () => {
  const q = `# a comment line
# another, with a SELECT keyword that must NOT win
ASK { ?s ?p ?o }`;
  assert.equal(classifyQueryForm(q), "ask");
});

test("classifyQueryForm does not mistake INSERTED-into-a-name etc. for whole words", () => {
  // A SELECT projecting a variable named like a keyword stays a SELECT; the Update
  // keywords only match as the leading whole token.
  assert.equal(classifyQueryForm("SELECT ?insert WHERE { ?insert ?p ?o }"), "select");
  // Unknown leading token defaults to select so the engine surfaces the parse error.
  assert.equal(classifyQueryForm("GIBBERISH foo bar"), "select");
});

test("isGraphForm is true exactly for CONSTRUCT and DESCRIBE", () => {
  assert.equal(isGraphForm("construct"), true);
  assert.equal(isGraphForm("describe"), true);
  assert.equal(isGraphForm("select"), false);
  assert.equal(isGraphForm("ask"), false);
  assert.equal(isGraphForm("update"), false);
});

// [OPUS-4.8] sq-xe4f — EXPLAIN / ANALYZE drive the query planner, which rejects SPARQL
// Update forms; the REPL grays those mode tabs out for Update examples. The decision is
// this pure predicate, so the table-of-cases test guards the gating rule directly.
test("modeSupportsForm: Run is available for every form", () => {
  for (const form of ["select", "ask", "construct", "describe", "update"]) {
    assert.equal(modeSupportsForm("run", form), true, form);
  }
});

test("modeSupportsForm: EXPLAIN / ANALYZE are available for the query forms", () => {
  for (const form of ["select", "ask", "construct", "describe"]) {
    assert.equal(modeSupportsForm("explain", form), true, `explain/${form}`);
    assert.equal(modeSupportsForm("analyze", form), true, `analyze/${form}`);
  }
});

test("modeSupportsForm: EXPLAIN / ANALYZE are UNavailable for SPARQL Update", () => {
  // The bead (sq-xe4f): selecting EXPLAIN or ANALYZE on an Update example yields an
  // engine parse error, so the toggle must gray those modes out for "update".
  assert.equal(modeSupportsForm("explain", "update"), false);
  assert.equal(modeSupportsForm("analyze", "update"), false);
  // Run still works — the Update can still be executed.
  assert.equal(modeSupportsForm("run", "update"), true);
});

// [OPUS-4.8] sq-daru — the dataset panel: named-graph list with per-graph triple counts.
// The two queries are reused engine-supported SPARQL (no new Store API); `parseGraphStats`
// is the pure parser that turns their SPARQL-JSON into the panel's typed rows. The
// SPARQL-JSON fixtures below are the EXACT shape the wasm engine returns (verified live:
// a GROUP BY ?g list of named graphs, and a standalone default-graph COUNT — each `?c` an
// xsd:integer literal whose `value` is the integer string).

test("NAMED_GRAPH_STATS_QUERY groups per named graph and excludes the default graph", () => {
  // GROUP BY over GRAPH ?g lists exactly the named graphs; the default graph is not a
  // GRAPH, so it is counted separately by DEFAULT_GRAPH_COUNT_QUERY.
  assert.match(NAMED_GRAPH_STATS_QUERY, /GRAPH \?g/);
  assert.match(NAMED_GRAPH_STATS_QUERY, /GROUP BY \?g/);
  assert.match(NAMED_GRAPH_STATS_QUERY, /COUNT\(\*\)/);
  // The default-graph count uses the unqualified BGP (matches only the default graph).
  assert.match(DEFAULT_GRAPH_COUNT_QUERY, /WHERE \{ \?s \?p \?o \}/);
  assert.doesNotMatch(DEFAULT_GRAPH_COUNT_QUERY, /GRAPH/);
});

const XSD_INT = "http://www.w3.org/2001/XMLSchema#integer";
const count = (v) => ({ type: "literal", value: v, datatype: XSD_INT });
const uri = (v) => ({ type: "uri", value: v });

test("parseGraphStats always emits the default graph first, then the named graphs in order", () => {
  // The exact wasm-engine result shape (verified live).
  const defaultJson = { head: { vars: ["c"] }, results: { bindings: [{ c: count("2") }] } };
  const namedJson = {
    head: { vars: ["g", "c"] },
    results: {
      bindings: [
        { g: uri("http://ex/g1"), c: count("2") },
        { g: uri("http://ex/g2"), c: count("1") },
      ],
    },
  };
  const stats = parseGraphStats(defaultJson, namedJson);
  assert.deepEqual(stats, [
    { iri: null, isDefault: true, count: 2 },
    { iri: "http://ex/g1", isDefault: false, count: 2 },
    { iri: "http://ex/g2", isDefault: false, count: 1 },
  ]);
});

test("parseGraphStats shows the default graph alone (count 0) for an empty store", () => {
  // An empty store: the default-graph COUNT is 0 and there are no named-graph rows. The
  // default graph is STILL listed (count 0) so the panel is honest about an empty dataset.
  const defaultJson = { head: { vars: ["c"] }, results: { bindings: [{ c: count("0") }] } };
  const namedJson = { head: { vars: ["g", "c"] }, results: { bindings: [] } };
  assert.deepEqual(parseGraphStats(defaultJson, namedJson), [
    { iri: null, isDefault: true, count: 0 },
  ]);
});

test("parseGraphStats handles a triple-only dataset (default graph only, no named graphs)", () => {
  // The common case: a Turtle dataset has only the default graph.
  const defaultJson = { results: { bindings: [{ c: count("142") }] } };
  const namedJson = { results: { bindings: [] } };
  assert.deepEqual(parseGraphStats(defaultJson, namedJson), [
    { iri: null, isDefault: true, count: 142 },
  ]);
});

test("parseGraphStats defaults a missing/unparseable count to 0 rather than NaN", () => {
  // A partial / malformed result must still render: an absent default binding -> 0, a
  // named-graph row with a non-numeric count -> 0 (never NaN in the UI).
  const defaultJson = { results: { bindings: [] } };
  const namedJson = {
    results: { bindings: [{ g: uri("http://ex/g1"), c: { type: "literal", value: "x" } }] },
  };
  assert.deepEqual(parseGraphStats(defaultJson, namedJson), [
    { iri: null, isDefault: true, count: 0 },
    { iri: "http://ex/g1", isDefault: false, count: 0 },
  ]);
});

test("parseGraphStats skips a defensive named-graph row that does not bind ?g", () => {
  const defaultJson = { results: { bindings: [{ c: count("1") }] } };
  const namedJson = {
    results: {
      bindings: [
        { c: count("9") }, // no ?g — skipped
        { g: uri("http://ex/g1"), c: count("3") },
      ],
    },
  };
  assert.deepEqual(parseGraphStats(defaultJson, namedJson), [
    { iri: null, isDefault: true, count: 1 },
    { iri: "http://ex/g1", isDefault: false, count: 3 },
  ]);
});
