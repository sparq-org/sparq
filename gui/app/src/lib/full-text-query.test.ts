// Unit tests for the Full-text tool's SPARQL builder (#6719).
//
// Run via:   npm run test:unit   (gui/app)
import { test } from "node:test";
import assert from "node:assert/strict";

import { buildSearchQuery } from "./full-text-query.js";

const ECHAR: Record<string, string> = { t: "\t", b: "\b", n: "\n", r: "\r", f: "\f", '"': '"', "'": "'", "\\": "\\" };

/** Re-parses the SPARQL `STRING_LITERAL2` starting at `open` (a `"`): returns its value and the
 *  index after the closing quote. Throws on a raw `"`-less end, raw CR/LF or a bad escape. */
function parseStringLiteral2(text: string, open: number): { value: string; end: number } {
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
      if (!/^[0-9A-Fa-f]+$/.test(hex) || hex.length !== n) throw new Error(`bad UCHAR at ${i}`);
      value += String.fromCodePoint(parseInt(hex, 16));
      i += 2 + n;
    } else {
      throw new Error(`bad escape \\${e} at ${i}`);
    }
  }
  throw new Error("unterminated literal");
}

const TERMS = [
  "plain words",
  'say "hi"',
  "back\\slash and \\n literally",
  "line one\nline two",
  "carriage\rreturn and \r\n CRLF",
  "angle > bracket <tag>",
  "tab\there, form\ffeed, bell\u0007, del\u007f",
  '"} ; DROP ALL ; SELECT * { ?s ?p "',
  "ünïcödé 日本 😀",
  "",
];

test("buildSearchQuery: the search term re-parses to the same value", () => {
  for (const term of TERMS) {
    const q = buildSearchQuery(term);
    const at = q.indexOf("text:matches ") + "text:matches ".length;
    const { value, end } = parseStringLiteral2(q, at);
    assert.equal(value, term, JSON.stringify(term));
    assert.equal(q.slice(end), " ; text:score ?score .\n} ORDER BY DESC(?score) LIMIT 50");
    // The query keeps its fixed shape: the term adds no lines of its own.
    assert.equal(q.split("\n").length, 5, JSON.stringify(term));
  }
});

test("buildSearchQuery: a lone surrogate is refused, not written", () => {
  assert.throws(() => buildSearchQuery("bad \uD800 half"), /lone surrogate/);
});
