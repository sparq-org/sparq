// [OPUS-4.8] sq-egy6 — unit tests for the pure SHACL-report rendering helpers that
// back the live /surface/shacl playground. These cover the framework-free CURIE
// shortening, component/severity local names, conformance summary, and the W3C
// Turtle serialisation. The wasm `Store.validate` binding itself is proven by the
// Rust tests in crates/sparq-wasm/src/shacl.rs; here we only test the JS rendering
// of the JSON report it returns. Run via `npm run test:unit`.
import { test } from "node:test";
import assert from "node:assert/strict";

import {
  shortenIri,
  componentName,
  severityName,
  reportSummary,
  reportToTurtle,
} from "../src/lib/shacl-report.ts";

// The `ex:age "thirty"` datatype violation, as the wasm validate binding returns it.
const DATATYPE_VIOLATION = {
  conforms: false,
  results: [
    {
      focusNode: "<http://example.org/alice>",
      path: "<http://example.org/age>",
      value: '"thirty"',
      sourceShape: "_:b0",
      sourceConstraintComponent:
        "http://www.w3.org/ns/shacl#DatatypeConstraintComponent",
      severity: "http://www.w3.org/ns/shacl#Violation",
      message: "ex:age must be exactly one xsd:integer",
    },
  ],
};

const CONFORMS = { conforms: true, results: [] };

test("shortenIri compacts the well-known namespaces and leaves the rest alone", () => {
  assert.equal(shortenIri("<http://example.org/alice>"), "ex:alice");
  assert.equal(shortenIri("<http://example.org/age>"), "ex:age");
  assert.equal(
    shortenIri("<http://www.w3.org/2001/XMLSchema#integer>"),
    "xsd:integer",
  );
  assert.equal(
    shortenIri("<http://www.w3.org/ns/shacl#Violation>"),
    "sh:Violation",
  );
  // Literal term strings and blank nodes pass through untouched.
  assert.equal(shortenIri('"thirty"'), '"thirty"');
  assert.equal(shortenIri("_:b0"), "_:b0");
  // An IRI outside the known prefixes is returned unchanged.
  assert.equal(shortenIri("<http://other.test/x>"), "<http://other.test/x>");
});

test("componentName / severityName take the local part after the hash", () => {
  assert.equal(
    componentName("http://www.w3.org/ns/shacl#DatatypeConstraintComponent"),
    "DatatypeConstraintComponent",
  );
  assert.equal(severityName("http://www.w3.org/ns/shacl#Violation"), "Violation");
});

test("reportSummary reflects conformance and violation count", () => {
  assert.equal(reportSummary(CONFORMS), "Conforms — no violations.");
  assert.equal(
    reportSummary(DATATYPE_VIOLATION),
    "Does not conform — 1 violation.",
  );
  assert.equal(
    reportSummary({ conforms: false, results: [{}, {}] }),
    "Does not conform — 2 violations.",
  );
});

test("reportToTurtle emits a conforming sh:ValidationReport with no results", () => {
  const ttl = reportToTurtle(CONFORMS);
  assert.match(ttl, /a sh:ValidationReport/);
  assert.match(ttl, /sh:conforms true \./);
  assert.ok(!ttl.includes("sh:result"), "no sh:result for a conforming report");
});

test("reportToTurtle emits the per-violation W3C vocabulary", () => {
  const ttl = reportToTurtle(DATATYPE_VIOLATION);
  assert.match(ttl, /sh:conforms false ;/);
  assert.match(ttl, /sh:result \[/);
  assert.match(ttl, /a sh:ValidationResult ;/);
  assert.match(ttl, /sh:focusNode <http:\/\/example\.org\/alice> ;/);
  assert.match(ttl, /sh:resultPath <http:\/\/example\.org\/age> ;/);
  assert.match(ttl, /sh:value "thirty" ;/);
  assert.match(
    ttl,
    /sh:sourceConstraintComponent <http:\/\/www\.w3\.org\/ns\/shacl#DatatypeConstraintComponent> ;/,
  );
  assert.match(
    ttl,
    /sh:resultMessage "ex:age must be exactly one xsd:integer"/,
  );
  // The blank-node and the report both terminate correctly.
  assert.match(ttl, /\] \./);
});

// #6719: message text and IRIs go through the shared sparq-client writer.
const ECHAR = { t: "\t", b: "\b", n: "\n", r: "\r", f: "\f", '"': '"', "'": "'", "\\": "\\" };

/** Re-parses the Turtle `STRING_LITERAL_QUOTE` starting at `open` (a `"`): its value and the
 *  index after the closing quote. Throws on raw CR/LF, a bad escape or no closing quote. */
function parseStringLiteralQuote(text, open) {
  assert.equal(text[open], '"', "literal starts with a double quote");
  let value = "";
  let i = open + 1;
  while (i < text.length) {
    const c = text[i];
    if (c === '"') return { value, end: i + 1 };
    if (c === "\n" || c === "\r") throw new Error(`raw line break at ${i}`);
    if (c !== "\\") {
      value += c;
      i += 1;
      continue;
    }
    const e = text[i + 1];
    if (e in ECHAR) {
      value += ECHAR[e];
      i += 2;
    } else if (e === "u" || e === "U") {
      const n = e === "u" ? 4 : 8;
      const hex = text.slice(i + 2, i + 2 + n);
      if (hex.length !== n || !/^[0-9A-Fa-f]+$/.test(hex)) throw new Error(`bad UCHAR at ${i}`);
      value += String.fromCodePoint(parseInt(hex, 16));
      i += 2 + n;
    } else {
      throw new Error(`bad escape \\${e} at ${i}`);
    }
  }
  throw new Error("unterminated literal");
}

const MESSAGES = [
  'a "quoted" value',
  "back\\slash",
  "line one\nline two",
  "CR\ronly and CRLF\r\nend",
  "angle > bracket <x>",
  'tab\t, bell\u0007, del\u007f and "] . <urn:x> <urn:y> <urn:z> .',
  "ünïcödé 😀",
];

test("reportToTurtle: a resultMessage re-parses to the same value", () => {
  for (const message of MESSAGES) {
    const result = { ...DATATYPE_VIOLATION.results[0], message };
    const ttl = reportToTurtle({ conforms: false, results: [result] });
    const at = ttl.indexOf("sh:resultMessage ") + "sh:resultMessage ".length;
    const { value, end } = parseStringLiteralQuote(ttl, at);
    assert.equal(value, message, JSON.stringify(message));
    assert.equal(ttl.slice(end), "\n  ] .\n");
    // The message adds no lines: one per property plus the report header and brackets.
    const clean = reportToTurtle({ conforms: false, results: [{ ...result, message: "m" }] });
    assert.equal(ttl.split("\n").length, clean.split("\n").length, JSON.stringify(message));
  }
});

test("reportToTurtle: an IRI that cannot be written as-is is refused", () => {
  const base = DATATYPE_VIOLATION.results[0];
  for (const bad of [
    { severity: "http://www.w3.org/ns/shacl#Violation> . <urn:x> <urn:y> <urn:z" },
    { sourceConstraintComponent: "http://x/a\nb" },
  ]) {
    assert.throws(
      () => reportToTurtle({ conforms: false, results: [{ ...base, ...bad }] }),
      /N-Triples IRI cannot hold/,
    );
  }
});
