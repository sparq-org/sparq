// Unit tests for the SPARQL form classifier and the Graph view tool's pre-run gate (#5783).
//
// Run via:   npm run test:unit   (gui/app)
import { test } from "node:test";
import assert from "node:assert/strict";

import { classifyQuery, graphViewRefusal } from "./query-form.js";

test("classifyQuery: forms, past PREFIX/BASE and comments", () => {
  assert.equal(classifyQuery("CONSTRUCT WHERE { ?s ?p ?o }"), "construct");
  assert.equal(classifyQuery("describe <http://ex/a>"), "describe");
  assert.equal(classifyQuery("ASK { ?s ?p ?o }"), "ask");
  assert.equal(classifyQuery("SELECT * WHERE { ?s ?p ?o }"), "select");
  assert.equal(
    classifyQuery(
      "# a comment with CONSTRUCT\nPREFIX ex: <http://ex/>\nINSERT DATA { ex:a ex:b ex:c }",
    ),
    "update",
  );
  assert.equal(classifyQuery("BASE <http://ex/> DELETE WHERE { ?s ?p ?o }"), "update");
  assert.equal(classifyQuery("CLEAR ALL"), "update");
});

test("graphViewRefusal: CONSTRUCT and DESCRIBE may run", () => {
  assert.equal(graphViewRefusal("CONSTRUCT WHERE { ?s ?p ?o } LIMIT 100"), null);
  assert.equal(graphViewRefusal("DESCRIBE <http://ex/a>"), null);
});

test("graphViewRefusal: an UPDATE is refused before it can run (#5783)", () => {
  for (const q of [
    "INSERT DATA { <http://ex/a> <http://ex/b> <http://ex/c> }",
    "PREFIX ex: <http://ex/>\nDELETE WHERE { ?s ?p ?o }",
    "DROP ALL",
  ]) {
    const why = graphViewRefusal(q);
    assert.ok(why, `expected a refusal for ${q}`);
    assert.match(why, /UPDATE/);
    assert.match(why, /not changed/);
  }
});

test("graphViewRefusal: SELECT and ASK are refused as non-graph forms", () => {
  assert.match(graphViewRefusal("SELECT * WHERE { ?s ?p ?o }") ?? "", /SELECT query/);
  assert.match(graphViewRefusal("ASK { ?s ?p ?o }") ?? "", /ASK query/);
});
