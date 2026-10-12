// Grammar checks shared by the RDF / SPARQL text writers in this package (#6719).
//
// Every writer that interpolates a term or a prefix binding into Turtle, TriG, SPARQL or SHACL
// Compact Syntax validates the piece against these productions first, so a crafted value is
// rejected rather than allowed to end its token early. Dependency-free so `index.ts`,
// `sparql-prefixes.ts` and `pretty-turtle.ts` can all import it without an import cycle.

// A lone UTF-16 surrogate has no UTF-8 encoding and no UCHAR form, so it cannot be written.
export const LONE_SURROGATE =
  /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/;
// Characters IRIREF forbids unescaped (N-Triples, Turtle, SPARQL): controls, space, <>"{}|^`\.
export const IRI_FORBIDDEN = /[\u0000- <>"{}|^`\\]/;
// LANGTAG without the leading '@' (N-Triples, Turtle, SPARQL); an RDF 1.2 direction is separate.
export const LANGTAG = /^[a-zA-Z]+(?:-[a-zA-Z0-9]+)*$/;
// PN_CHARS_BASE, as a character-class body for a `u`-flag RegExp.
export const PN_CHARS_BASE =
  "A-Za-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF" +
  "\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD" +
  "\\u{10000}-\\u{EFFFF}";
// Turtle / SPARQL PN_CHARS. Unlike N-Triples, PN_CHARS_U here does not include ':'.
const PN_CHARS = `${PN_CHARS_BASE}_\\-0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040`;
// Turtle / SPARQL PN_PREFIX, or the empty prefix.
const PN_PREFIX = new RegExp(`^(?:[${PN_CHARS_BASE}](?:[${PN_CHARS}.]*[${PN_CHARS}])?)?$`, "u");
// Turtle / SPARQL BLANK_NODE_LABEL (the part after `_:`).
const TURTLE_BLANK_NODE_LABEL = new RegExp(
  `^[${PN_CHARS_BASE}_0-9](?:[${PN_CHARS}.]*[${PN_CHARS}])?$`,
  "u",
);

/** True when `value` can be written between `<` and `>` as an IRIREF without escaping. */
export function isWritableIri(value: string): boolean {
  return !IRI_FORBIDDEN.test(value) && !LONE_SURROGATE.test(value);
}

/** True when `value` holds a lone UTF-16 surrogate, which no RDF syntax can write. */
export function hasLoneSurrogate(value: string): boolean {
  return LONE_SURROGATE.test(value);
}

/** True when `tag` is a `LANGTAG` (without the leading `@`). */
export function isLangTag(tag: string): boolean {
  return LANGTAG.test(tag);
}

/** True when `label` is a Turtle / SPARQL `PN_PREFIX` (the empty string is the `:` prefix). */
export function isPrefixLabel(label: string): boolean {
  return PN_PREFIX.test(label);
}

/** True when `label` is a Turtle / SPARQL `BLANK_NODE_LABEL` (without the leading `_:`). */
export function isTurtleBlankNodeLabel(label: string): boolean {
  return TURTLE_BLANK_NODE_LABEL.test(label);
}

/** True when a `prefix -> namespace IRI` binding can be declared as written. */
export function isWritablePrefixBinding(b: { prefix: string; iri: string }): boolean {
  return isPrefixLabel(b.prefix) && isWritableIri(b.iri);
}
