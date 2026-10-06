// #6044: SPARQL 1.2 Query Results JSON carries an RDF 1.2 triple term as
// {"type":"triple","value":{"subject":…,"predicate":…,"object":…}} (the exact shape
// crates/sparq-engine/src/json.rs emits). The shared term helpers must handle it rather than
// treating `value` as a string.
import assert from "node:assert/strict";
import { test } from "node:test";

import {
  formatTerm,
  isTripleTerm,
  parseNTriples,
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

// RDF 1.2 base direction: SPARQL 1.2 results carry it as a separate `its:dir` field next to
// the bare `xml:lang` tag. `"hi"@en--ltr` and `"hi"@en` are different RDF terms, so every
// writer must keep the direction, at the top level and inside a triple term.
const hiLtr = { type: "literal", value: "hi", "xml:lang": "en", "its:dir": "ltr" };
const hiRtl = { type: "literal", value: "hi", "xml:lang": "en", "its:dir": "rtl" };
const hiPlain = { type: "literal", value: "hi", "xml:lang": "en" };

test("termToNTriples keeps a top-level literal's base direction", () => {
  assert.equal(termToNTriples(hiLtr), '"hi"@en--ltr');
  assert.equal(termToNTriples(hiRtl), '"hi"@en--rtl');
  assert.equal(termToNTriples(hiPlain), '"hi"@en');
});

test("termToNTriples keeps base direction inside a (nested) triple term", () => {
  const tt = (o) => ({ type: "triple", value: { subject: alice, predicate: knows, object: o } });
  assert.equal(
    termToNTriples(tt(hiLtr)),
    '<<( <http://ex/alice> <http://ex/knows> "hi"@en--ltr )>>',
  );
  assert.equal(
    termToNTriples(tt(tt(hiRtl))),
    '<<( <http://ex/alice> <http://ex/knows> <<( <http://ex/alice> <http://ex/knows> "hi"@en--rtl )>> )>>',
  );
  // Terms differing only by direction must not collapse.
  const forms = new Set([hiLtr, hiRtl, hiPlain].map((o) => termToNTriples(tt(o))));
  assert.equal(forms.size, 3);
});

test("formatTerm keeps base direction, top-level and nested", () => {
  assert.equal(formatTerm(hiLtr), '"hi"@en--ltr');
  assert.equal(formatTerm(hiPlain), '"hi"@en');
  assert.equal(
    formatTerm({ type: "triple", value: { subject: alice, predicate: knows, object: hiRtl } }),
    '<<( <http://ex/alice> <http://ex/knows> "hi"@en--rtl )>>',
  );
});

test("a directional literal nested in a triple term survives a snapshot round trip", () => {
  const o = { type: "triple", value: { subject: alice, predicate: knows, object: hiLtr } };
  const line = `${termToNTriples(alice)} ${termToNTriples(knows)} ${termToNTriples(o)} .`;
  const { statements } = parseNTriples(line);
  assert.equal(statements.length, 1);
  const back = statements[0].o;
  assert.equal(back.kind, "triple");
  assert.equal(back.o.kind, "literal");
  assert.equal(back.o.value, "hi");
  assert.equal(back.o.lang, "en--ltr");
});
