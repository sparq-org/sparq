// [FABLE-5] sq-ohnj1 — zero-dependency RDF/JS glue for the eye-js compat layer.
//
// sparq's reasoner speaks canonical N-Triples strings across the wasm boundary; eye-js's
// `n3reasoner` resolves its Quad[] overloads to RDF/JS `Quad`s. This module bridges the two
// with NO runtime dependency:
//   - `writeQuads`  : RDF/JS Quad[] -> an N-Triples string the reasoner accepts as input.
//   - `parseNTriples`: the reasoner's canonical N-Triples output -> RDF/JS Quad[].
//   - a minimal RDF/JS `DataFactory` (the term/quad shapes `@rdfjs/types` specifies).
//
// The parser targets sparq's canonical N-Triples (oxrdf `Display`): one `s p o .` per line,
// IRIs `<…>`, blank nodes `_:label`, and literals `"lex"` / `"lex"@lang` / `"lex"^^<dt>` with
// the standard escape set. It is intentionally not a general Turtle parser.

import type {
  BlankNode,
  DefaultGraph,
  Literal,
  NamedNode,
  Quad,
  Quad_Object,
  Quad_Subject,
  Term,
} from '@rdfjs/types';

const XSD_STRING = 'http://www.w3.org/2001/XMLSchema#string';
const RDF_LANG_STRING = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#langString';

function eq(a: Term | null | undefined, b: Term | null | undefined): boolean {
  return !!a && !!b && a.termType === b.termType && a.value === b.value
    && (a.termType !== 'Literal' || (
      (a as Literal).language === (b as Literal).language
      && (a as Literal).datatype.value === (b as Literal).datatype.value));
}

function namedNode(value: string): NamedNode {
  return { termType: 'NamedNode', value, equals: (o) => eq({ termType: 'NamedNode', value } as NamedNode, o) };
}
function blankNode(value: string): BlankNode {
  return { termType: 'BlankNode', value, equals: (o) => eq({ termType: 'BlankNode', value } as BlankNode, o) };
}
function defaultGraph(): DefaultGraph {
  return { termType: 'DefaultGraph', value: '', equals: (o) => !!o && o.termType === 'DefaultGraph' };
}
function literal(value: string, languageOrDatatype?: string | NamedNode): Literal {
  let language = '';
  let datatype: NamedNode;
  if (typeof languageOrDatatype === 'string') {
    language = languageOrDatatype;
    datatype = namedNode(RDF_LANG_STRING);
  } else if (languageOrDatatype) {
    datatype = languageOrDatatype;
  } else {
    datatype = namedNode(XSD_STRING);
  }
  const self = { termType: 'Literal', value, language, datatype } as Literal;
  (self as { equals: Literal['equals'] }).equals = (o) => eq(self, o);
  return self;
}
function quad(subject: Quad_Subject, predicate: NamedNode, object: Quad_Object, graph?: DefaultGraph): Quad {
  const g = graph ?? defaultGraph();
  const self = { termType: 'Quad', value: '', subject, predicate, object, graph: g } as Quad;
  (self as { equals: Quad['equals'] }).equals = (o) => !!o && o.termType === 'Quad'
    && eq(subject, (o as Quad).subject) && eq(predicate, (o as Quad).predicate)
    && eq(object, (o as Quad).object) && eq(g, (o as Quad).graph);
  return self;
}

/** A minimal RDF/JS `DataFactory` (no external dependency). */
export const dataFactory = { namedNode, blankNode, literal, defaultGraph, quad };

// Term parts with no escape form are validated, never rewritten, so a crafted value cannot end
// its token early and inject statements into the reasoner input. The rules are the N-Triples
// IRIREF characters, BLANK_NODE_LABEL and LANGTAG productions.
const LONE_SURROGATE = /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/;
const IRI_FORBIDDEN = /[\u0000-\u0020<>"{}|^`\\]/;
const PN_CHARS_BASE =
  'A-Za-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF'
  + '\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD'
  + '\\u{10000}-\\u{EFFFF}';
const PN_CHARS = `${PN_CHARS_BASE}_:\\-0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040`;
const BLANK_NODE_LABEL = new RegExp(`^[${PN_CHARS_BASE}_:0-9](?:[${PN_CHARS}.]*[${PN_CHARS}])?$`, 'u');
const LANGTAG = /^[a-zA-Z]+(?:-[a-zA-Z0-9]+)*$/;
const ECHAR: Record<string, string> = {
  '\b': '\\b', '\t': '\\t', '\n': '\\n', '\f': '\\f', '\r': '\\r', '"': '\\"', '\\': '\\\\',
};

function writeIri(value: string, what: string): string {
  if (IRI_FORBIDDEN.test(value) || LONE_SURROGATE.test(value)) {
    throw new Error(`cannot serialise ${what} ${JSON.stringify(value)} as an N-Triples IRI`);
  }
  return `<${value}>`;
}

/** Serialise one RDF/JS term as an N-Triples token. */
function writeTerm(t: Term): string {
  switch (t.termType) {
    case 'NamedNode':
      return writeIri(t.value, 'IRI');
    case 'BlankNode':
      if (!BLANK_NODE_LABEL.test(t.value)) {
        throw new Error(`cannot serialise blank node label ${JSON.stringify(t.value)} to N-Triples`);
      }
      return `_:${t.value}`;
    case 'Literal': {
      if (LONE_SURROGATE.test(t.value)) {
        throw new Error(`cannot serialise literal ${JSON.stringify(t.value)}: lone surrogate`);
      }
      const lex = `"${t.value.replace(/[\u0000-\u001F\u007F"\\]/g, (c) => ECHAR[c] ?? `\\u${c.charCodeAt(0).toString(16).toUpperCase().padStart(4, '0')}`)}"`;
      const lang = (t as Literal).language;
      if (lang) {
        if (!LANGTAG.test(lang)) {
          throw new Error(`cannot serialise language tag ${JSON.stringify(lang)} to N-Triples`);
        }
        return `${lex}@${lang}`;
      }
      const dt = (t as Literal).datatype.value;
      if (dt === XSD_STRING) return lex;
      return `${lex}^^${writeIri(dt, 'datatype IRI')}`;
    }
    default:
      throw new Error(`cannot serialise term of type ${t.termType} to N-Triples`);
  }
}

/** Serialise RDF/JS quads as an N-Triples document (the reasoner input format). */
export function writeQuads(quads: Quad[]): string {
  return quads.map((q) => `${writeTerm(q.subject)} ${writeTerm(q.predicate)} ${writeTerm(q.object)} .`).join('\n');
}

// ---- N-Triples parsing (canonical output only) ----

function unescape(s: string): string {
  return s.replace(/\\(u[0-9A-Fa-f]{4}|U[0-9A-Fa-f]{8}|[tbnrf"'\\])/g, (_m, esc) => {
    if (esc[0] === 'u' || esc[0] === 'U') return String.fromCodePoint(parseInt(esc.slice(1), 16));
    return ({ t: '\t', b: '\b', n: '\n', r: '\r', f: '\f', '"': '"', "'": "'", '\\': '\\' } as Record<string, string>)[esc];
  });
}

/** Parse one N-Triples term at cursor `i`; returns the term and the next cursor. */
function readTerm(line: string, i: number): { term: Term; next: number } {
  while (i < line.length && line[i] === ' ') i += 1;
  if (line[i] === '<') {
    const end = line.indexOf('>', i);
    if (end < 0) throw new Error(`unterminated IRI in: ${line}`);
    return { term: namedNode(unescape(line.slice(i + 1, end))), next: end + 1 };
  }
  if (line[i] === '_' && line[i + 1] === ':') {
    let j = i + 2;
    while (j < line.length && !' \t'.includes(line[j])) j += 1;
    return { term: blankNode(line.slice(i + 2, j)), next: j };
  }
  if (line[i] === '"') {
    // Consume the quoted lexical form, honouring backslash escapes.
    let j = i + 1;
    let lex = '';
    while (j < line.length) {
      const c = line[j];
      if (c === '\\') { lex += line.slice(j, j + 2); j += 2; continue; }
      if (c === '"') break;
      lex += c;
      j += 1;
    }
    j += 1; // past closing quote
    if (line[j] === '@') {
      let k = j + 1;
      while (k < line.length && !' \t'.includes(line[k])) k += 1;
      return { term: literal(unescape(lex), line.slice(j + 1, k)), next: k };
    }
    if (line[j] === '^' && line[j + 1] === '^') {
      const dtStart = line.indexOf('<', j);
      const dtEnd = line.indexOf('>', dtStart);
      return { term: literal(unescape(lex), namedNode(unescape(line.slice(dtStart + 1, dtEnd)))), next: dtEnd + 1 };
    }
    return { term: literal(unescape(lex)), next: j };
  }
  throw new Error(`unexpected token at ${i} in: ${line}`);
}

/** Parse a canonical N-Triples document into RDF/JS quads (default graph). */
export function parseNTriples(nt: string): Quad[] {
  const out: Quad[] = [];
  for (const raw of nt.split('\n')) {
    const line = raw.trim();
    if (line.length === 0 || line.startsWith('#')) continue;
    const s = readTerm(line, 0);
    const p = readTerm(line, s.next);
    const o = readTerm(line, p.next);
    out.push(quad(s.term as Quad_Subject, p.term as NamedNode, o.term as Quad_Object));
  }
  return out;
}
