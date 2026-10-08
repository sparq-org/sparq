// [FABLE-5] sq-ixc3.20 — unit tests for the canonical inferred-fact identity logic
// (node test runner). The load-bearing property: the SAME triple keyed from a SPARQL-JSON
// binding (decoded lexical form) and from a parsed N-Triples line (verbatim-escaped) yields
// the SAME key — regardless of `^^xsd:string` suppression or escape spelling.

import { strict as assert } from "node:assert";
import { test } from "node:test";

import { parseNTriples, type SparqlTerm } from "@sparq/client";

import {
  entailedFactsFromClosure,
  entailedKeysFromClosure,
  inferredFactsMatchingKeys,
  keyOfRdfTerm,
  keyOfSparqlTerm,
  termToNT,
  tripleKeyOfBindings,
  tripleKeyOfTerms,
  tripleKeysOfNTriples,
  unescapeNT,
} from "./inferred-facts.js";

/** Key a one-line N-Triples statement through the PARSED (RdfTerm) path. */
function keyOfLine(line: string): string {
  const { statements } = parseNTriples(line);
  assert.equal(statements.length, 1, `must parse: ${line}`);
  const st = statements[0];
  return tripleKeyOfTerms(st.s, st.p, st.o);
}

test("unescapeNT decodes the N-Triples escapes and keeps malformed ones verbatim", () => {
  assert.equal(unescapeNT('a\\"b\\nc\\\\d\\te'), 'a"b\nc\\d\te');
  assert.equal(unescapeNT("caf\\u00e9"), "café");
  assert.equal(unescapeNT("\\U0001F600"), "😀");
  assert.equal(unescapeNT("plain"), "plain");
  assert.equal(unescapeNT("bad\\uZZ"), "bad\\uZZ");
});

test("SPARQL-JSON and parsed-N-Triples keys agree for every term shape", () => {
  const iri: SparqlTerm = { type: "uri", value: "http://ex/s" };
  const bnode: SparqlTerm = { type: "bnode", value: "b0" };
  const plain: SparqlTerm = { type: "literal", value: "hi" };
  const typedString: SparqlTerm = {
    type: "literal",
    value: "hi",
    datatype: "http://www.w3.org/2001/XMLSchema#string",
  };
  const langed: SparqlTerm = { type: "literal", value: "hi", "xml:lang": "en" };
  const typed: SparqlTerm = {
    type: "literal",
    value: "5",
    datatype: "http://www.w3.org/2001/XMLSchema#integer",
  };
  const escaped: SparqlTerm = { type: "literal", value: 'a"b\nc' };
  // RDF 1.2 triple term (#6044): SPARQL 1.2 JSON nests the triple under `value`.
  const tripleTerm: SparqlTerm = {
    type: "triple",
    value: { subject: iri, predicate: iri, object: escaped },
  };

  // xml:lang / datatype normalisation: an explicit ^^xsd:string equals a plain literal.
  assert.equal(keyOfSparqlTerm(plain), keyOfSparqlTerm(typedString));

  for (const t of [iri, bnode, plain, typedString, langed, typed, escaped, tripleTerm]) {
    const line: string = `${termToNT(iri)} ${termToNT(iri)} ${termToNT(t)} .`;
    const { statements } = parseNTriples(line);
    assert.equal(statements.length, 1, `round-trip parse: ${line}`);
    assert.equal(
      keyOfRdfTerm(statements[0].o),
      keyOfSparqlTerm(t),
      `parsed key must equal binding key for ${JSON.stringify(t)}`,
    );
  }
});

test("triple keys agree across the binding and parsed paths", () => {
  const s: SparqlTerm = { type: "uri", value: "http://ex/rex" };
  const p: SparqlTerm = { type: "uri", value: "http://www.w3.org/1999/02/22-rdf-syntax-ns#type" };
  const o: SparqlTerm = { type: "uri", value: "http://ex/Animal" };
  assert.equal(
    tripleKeyOfBindings(s, p, o),
    keyOfLine(`${termToNT(s)} ${termToNT(p)} ${termToNT(o)} .`),
  );
});

test("tripleKeysOfNTriples folds named graphs and skips junk lines", () => {
  const doc = [
    "<http://ex/a> <http://ex/p> <http://ex/b> .",
    "<http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g> .", // same s/p/o, named graph
    "this line is not a statement",
    "",
  ].join("\n");
  const keys = tripleKeysOfNTriples(doc);
  assert.equal(keys.size, 1, "graph term ignored + dedup + junk skipped");
});

test("entailedKeysFromClosure is exactly closure minus base", () => {
  const base = tripleKeysOfNTriples("<http://ex/a> <http://ex/p> <http://ex/b> .");
  const closure = [
    "<http://ex/a> <http://ex/p> <http://ex/b> .",
    "<http://ex/a> <http://ex/q> <http://ex/b> .",
  ].join("\n");
  const entailed = entailedKeysFromClosure(closure, base);
  assert.equal(entailed.size, 1);
  assert.ok(entailed.has(keyOfLine("<http://ex/a> <http://ex/q> <http://ex/b> .")));
  // Membership is the affordance gate: the asserted triple is NOT marked.
  assert.ok(!entailed.has(keyOfLine("<http://ex/a> <http://ex/p> <http://ex/b> .")));
});

test("entailed facts retain one exact, explainable N-Triples line per added triple", () => {
  // [GPT-5.6] Exact expected values make this non-vacuous: changing q→r, retaining the asserted
  // p fact, or failing to fold the duplicate named-graph statement makes the test fail.
  const base = tripleKeysOfNTriples("<http://ex/a> <http://ex/p> <http://ex/b> .");
  const closure = [
    "<http://ex/a> <http://ex/p> <http://ex/b> .",
    '<http://ex/a> <http://ex/q> "entailed"@en .',
    '<http://ex/a> <http://ex/q> "entailed"@en <http://ex/graph> .',
  ].join("\n");
  const entailed = entailedFactsFromClosure(closure, base);

  assert.equal(entailed.keys.size, 1);
  assert.deepEqual(entailed.facts, [
    {
      key: keyOfLine('<http://ex/a> <http://ex/q> "entailed"@en .'),
      s: "<http://ex/a>",
      p: "<http://ex/q>",
      o: '"entailed"@en',
      ntriples: '<http://ex/a> <http://ex/q> "entailed"@en .',
    },
  ]);
  assert.deepEqual(inferredFactsMatchingKeys(closure, entailed.keys), entailed.facts);
});

test("an asserted triple term matches the reasoner's closure spelling (escaped tab)", () => {
  // The shared JS writer emits a raw TAB inside a literal; the Rust closure writer emits `\t`.
  // Both spell the SAME asserted fact, so it must never be reported as inferred.
  const a: SparqlTerm = { type: "uri", value: "http://ex/a" };
  const p: SparqlTerm = { type: "uri", value: "http://ex/p" };
  const r: SparqlTerm = { type: "uri", value: "http://ex/r" };
  const tabbed: SparqlTerm = { type: "literal", value: "x\ty" };
  const quoted: SparqlTerm = {
    type: "triple",
    value: { subject: a, predicate: p, object: tabbed },
  };
  const snapshot = `${termToNT(r)} ${termToNT(p)} ${termToNT(quoted)} .`;
  assert.ok(snapshot.includes("x\ty"), "JS writer keeps the raw tab");
  const closure = '<http://ex/r> <http://ex/p> <<( <http://ex/a> <http://ex/p> "x\\ty" )>> .';

  const base = tripleKeysOfNTriples(snapshot);
  const entailed = entailedFactsFromClosure(closure, base);
  assert.equal(entailed.keys.size, 0, "asserted triple-term fact is not inferred");
  assert.deepEqual(entailed.facts, []);
  // The SPARQL binding key must match the closure key too.
  assert.equal(tripleKeyOfBindings(r, p, quoted), keyOfLine(closure));
});

test("triple-term keys are built from decoded components, not spelling", () => {
  const plain = keyOfLine('<http://ex/r> <http://ex/p> <<( <http://ex/a> <http://ex/p> "v" )>> .');
  // Explicit ^^xsd:string, extra whitespace and a \u escape spell the same triple term.
  assert.equal(
    keyOfLine(
      '<http://ex/r> <http://ex/p> <<(  <http://ex/a>   <http://ex/p> "\\u0076"^^<http://www.w3.org/2001/XMLSchema#string>  )>> .',
    ),
    plain,
  );
  // Nested triple terms canonicalise recursively.
  const nestedA = keyOfLine(
    '<http://ex/r> <http://ex/p> <<( <http://ex/a> <http://ex/p> <<( <http://ex/a> <http://ex/p> "x\\ty" )>> )>> .',
  );
  const a: SparqlTerm = { type: "uri", value: "http://ex/a" };
  const p: SparqlTerm = { type: "uri", value: "http://ex/p" };
  const inner: SparqlTerm = {
    type: "triple",
    value: {
      subject: a,
      predicate: p,
      object: { type: "literal", value: "x\ty" },
    },
  };
  const outer: SparqlTerm = {
    type: "triple",
    value: { subject: a, predicate: p, object: inner },
  };
  assert.equal(tripleKeyOfBindings({ type: "uri", value: "http://ex/r" }, p, outer), nestedA);
  // Different components still differ.
  assert.notEqual(
    keyOfLine('<http://ex/r> <http://ex/p> <<( <http://ex/a> <http://ex/p> "w" )>> .'),
    plain,
  );
  assert.notEqual(
    keyOfLine('<http://ex/r> <http://ex/p> <<( <http://ex/a> <http://ex/p> "v"@en )>> .'),
    plain,
  );
});

test("literal base direction is a key component on both paths", () => {
  const ltr = keyOfLine('<http://ex/a> <http://ex/p> "hi"@en--ltr .');
  assert.notEqual(ltr, keyOfLine('<http://ex/a> <http://ex/p> "hi"@en .'));
  assert.notEqual(ltr, keyOfLine('<http://ex/a> <http://ex/p> "hi"@en--rtl .'));
  const dirLit = {
    type: "literal",
    value: "hi",
    "xml:lang": "en",
    "its:dir": "ltr",
  } as SparqlTerm;
  const a: SparqlTerm = { type: "uri", value: "http://ex/a" };
  const p: SparqlTerm = { type: "uri", value: "http://ex/p" };
  assert.equal(tripleKeyOfBindings(a, p, dirLit), ltr);
});

test("a directional literal inside a triple term round-trips through termToNT", () => {
  const a: SparqlTerm = { type: "uri", value: "http://ex/a" };
  const p: SparqlTerm = { type: "uri", value: "http://ex/p" };
  const tt = (dir: string | undefined): SparqlTerm => ({
    type: "triple",
    value: {
      subject: a,
      predicate: p,
      object: { type: "literal", value: "hi", "xml:lang": "en", "its:dir": dir },
    },
  });
  for (const t of [tt("ltr"), tt("rtl"), tt(undefined)]) {
    const line: string = `${termToNT(a)} ${termToNT(p)} ${termToNT(t)} .`;
    assert.equal(keyOfLine(line), tripleKeyOfBindings(a, p, t));
  }
  // The snapshot keeps the direction, so terms differing only by it stay distinct.
  assert.equal(termToNT(tt("ltr")), '<<( <http://ex/a> <http://ex/p> "hi"@en--ltr )>>');
  assert.notEqual(termToNT(tt("ltr")), termToNT(tt("rtl")));
  assert.notEqual(termToNT(tt("ltr")), termToNT(tt(undefined)));
});
