import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { rewriteForGraphs } from "../src/lib/solid-acl.ts";

const graph = "https://alice.pod.example/health/records";
const group = "WHERE { GRAPH ?g { ?s ?p ?o } }";
const longLiteral = '"""\n) WHERE {"""';
const escapedIris = [String.raw`<urn:a\u0041#x>`, String.raw`<urn:a\U00000041#x>`, "<urn:aA#x>"];

// Like the browser engine tests, parser parity uses the optional synced WASM bundle.
// The boundary-placement assertions below also run without that bundle.
const wasmPath = new URL("../public/wasm/sparq_wasm_bg.wasm", import.meta.url);
const engine = existsSync(wasmPath) ? await import("../public/wasm/sparq_wasm.js") : undefined;
if (engine) engine.initSync({ module: readFileSync(wasmPath) });
const store = engine?.Store.loadDataset(
  `<urn:s> <urn:p> <urn:o> <${graph}> .\n<urn:other> <urn:p> <urn:o> <urn:other-graph> .`,
  "nquads",
);
after(() => store?.free());
const engineRequired = { skip: !store && "requires the lean WASM bundle in public/wasm" };

function assertInvalidBeforeAndAfter(query) {
  assert.throws(() => store.explainPlanJson(query), /error at .*expected/s, query);
  assert.throws(
    () => store.explainPlanJson(rewriteForGraphs(query, [graph])),
    /error at .*expected/s,
    "rewriting must preserve the engine's rejection",
  );
}

test("ASK accepts an opening group with or without WHERE", () => {
  for (const [head, rewrittenHead] of [
    ["ASK ", `ASK FROM NAMED <${graph}>\n`],
    ["ASK", `ASK\nFROM NAMED <${graph}>\n`],
    ["ask\n", `ask\nFROM NAMED <${graph}>\n`],
    ["ASK WHERE ", `ASK FROM NAMED <${graph}>\nWHERE `],
    ["ask where\n", `ask FROM NAMED <${graph}>\nwhere\n`],
  ]) {
    const group = "{ GRAPH ?g { ?s ?p ?o } }";
    assert.equal(rewriteForGraphs(head + group, [graph]), rewrittenHead + group);
  }
  assert.equal(rewriteForGraphs("ASK{}", []), "ASK\nFROM NAMED <urn:sparq:nothing>\n{}");
});

test("ASK inserts before its outer group, skipping prologue IRIs and comments", () => {
  const prefix = "BASE <https://example.org/WHERE>\nPREFIX ex: <https://example.org/#>\n# WHERE { ignored\nASK FROM NAMED <urn:WHERE> # { another comment\n";
  const group = '{ GRAPH ?g { ?s ?p "WHERE {" } FILTER EXISTS { ?s ?p ?o } }';
  assert.equal(
    rewriteForGraphs(prefix + group, [graph, "urn:another"]),
    `${prefix}FROM NAMED <${graph}>\nFROM NAMED <urn:another>\n${group}`,
  );
});

test("WHERE in projection literals and nested patterns is not the outer WHERE", () => {
  const prefix = `PREFIX ex: <https://example.org/#>\nSELECT ("WHERE {" AS ?label) (EXISTS { SELECT ?s WHERE { ?s ?p ?o } } AS ?found) `;
  const group = "WHERE { GRAPH ?g { ?s ?p ?o } }";
  assert.equal(rewriteForGraphs(prefix + group, [graph]), `${prefix}FROM NAMED <${graph}>\n${group}`);
});

test("CONSTRUCT keeps its template ahead of the dataset clause", () => {
  const prefix = "CONSTRUCT { ?s ?p ?o } ";
  const group = "WHERE { GRAPH ?g { ?s ?p ?o } }";
  assert.equal(rewriteForGraphs(prefix + group, [graph]), `${prefix}FROM NAMED <${graph}>\n${group}`);
});

test("ASK dataset prefixed names keep escaped punctuation opaque", () => {
  for (const escaped of ["#", "(", ")", "'", "/", "?", ";", "="]) {
    for (const where of ["", "WHERE "]) {
      const prefix = `PREFIX ex: <https://example.org/> ASK FROM NAMED ex:part\\${escaped}WHERE `;
      const group = `${where}{ GRAPH ?g { ?s ?p ?o } }`;
      assert.equal(rewriteForGraphs(prefix + group, [graph]), `${prefix}FROM NAMED <${graph}>\n${group}`);
    }
  }
});

test("comments end at CR, LF or CRLF before ASK groups and WHERE", () => {
  for (const newline of ["\r", "\n", "\r\n"]) {
    for (const form of ["ASK", "SELECT ?WHERE"]) {
      const prefix = `${form} # WHERE { ignored${newline}`;
      const group = `${form === "ASK" ? "" : "WHERE "}{ GRAPH ?g { ?s ?p ?o } }`;
      assert.equal(rewriteForGraphs(prefix + group, [graph]), `${prefix}FROM NAMED <${graph}>\n${group}`);
    }
  }
});

test("SELECT comparison operators do not swallow the outer WHERE", () => {
  const prefix = "SELECT (1 < 2 AS ?less) ";
  const group = "WHERE { GRAPH ?g { ?s ?p ?o FILTER (?o > 3) } }";
  assert.equal(rewriteForGraphs(prefix + group, [graph]), `${prefix}FROM NAMED <${graph}>\n${group}`);
});

for (const [name, prefix] of [
  ["compact IF comparison", "SELECT (IF(?a<(1),1,2)>0 AS ?x) "],
  ["compact comparison with an EXISTS subselect", "SELECT ((?a<(((1)>2)))&&EXISTS{SELECT*WHERE{?s?p?o}} AS ?x) "],
]) {
  test(`${name} remains invalid after rewriting`, engineRequired, () => {
    assertInvalidBeforeAndAfter(prefix + "WHERE {?s ?p ?o}");
  });
}

test("compact comparisons after values and comments remain invalid", engineRequired, () => {
  for (const prefix of [
    "SELECT (IF(1<(2),1,2)>0 AS ?x) ",
    "SELECT (IF((?a)<(1),1,2)>0 AS ?x) ",
    'SELECT (IF("a"<(1),1,2)>0 AS ?x) ',
    'SELECT (IF("a"@en<(1),1,2)>0 AS ?x) ',
    "SELECT (IF(true<(1),1,2)>0 AS ?x) ",
    "PREFIX ex: <urn:example:> SELECT (IF(ex:a<(1),1,2)>0 AS ?x) ",
    "SELECT (IF(<urn:a><(1),1,2)>0 AS ?x) ",
    "SELECT (IF(?a # comparison follows\n <(1),1,2)>0 AS ?x) ",
  ]) {
    assertInvalidBeforeAndAfter(prefix + "WHERE {?s ?p ?o}");
  }
});

test("a spaced IF comparison retains the outer WHERE", () => {
  const prefix = "SELECT (IF(?a < (1),1,2) > 0 AS ?x) ";
  assert.equal(rewriteForGraphs(prefix + group, [graph]), `${prefix}FROM NAMED <${graph}>\n${group}`);
});

test("the engine evaluates a rewritten spaced IF comparison", engineRequired, () => {
  const query = "SELECT (IF(?a < (1),1,2) > 0 AS ?x) WHERE { VALUES ?a { 0 } }";
  const expected = [{ x: { type: "literal", value: "true", datatype: "http://www.w3.org/2001/XMLSchema#boolean" } }];
  assert.deepEqual(JSON.parse(store.query(query)).results.bindings, expected);
  assert.deepEqual(JSON.parse(store.query(rewriteForGraphs(query, [graph]))).results.bindings, expected);
});

const iriPunctuationPrefixes = [
  "BASE <urn:WHERE(,> SELECT ?s ",
  "PREFIX ex: <urn:WHERE),> SELECT ?s ",
  "SELECT (<urn:WHERE(,> AS ?iri) ",
  "SELECT ?s FROM NAMED <urn:WHERE(,> ",
  "SELECT ?s FROM <urn:default> FROM NAMED <urn:existing> ",
  "SELECT (EXISTS { ?s ?p <urn:WHERE(,> } AS ?x) ",
  "CONSTRUCT { ?s ?p (?o <urn:WHERE(,>) } ",
];

test("legal IRI punctuation stays opaque", () => {
  for (const prefix of iriPunctuationPrefixes) {
    const group = "WHERE {?s ?p ?o}";
    assert.equal(rewriteForGraphs(prefix + group, [graph]), `${prefix}FROM NAMED <${graph}>\n${group}`);
  }
});

test("the engine accepts legal IRI punctuation before and after rewriting", engineRequired, () => {
  for (const prefix of iriPunctuationPrefixes) {
    const query = prefix + "WHERE {?s ?p ?o}";
    assert.doesNotThrow(() => store.explainPlanJson(query), query);
    assert.doesNotThrow(() => store.explainPlanJson(rewriteForGraphs(query, [graph])), query);
  }
});

test("nested compact comparisons remain invalid despite legal IRI punctuation", engineRequired, () => {
  for (const prefix of [
    "SELECT (EXISTS { SELECT (IF(?a<(1),1,2)>0 AS ?x) WHERE {?s ?p ?o} } AS ?x) ",
    "SELECT (EXISTS { FILTER IF(?a<(1),true,2>0) } AS ?x) ",
    "SELECT (EXISTS { FILTER <urn:f>(?a<(1),true,2>0) } AS ?x) ",
  ]) {
    assertInvalidBeforeAndAfter(prefix + "WHERE {?s ?p ?o}");
  }
});

test("subtraction before an IRI keeps the dataset clause before the real WHERE", () => {
  for (const operand of ["1", "?x"]) {
    const prefix = `SELECT (${operand}-<urn:f#b>(?x) AS ?y) (${longLiteral} AS ?z) `;
    assert.equal(rewriteForGraphs(prefix + group, [graph]), `${prefix}FROM NAMED <${graph}>\n${group}`);
  }
});

test("the engine parses subtraction before an IRI before and after rewriting", engineRequired, () => {
  for (const operand of ["1", "?x"]) {
    for (const literal of ["", ` (${longLiteral} AS ?z)`]) {
      const query = `SELECT (${operand}-<urn:f#b>(?x) AS ?y)${literal} ${group}`;
      assert.doesNotThrow(() => store.explainPlanJson(query), query);
      assert.doesNotThrow(() => store.explainPlanJson(rewriteForGraphs(query, [graph])), query);
    }
  }
});

test("UCHAR escapes in an IRI keep the dataset clause before the real WHERE", () => {
  for (const iri of escapedIris) {
    const prefix = `SELECT (${iri} AS ?y) (${longLiteral} AS ?z) `;
    assert.equal(rewriteForGraphs(prefix + group, [graph]), `${prefix}FROM NAMED <${graph}>\n${group}`);
  }
});

test("the engine evaluates rewritten UCHAR IRIs with the supplied dataset", engineRequired, () => {
  for (const iri of escapedIris) {
    const query = `SELECT (${iri} AS ?y) (${longLiteral} AS ?z) ${group}`;
    assert.equal(JSON.parse(store.query(query)).results.bindings.length, 2);
    assert.deepEqual(JSON.parse(store.query(rewriteForGraphs(query, [graph]))).results.bindings, [
      { y: { type: "uri", value: "urn:aA#x" }, z: { type: "literal", value: "\n) WHERE {" } },
    ]);
    assert.deepEqual(JSON.parse(store.query(rewriteForGraphs(query, []))).results.bindings, []);
  }
});

test("escaped and long projection strings do not contribute pattern boundaries", () => {
  for (const literal of [
    String.raw`'WHERE \' {'`,
    String.raw`"WHERE \" {"`,
    "'''WHERE\n'' {'''",
    '"""WHERE\n"" {"""',
  ]) {
    const prefix = `BASE <urn:WHERE> SELECT (${literal} AS ?WHERE) `;
    const group = "WHERE { GRAPH ?g { ?s ?p ?o } }";
    assert.equal(rewriteForGraphs(prefix + group, [graph]), `${prefix}FROM NAMED <${graph}>\n${group}`);
  }
});

for (const [name, literal] of [
  ["double-quoted", '"""a"b WHERE {"""'],
  ["single-quoted", "'''a'b WHERE {'''"],
]) {
  test(`long ${name} strings retain isolated matching quotes before WHERE text`, () => {
    const prefix = `SELECT (${literal} AS ?x) `;
    const group = "WHERE {?s ?p ?o}";
    assert.equal(rewriteForGraphs(prefix + group, [graph]), `${prefix}FROM NAMED <${graph}>\n${group}`);
  });
}

test("Unicode name characters are not SPARQL token separators", () => {
  for (const name of ["ex:\u1680WHERE", "ex:\uFEFFWHERE", "ex:graph\uFEFF"]) {
    const prefix = `PREFIX ex: <https://example.org/> ASK FROM NAMED ${name}`;
    const group = "{ GRAPH ?g { ?s ?p ?o } }";
    assert.equal(rewriteForGraphs(prefix + group, [graph]), `${prefix}\nFROM NAMED <${graph}>\n${group}`);
  }
});
