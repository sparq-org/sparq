// [OPUS-4.8] sq-n5aw — framework-agnostic common-prefix support for the SPARQL editor.
//
// The editor uplift (`research/gui-design.md` §2, MVP item 1) calls for "prefix awareness".
// This module is the dependency-free, DOM-free core of that: a curated registry of well-known
// RDF vocabulary prefixes, plus pure helpers to (a) find which well-known prefixes a query
// USES but does not DECLARE, and (b) render the PREFIX lines to prepend. No React, no DOM — so
// the same logic serves the Next.js site and the Tauri 2 webview, and is unit-testable in
// `node:test`. It takes no opinion on UI; a component decides how to surface the suggestion.

import { isWritablePrefixBinding } from "./rdf-syntax.js";

/** A well-known prefix → namespace IRI binding. */
export interface PrefixBinding {
  prefix: string;
  iri: string;
}

/**
 * The curated registry of common RDF vocabulary prefixes the editor can offer. These are the
 * widely-used, stable namespaces (the same families the site's built-in datasets and example
 * queries use: `ex:`/`foaf:`/`rdf:`/`rdfs:`/`xsd:`/`dc:`). Ordered roughly by ubiquity so a
 * "insert all common prefixes" affordance produces a sensible block. Not exhaustive by design
 * — it is a STARTER set for the playground, not a vocabulary directory.
 */
export const COMMON_PREFIXES: readonly PrefixBinding[] = [
  { prefix: "rdf", iri: "http://www.w3.org/1999/02/22-rdf-syntax-ns#" },
  { prefix: "rdfs", iri: "http://www.w3.org/2000/01/rdf-schema#" },
  { prefix: "owl", iri: "http://www.w3.org/2002/07/owl#" },
  { prefix: "xsd", iri: "http://www.w3.org/2001/XMLSchema#" },
  { prefix: "foaf", iri: "http://xmlns.com/foaf/0.1/" },
  { prefix: "dc", iri: "http://purl.org/dc/terms/" },
  { prefix: "dcterms", iri: "http://purl.org/dc/terms/" },
  { prefix: "skos", iri: "http://www.w3.org/2004/02/skos/core#" },
  { prefix: "schema", iri: "https://schema.org/" },
  { prefix: "sh", iri: "http://www.w3.org/ns/shacl#" },
  { prefix: "prov", iri: "http://www.w3.org/ns/prov#" },
  { prefix: "geo", iri: "http://www.opengis.net/ont/geosparql#" },
  { prefix: "void", iri: "http://rdfs.org/ns/void#" },
  { prefix: "ex", iri: "http://example.org/" },
];

// `PREFIX foo: <iri>` declaration matcher (case-insensitive `PREFIX`, tolerant of the empty
// prefix `:`). Used to read which prefixes a query already declares.
const DECLARED_RE = /(?:^|\s)PREFIX\s+([A-Za-z][\w.-]*)?:\s*<[^>]*>/gi;

// As above, but also CAPTURES the namespace IRI between the angle brackets, so a query's full
// `prefix -> IRI` bindings can be recovered (e.g. to abbreviate result IRIs with the user's own
// prefixes). Group 1 is the (optional) label, group 2 the IRI.
const DECLARED_WITH_IRI_RE = /(?:^|\s)PREFIX\s+([A-Za-z][\w.-]*)?:\s*<([^>]*)>/gi;

// A used prefixed-name's prefix label, e.g. `foaf` in `foaf:name`. Excludes IRIs, variables,
// and the `_:bnode` form. The label must start a token (preceded by start/whitespace/`{(,;.[`).
const USED_RE = /(?:^|[\s{(,;.[])([A-Za-z][\w.-]*):[A-Za-z_]/g;

/** The set of prefix labels a query DECLARES (via `PREFIX foo: <…>`). The empty prefix `:`
 * is represented as the empty string. */
export function declaredPrefixes(query: string): Set<string> {
  const out = new Set<string>();
  for (const m of query.matchAll(DECLARED_RE)) out.add(m[1] ?? "");
  return out;
}

/**
 * The full `prefix -> namespace-IRI` bindings a query DECLARES (via `PREFIX foo: <…>`). The
 * empty prefix `:` is represented by the empty-string label. First declaration of a label wins.
 * Best-effort over the highlight grammar (not a full parse) — intended for re-using the user's
 * own prefixes when abbreviating result IRIs (see `prettyTurtle`). [OPUS-4.8] sq-gb4o (#805)
 *
 * A declaration whose label is not a `PN_PREFIX` or whose IRI holds a character IRIREF forbids
 * is skipped, so every returned binding can be written back as a declaration unchanged (#6719).
 */
export function declaredPrefixBindings(query: string): PrefixBinding[] {
  const out: PrefixBinding[] = [];
  const seen = new Set<string>();
  for (const m of query.matchAll(DECLARED_WITH_IRI_RE)) {
    const binding = { prefix: m[1] ?? "", iri: m[2] };
    if (seen.has(binding.prefix) || !isWritablePrefixBinding(binding)) continue;
    seen.add(binding.prefix);
    out.push(binding);
  }
  return out;
}

/** The set of prefix labels a query USES (a prefixed name `foo:bar`). Best-effort over the
 * highlight grammar, not a full parse; intended for the "you used `foaf:` but didn't declare
 * it" hint. */
export function usedPrefixes(query: string): Set<string> {
  const out = new Set<string>();
  for (const m of query.matchAll(USED_RE)) out.add(m[1]);
  return out;
}

/**
 * The well-known prefixes a query USES but has not DECLARED — i.e. the {@link COMMON_PREFIXES}
 * the editor can offer to insert. Returns them in {@link COMMON_PREFIXES} order. A prefix the
 * query uses but that is NOT in the common registry is omitted (the editor has no IRI to
 * suggest for it).
 */
export function missingCommonPrefixes(query: string): PrefixBinding[] {
  const declared = declaredPrefixes(query);
  const used = usedPrefixes(query);
  return COMMON_PREFIXES.filter((b) => used.has(b.prefix) && !declared.has(b.prefix));
}

/** Render `PREFIX foo: <iri>` lines for the given bindings (one per line, no trailing
 * newline). Throws when a label is not a `PN_PREFIX` or an IRI holds a character IRIREF
 * forbids, rather than emit a declaration that ends early (#6719). */
export function renderPrefixLines(bindings: readonly PrefixBinding[]): string {
  return bindings
    .map((b) => {
      if (!isWritablePrefixBinding(b)) {
        throw new Error(
          `renderPrefixLines: ${JSON.stringify(b.prefix)} -> ${JSON.stringify(b.iri)} is not a writable PREFIX declaration`,
        );
      }
      return `PREFIX ${b.prefix}: <${b.iri}>`;
    })
    .join("\n");
}

/**
 * Prepend the given prefix declarations to a query, skipping any the query already declares,
 * and separating the inserted block from the body with a blank line. Returns the query
 * unchanged when there is nothing new to add. Pure — does not mutate its input. Throws as
 * {@link renderPrefixLines} does for a binding that cannot be declared as written.
 */
export function withPrefixes(query: string, bindings: readonly PrefixBinding[]): string {
  const declared = declaredPrefixes(query);
  const toAdd = bindings.filter((b) => !declared.has(b.prefix));
  if (toAdd.length === 0) return query;
  const block = renderPrefixLines(toAdd);
  const body = query.replace(/^\s+/, ""); // trim leading whitespace so the block sits flush
  return body.length > 0 ? `${block}\n\n${body}` : block;
}
