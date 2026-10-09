/**
 * SPARQL-side helpers: converting SPARQL 1.1 JSON result terms to RDF/JS
 * terms, serialising RDF/JS terms to SPARQL/N-Triples syntax, and a small
 * token scanner to detect the query form (SELECT/ASK/…) without a parser.
 */
import type * as RDF from '@rdfjs/types';
import { BlankNode, DataFactory, Literal, NamedNode, Quad } from './terms.js';

// --- SPARQL 1.1 JSON results → RDF/JS terms -----------------------------------------------------

export interface SparqlJsonTerm {
  type: 'uri' | 'literal' | 'typed-literal' | 'bnode';
  value: string;
  'xml:lang'?: string;
  /**
   * RDF 1.2 base direction of a directional language-tagged literal
   * (`"x"@en--ltr`). SPARQL 1.2 JSON results keep it separate from
   * `xml:lang`; sparq's engine emits the same split (`its:dir`).
   */
  'its:dir'?: 'ltr' | 'rtl';
  datatype?: string;
}

export interface SparqlJsonResults {
  head: { vars?: string[] };
  results?: { bindings: Record<string, SparqlJsonTerm>[] };
  boolean?: boolean;
}

export function termFromSparqlJson(term: SparqlJsonTerm): RDF.Term {
  switch (term.type) {
    case 'uri':
      return new NamedNode(term.value);
    case 'bnode':
      return new BlankNode(term.value);
    case 'literal':
    case 'typed-literal': // legacy alias emitted by some endpoints
      if (term['xml:lang'] !== undefined) {
        return new Literal(term.value, { language: term['xml:lang'], direction: term['its:dir'] });
      }
      if (term.datatype !== undefined) return new Literal(term.value, new NamedNode(term.datatype));
      return new Literal(term.value);
    default:
      throw new Error(`unsupported SPARQL JSON term type: ${(term as { type: string }).type}`);
  }
}

/**
 * Incremental parser for a *chunked* SPARQL 1.1 JSON results document (the
 * engine's chunk sequence concatenates to one valid document): feed chunks
 * with `push()`, get back the solution rows completed by each chunk. Holds at
 * most one chunk plus a partial row in memory — never the whole document.
 * Rows are extracted with a string-aware brace scanner, so it stays correct
 * even if a future engine splits a chunk mid-row.
 */
export class SparqlJsonRowsParser {
  #buf = '';
  #inBindings = false;
  #done = false;

  /** Set when the document is the ASK boolean form (it then has no rows). */
  boolean: boolean | undefined;

  /** Feeds the next chunk; returns the rows it completed. */
  push(chunk: string): Record<string, SparqlJsonTerm>[] {
    if (this.#done) return [];
    this.#buf += chunk;
    if (!this.#inBindings) {
      const start = this.#buf.indexOf('"bindings":[');
      if (start < 0) {
        // The ASK boolean form has no bindings array at all.
        const bool = /"boolean"\s*:\s*(true|false)/.exec(this.#buf);
        if (bool) {
          this.boolean = bool[1] === 'true';
          this.#done = true;
          this.#buf = '';
        }
        return []; // head not complete yet — keep buffering
      }
      this.#buf = this.#buf.slice(start + '"bindings":['.length);
      this.#inBindings = true;
    }
    return this.#scanRows();
  }

  /** Extracts the complete top-level `{…}` row objects currently buffered. */
  #scanRows(): Record<string, SparqlJsonTerm>[] {
    const rows: Record<string, SparqlJsonTerm>[] = [];
    const buf = this.#buf;
    let i = 0;
    while (i < buf.length) {
      const c = buf[i]!;
      if (c === ',' || c === ' ' || c === '\n' || c === '\t' || c === '\r') {
        i++;
      } else if (c === ']') {
        this.#done = true; // end of the bindings array; trailing "}}"' ignored
        this.#buf = '';
        return rows;
      } else if (c === '{') {
        // Scan to the matching close brace, JSON-string-aware.
        let depth = 0;
        let inString = false;
        let j = i;
        for (; j < buf.length; j++) {
          const ch = buf[j]!;
          if (inString) {
            if (ch === '\\') j++;
            else if (ch === '"') inString = false;
          } else if (ch === '"') inString = true;
          else if (ch === '{') depth++;
          else if (ch === '}' && --depth === 0) break;
        }
        if (j >= buf.length) break; // row incomplete — wait for the next chunk
        rows.push(JSON.parse(buf.slice(i, j + 1)) as Record<string, SparqlJsonTerm>);
        i = j + 1;
      } else {
        throw new Error(`malformed SPARQL JSON results: unexpected ${JSON.stringify(c)} in bindings array`);
      }
    }
    this.#buf = buf.slice(i);
    return rows;
  }
}

// --- RDF/JS terms → N-Triples / SPARQL syntax ---------------------------------------------------

/**
 * Escapes a literal lexical form for the inside of a `"…"` N-Triples /
 * SPARQL `STRING_LITERAL_QUOTE`. Escapes `\`, `"`, and the whole C0 control
 * range plus DEL — the SPARQL grammar forbids a raw `#x22`/`#x5C`/`#xA`/`#xD`
 * inside a single-quoted string, and a raw control byte (e.g. TAB or NUL) is
 * exactly what an attacker would use to slip past a naive `"`-only escaper.
 * `\n`/`\r`/`\t`/`\b`/`\f` use their short forms; every other control char
 * becomes a `\uXXXX` escape.
 *
 * This is the SPARQL-injection guard for literal *values*: the output cannot
 * close its own quote or emit a raw newline, so a hostile value (e.g. an
 * ACL-derived label) stays confined to the literal token when the result is
 * re-parsed by sparq's own SPARQL lexer.
 */
function escapeLiteral(value: string): string {
  // " \  and C0 controls (#x00–#x1F) + DEL (#x7F).
  // eslint-disable-next-line no-control-regex
  return value.replace(/["\\\x00-\x1f\x7f]/g, (c) => {
    switch (c) {
      case '\\':
        return '\\\\';
      case '"':
        return '\\"';
      case '\n':
        return '\\n';
      case '\r':
        return '\\r';
      case '\t':
        return '\\t';
      case '\b':
        return '\\b';
      case '\f':
        return '\\f';
      default:
        return `\\u${c.charCodeAt(0).toString(16).toUpperCase().padStart(4, '0')}`;
    }
  });
}

/**
 * Percent-encodes the characters the SPARQL/Turtle `IRIREF` production forbids
 * inside `<…>` — `< > " { } | ^` `` ` `` `\` and every codepoint in `#x00–#x20`
 * (controls and space). Without this a hostile IRI value — most importantly one
 * carrying a `>`, e.g. an ACL pointer IRI taken from untrusted input — could
 * close its own `<…>` bracket and inject arbitrary SPARQL.
 *
 * This is the SPARQL-injection guard for IRI *values*; it matches the illegal
 * set QLever's lexer rejects, so a value that round-trips here parses to the
 * same single IRI term in sparq's parser. Percent-encoding is IRI-preserving:
 * an endpoint dereferences the encoded form to the same resource.
 */
function escapeIri(value: string): string {
  // IRIREF illegal set: < > " { } | ^ ` \ and #x00–#x20 (controls + space).
  // eslint-disable-next-line no-control-regex
  return value.replace(/[<>"{}|^`\\\x00-\x20]/g, (c) => {
    const code = c.charCodeAt(0);
    return `%${code.toString(16).toUpperCase().padStart(2, '0')}`;
  });
}

const XSD_STRING = 'http://www.w3.org/2001/XMLSchema#string';

// Term parts with no escape form are validated, never rewritten: a crafted blank-node label,
// language tag or direction would otherwise end its token and inject statements (N-Quads) or
// query text (SPARQL). The rules are the N-Triples BLANK_NODE_LABEL and LANGTAG productions
// plus the RDF 1.2 `ltr`/`rtl` direction. A lone UTF-16 surrogate has no UTF-8 or UCHAR form,
// so it is rejected in IRIs and literals.
const LONE_SURROGATE = /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/;
const PN_CHARS_BASE =
  'A-Za-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF'
  + '\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD'
  + '\\u{10000}-\\u{EFFFF}';
const PN_CHARS = `${PN_CHARS_BASE}_:\\-0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040`;
const BLANK_NODE_LABEL = new RegExp(`^[${PN_CHARS_BASE}_:0-9](?:[${PN_CHARS}.]*[${PN_CHARS}])?$`, 'u');
const LANGTAG = /^[a-zA-Z]+(?:-[a-zA-Z0-9]+)*$/;

function noLoneSurrogate(value: string, what: string): string {
  if (LONE_SURROGATE.test(value)) {
    throw new Error(`cannot serialise ${what} ${JSON.stringify(value)}: it contains a lone surrogate`);
  }
  return value;
}

/**
 * Serialises a term to its N-Triples (and SPARQL constant) form. IRIs are percent-encoded
 * (see {@link escapeIri}) and literal values escaped; a blank-node label that is not a
 * `BLANK_NODE_LABEL`, a language tag that is not a `LANGTAG`, a direction other than
 * `ltr`/`rtl`, or a lone surrogate throws an `Error` instead.
 */
export function termToNT(term: RDF.Term): string {
  switch (term.termType) {
    case 'NamedNode':
      return `<${escapeIri(noLoneSurrogate(term.value, 'IRI'))}>`;
    case 'BlankNode':
      if (!BLANK_NODE_LABEL.test(term.value)) {
        throw new Error(`cannot serialise blank node label ${JSON.stringify(term.value)}: not an N-Triples BLANK_NODE_LABEL`);
      }
      return `_:${term.value}`;
    case 'Literal': {
      const quoted = `"${escapeLiteral(noLoneSurrogate(term.value, 'literal'))}"`;
      if (term.language !== '') {
        if (!LANGTAG.test(term.language)) {
          throw new Error(`cannot serialise language tag ${JSON.stringify(term.language)}: not an N-Triples LANGTAG`);
        }
        let dir = '';
        if (term.direction != null && term.direction !== '') {
          if (term.direction !== 'ltr' && term.direction !== 'rtl') {
            throw new Error(`cannot serialise base direction ${JSON.stringify(term.direction)}: must be "ltr" or "rtl"`);
          }
          dir = `--${term.direction}`;
        }
        return `${quoted}@${term.language}${dir}`;
      }
      if (term.datatype.value !== XSD_STRING) {
        return `${quoted}^^<${escapeIri(noLoneSurrogate(term.datatype.value, 'datatype IRI'))}>`;
      }
      return quoted;
    }
    case 'DefaultGraph':
      return '';
    default:
      throw new Error(`cannot serialise term type ${term.termType} to N-Triples`);
  }
}

/**
 * Serialises quads to N-Quads (a superset of N-Triples; default-graph quads
 * produce plain N-Triples lines). The sparq engine folds named graphs into the
 * default graph on load.
 */
export function quadsToNQuads(quads: Iterable<RDF.Quad>): string {
  let out = '';
  for (const quad of quads) {
    if (quad.subject.termType === 'Quad' || quad.object.termType === 'Quad') {
      throw new Error('RDF 1.2 triple terms are not supported');
    }
    const graph = quad.graph.termType === 'DefaultGraph' ? '' : ` ${termToNT(quad.graph)}`;
    out += `${termToNT(quad.subject)} ${termToNT(quad.predicate)} ${termToNT(quad.object)}${graph} .\n`;
  }
  return out;
}

// --- N-Triples → RDF/JS quads -------------------------------------------------------------------

/**
 * Decodes the N-Triples / Turtle string-escape sequences the engine's
 * serialiser can emit inside an `IRIREF` or a quoted literal: the single-char
 * escapes (`\t \b \n \r \f \" \' \\`) and the numeric `\uXXXX` / `\UXXXXXXXX`
 * forms. `pos` points just past the opening delimiter; returns the decoded
 * value and the index of the closing `end` delimiter.
 */
function unescapeNT(input: string, pos: number, end: '"' | '>'): { value: string; next: number } {
  let out = '';
  let i = pos;
  const n = input.length;
  while (i < n) {
    const c = input[i]!;
    if (c === end) return { value: out, next: i };
    if (c === '\\') {
      const e = input[i + 1];
      switch (e) {
        case 't': out += '\t'; i += 2; break;
        case 'b': out += '\b'; i += 2; break;
        case 'n': out += '\n'; i += 2; break;
        case 'r': out += '\r'; i += 2; break;
        case 'f': out += '\f'; i += 2; break;
        case '"': out += '"'; i += 2; break;
        case "'": out += "'"; i += 2; break;
        case '\\': out += '\\'; i += 2; break;
        case 'u': {
          out += String.fromCodePoint(parseInt(input.slice(i + 2, i + 6), 16));
          i += 6;
          break;
        }
        case 'U': {
          out += String.fromCodePoint(parseInt(input.slice(i + 2, i + 10), 16));
          i += 10;
          break;
        }
        default:
          throw new Error(`invalid N-Triples escape \\${e ?? ''}`);
      }
    } else {
      out += c;
      i++;
    }
  }
  throw new Error(`unterminated N-Triples ${end === '"' ? 'literal' : 'IRI'}`);
}

/** Reads one N-Triples term (IRI / blank node / literal) starting at `pos`. */
function parseTerm(line: string, pos: number): { term: RDF.Term; next: number } {
  const c = line[pos];
  if (c === '<') {
    const { value, next } = unescapeNT(line, pos + 1, '>');
    return { term: new NamedNode(value), next: next + 1 };
  }
  if (c === '_' && line[pos + 1] === ':') {
    let i = pos + 2;
    // PN_CHARS-ish run: stop at whitespace or the line terminator dot.
    while (i < line.length && !/[\s]/.test(line[i]!)) i++;
    return { term: new BlankNode(line.slice(pos + 2, i)), next: i };
  }
  if (c === '"') {
    const { value, next } = unescapeNT(line, pos + 1, '"');
    let i = next + 1;
    if (line[i] === '@') {
      let j = i + 1;
      while (j < line.length && /[A-Za-z0-9-]/.test(line[j]!)) j++;
      const tag = line.slice(i + 1, j);
      // RDF 1.2 base-direction: `@lang--dir`.
      const dirMatch = /^(.*?)--(ltr|rtl)$/.exec(tag);
      if (dirMatch) {
        return { term: new Literal(value, { language: dirMatch[1]!, direction: dirMatch[2] as 'ltr' | 'rtl' }), next: j };
      }
      return { term: new Literal(value, tag), next: j };
    }
    if (line[i] === '^' && line[i + 1] === '^') {
      const dt = parseTerm(line, i + 2);
      return { term: new Literal(value, dt.term as RDF.NamedNode), next: dt.next };
    }
    return { term: new Literal(value), next: i };
  }
  throw new Error(`unexpected token ${JSON.stringify(c ?? '')} parsing N-Triples term`);
}

/**
 * Parses an N-Triples document (the exact form the engine's CONSTRUCT /
 * DESCRIBE serialiser emits — `<s> <p> <o> .`, one triple per line, absolute
 * IRIs, canonical literal escapes) into RDF/JS {@link Quad}s in the default
 * graph. Blank-node labels are preserved verbatim (the engine freshens
 * template bnodes per solution, so labels are already globally unique). Blank
 * lines are skipped; a malformed line throws.
 */
export function parseNTriples(nt: string): Quad[] {
  const quads: Quad[] = [];
  for (const raw of nt.split('\n')) {
    const line = raw.trim();
    if (line === '') continue;
    const s = parseTerm(line, 0);
    let i = s.next;
    while (line[i] === ' ' || line[i] === '\t') i++;
    const p = parseTerm(line, i);
    i = p.next;
    while (line[i] === ' ' || line[i] === '\t') i++;
    const o = parseTerm(line, i);
    quads.push(
      new Quad(
        s.term as RDF.Quad_Subject,
        p.term as RDF.Quad_Predicate,
        o.term as RDF.Quad_Object,
      ),
    );
  }
  return quads;
}

// --- query-form detection ------------------------------------------------------------------------

export type QueryForm = 'SELECT' | 'ASK' | 'CONSTRUCT' | 'DESCRIBE';
const FORMS = new Set(['SELECT', 'ASK', 'CONSTRUCT', 'DESCRIBE']);

/**
 * Finds the query form keyword (the first bare SELECT/ASK/CONSTRUCT/DESCRIBE
 * token after the PREFIX/BASE prologue), skipping comments, IRIs and string
 * literals so e.g. `PREFIX ask: <http://ex/ASK#>` cannot confuse it.
 * Returns the form and its character offset, or `undefined` if none is found.
 */
export function detectQueryForm(sparql: string): { form: QueryForm; index: number; length: number } | undefined {
  let i = 0;
  const n = sparql.length;
  while (i < n) {
    const c = sparql[i]!;
    if (c === '#') {
      while (i < n && sparql[i] !== '\n') i++;
    } else if (c === '<') {
      while (i < n && sparql[i] !== '>') i++;
      i++;
    } else if (c === '"' || c === "'") {
      const quote = c;
      const long = sparql.startsWith(quote.repeat(3), i);
      const end = long ? quote.repeat(3) : quote;
      i += end.length;
      while (i < n) {
        if (sparql[i] === '\\') i += 2;
        else if (sparql.startsWith(end, i)) {
          i += end.length;
          break;
        } else i++;
      }
    } else if (/[A-Za-z]/.test(c)) {
      let j = i;
      while (j < n && /[A-Za-z]/.test(sparql[j]!)) j++;
      const word = sparql.slice(i, j).toUpperCase();
      // Not a keyword if part of a prefixed name (`ask:x`, `ex:ASK`) or a
      // variable (`?ask`): inspect the adjacent characters.
      const before = i > 0 ? sparql[i - 1]! : ' ';
      const after = j < n ? sparql[j]! : ' ';
      const isBareToken = !/[:?$@\w.-]/.test(before) && !/[:\w-]/.test(after);
      if (isBareToken && FORMS.has(word)) return { form: word as QueryForm, index: i, length: j - i };
      i = j;
    } else {
      i++;
    }
  }
  return undefined;
}

export { DataFactory };
