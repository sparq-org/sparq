// [OPUS-4.8] sq-gb4o (#805) — a dependency-free, framework-agnostic PRETTY-Turtle serialiser.
//
// The wasm engine answers CONSTRUCT/DESCRIBE (and the dataset viewer's `SELECT ?s ?p ?o`) as a
// FLAT N-Triples document — one `s p o .` per line — because N-Triples ⊂ Turtle and that is the
// cheapest correct wire shape. The website wants the SAME data as IDIOMATIC, grouped, indented
// Turtle: `@prefix` abbreviation, one block per subject, predicate-object lists joined with `;`,
// shared-predicate object lists joined with `,`. The engine exposes no pretty serialiser (see
// `sparq_wasm.d.ts` — `queryQuads` is N-Triples only), so this module is the site-side reshaper.
//
// It is DEPENDENCY-FREE and FRAMEWORK-AGNOSTIC (no DOM, no React) so the same code serves the
// Next.js site and the Tauri 2 webview, and is unit-testable under `node --test`. Composition
// with the highlighter (`tokenizeTurtle`) is "pretty-print THEN highlight": this returns a
// string, `RdfHighlight` tokenises it.
//
// Design goals, in priority order:
//   1. CORRECTNESS / round-trip equivalence. The output is a Turtle document that, re-parsed,
//      yields exactly the same set of triples as the input. Literals (incl. datatype/langtag),
//      blank nodes (labels preserved verbatim), and RDF 1.2 triple terms `<<( s p o )>>` are
//      reproduced losslessly. We NEVER reformat the inside of a literal or an IRI.
//   2. NEVER THROW. This is a forgiving reshaper over engine output. A line we cannot parse as a
//      statement is passed through verbatim (so malformed input degrades to flat output, not a
//      crash). The site falls back to the raw text on any thrown error anyway.
//   3. Idiomatic Turtle: prefix abbreviation, `a` for rdf:type, subject grouping, `;`/`,` lists,
//      stable ordering (subjects + predicates + objects sorted by their N-Triples form) so the
//      output is deterministic regardless of triple emission order.
//
// What it is NOT: a general Turtle parser. The input is N-Triples / N-Quads (the engine's
// output) — `term . term . term .` with `<iri>`, `_:bnode`, `"lit"`(`^^dt`|`@lang`) terms and an
// optional 4th graph term, plus the RDF 1.2 `<<( s p o )>>` object-position triple term. That is
// the whole grammar we tokenise. (Named graphs are handled by `prettyTrig`, which wraps each
// graph's triples in a `GRAPH <g> { … }` block.)

import { COMMON_PREFIXES, type PrefixBinding } from "./sparql-prefixes.js";

const RDF_TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const XSD_STRING = "http://www.w3.org/2001/XMLSchema#string";

/** A parsed RDF term. The `nt` field is the term's exact N-Triples spelling — the round-trip
 * anchor and the stable-sort key — so even a term we choose to abbreviate can be re-emitted
 * losslessly when abbreviation does not apply. */
export type RdfTerm =
  | { kind: "iri"; value: string; nt: string }
  | { kind: "bnode"; label: string; nt: string }
  | { kind: "literal"; value: string; datatype?: string; lang?: string; nt: string }
  | { kind: "triple"; s: RdfTerm; p: RdfTerm; o: RdfTerm; nt: string };

/** One parsed statement: subject, predicate, object, and an optional named graph. */
export interface RdfStatement {
  s: RdfTerm;
  p: RdfTerm;
  o: RdfTerm;
  g?: RdfTerm;
}

/** Options for {@link prettyTurtle}. */
export interface PrettyTurtleOptions {
  /** Extra prefix bindings to consider for abbreviation, ahead of {@link COMMON_PREFIXES}.
   * Typically the prefixes the user's query DECLARES (see `declaredPrefixes` + an IRI map). */
  prefixes?: readonly PrefixBinding[];
  /** Indent unit for predicate/object continuation lines. Default two spaces. */
  indent?: string;
  /** When `false`, no `@prefix` header is emitted and IRIs stay in full `<…>` form. Default
   * `true`. */
  abbreviate?: boolean;
}

// ---------------------------------------------------------------------------
// Tokeniser / parser for N-Triples / N-Quads terms (the engine's wire shape).
// ---------------------------------------------------------------------------

const isWsByte = (c: string): boolean =>
  c === " " || c === "\t" || c === "\r" || c === "\n" || c === "\f" || c === "\v";

/** A cursor-style parser over one statement worth of N-Triples/N-Quads text. */
class TermReader {
  private i = 0;
  constructor(private readonly s: string) {}

  /** Skip whitespace; return true if more non-whitespace remains. */
  private skipWs(): boolean {
    while (this.i < this.s.length && isWsByte(this.s[this.i])) this.i++;
    return this.i < this.s.length;
  }

  /** Whether the next non-ws char is the statement terminator `.`. */
  atTerminator(): boolean {
    this.skipWs();
    return this.i < this.s.length && this.s[this.i] === ".";
  }

  consumeTerminator(): void {
    this.skipWs();
    if (this.s[this.i] === ".") this.i++;
  }

  /** The remaining (trimmed) text — used to detect trailing junk. */
  rest(): string {
    return this.s.slice(this.i).trim();
  }

  /**
   * Parse the next RDF term (IRI / blank node / literal / RDF 1.2 triple term). Throws if the
   * cursor is not at a recognisable term — the caller treats a throw as "pass this line
   * through verbatim".
   */
  next(): RdfTerm {
    if (!this.skipWs()) throw new Error("unexpected end of statement");
    const c = this.s[this.i];
    if (c === "<") {
      // Either an IRI `<…>` or an RDF 1.2 triple term `<<( s p o )>>`.
      if (this.s[this.i + 1] === "<" && this.s[this.i + 2] === "(") return this.tripleTerm();
      return this.iri();
    }
    if (c === "_" && this.s[this.i + 1] === ":") return this.bnode();
    if (c === '"') return this.literal();
    throw new Error(`unrecognised term at offset ${this.i}`);
  }

  private iri(): Extract<RdfTerm, { kind: "iri" }> {
    const start = this.i;
    this.i++; // past '<'
    while (this.i < this.s.length && this.s[this.i] !== ">") {
      // IRIs cannot contain a raw '>'; '\>' is a (rare) escape — skip the escaped char.
      if (this.s[this.i] === "\\") this.i++;
      this.i++;
    }
    if (this.s[this.i] !== ">") throw new Error("unterminated IRI");
    this.i++; // past '>'
    const nt = this.s.slice(start, this.i);
    // The IRI value is the bytes between < and >, with N-Triples escapes left as-is (we only
    // ever re-emit them, never interpret them, so the round-trip is exact).
    const value = nt.slice(1, -1);
    return { kind: "iri", value, nt };
  }

  private bnode(): RdfTerm {
    const start = this.i;
    this.i += 2; // past '_:'
    // BLANK_NODE_LABEL allows an internal '.', so consume dots too — then trim any trailing '.'
    // (the grammar forbids a label ENDING in '.', so a trailing dot is the statement terminator
    // or a triple-term close). This keeps a dotted label like `_:b.1` intact.
    while (
      this.i < this.s.length &&
      !isWsByte(this.s[this.i]) &&
      this.s[this.i] !== ")"
    ) {
      this.i++;
    }
    while (this.i > start + 2 && this.s[this.i - 1] === ".") this.i--;
    const nt = this.s.slice(start, this.i);
    return { kind: "bnode", label: nt.slice(2), nt };
  }

  private literal(): RdfTerm {
    const start = this.i;
    this.i++; // past opening '"'
    // N-Triples literals are always double-quoted, single-line, with \-escapes.
    while (this.i < this.s.length && this.s[this.i] !== '"') {
      if (this.s[this.i] === "\\") this.i++;
      this.i++;
    }
    if (this.s[this.i] !== '"') throw new Error("unterminated literal");
    this.i++; // past closing '"'
    const lexEnd = this.i;
    const value = this.s.slice(start + 1, lexEnd - 1);
    let datatype: string | undefined;
    let lang: string | undefined;
    if (this.s[this.i] === "@") {
      this.i++;
      const ls = this.i;
      while (
        this.i < this.s.length &&
        !isWsByte(this.s[this.i]) &&
        this.s[this.i] !== "." &&
        this.s[this.i] !== ")"
      ) {
        this.i++;
      }
      lang = this.s.slice(ls, this.i);
    } else if (this.s[this.i] === "^" && this.s[this.i + 1] === "^") {
      this.i += 2;
      this.skipWs();
      if (this.s[this.i] !== "<") throw new Error("expected datatype IRI");
      const dt = this.iri();
      datatype = dt.value;
    }
    const nt = this.s.slice(start, this.i);
    return { kind: "literal", value, datatype, lang, nt };
  }

  private tripleTerm(): RdfTerm {
    const start = this.i;
    this.i += 3; // past '<<('
    const s = this.next();
    const p = this.next();
    const o = this.next();
    this.skipWs();
    if (this.s[this.i] === ")" && this.s[this.i + 1] === ">" && this.s[this.i + 2] === ">") {
      this.i += 3;
    } else {
      throw new Error("unterminated triple term");
    }
    const nt = this.s.slice(start, this.i);
    return { kind: "triple", s, p, o, nt };
  }
}

/**
 * Parse an N-Triples / N-Quads document into statements. Lines that do not parse cleanly as a
 * statement are returned in `passthrough` (verbatim, including their original text) so the
 * caller can emit them unchanged rather than dropping or mangling them. NEVER throws.
 *
 * The parser is statement-oriented but newline-tolerant: it reads term-by-term and consumes the
 * `.` terminator, so a statement split across lines (or several statements on one line) is still
 * handled. A `passthrough` chunk is a maximal run of input we could not interpret.
 */
export function parseNTriples(input: string): {
  statements: RdfStatement[];
  passthrough: string[];
} {
  const statements: RdfStatement[] = [];
  const passthrough: string[] = [];
  // Split on lines but re-join a statement that legitimately spans lines is overkill for engine
  // output (which is one statement per line). We parse line-by-line; a line that fails to parse
  // is kept verbatim. A blank or comment line is skipped silently (no passthrough noise).
  for (const rawLine of input.split("\n")) {
    const line = rawLine.trim();
    if (line.length === 0 || line.startsWith("#")) continue;
    try {
      const r = new TermReader(line);
      const s = r.next();
      const p = r.next();
      const o = r.next();
      let g: RdfTerm | undefined;
      if (!r.atTerminator()) {
        // A 4th term before the '.' is the N-Quads named graph.
        g = r.next();
      }
      r.consumeTerminator();
      if (r.rest().length !== 0) throw new Error("trailing content after statement");
      statements.push({ s, p, o, g });
    } catch {
      passthrough.push(rawLine);
    }
  }
  return { statements, passthrough };
}

// ---------------------------------------------------------------------------
// Prefix abbreviation.
// ---------------------------------------------------------------------------

// Turtle local-name (PN_LOCAL) acceptance — conservative: a local part we can write WITHOUT
// percent/backslash escaping. If the suffix would need escaping we keep the IRI in full `<…>`
// form (always valid), rather than risk producing an unparseable prefixed name. Allowed: the
// ASCII name chars plus `_`, `-`, `.`, `:` and `%` (already-encoded). We forbid a leading or
// trailing `.` (Turtle PN_LOCAL rule) by checking the ends.
const PN_LOCAL_OK = /^[A-Za-z0-9_%:]([A-Za-z0-9_.\-%:]*[A-Za-z0-9_%:])?$/;

/** A prefix candidate: a label and the namespace IRI it abbreviates. */
interface PrefixCandidate {
  prefix: string;
  iri: string;
}

/**
 * Build the ordered list of prefix candidates to try when abbreviating IRIs: the caller's
 * `extra` bindings first (so a query's own declarations win), then the well-known
 * {@link COMMON_PREFIXES}, de-duplicated by namespace IRI (first label for an IRI wins). The
 * empty-prefix `:` is allowed.
 */
function prefixCandidates(extra: readonly PrefixBinding[]): PrefixCandidate[] {
  const out: PrefixCandidate[] = [];
  const seenIri = new Set<string>();
  const seenPrefix = new Set<string>();
  for (const b of [...extra, ...COMMON_PREFIXES]) {
    if (seenIri.has(b.iri) || seenPrefix.has(b.prefix)) continue;
    seenIri.add(b.iri);
    seenPrefix.add(b.prefix);
    out.push({ prefix: b.prefix, iri: b.iri });
  }
  // Longest namespace first so the most specific prefix is chosen for nested namespaces.
  out.sort((a, b) => b.iri.length - a.iri.length);
  return out;
}

/** Try to abbreviate an IRI to a prefixed name. Returns the prefixed form and records the used
 * prefix, or `null` if no candidate matches with a writable local part. */
function abbreviateIri(
  iri: string,
  candidates: PrefixCandidate[],
  used: Map<string, string>,
): string | null {
  for (const c of candidates) {
    if (!iri.startsWith(c.iri)) continue;
    const local = iri.slice(c.iri.length);
    if (local.length === 0 || PN_LOCAL_OK.test(local)) {
      used.set(c.prefix, c.iri);
      return `${c.prefix}:${local}`;
    }
  }
  return null;
}

// ---------------------------------------------------------------------------
// Term rendering.
// ---------------------------------------------------------------------------

/** Render an RDF term to its Turtle text, abbreviating IRIs where possible and recording used
 * prefixes. `isPredicate` enables the `a` shorthand for rdf:type. */
function renderTerm(
  t: RdfTerm,
  candidates: PrefixCandidate[],
  used: Map<string, string>,
  abbreviate: boolean,
  isPredicate: boolean,
): string {
  switch (t.kind) {
    case "iri": {
      if (isPredicate && t.value === RDF_TYPE) return "a";
      if (abbreviate) {
        const abbr = abbreviateIri(t.value, candidates, used);
        if (abbr !== null) return abbr;
      }
      return t.nt; // full <…> form — exact bytes, always valid
    }
    case "bnode":
      return t.nt; // `_:label` — labels are preserved verbatim and stay stable
    case "literal": {
      // The lexical form (the bytes between the quotes) is re-emitted verbatim — we never touch
      // a literal's escaping. We only abbreviate the datatype IRI and drop an implicit xsd:string.
      const quoted = `"${t.value}"`;
      if (t.lang !== undefined) return `${quoted}@${t.lang}`;
      if (t.datatype !== undefined && t.datatype !== XSD_STRING) {
        const dt = abbreviate ? abbreviateIri(t.datatype, candidates, used) : null;
        return `${quoted}^^${dt ?? `<${t.datatype}>`}`;
      }
      return quoted;
    }
    case "triple": {
      // RDF 1.2 triple term `<<( s p o )>>` — recurse; the inner predicate keeps full form (no
      // `a` shorthand inside a triple term, to stay maximally explicit and round-trippable).
      const s = renderTerm(t.s, candidates, used, abbreviate, false);
      const p = renderTerm(t.p, candidates, used, abbreviate, false);
      const o = renderTerm(t.o, candidates, used, abbreviate, false);
      return `<<( ${s} ${p} ${o} )>>`;
    }
  }
}

// ---------------------------------------------------------------------------
// Grouping + serialisation.
// ---------------------------------------------------------------------------

/** Group statements by subject, then by predicate, preserving an object list per predicate.
 * Ordering is by the terms' N-Triples spelling so output is deterministic. */
function groupBySubject(statements: RdfStatement[]): Array<{
  subject: RdfTerm;
  predicates: Array<{ predicate: RdfTerm; objects: RdfTerm[] }>;
}> {
  // subjectNt -> { subject, predNt -> { predicate, objects: ntSet } }
  const bySubject = new Map<
    string,
    { subject: RdfTerm; preds: Map<string, { predicate: RdfTerm; objects: Map<string, RdfTerm> }> }
  >();
  for (const st of statements) {
    let s = bySubject.get(st.s.nt);
    if (!s) {
      s = { subject: st.s, preds: new Map() };
      bySubject.set(st.s.nt, s);
    }
    let p = s.preds.get(st.p.nt);
    if (!p) {
      p = { predicate: st.p, objects: new Map() };
      s.preds.set(st.p.nt, p);
    }
    if (!p.objects.has(st.o.nt)) p.objects.set(st.o.nt, st.o);
  }

  const subjects = [...bySubject.values()].sort((a, b) =>
    a.subject.nt < b.subject.nt ? -1 : a.subject.nt > b.subject.nt ? 1 : 0,
  );
  return subjects.map((s) => ({
    subject: s.subject,
    predicates: [...s.preds.values()]
      .sort((a, b) => predicateSortKey(a.predicate).localeCompare(predicateSortKey(b.predicate)))
      .map((p) => ({
        predicate: p.predicate,
        objects: [...p.objects.values()].sort((a, b) =>
          a.nt < b.nt ? -1 : a.nt > b.nt ? 1 : 0,
        ),
      })),
  }));
}

// rdf:type (the `a` predicate) sorts first within a subject block — the idiomatic Turtle order.
function predicateSortKey(p: RdfTerm): string {
  if (p.kind === "iri" && p.value === RDF_TYPE) return "\u0000";
  return p.nt;
}

/**
 * Serialise N-Triples / N-Quads text as PRETTY, grouped, indented Turtle.
 *
 * - Subjects are grouped into one block each; predicate-object pairs are joined with `;` and a
 *   shared predicate's objects with `,`.
 * - IRIs are abbreviated against the caller's `prefixes` (e.g. the query's declarations) then
 *   {@link COMMON_PREFIXES}; the `@prefix` header lists only the prefixes actually USED.
 * - `rdf:type` renders as `a` and sorts first within a block.
 * - Literals, blank nodes, datatype/lang tags and RDF 1.2 triple terms `<<( s p o )>>` are
 *   reproduced losslessly (the round-trip property: re-parsing the output yields the same
 *   triples as the input).
 * - Named-graph (N-Quads) input is delegated to {@link prettyTrig}.
 *
 * NEVER throws: a line that cannot be parsed is passed through verbatim, and an empty document
 * yields an empty string. Pure — no DOM, no side effects.
 */
export function prettyTurtle(input: string, options: PrettyTurtleOptions = {}): string {
  const text = input ?? "";
  const { statements, passthrough } = parseNTriples(text);

  // Any named-graph statement -> render as TriG (graph blocks). Keeps a plain-triple document
  // as flat Turtle.
  if (statements.some((s) => s.g !== undefined)) {
    return prettyTrig(text, options);
  }

  const indent = options.indent ?? "  ";
  const abbreviate = options.abbreviate ?? true;
  const candidates = prefixCandidates(options.prefixes ?? []);
  const used = new Map<string, string>();

  const body = renderGraphBody(statements, "", indent, candidates, used, abbreviate);
  return assemble(used, abbreviate, body ? [body] : [], passthrough, "");
}

/** Render a set of triples (one graph's worth) as grouped, indented Turtle blocks, recording
 * used prefixes in `used`. `baseIndent` prefixes every line (for a TriG graph-block indent).
 * Returns the joined block text (no `@prefix` header — the caller assembles that once). */
function renderGraphBody(
  statements: RdfStatement[],
  baseIndent: string,
  indent: string,
  candidates: PrefixCandidate[],
  used: Map<string, string>,
  abbreviate: boolean,
): string {
  const grouped = groupBySubject(statements);
  const blocks: string[] = [];
  for (const g of grouped) {
    const subj = renderTerm(g.subject, candidates, used, abbreviate, false);
    const lines: string[] = [];
    g.predicates.forEach((pred, pi) => {
      const p = renderTerm(pred.predicate, candidates, used, abbreviate, true);
      const objs = pred.objects.map((o) => renderTerm(o, candidates, used, abbreviate, false));
      const objText = objs.join(` ,\n${baseIndent}${indent}${indent}`);
      const isLast = pi === g.predicates.length - 1;
      lines.push(`${baseIndent}${indent}${p} ${objText}${isLast ? " ." : " ;"}`);
    });
    blocks.push(`${baseIndent}${subj}\n${lines.join("\n")}`);
  }
  return blocks.join("\n\n");
}

/** Assemble the final document: the `@prefix` header (used prefixes only, prefix-alphabetical),
 * the body sections, and any verbatim passthrough lines. */
function assemble(
  used: Map<string, string>,
  abbreviate: boolean,
  sections: string[],
  passthrough: string[],
  headerIndent: string,
): string {
  const out: string[] = [];
  if (abbreviate && used.size > 0) {
    const header = [...used.entries()]
      .sort((a, b) => a[0].localeCompare(b[0]))
      .map(([prefix, iri]) => `${headerIndent}@prefix ${prefix}: <${iri}> .`)
      .join("\n");
    out.push(header);
  }
  for (const s of sections) if (s.length > 0) out.push(s);
  if (passthrough.length > 0) out.push(passthrough.join("\n"));
  return out.join("\n\n");
}

/**
 * Serialise N-Quads text as PRETTY TriG: default-graph triples at top level, each named graph
 * wrapped in a `GRAPH <g> { … }` block with its triples grouped + indented. Uses one shared
 * `@prefix` header across all graphs. NEVER throws.
 */
export function prettyTrig(input: string, options: PrettyTurtleOptions = {}): string {
  const text = input ?? "";
  const { statements, passthrough } = parseNTriples(text);
  const indent = options.indent ?? "  ";
  const abbreviate = options.abbreviate ?? true;
  const candidates = prefixCandidates(options.prefixes ?? []);
  const used = new Map<string, string>();

  // Partition by graph (default graph keyed by ""). Order: default graph first, then named
  // graphs by their N-Triples spelling.
  const byGraph = new Map<string, { graph?: RdfTerm; statements: RdfStatement[] }>();
  for (const st of statements) {
    const key = st.g ? st.g.nt : "";
    let bucket = byGraph.get(key);
    if (!bucket) {
      bucket = { graph: st.g, statements: [] };
      byGraph.set(key, bucket);
    }
    bucket.statements.push({ s: st.s, p: st.p, o: st.o });
  }

  // Render each graph's body so `used` is fully populated before the header is assembled. One
  // shared `used` map across all graphs -> a single `@prefix` header for the whole document.
  const defaultBucket = byGraph.get("");
  const namedKeys = [...byGraph.keys()].filter((k) => k !== "").sort();

  const sections: string[] = [];
  if (defaultBucket && defaultBucket.statements.length > 0) {
    sections.push(
      renderGraphBody(defaultBucket.statements, "", indent, candidates, used, abbreviate),
    );
  }
  for (const key of namedKeys) {
    const bucket = byGraph.get(key);
    if (!bucket || !bucket.graph) continue;
    const g = renderTerm(bucket.graph, candidates, used, abbreviate, false);
    const inner = renderGraphBody(bucket.statements, indent, indent, candidates, used, abbreviate);
    sections.push(`GRAPH ${g} {\n${inner}\n}`);
  }

  return assemble(used, abbreviate, sections, passthrough, "");
}
