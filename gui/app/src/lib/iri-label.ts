// The GUI's one IRI-shortening rule for display captions (graph-view node/edge labels, the
// SELECT-result graph, proof-term captions). Display only, never identity.

import { COMMON_PREFIXES } from "@sparq/client";

/** Abbreviate an IRI with a common prefix (`foaf:name`), else shorten it to its fragment / last
 *  path segment so a bare IRI is still legible. An IRI ending in `#` or `/` is returned whole. */
export function abbreviateIri(iri: string): string {
  for (const { prefix, iri: ns } of COMMON_PREFIXES) {
    if (iri.startsWith(ns)) return `${prefix}:${iri.slice(ns.length)}`;
  }
  const cut = Math.max(iri.lastIndexOf("#"), iri.lastIndexOf("/"));
  return cut >= 0 && cut < iri.length - 1 ? iri.slice(cut + 1) : iri;
}
