// [review #3601] The CHEAP eligibility predicate behind the home hero's Table | Graph toggle.
//
// Split out from result-graph.ts (deriveGraph) on purpose. The home hero-runner chunk only needs to
// decide WHETHER to offer the Graph toggle; it must NOT statically pull in the full node-link
// derivation (result-graph.ts) or the SVG renderer (repl-graph-view.tsx) — those are a rarely-used,
// net-new view that must arrive on the invocation path (site bundle policy). This module imports
// nothing heavy, so the hero chunk carries only this cheap scan (a bounded Set of at most
// MAX_GRAPH_NODES term keys — no node/edge maps); deriveGraph + the SVG renderer load lazily
// (`next/dynamic` with SSR disabled on ResultGraphView) the first time a visitor switches to Graph.
//
// SINGLE SOURCE OF TRUTH: deriveGraph uses this as its precondition gate AND this predicate
// exactly models deriveGraph's capped admission (the first MAX_GRAPH_NODES distinct terms in scan
// order; only edges between ADMITTED endpoints and only ADMITTED resource nodes count), so the
// toggle's "is this graph-shaped?" answer and the derivation's own decline can never drift — not
// even at the >MAX_GRAPH_NODES cap boundary, where a result's only qualifying edge/resource can
// sit entirely past the cap and must therefore NOT be offered a Graph view it cannot draw.

import type { SparqlResults, SparqlTerm } from "./sparq-wasm";

/** The most nodes the compact node-link view draws; beyond this the caller shows a truncation note. */
export const MAX_GRAPH_NODES = 24;

/**
 * A stable, INJECTIVE identity key for a term: same lexical value but different datatype/lang/dir ⇒
 * different node. Shared with deriveGraph (result-graph.ts) so the predicate's admission order and
 * the derivation's node identity can never disagree.
 */
export function termKey(t: SparqlTerm): string {
  const dt = t.type === "literal" ? (t.datatype ?? "") : "";
  const lang = t.type === "literal" ? (t["xml:lang"] ?? "") : "";
  // RDF 1.2 base direction is part of the literal's identity (`"hi"@en--ltr` ≠ `"hi"@en`).
  const dir = t.type === "literal" ? (t["its:dir"] ?? "") : "";
  return JSON.stringify([t.type, t.value, dt, lang, dir]);
}

/**
 * Cheap eligibility check: does this result have the entity-relationship shape the Graph view draws
 * — ≥2 columns, ≥1 row, at least one DRAWABLE edge (two adjacent BOUND columns bound to DIFFERENT
 * admitted terms), and at least one admitted RESOURCE (uri/bnode) node? "Admitted" replays
 * deriveGraph's node cap: the first {@link MAX_GRAPH_NODES} distinct term keys in `head.vars` × row
 * scan order, so an edge or resource whose terms all fall past the cap (and would be dropped from
 * the drawing) never makes a result eligible. Exactly models deriveGraph's decline conditions
 * WITHOUT building the node/edge maps; early-exits as soon as both an edge and a resource are seen.
 */
export function isGraphShaped(results: SparqlResults): boolean {
  const vars = results.head?.vars ?? [];
  const rows = results.results?.bindings ?? [];
  if (vars.length < 2 || rows.length === 0) return false;

  const admitted = new Set<string>();
  let hasEdge = false;
  let hasResource = false;
  for (const row of rows) {
    let prevKey: string | null = null;
    let prevAdmitted = false;
    for (const v of vars) {
      const t = row[v];
      if (!t) continue; // unbound cell: skip so an OPTIONAL gap never breaks the adjacency chain
      const key = termKey(t);
      let ok = admitted.has(key);
      if (!ok && admitted.size < MAX_GRAPH_NODES) {
        admitted.add(key);
        ok = true;
      }
      if (!hasResource && ok && t.type !== "literal") hasResource = true;
      // A drawable edge needs BOTH endpoints admitted and DIFFERENT terms (a row binding two
      // adjacent columns to one term is a self-loop, which draws no edge).
      if (!hasEdge && ok && prevAdmitted && prevKey !== key) hasEdge = true;
      prevKey = key;
      prevAdmitted = ok;
    }
    if (hasEdge && hasResource) return true;
  }
  return false;
}
