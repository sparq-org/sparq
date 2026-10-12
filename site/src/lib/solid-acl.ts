// [OPUS-4.8] sq-4r4b — the in-tab FROM NAMED enforcement for the Solid flagship.
//
// This is a faithful mirror of `sparq-solid`'s `rewrite_for(sparql, allowed)` transform
// (crates/sparq-solid/src/rewrite.rs:67): given a query and the authorized named-graph
// set for an (agent, client) pair, inject a `FROM NAMED <g>` clause for each authorized
// graph so the query is evaluated over EXACTLY those graphs and no others. The empty set
// maps to a guaranteed-absent sentinel graph (`<urn:sparq:nothing>`) so the result stays
// FAIL-CLOSED — zero rows, never an accidental union-of-everything.
//
// The wasm engine then runs the rewritten query for real. The access-control DECISION
// (which graphs are authorized) is the materialized `sparq-solid` output, precomputed at
// build time per the honesty framing; the RESTRICTION is the real SPARQL engine.

import { termToNTriples } from "@sparq/client";

/** Guaranteed-absent graph used when a session has no authorized graphs (fail-closed). */
export const NOTHING_SENTINEL = "urn:sparq:nothing";

/**
 * Rewrite `sparql` so it is evaluated only over `allowed` named graphs, by injecting a
 * `FROM NAMED` dataset clause. Mirrors sparq-solid's `rewrite_for`.
 *
 * The clause is inserted after the SELECT projection and before the WHERE clause, which
 * is where SPARQL 1.1 expects dataset clauses. An empty `allowed` set injects the
 * sentinel graph so the query matches nothing (no grant ⇒ no data). Each graph IRI goes
 * through the shared `termToNTriples` writer, which throws on a character an IRIREF cannot
 * hold, so a graph name cannot close its `<…>` early and add clauses to the query (#6719).
 */
export function rewriteForGraphs(sparql: string, allowed: string[]): string {
  const graphs = allowed.length > 0 ? allowed : [NOTHING_SENTINEL];
  const fromNamed = graphs
    .map((g) => `FROM NAMED ${termToNTriples({ type: "uri", value: g })}`)
    .join("\n");

  // Find the WHERE keyword (case-insensitive) at a clause boundary and insert before it.
  const whereIdx = findWhereIndex(sparql);
  if (whereIdx === -1) {
    // No WHERE found (shouldn't happen for our SELECT) — append defensively.
    return `${sparql}\n${fromNamed}`;
  }
  return `${sparql.slice(0, whereIdx)}${fromNamed}\n${sparql.slice(whereIdx)}`;
}

/** Locate the top-level WHERE keyword so the dataset clause goes in the right place. */
function findWhereIndex(sparql: string): number {
  const re = /\bWHERE\b/gi;
  let m: RegExpExecArray | null;
  while ((m = re.exec(sparql)) !== null) {
    // Skip matches inside a comment line.
    const lineStart = sparql.lastIndexOf("\n", m.index) + 1;
    const linePrefix = sparql.slice(lineStart, m.index);
    if (!linePrefix.includes("#")) return m.index;
  }
  return -1;
}
