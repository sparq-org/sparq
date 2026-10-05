// [FABLE-5] sq-ixc3.20 — pure term/triple identity logic for marking INFERRED facts in the
// results views (no React, no DOM), unit-tested with the gui/app node test runner.
//
// The problem this file owns: the closure text the reasoner emits (Rust N-Triples writer),
// the base-store snapshot (the JS `termToNT` writer below), the graph view's parsed
// `RdfTerm`s (verbatim-escaped `parseNTriples` slices), and the table view's SPARQL-JSON
// `SparqlTerm`s (DECODED lexical forms) are FOUR spellings of the same triples. Deciding
// "is this row/edge inferred?" by comparing raw serialisations would break the moment two
// writers disagree on an escape or on `^^xsd:string` suppression — so every source is
// reduced to ONE canonical, decoded triple key here, and the entailed set is a `Set` of
// those keys. Membership is therefore exact: an affordance can never appear on a fact the
// closure does not actually add (false positives are structurally impossible). RDF 1.2
// triple terms are keyed recursively from their decoded components, never from their text.

import { parseNTriples, termToNTriples, type RdfTerm, type SparqlTerm } from "@sparq/client";

const XSD_STRING = "http://www.w3.org/2001/XMLSchema#string";
/** Key-field separator: a control char no IRI / lang tag / decoded lexical form contains
 *  un-escaped ambiguity with (it may appear IN a literal, but then it appears identically
 *  on both sides of any comparison — the key stays injective per term kind). */
const SEP = "";

/** [GPT-5.6] One closure-added fact in the exact N-Triples term form accepted by why(). */
export interface InferredFact {
  /** Canonical decoded identity used by every inferred-fact affordance. */
  key: string;
  s: string;
  p: string;
  o: string;
  /** A displayable N-Triples statement (the reasoner folds named graphs to s/p/o). */
  ntriples: string;
}

/** [GPT-5.6] The exact closure-minus-base result retained for UI membership + browsing. */
export interface EntailedFacts {
  keys: Set<string>;
  facts: InferredFact[];
}

// ---------------------------------------------------------------------------
// N-Triples term writing (the engine wire shape) — moved here from engine-context so the
// click-to-explain path and the snapshot writer share ONE writer.
// ---------------------------------------------------------------------------

/**
 * Emit a single canonical N-Triples/N-Quads TERM (IRI / blank node / literal / RDF 1.2
 * triple term) from a SPARQL-JSON term. Unlike a DISPLAY helper this writes the FULL
 * datatype IRI and escapes the lexical form, so it round-trips losslessly through the
 * engine's parsers — and it is exactly the term form the reasoner's `why()` expects for the
 * clicked triple. Delegates to the shared `@sparq/client` writer so every snapshot writer
 * handles every term kind (#6044).
 */
export function termToNT(t: SparqlTerm): string {
  return termToNTriples(t);
}

// ---------------------------------------------------------------------------
// Canonical (decoded) term keys.
// ---------------------------------------------------------------------------

/**
 * Decode the N-Triples string escapes (`\\`, `\"`, `\n`, `\r`, `\t`, `\b`, `\f`, `\uXXXX`,
 * `\UXXXXXXXX`) of a verbatim-escaped lexical form. An unrecognised escape is kept verbatim
 * (never dropped), so a malformed input still yields a deterministic key.
 */
export function unescapeNT(s: string): string {
  if (!s.includes("\\")) return s;
  let out = "";
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c !== "\\" || i === s.length - 1) {
      out += c;
      continue;
    }
    const e = s[i + 1];
    if (e === "\\" || e === '"' || e === "'") {
      out += e;
      i++;
    } else if (e === "n") {
      out += "\n";
      i++;
    } else if (e === "r") {
      out += "\r";
      i++;
    } else if (e === "t") {
      out += "\t";
      i++;
    } else if (e === "b") {
      out += "\b";
      i++;
    } else if (e === "f") {
      out += "\f";
      i++;
    } else if (e === "u" || e === "U") {
      const width = e === "u" ? 4 : 8;
      const hex = s.slice(i + 2, i + 2 + width);
      if (hex.length === width && /^[0-9A-Fa-f]+$/.test(hex)) {
        out += String.fromCodePoint(Number.parseInt(hex, 16));
        i += 1 + width;
      } else {
        out += c; // malformed escape: keep verbatim
      }
    } else {
      out += c; // unrecognised escape: keep verbatim
    }
  }
  return out;
}

/**
 * Key of a decoded literal: lexical form, then either `@lang[--dir]` (lang lower-cased, the
 * RDF 1.2 base direction kept as its own component) or the datatype (`xsd:string` implicit).
 */
function literalKey(
  value: string,
  lang: string | undefined,
  dir: string | undefined,
  datatype: string | undefined,
): string {
  if (lang) {
    // An N-Triples `@en--ltr` tag carries the direction inline; split it off.
    const cut = lang.indexOf("--");
    const tag = cut < 0 ? lang : lang.slice(0, cut);
    const direction = dir ?? (cut < 0 ? undefined : lang.slice(cut + 2));
    return `L${SEP}${value}${SEP}@${tag.toLowerCase()}${direction ? `--${direction}` : ""}`;
  }
  return `L${SEP}${value}${SEP}${datatype ?? XSD_STRING}`;
}

/** Key of an RDF 1.2 triple term from its three component keys (JSON keeps it injective). */
function tripleTermKey(s: string, p: string, o: string): string {
  return `T${SEP}${JSON.stringify([s, p, o])}`;
}

/** Canonical key of a parsed (verbatim-escaped) `RdfTerm` from {@link parseNTriples}. */
export function keyOfRdfTerm(t: RdfTerm): string {
  switch (t.kind) {
    case "iri":
      return `I${SEP}${unescapeNT(t.value)}`;
    case "bnode":
      return `B${SEP}${t.label}`;
    case "literal":
      return literalKey(unescapeNT(t.value), t.lang, undefined, t.datatype);
    case "triple":
      // RDF 1.2 triple term: keyed recursively from its decoded components, never from its
      // serialised `nt` (writers disagree on escapes such as a raw TAB vs `\t`).
      return tripleTermKey(keyOfRdfTerm(t.s), keyOfRdfTerm(t.p), keyOfRdfTerm(t.o));
  }
}

/** Canonical key of a SPARQL-JSON term (already-decoded lexical form). */
export function keyOfSparqlTerm(t: SparqlTerm): string {
  if (t.type === "uri") return `I${SEP}${t.value}`;
  if (t.type === "bnode") return `B${SEP}${t.value}`;
  if (t.type === "triple") {
    const { subject, predicate, object } = t.value;
    return tripleTermKey(
      keyOfSparqlTerm(subject),
      keyOfSparqlTerm(predicate),
      keyOfSparqlTerm(object),
    );
  }
  // SPARQL 1.2 results carry the base direction as a separate `its:dir` field.
  return literalKey(t.value, t["xml:lang"], t["its:dir"], t.datatype);
}

/** Canonical key of a whole triple from three SPARQL-JSON terms. */
export function tripleKeyOfBindings(s: SparqlTerm, p: SparqlTerm, o: SparqlTerm): string {
  return `${keyOfSparqlTerm(s)} ${keyOfSparqlTerm(p)} ${keyOfSparqlTerm(o)}`;
}

/** Canonical key of a whole triple from three parsed `RdfTerm`s (the graph view's shape). */
export function tripleKeyOfTerms(s: RdfTerm, p: RdfTerm, o: RdfTerm): string {
  return `${keyOfRdfTerm(s)} ${keyOfRdfTerm(p)} ${keyOfRdfTerm(o)}`;
}

/** [GPT-5.6] Preserve a parsed statement in the term spelling required by the proof API. */
function inferredFactOfTerms(s: RdfTerm, p: RdfTerm, o: RdfTerm): InferredFact {
  return {
    key: tripleKeyOfTerms(s, p, o),
    s: s.nt,
    p: p.nt,
    o: o.nt,
    ntriples: `${s.nt} ${p.nt} ${o.nt} .`,
  };
}

// ---------------------------------------------------------------------------
// Entailed-set construction (fed by the closure build in engine-context).
// ---------------------------------------------------------------------------

/**
 * The canonical triple keys of every statement in an N-Triples/N-Quads document (a named
 * graph term, if present, is IGNORED — the reasoner folds named graphs into the default
 * graph, so identity is s/p/o). Unparseable lines are skipped (they cannot be clicked as
 * facts either).
 */
export function tripleKeysOfNTriples(text: string): Set<string> {
  const keys = new Set<string>();
  if (!text.trim()) return keys;
  const { statements } = parseNTriples(text);
  for (const st of statements) keys.add(tripleKeyOfTerms(st.s, st.p, st.o));
  return keys;
}

/**
 * [GPT-5.6] Facts in an N-Triples/N-Quads document whose canonical identities are in
 * `includedKeys`. The first occurrence wins, so named-graph folding and duplicate closure
 * emission still produce one browser row per distinct inferred triple.
 */
export function inferredFactsMatchingKeys(
  text: string,
  includedKeys: ReadonlySet<string>,
): InferredFact[] {
  if (!text.trim() || includedKeys.size === 0) return [];
  const facts: InferredFact[] = [];
  const seen = new Set<string>();
  const { statements } = parseNTriples(text);
  for (const st of statements) {
    const fact = inferredFactOfTerms(st.s, st.p, st.o);
    if (!includedKeys.has(fact.key) || seen.has(fact.key)) continue;
    seen.add(fact.key);
    facts.push(fact);
  }
  return facts;
}

/**
 * [GPT-5.6] Compute closure minus base once while retaining both canonical membership keys and
 * displayable/provable N-Triples facts. This is the shared source for result affordances and the
 * Inference tool's entailed-facts browser.
 */
export function entailedFactsFromClosure(
  closureText: string,
  baseKeys: ReadonlySet<string>,
): EntailedFacts {
  const keys = new Set<string>();
  const facts: InferredFact[] = [];
  if (!closureText.trim()) return { keys, facts };
  const { statements } = parseNTriples(closureText);
  for (const st of statements) {
    const fact = inferredFactOfTerms(st.s, st.p, st.o);
    if (baseKeys.has(fact.key) || keys.has(fact.key)) continue;
    keys.add(fact.key);
    facts.push(fact);
  }
  return { keys, facts };
}

/**
 * The ENTAILED triple keys: every key of `closureText` that is not a key of `baseKeys`
 * (set difference — `closureText` is the reasoner's base+entailed output; what remains is
 * exactly what reasoning added).
 */
export function entailedKeysFromClosure(
  closureText: string,
  baseKeys: ReadonlySet<string>,
): Set<string> {
  const keys = tripleKeysOfNTriples(closureText);
  for (const key of baseKeys) keys.delete(key);
  return keys;
}
