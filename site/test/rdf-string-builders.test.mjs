// #6719 — the RDF / SPARQL text builders outside the shared N-Triples term writer: the SPARQL
// PREFIX helpers, the pretty-Turtle formatter's prefix header, the Solid FROM NAMED rewrite and
// the SHACL Compact Syntax serializer. Each test feeds a value that the old builder interpolated
// unchecked and asserts on what the output means: the Turtle re-parses (with n3) to exactly the
// input triples, or the crafted value is rejected instead of adding clauses or shapes.
// Run via `npm run test:unit`.
import { test } from "node:test";
import assert from "node:assert/strict";

import N3 from "n3";
import { parse as parseShaclc } from "shaclc-parse";

import {
  declaredPrefixBindings,
  renderPrefixLines,
  withPrefixes,
} from "../../packages/sparq-client/src/sparql-prefixes.ts";
import { prettyTrig, prettyTurtle } from "../../packages/sparq-client/src/pretty-turtle.ts";
import { rewriteForGraphs } from "../src/lib/solid-acl.ts";
import { shapesToCompact } from "../src/lib/shacl-compact.ts";

const SH = "http://www.w3.org/ns/shacl#";
const RDF_TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/** Parse Turtle / TriG with n3 and return the quads as a sorted list of N-Quads lines. */
function quadsOf(text, format) {
  const writer = new N3.Writer({ format: "N-Quads" });
  const quads = new N3.Parser({ format }).parse(text);
  return quads.map((q) => writer.quadToString(q.subject, q.predicate, q.object, q.graph)).sort();
}

// ---------------------------------------------------------------------------
// SPARQL PREFIX helpers
// ---------------------------------------------------------------------------

test("renderPrefixLines rejects an IRI that would close its <…> and add a clause", () => {
  const bindings = [{ prefix: "ex", iri: "http://example.org/> SELECT * WHERE { ?s ?p ?o } #" }];
  assert.throws(() => renderPrefixLines(bindings), /not a writable PREFIX declaration/);
  assert.throws(() => withPrefixes("SELECT * { ex:a ?p ?o }", bindings));
});

test("renderPrefixLines rejects a label that is not a PN_PREFIX", () => {
  for (const prefix of ["ex: <http://evil/>\nSELECT", "ex.", "1ex", "e x"]) {
    assert.throws(
      () => renderPrefixLines([{ prefix, iri: "http://example.org/" }]),
      /not a writable PREFIX declaration/,
      `label ${JSON.stringify(prefix)}`,
    );
  }
  // The empty prefix and a dotted label are PN_PREFIX and still render.
  assert.equal(
    renderPrefixLines([
      { prefix: "", iri: "http://base/" },
      { prefix: "a.b", iri: "http://ab/" },
    ]),
    "PREFIX : <http://base/>\nPREFIX a.b: <http://ab/>",
  );
});

test("declaredPrefixBindings skips declarations that cannot be written back", () => {
  const q = [
    "PREFIX bad: <http://example.org/a\nb>",
    "PREFIX bad: <http://example.org/>",
    "PREFIX ex.: <http://dot.example/>",
    "PREFIX ok: <http://ok.example/>",
    "SELECT * {}",
  ].join("\n");
  assert.deepEqual(declaredPrefixBindings(q), [
    { prefix: "bad", iri: "http://example.org/" },
    { prefix: "ok", iri: "http://ok.example/" },
  ]);
});

// ---------------------------------------------------------------------------
// prettyTurtle / prettyTrig
// ---------------------------------------------------------------------------

test("prettyTurtle never declares a prefix label that is not a PN_PREFIX", () => {
  const nt = "<http://example.org/s> <http://example.org/p> <http://example.org/o> .";
  const pretty = prettyTurtle(nt, { prefixes: [{ prefix: "ex.", iri: "http://example.org/" }] });
  assert.deepEqual(quadsOf(pretty, "text/turtle"), quadsOf(nt, "N-Triples"));
});

test("prettyTurtle only abbreviates a local name with a complete %XX escape", () => {
  const nt = [
    "<http://example.org/a%zz> <http://example.org/p> <http://example.org/b%2F> .",
    "<http://example.org/c%2> <http://example.org/p> <http://example.org/d-> .",
  ].join("\n");
  const pretty = prettyTurtle(nt);
  assert.deepEqual(quadsOf(pretty, "text/turtle"), quadsOf(nt, "N-Triples"));
  assert.match(pretty, /ex:b%2F/);
  assert.match(pretty, /<http:\/\/example\.org\/a%zz>/);
});

test("prettyTrig keeps a crafted query prefix out of the header", () => {
  const nq = "<http://example.org/s> <http://example.org/p> <http://example.org/o> <http://example.org/g> .";
  const prefixes = declaredPrefixBindings("PREFIX ex.: <http://example.org/>\nSELECT * {}");
  const pretty = prettyTrig(nq, { prefixes: [...prefixes, { prefix: "g g", iri: "http://example.org/" }] });
  assert.deepEqual(quadsOf(pretty, "application/trig"), quadsOf(nq, "N-Quads"));
});

// ---------------------------------------------------------------------------
// Solid FROM NAMED rewrite
// ---------------------------------------------------------------------------

test("rewriteForGraphs rejects a graph IRI that would add a second FROM NAMED", () => {
  const q = "SELECT ?s WHERE { GRAPH ?g { ?s ?p ?o } }";
  assert.throws(
    () => rewriteForGraphs(q, ["https://pod.example/a> FROM NAMED <https://pod.example/secret"]),
    /N-Triples IRI cannot hold/,
  );
  assert.equal(
    rewriteForGraphs(q, ["https://pod.example/a"]),
    "SELECT ?s FROM NAMED <https://pod.example/a>\nWHERE { GRAPH ?g { ?s ?p ?o } }",
  );
});

// ---------------------------------------------------------------------------
// SHACL Compact Syntax
// ---------------------------------------------------------------------------

const nn = (value) => ({ termType: "NamedNode", value });
const lit = (value, datatype, language) => ({ termType: "Literal", value, datatype, language });
const bn = (value) => ({ termType: "BlankNode", value });

/** A one-property node shape `ex:S` over `ex:p`, with extra property-shape triples. */
function shapeWith(extra) {
  const S = nn("http://example.org/S");
  const P = bn("p0");
  return [
    { subject: S, predicate: nn(RDF_TYPE), object: nn(`${SH}NodeShape`) },
    { subject: S, predicate: nn(`${SH}property`), object: P },
    { subject: P, predicate: nn(`${SH}path`), object: nn("http://example.org/p") },
    ...extra.map(([p, o]) => ({ subject: P, predicate: nn(`${SH}${p}`), object: o })),
  ];
}

const shapeCount = (text) => (text.match(/^shape /gm) ?? []).length;

test("shapesToCompact rejects an IRI that would close its <…> and open a new shape", () => {
  const triples = shapeWith([["class", nn("http://example.org/C> .\n}\nshape <http://evil/S> {")]]);
  assert.throws(() => shapesToCompact(triples), /N-Triples IRI cannot hold/);
});

test("shapesToCompact rejects a blank-node label that is not a BLANK_NODE_LABEL", () => {
  const triples = shapeWith([["hasValue", bn("b0 .\n}\nshape <http://evil/S> {\n\tx")]]);
  assert.throws(() => shapesToCompact(triples), /not a BLANK_NODE_LABEL/);
});

test("shapesToCompact escapes quotes, backslashes and line breaks in a string literal", () => {
  const message = 'line1\nline2\r\t\b\f"\\\u0001end';
  const { text } = shapesToCompact(shapeWith([["message", lit(message)]]));
  assert.equal(shapeCount(text), 1);
  const written = /message=("(?:[^"\\]|\\.)*")/.exec(text)?.[1];
  assert.ok(written, text);
  assert.doesNotMatch(written, /[\n\r]/);
  // The written form is a Turtle string literal that decodes back to the original message.
  const [q] = new N3.Parser().parse(`<urn:s> <urn:p> ${written} .`);
  assert.equal(q.object.value, message);
});

test("shapesToCompact rejects a language tag that is not a LANGTAG", () => {
  const triples = shapeWith([["message", lit("hi", undefined, "en .\n}\nshape <http://evil/S> {")]]);
  assert.throws(() => shapesToCompact(triples), /not a LANGTAG/);
});

test("shapesToCompact omits a property whose count is not a non-negative integer", () => {
  const triples = shapeWith([
    ["minCount", lit("1..1] .\n}\nshape <http://evil/S> {\n\tex:q [0", `http://www.w3.org/2001/XMLSchema#integer`)],
  ]);
  const { text, unsupported } = shapesToCompact(triples);
  assert.equal(shapeCount(text), 1);
  assert.doesNotMatch(text, /evil/);
  assert.equal(unsupported.length, 1);
  assert.match(unsupported[0], /not a non-negative integer/);
});

test("shapesToCompact keeps a signed count whose value is non-negative", () => {
  const XSD_INT = "http://www.w3.org/2001/XMLSchema#integer";
  for (const [min, max] of [["+1", "-0"], ["0", "+00"]]) {
    const { text, unsupported } = shapesToCompact(
      shapeWith([["minCount", lit(min, XSD_INT)], ["maxCount", lit(max, XSD_INT)]]),
    );
    assert.deepEqual(unsupported, []);
    assert.ok(text.includes(`[${min}..${max}]`), text);
  }
  const { unsupported } = shapesToCompact(shapeWith([["minCount", lit("-1", XSD_INT)]]));
  assert.equal(unsupported.length, 1);
});

test("shapesToCompact output parses as a whole SHACLC document", () => {
  const XSD = "http://www.w3.org/2001/XMLSchema#";
  const triples = shapeWith([
    ["minCount", lit("+1", `${XSD}integer`)],
    ["maxCount", lit("-0", `${XSD}integer`)],
    ["datatype", nn(`${XSD}string`)],
    ["message", lit("raw \u0001 control")],
    ["class", nn("http://example.org/C.")],
  ]);
  const { text, unsupported } = shapesToCompact(triples, [
    { prefix: "ex", iri: "http://example.org/" },
    { prefix: "ex.", iri: "http://dot.example/" },
  ]);
  assert.deepEqual(unsupported, []);
  const quads = parseShaclc(text, { extendedSyntax: true });
  const has = (p, test) => quads.some((q) => q.predicate.value === `${SH}${p}` && test(q.object));
  assert.ok(has("path", (o) => o.value === "http://example.org/p"));
  assert.ok(has("minCount", (o) => o.value === "+1"));
  assert.ok(has("maxCount", (o) => o.value === "-0"));
  assert.ok(has("datatype", (o) => o.value === `${XSD}string`));
  assert.ok(has("message", (o) => o.value === "raw \u0001 control"));
  assert.ok(has("class", (o) => o.value === "http://example.org/C."));
  assert.ok(quads.some((q) => q.subject.value === "http://example.org/S" && q.object.value === `${SH}NodeShape`));
});

test("shapesToCompact drops caller prefixes that cannot be declared and keeps full IRIs", () => {
  const triples = shapeWith([["class", nn("http://example.org/C")]]);
  const { text } = shapesToCompact(triples, [
    { prefix: "ex.", iri: "http://example.org/" },
    { prefix: "ev", iri: "http://example.org/\nshape <http://evil/S> {" },
  ]);
  assert.equal(shapeCount(text), 1);
  assert.doesNotMatch(text, /^PREFIX (ex\.|ev):/m);
  assert.match(text, /<http:\/\/example\.org\/S>/);
  assert.match(text, /class=<http:\/\/example\.org\/C>/);
});

test("shapesToCompact writes a local name ending in '.' as a full IRI", () => {
  const triples = shapeWith([["class", nn("http://example.org/C.")]]);
  const { text } = shapesToCompact(triples, [{ prefix: "ex", iri: "http://example.org/" }]);
  assert.match(text, /class=<http:\/\/example\.org\/C\.>/);
  assert.match(text, /^shape ex:S /m);
});
