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

import { termToNTriples, tokenizeSparql } from "@sparq/client";

/** Guaranteed-absent graph used when a session has no authorized graphs (fail-closed). */
export const NOTHING_SENTINEL = "urn:sparq:nothing";

/**
 * Rewrite `sparql` so it is evaluated only over `allowed` named graphs, by injecting a
 * `FROM NAMED` dataset clause. Mirrors sparq-solid's `rewrite_for`.
 *
 * The clause is inserted before WHERE (or an ASK's opening group when WHERE is omitted),
 * where SPARQL 1.1 expects dataset clauses. An empty `allowed` set injects the
 * sentinel graph so the query matches nothing (no grant ⇒ no data). Each graph IRI goes
 * through the shared `termToNTriples` writer, which throws on a character an IRIREF cannot
 * hold, so a graph name cannot close its `<…>` early and add clauses to the query (#6719).
 */
export function rewriteForGraphs(sparql: string, allowed: string[]): string {
  const graphs = allowed.length > 0 ? allowed : [NOTHING_SENTINEL];
  const fromNamed = graphs
    .map((g) => `FROM NAMED ${termToNTriples({ type: "uri", value: g })}`)
    .join("\n");

  // Find the pattern boundary, including ASK's legal omission of WHERE.
  const whereIdx = findWhereIndex(sparql);
  if (whereIdx === -1) {
    // Leave unsupported or incomplete forms for the engine to diagnose.
    return `${sparql}\n${fromNamed}`;
  }
  const prefix = sparql.slice(0, whereIdx);
  return `${prefix}${/\s$/.test(prefix) ? "" : "\n"}${fromNamed}\n${sparql.slice(whereIdx)}`;
}

/** Locate WHERE or ASK's group, ignoring comments, IRIs, literals and nested expressions. */
function findWhereIndex(sparql: string): number {
  let offset = 0;
  let depth = 0;
  let ask = false;
  for (const token of tokenizeSparql(sparql)) {
    if (depth === 0 && token.type === "keyword") {
      const keyword = token.text.toUpperCase();
      if (keyword === "ASK") ask = true;
      if (keyword === "WHERE") return offset;
    }
    if (token.type === "punctuation") {
      if (token.text === "{" && depth === 0 && ask) return offset;
      if (token.text === "{" || token.text === "(") depth += 1;
      if (token.text === "}" || token.text === ")") depth -= 1;
    }
    offset += token.text.length;
  }
  return -1;
}
