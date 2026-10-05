// #6044: SPARQL 1.2 Query Results JSON carries an RDF 1.2 triple term as
// {"type":"triple","value":{"subject":…,"predicate":…,"object":…}} (the exact shape
// crates/sparq-engine/src/json.rs emits). The shared term helpers must handle it rather than
// treating `value` as a string.
import assert from "node:assert/strict";
import { test } from "node:test";

import {
  formatTerm,
  isTripleTerm,
  resultsToCsv,
  termToNTriples,
  termValue,
} from "../src/index.ts";

const alice = { type: "uri", value: "http://ex/alice" };
const knows = { type: "uri", value: "http://ex/knows" };
const bob = { type: "literal", value: 'Bob "B"\n', "xml:lang": "en" };
const triple = { type: "triple", value: { subject: alice, predicate: knows, object: bob } };
const nested = { type: "triple", value: { subject: alice, predicate: knows, object: triple } };

test("termToNTriples writes every atomic term kind", () => {
  assert.equal(termToNTriples(alice), "<http://ex/alice>");
  assert.equal(termToNTriples({ type: "bnode", value: "b0" }), "_:b0");
  assert.equal(termToNTriples(bob), '"Bob \\"B\\"\\n"@en');
  assert.equal(
    termToNTriples({
      type: "literal",
      value: "5",
      datatype: "http://www.w3.org/2001/XMLSchema#integer",
    }),
    '"5"^^<http://www.w3.org/2001/XMLSchema#integer>',
  );
  assert.equal(
    termToNTriples({
      type: "literal",
      value: "s",
      datatype: "http://www.w3.org/2001/XMLSchema#string",
    }),
    '"s"',
  );
});

test("termToNTriples writes an RDF 1.2 triple term, including nested ones", () => {
  const inner = '<<( <http://ex/alice> <http://ex/knows> "Bob \\"B\\"\\n"@en )>>';
  assert.equal(termToNTriples(triple), inner);
  assert.equal(termToNTriples(nested), `<<( <http://ex/alice> <http://ex/knows> ${inner} )>>`);
});

test("termToNTriples rejects an unknown term kind instead of emitting undefined", () => {
  assert.throws(() => termToNTriples({ type: "quoted", value: "x" }), /unsupported/);
});

test("isTripleTerm / termValue / formatTerm handle triple terms", () => {
  assert.equal(isTripleTerm(triple), true);
  assert.equal(isTripleTerm(alice), false);
  assert.equal(termValue(alice), "http://ex/alice");
  assert.equal(termValue(undefined), undefined);
  assert.equal(termValue(triple), termToNTriples(triple));
  assert.equal(formatTerm(triple), '<<( <http://ex/alice> <http://ex/knows> "Bob "B"\n"@en )>>');
});

test("CSV export renders a triple term as its N-Triples form, not [object Object]", () => {
  const csv = resultsToCsv({
    head: { vars: ["t"] },
    results: { bindings: [{ t: { type: "triple", value: { subject: alice, predicate: knows, object: alice } } }] },
  });
  assert.equal(csv, "t\r\n<<( <http://ex/alice> <http://ex/knows> <http://ex/alice> )>>");
});
