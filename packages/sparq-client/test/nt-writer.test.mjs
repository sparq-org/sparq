// termToNTriples is the shared N-Triples / N-Quads term writer behind the GUI and site
// snapshot writers. A term part with no escape form (IRI, blank-node label, language tag,
// direction, datatype IRI) must be validated: if a crafted value were spliced in verbatim,
// a `>`, space, newline or `"` could end the token and inject extra statements.
import assert from "node:assert/strict";
import { test } from "node:test";

import { countQuads, matchQuads, parseNTriples, termToNTriples } from "../src/index.ts";

const iri = (value) => ({ type: "uri", value });
const bnode = (value) => ({ type: "bnode", value });
const lit = (value, extra = {}) => ({ type: "literal", value, ...extra });
const S = iri("http://ex/s");
const P = iri("http://ex/p");

/** Writes one statement the way the snapshot writers do. */
const line = (o) => `${termToNTriples(S)} ${termToNTriples(P)} ${termToNTriples(o)} .`;

/** Decodes the N-Triples escapes the writer emits (all of them are also JSON escapes). */
const unescape = (lex) => JSON.parse(`"${lex}"`);

const PAYLOAD = '> <http://evil/p> <http://evil/o> .\n<http://evil/s';

test("an IRI that could end its token is rejected", () => {
  for (const value of [
    `http://ex/a${PAYLOAD}`,
    "http://ex/a b",
    "http://ex/a\nb",
    "http://ex/a\rb",
    "http://ex/a\tb",
    "http://ex/a\u0000b",
    'http://ex/a"b',
    "http://ex/a<b",
    "http://ex/a{b}",
    "http://ex/a|b",
    "http://ex/a^b",
    "http://ex/a`b",
    "http://ex/a\\u003Eb",
    "http://ex/a\uD800b",
  ]) {
    assert.throws(() => termToNTriples(iri(value)), /termToNTriples: IRI/, JSON.stringify(value));
    assert.throws(() => line(iri(value)), /IRI/);
  }
});

test("a datatype IRI follows the IRI rule", () => {
  assert.throws(
    () => termToNTriples(lit("1", { datatype: `http://ex/dt${PAYLOAD}` })),
    /datatype IRI/,
  );
  assert.throws(() => termToNTriples(lit("1", { datatype: "http://ex/d t" })), /datatype IRI/);
});

test("a blank-node label must be a BLANK_NODE_LABEL", () => {
  for (const value of [
    "",
    "b0 <http://evil/p> <http://evil/o> .\n_:x",
    "b 0",
    "b\n0",
    "b0.",
    ".b0",
    "-b0",
    "b>0",
    'b"0',
    "b0)",
  ]) {
    assert.throws(
      () => termToNTriples(bnode(value)),
      /BLANK_NODE_LABEL/,
      JSON.stringify(value),
    );
  }
});

test("a language tag must be a LANGTAG and the direction ltr/rtl", () => {
  for (const lang of [
    "en <http://evil/p> <http://evil/o>",
    "en .\n<http://evil/s> <http://evil/p> <http://evil/o>",
    "en-",
    "-en",
    "en--ltr",
    "e n",
    'en"',
    "1en",
  ]) {
    assert.throws(() => termToNTriples(lit("x", { "xml:lang": lang })), /LANGTAG/, lang);
  }
  for (const dir of ["LTR", "up", "ltr .\n<http://evil/s> <http://evil/p> <http://evil/o>"]) {
    assert.throws(
      () => termToNTriples(lit("x", { "xml:lang": "en", "its:dir": dir })),
      /base direction/,
      dir,
    );
  }
});

test("invalid parts nested in a triple term are rejected too", () => {
  const tt = (o) => ({ type: "triple", value: { subject: S, predicate: P, object: o } });
  assert.throws(() => termToNTriples(tt(iri(`http://ex/a${PAYLOAD}`))), /IRI/);
  assert.throws(() => termToNTriples(tt(tt(bnode("a )>> <http://evil/o>")))), /BLANK_NODE_LABEL/);
});

test("literal lexical forms use the canonical ECHAR / UCHAR escapes", () => {
  assert.equal(
    termToNTriples(lit('a"b\\c\nd\re\tf\bg\fh')),
    '"a\\"b\\\\c\\nd\\re\\tf\\bg\\fh"',
  );
  assert.equal(termToNTriples(lit("\u0000\u0001\u001F\u007F")), '"\\u0000\\u0001\\u001F\\u007F"');
  // Non-control characters, including non-BMP ones, are written as-is.
  assert.equal(termToNTriples(lit("é 😀 \u0080")), '"é 😀 \u0080"');
  assert.throws(() => termToNTriples(lit("a\uDC00")), /lone surrogate/);
});

test("a literal carrying an injection payload stays one statement", () => {
  const value = `x" .\n<http://evil/s> <http://evil/p> "y`;
  const { statements, passthrough } = parseNTriples(line(lit(value)));
  assert.equal(statements.length, 1);
  assert.deepEqual(passthrough, []);
  assert.equal(unescape(statements[0].o.value), value);
});

test("valid terms round-trip through a snapshot line", () => {
  const cases = [
    iri("http://ex/a"),
    iri("urn:x:é#frag?q=1&r=%20"),
    iri("http://ex/😀"),
    bnode("b0"),
    bnode("a.b"),
    bnode("a..b"),
    bnode("_x-1"),
    bnode("0abc"),
    bnode("a:b"),
    bnode("é·̀"),
    lit("plain"),
    lit("s", { datatype: "http://www.w3.org/2001/XMLSchema#string" }),
    lit("5", { datatype: "http://www.w3.org/2001/XMLSchema#integer" }),
    lit("hi", { "xml:lang": "en" }),
    lit("hi", { "xml:lang": "en-GB-oed" }),
    lit("hi", { "xml:lang": "x-a1-b2" }),
    lit("hi", { "xml:lang": "ar", "its:dir": "rtl" }),
    lit("hi", { "xml:lang": "en", "its:dir": "ltr" }),
    lit("tab\there\u0001"),
  ];
  for (const o of cases) {
    const { statements, passthrough } = parseNTriples(line(o));
    assert.deepEqual(passthrough, [], JSON.stringify(o));
    assert.equal(statements.length, 1, JSON.stringify(o));
    const back = statements[0].o;
    if (o.type === "uri") {
      assert.equal(back.kind, "iri");
      assert.equal(back.value, o.value);
    } else if (o.type === "bnode") {
      assert.equal(back.kind, "bnode");
      assert.equal(back.label, o.value);
    } else {
      assert.equal(back.kind, "literal");
      assert.equal(unescape(back.value), o.value);
      const lang = o["xml:lang"] && (o["its:dir"] ? `${o["xml:lang"]}--${o["its:dir"]}` : o["xml:lang"]);
      assert.equal(back.lang, lang);
      const dt = o.datatype === "http://www.w3.org/2001/XMLSchema#string" ? undefined : o.datatype;
      assert.equal(back.datatype, dt);
    }
  }
  // A quad (the N-Quads graph slot) goes through the same writer.
  const quad = `${line(bnode("b0")).slice(0, -2)} ${termToNTriples(iri("http://ex/g"))} .`;
  assert.equal(parseNTriples(quad).statements[0].g.value, "http://ex/g");
});

// matchQuads / countQuads inline caller-supplied term strings into a generated SELECT, so
// each one must be exactly one N-Triples term token.
test("matchQuads / countQuads accept only a single N-Triples term per position", () => {
  const seen = [];
  const store = {
    query: (q) => {
      seen.push(q);
      return JSON.stringify({ head: { vars: [] }, results: { bindings: [] } });
    },
    count: (q) => {
      seen.push(q);
      return 0;
    },
  };
  for (const ok of [
    "<http://ex/a>",
    " <http://ex/a> ",
    "_:b0",
    '"x"',
    '"a\\"b\\n"@en-GB--rtl',
    '"5"^^<http://ex/dt>',
    '"} ; DROP ALL ; #"',
  ]) {
    matchQuads(store, ok, null, null, null);
    countQuads(store, null, null, ok, "<http://ex/g>");
  }
  assert.equal(seen.length, 14);
  assert.match(seen[1], /^SELECT \* WHERE \{ GRAPH <http:\/\/ex\/g> \{ \?s \?p <http:\/\/ex\/a> \} \}$/);
  for (const bad of [
    "<http://ex/a> . ?x ?y ?z",
    "<http://ex/a> } UNION { ?s ?p ?o",
    "<a\\u003E>",
    '"x\\u0022 } UNION { ?s ?p ?o"',
    "?x",
    '"x"@en ?y',
    '"x"@en--up',
    "<a> <b>",
    '"x"^^<a b>',
    "_:a b",
    '"a\nb"',
    '"a\uD800"',
  ]) {
    assert.throws(() => countQuads(store, null, bad, null), /single N-Triples/, bad);
    assert.throws(() => matchQuads(store, null, null, bad), /single N-Triples/, bad);
    assert.throws(() => matchQuads(store, null, null, null, bad), /single N-Triples/, bad);
  }
  assert.equal(seen.length, 14);
});
