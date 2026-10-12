// [OPUS-4.8] sq-pvg2 (#796) — a dependency-free SHACL Compact Syntax (SCS) SERIALIZER.
//
// SCS is the W3C SHACL Community Group's compact notation for SHACL shapes
// (https://w3c.github.io/shacl/shacl-compact-syntax/ ; carried forward as the
// SHACL 1.2 Compact Syntax). This module renders an *already-parsed* SHACL shapes
// graph — a flat list of triples — into the compact text. It is the DISPLAY
// direction only: shapes graph (Turtle / RDF, parsed by the wasm engine) -> SCS.
//
// The PARSE direction (SCS text -> shapes triples) is a from-scratch grammar
// (ANTLR `SHACLC.g4`, ~117 rules with path expressions + prefix scoping) and is
// the engine's job — tracked separately as a sparq-shacl bead (#796 parse slice),
// NOT done here. Forcing a half-working hand-rolled parser into the site would be
// dishonest; a clean serializer is the bounded, high-value slice.
//
// Scope honesty: this serializer covers the SHACL Core constructs the compact
// grammar expresses — node shapes, `sh:targetClass`/`sh:targetNode`/
// `sh:targetObjectsOf`/`sh:targetSubjectsOf`, property shapes over IRI / inverse /
// sequence / alternative / `?*+` paths, min/max count, the node-kind/value-range/
// string/in/hasValue/class/datatype constraints in the grammar, and `closed`. A
// shape (or constraint) the compact grammar cannot express is NOT silently dropped:
// it is reported back to the caller so the UI can show "N constructs not
// expressible in compact syntax" rather than emit a lossy file that looks complete.
//
// Framework-free + side-effect-free (no React, no DOM, no wasm) so it unit-tests
// directly under `node --test` against hand-built triples.
//
// Every IRI, literal, blank-node label and prefix binding is written through the shared
// sparq-client term writer or checked against the shared grammar checks, so a term recovered from editable
// shapes text cannot end its token early and add shapes or constraints to the output (#6719).

import {
  hasLoneSurrogate,
  isLangTag,
  isTurtleBlankNodeLabel,
  isWritablePrefixBinding,
  termToNTriples,
} from "@sparq/client";

const SH = "http://www.w3.org/ns/shacl#";
const RDF = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS = "http://www.w3.org/2000/01/rdf-schema#";
const XSD = "http://www.w3.org/2001/XMLSchema#";

const RDF_FIRST = `${RDF}first`;
const RDF_REST = `${RDF}rest`;
const RDF_NIL = `${RDF}nil`;
const RDF_TYPE = `${RDF}type`;

/** A simple RDF term, matching the shape the wasm SELECT JSON yields. */
export type ScsTerm =
  | { termType: "NamedNode"; value: string }
  | { termType: "BlankNode"; value: string }
  | {
      termType: "Literal";
      value: string;
      datatype?: string;
      language?: string;
    };

/** A flat triple of the shapes graph. */
export interface ScsTriple {
  subject: ScsTerm;
  predicate: ScsTerm;
  object: ScsTerm;
}

/** A `PREFIX p: <iri>` binding. */
export interface ScsPrefix {
  prefix: string;
  iri: string;
}

export interface ScsResult {
  /** The serialized SHACL Compact Syntax document. */
  text: string;
  /**
   * Human-readable notes about shapes/constraints that the compact grammar cannot
   * express and were therefore OMITTED from `text` (kept honest — never silently
   * dropped). Empty when the round-trip is lossless for the input.
   */
  unsupported: string[];
}

// The default prefixes the SCS spec treats as implicit-but-conventional. We always
// emit any prefix that is actually USED, so these are only a naming fallback.
const WELL_KNOWN_PREFIXES: ScsPrefix[] = [
  { prefix: "sh", iri: SH },
  { prefix: "rdf", iri: RDF },
  { prefix: "rdfs", iri: RDFS },
  { prefix: "xsd", iri: XSD },
];

// ---------------------------------------------------------------------------
// Small triple-store view over the flat triple list.
// ---------------------------------------------------------------------------

function termKey(t: ScsTerm): string {
  if (t.termType === "Literal") {
    return `L:${t.value} ${t.datatype ?? ""} ${t.language ?? ""}`;
  }
  return `${t.termType === "BlankNode" ? "B" : "N"}:${t.value}`;
}

class Graph {
  private bySubject = new Map<string, ScsTriple[]>();

  constructor(triples: ScsTriple[]) {
    for (const t of triples) {
      const k = termKey(t.subject);
      const arr = this.bySubject.get(k);
      if (arr) arr.push(t);
      else this.bySubject.set(k, [t]);
    }
  }

  out(subject: ScsTerm): ScsTriple[] {
    return this.bySubject.get(termKey(subject)) ?? [];
  }

  objects(subject: ScsTerm, predicateIri: string): ScsTerm[] {
    return this.out(subject)
      .filter(
        (t) => t.predicate.termType === "NamedNode" && t.predicate.value === predicateIri,
      )
      .map((t) => t.object);
  }

  one(subject: ScsTerm, predicateIri: string): ScsTerm | undefined {
    return this.objects(subject, predicateIri)[0];
  }

  /** Reads an RDF list (`rdf:first`/`rdf:rest`) into a term array; [] if not a list. */
  list(head: ScsTerm): ScsTerm[] {
    const out: ScsTerm[] = [];
    let node: ScsTerm | undefined = head;
    const seen = new Set<string>();
    while (node && !(node.termType === "NamedNode" && node.value === RDF_NIL)) {
      const key = termKey(node);
      if (seen.has(key)) break; // cycle guard
      seen.add(key);
      const first = this.one(node, RDF_FIRST);
      if (!first) break;
      out.push(first);
      node = this.one(node, RDF_REST);
    }
    return out;
  }
}

// ---------------------------------------------------------------------------
// Prefix handling.
// ---------------------------------------------------------------------------

class Prefixer {
  // longest-iri-first so a longer namespace wins over a shorter overlapping one.
  private entries: { prefix: string; iri: string }[];
  used = new Set<string>();

  constructor(prefixes: ScsPrefix[]) {
    const merged = new Map<string, string>();
    for (const p of [...WELL_KNOWN_PREFIXES, ...prefixes]) {
      // A binding that cannot be declared as written is dropped; its IRIs stay in full form.
      if (!isWritablePrefixBinding(p)) continue;
      // caller-supplied prefixes override the well-known fallback
      merged.set(p.prefix, p.iri);
    }
    this.entries = [...merged.entries()]
      .map(([prefix, iri]) => ({ prefix, iri }))
      .sort((a, b) => b.iri.length - a.iri.length);
  }

  /** Render an IRI as a prefixed name if possible, else `<full>`. */
  iri(value: string): string {
    for (const { prefix, iri } of this.entries) {
      if (value.startsWith(iri)) {
        const local = value.slice(iri.length);
        // Only use a prefixed name when the local part is a legal PN_LOCAL-ish token
        // (no spaces / no chars that would need escaping, no trailing '.'). Otherwise fall
        // back to <>.
        if (/^[A-Za-z0-9_](?:[A-Za-z0-9_.\-]*[A-Za-z0-9_\-])?$/.test(local) || local === "") {
          this.used.add(prefix);
          return `${prefix}:${local}`;
        }
      }
    }
    // The shared writer throws on a character an IRIREF cannot hold.
    return termToNTriples({ type: "uri", value });
  }

  prefixLines(): string[] {
    const lines: string[] = [];
    for (const { prefix, iri } of [...this.entries].sort((a, b) =>
      a.prefix.localeCompare(b.prefix),
    )) {
      if (this.used.has(prefix)) lines.push(`PREFIX ${prefix}: <${iri}>`);
    }
    return lines;
  }
}

// ---------------------------------------------------------------------------
// Term -> SCS text.
// ---------------------------------------------------------------------------

// SCS string literals follow SPARQL's STRING_LITERAL2: only `"`, `\`, LF and CR must be
// escaped, ECHAR is the only escape form, and every other character may appear raw.
const SCS_ECHAR: Record<string, string> = {
  "\b": "\\b",
  "\t": "\\t",
  "\n": "\\n",
  "\f": "\\f",
  "\r": "\\r",
  '"': '\\"',
  "\\": "\\\\",
};

// A quoted string literal, optionally with a language tag checked against LANGTAG.
function quoted(value: string, language?: string): string {
  if (hasLoneSurrogate(value)) {
    throw new Error(`shapesToCompact: literal ${JSON.stringify(value)} contains a lone surrogate`);
  }
  const lexical = `"${value.replace(/[\b\t\n\f\r"\\]/g, (c) => SCS_ECHAR[c])}"`;
  if (!language) return lexical;
  if (!isLangTag(language)) {
    throw new Error(`shapesToCompact: language tag ${JSON.stringify(language)} is not a LANGTAG`);
  }
  return `${lexical}@${language}`;
}

function blankNode(label: string): string {
  if (!isTurtleBlankNodeLabel(label)) {
    throw new Error(`shapesToCompact: ${JSON.stringify(label)} is not a BLANK_NODE_LABEL`);
  }
  return `_:${label}`;
}

function termToScs(t: ScsTerm, p: Prefixer): string {
  if (t.termType === "NamedNode") return p.iri(t.value);
  if (t.termType === "BlankNode") return blankNode(t.value);
  // Literal
  if (t.language) return quoted(t.value, t.language);
  const dt = t.datatype;
  if (!dt || dt === `${XSD}string`) {
    return quoted(t.value);
  }
  // numeric / boolean literals render bare per the grammar (numericLiteral/booleanLiteral)
  if (dt === `${XSD}integer` && /^[+-]?\d+$/.test(t.value)) return t.value;
  if (dt === `${XSD}decimal` && /^[+-]?\d*\.\d+$/.test(t.value)) return t.value;
  if (dt === `${XSD}boolean` && (t.value === "true" || t.value === "false")) return t.value;
  if (
    dt === `${XSD}double` &&
    /^[+-]?(\d+\.\d*[eE][+-]?\d+|\.?\d+[eE][+-]?\d+)$/.test(t.value)
  ) {
    return t.value;
  }
  return `${quoted(t.value)}^^${p.iri(dt)}`;
}

// ---------------------------------------------------------------------------
// Path -> SCS path expression.
// ---------------------------------------------------------------------------

// Returns the compact path string, or undefined if the path uses a construct the
// compact grammar cannot express.
function pathToScs(node: ScsTerm, g: Graph, p: Prefixer): string | undefined {
  if (node.termType === "NamedNode") return p.iri(node.value);
  // Blank-node path: inspect its single structural predicate.
  const inverse = g.one(node, `${SH}inversePath`);
  if (inverse) {
    const inner = pathToScs(inverse, g, p);
    return inner === undefined ? undefined : `^${atomize(inner)}`;
  }
  const alt = g.one(node, `${SH}alternativePath`);
  if (alt) {
    const members = g.list(alt).map((m) => pathToScs(m, g, p));
    if (members.some((m) => m === undefined)) return undefined;
    return members.map((m) => atomize(m as string)).join(" | ");
  }
  const zeroOrMore = g.one(node, `${SH}zeroOrMorePath`);
  if (zeroOrMore) {
    const inner = pathToScs(zeroOrMore, g, p);
    return inner === undefined ? undefined : `${atomize(inner)}*`;
  }
  const oneOrMore = g.one(node, `${SH}oneOrMorePath`);
  if (oneOrMore) {
    const inner = pathToScs(oneOrMore, g, p);
    return inner === undefined ? undefined : `${atomize(inner)}+`;
  }
  const zeroOrOne = g.one(node, `${SH}zeroOrOnePath`);
  if (zeroOrOne) {
    const inner = pathToScs(zeroOrOne, g, p);
    return inner === undefined ? undefined : `${atomize(inner)}?`;
  }
  // Otherwise the blank node is the head of an RDF list = a sequence path.
  const seq = g.list(node);
  if (seq.length > 0) {
    const members = seq.map((m) => pathToScs(m, g, p));
    if (members.some((m) => m === undefined)) return undefined;
    return members.map((m) => atomize(m as string)).join(" / ");
  }
  return undefined;
}

// Wrap a compound path in parens when it must bind as a primary inside a combinator.
function atomize(path: string): string {
  return /[ |/^*+?]/.test(path) ? `(${path})` : path;
}

// ---------------------------------------------------------------------------
// Constraint serialization.
// ---------------------------------------------------------------------------

// sh:* predicate -> compact `param` keyword for value constraints emitted as
// `param=value`. (Counts, paths, targets and node-kind handled separately.)
const VALUE_PARAMS: { iri: string; keyword: string; list?: boolean }[] = [
  { iri: `${SH}class`, keyword: "class" },
  { iri: `${SH}datatype`, keyword: "datatype" },
  { iri: `${SH}minExclusive`, keyword: "minExclusive" },
  { iri: `${SH}minInclusive`, keyword: "minInclusive" },
  { iri: `${SH}maxExclusive`, keyword: "maxExclusive" },
  { iri: `${SH}maxInclusive`, keyword: "maxInclusive" },
  { iri: `${SH}minLength`, keyword: "minLength" },
  { iri: `${SH}maxLength`, keyword: "maxLength" },
  { iri: `${SH}pattern`, keyword: "pattern" },
  { iri: `${SH}flags`, keyword: "flags" },
  { iri: `${SH}languageIn`, keyword: "languageIn", list: true },
  { iri: `${SH}uniqueLang`, keyword: "uniqueLang" },
  { iri: `${SH}equals`, keyword: "equals" },
  { iri: `${SH}disjoint`, keyword: "disjoint" },
  { iri: `${SH}lessThan`, keyword: "lessThan" },
  { iri: `${SH}lessThanOrEquals`, keyword: "lessThanOrEquals" },
  { iri: `${SH}hasValue`, keyword: "hasValue" },
  { iri: `${SH}in`, keyword: "in", list: true },
  { iri: `${SH}severity`, keyword: "severity" },
  { iri: `${SH}deactivated`, keyword: "deactivated" },
  { iri: `${SH}message`, keyword: "message" },
  { iri: `${SH}qualifiedMinCount`, keyword: "qualifiedMinCount" },
  { iri: `${SH}qualifiedMaxCount`, keyword: "qualifiedMaxCount" },
];

const NODE_KIND_LOCAL: Record<string, string> = {
  [`${SH}BlankNode`]: "BlankNode",
  [`${SH}IRI`]: "IRI",
  [`${SH}Literal`]: "Literal",
  [`${SH}BlankNodeOrIRI`]: "BlankNodeOrIRI",
  [`${SH}BlankNodeOrLiteral`]: "BlankNodeOrLiteral",
  [`${SH}IRIOrLiteral`]: "IRIOrLiteral",
};

function valueOrArray(terms: ScsTerm[], g: Graph, p: Prefixer): string {
  // an `in` / `languageIn` value is the head of an RDF list -> `[ a b c ]`.
  const members = g.list(terms[0]).map((m) => termToScs(m, p));
  return `[ ${members.join(" ")} ]`;
}

// Emit the constraint lines of a shape (each terminated with " ."). On a property
// shape body these are reduced to inline params by stripping the trailing " .".
function shapeConstraints(
  shape: ScsTerm,
  g: Graph,
  p: Prefixer,
  unsupported: string[],
  shapeLabel: string,
): string[] {
  const lines: string[] = [];

  // node kind
  const nk = g.one(shape, `${SH}nodeKind`);
  if (nk && nk.termType === "NamedNode" && NODE_KIND_LOCAL[nk.value]) {
    lines.push(`${NODE_KIND_LOCAL[nk.value]} .`);
  }

  // closed shapes: `closed=true` with optional ignoredProperties.
  const closed = g.one(shape, `${SH}closed`);
  if (closed) {
    let line = `closed=${termToScs(closed, p)}`;
    const ignored = g.one(shape, `${SH}ignoredProperties`);
    if (ignored) {
      const props = g.list(ignored).map((m) => termToScs(m, p));
      line += ` ignoredProperties=[ ${props.join(" ")} ]`;
    }
    lines.push(`${line} .`);
  }

  // value/string/range/in/hasValue/class/datatype params -> `param=value .`
  for (const { iri, keyword, list } of VALUE_PARAMS) {
    const objs = g.objects(shape, iri);
    if (objs.length === 0) continue;
    if (list) {
      lines.push(`${keyword}=${valueOrArray(objs, g, p)} .`);
    } else {
      for (const o of objs) {
        lines.push(`${keyword}=${termToScs(o, p)} .`);
      }
    }
  }

  // Logical / shape-reference constraints the grammar cannot express compactly.
  for (const unsup of [
    `${SH}and`,
    `${SH}or`,
    `${SH}xone`,
    `${SH}not`,
    `${SH}node`,
    `${SH}qualifiedValueShape`,
  ]) {
    if (g.objects(shape, unsup).length > 0) {
      const local = unsup.slice(SH.length);
      unsupported.push(
        `${shapeLabel}: sh:${local} has no SHACL Compact Syntax form — omitted.`,
      );
    }
  }

  return lines;
}

// ---------------------------------------------------------------------------
// Top-level shape serialization.
// ---------------------------------------------------------------------------

function isPropertyShape(shape: ScsTerm, g: Graph): boolean {
  return g.objects(shape, `${SH}path`).length > 0;
}

function serializeProperty(
  prop: ScsTerm,
  g: Graph,
  p: Prefixer,
  unsupported: string[],
  shapeLabel: string,
): string | undefined {
  const pathTerm = g.one(prop, `${SH}path`);
  if (!pathTerm) return undefined;
  const path = pathToScs(pathTerm, g, p);
  if (path === undefined) {
    unsupported.push(
      `${shapeLabel}: a property path is not expressible in compact syntax — omitted.`,
    );
    return undefined;
  }

  const parts: string[] = [path];

  // count: `[min..max]` (only when at least one is set).
  const minTerm = g.one(prop, `${SH}minCount`);
  const maxTerm = g.one(prop, `${SH}maxCount`);
  // The count range takes an INTEGER token (`[+-]?[0-9]+`) whose value is non-negative.
  const isCount = (t: ScsTerm): boolean =>
    t.termType === "Literal" && /^(?:\+?\d+|-0+)$/.test(t.value);
  if ([minTerm, maxTerm].some((t) => t && !isCount(t))) {
    unsupported.push(
      `${shapeLabel}: a sh:minCount/sh:maxCount that is not a non-negative integer — property omitted.`,
    );
    return undefined;
  }
  if (minTerm || maxTerm) {
    const min = minTerm ? minTerm.value : "0";
    const max = maxTerm ? maxTerm.value : "*";
    parts.push(`[${min}..${max}]`);
  }

  // the remaining param constraints (datatype, class, nodeKind, value ranges, ...)
  const body = shapeConstraints(prop, g, p, unsupported, shapeLabel);
  for (const line of body) {
    parts.push(line.replace(/ \.$/, ""));
  }

  return `\t${parts.join(" ")} .`;
}

function serializeNodeShape(
  shape: ScsTerm,
  g: Graph,
  p: Prefixer,
  unsupported: string[],
): string {
  const label = shape.termType === "NamedNode" ? p.iri(shape.value) : blankNode(shape.value);
  const shapeLabel = `shape ${label}`;

  // targets: `-> class1 class2` for sh:targetClass; sh:targetNode/objectsOf/subjectsOf
  // are emitted as params.
  const targetClasses = g.objects(shape, `${SH}targetClass`);
  let header = `shape ${label}`;
  if (targetClasses.length > 0) {
    header += ` -> ${targetClasses.map((t) => termToScs(t, p)).join(" ")}`;
  }

  const bodyLines: string[] = [];

  // non-targetClass targets as params
  for (const [iri, kw] of [
    [`${SH}targetNode`, "targetNode"],
    [`${SH}targetObjectsOf`, "targetObjectsOf"],
    [`${SH}targetSubjectsOf`, "targetSubjectsOf"],
  ] as const) {
    for (const o of g.objects(shape, iri)) {
      bodyLines.push(`\t${kw}=${termToScs(o, p)} .`);
    }
  }

  // node-level constraints on the shape itself
  for (const line of shapeConstraints(shape, g, p, unsupported, shapeLabel)) {
    bodyLines.push(`\t${line}`);
  }

  // property shapes attached via sh:property
  for (const propRef of g.objects(shape, `${SH}property`)) {
    const line = serializeProperty(propRef, g, p, unsupported, shapeLabel);
    if (line) bodyLines.push(line);
  }

  if (bodyLines.length === 0) return `${header} {\n}`;
  return `${header} {\n${bodyLines.join("\n")}\n}`;
}

// ---------------------------------------------------------------------------
// Public entry point.
// ---------------------------------------------------------------------------

/**
 * Serialize a SHACL shapes graph (a flat triple list) to SHACL Compact Syntax.
 *
 * Honest + lossless-or-reported: any shape or constraint the compact grammar can
 * not express is recorded in `result.unsupported` rather than dropped silently.
 */
export function shapesToCompact(
  triples: ScsTriple[],
  prefixes: ScsPrefix[] = [],
): ScsResult {
  const g = new Graph(triples);
  const p = new Prefixer(prefixes);
  const unsupported: string[] = [];

  // collect the distinct subjects in document order.
  const subjectKeys = new Set<string>();
  const subjects: ScsTerm[] = [];
  for (const t of triples) {
    const k = termKey(t.subject);
    if (!subjectKeys.has(k)) {
      subjectKeys.add(k);
      subjects.push(t.subject);
    }
  }

  const shapeBlocks: string[] = [];
  for (const s of subjects) {
    if (s.termType !== "NamedNode") continue; // only named shapes at top level
    if (isPropertyShape(s, g)) continue; // a bare property shape is not a node-shape block

    const types = g.objects(s, RDF_TYPE).map((t) => t.value);
    const isNodeShape = types.includes(`${SH}NodeShape`);
    const hasShapeStuff =
      g.objects(s, `${SH}targetClass`).length > 0 ||
      g.objects(s, `${SH}targetNode`).length > 0 ||
      g.objects(s, `${SH}targetObjectsOf`).length > 0 ||
      g.objects(s, `${SH}targetSubjectsOf`).length > 0 ||
      g.objects(s, `${SH}property`).length > 0 ||
      g.objects(s, `${SH}closed`).length > 0 ||
      g.objects(s, `${SH}nodeKind`).length > 0;

    if (!isNodeShape && !hasShapeStuff) continue;
    shapeBlocks.push(serializeNodeShape(s, g, p, unsupported));
  }

  const prefixLines = p.prefixLines();
  const parts: string[] = [];
  if (prefixLines.length > 0) parts.push(prefixLines.join("\n"));
  if (shapeBlocks.length > 0) parts.push(shapeBlocks.join("\n\n"));

  // de-dup unsupported notes while preserving order
  const seen = new Set<string>();
  const uniqueUnsupported = unsupported.filter((u) =>
    seen.has(u) ? false : (seen.add(u), true),
  );

  return {
    text: parts.join("\n\n") + (parts.length ? "\n" : ""),
    unsupported: uniqueUnsupported,
  };
}
