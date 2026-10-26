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
  return `${prefix}${/[ \t\r\n]$/.test(prefix) ? "" : "\n"}${fromNamed}\n${sparql.slice(whereIdx)}`;
}

/** Locate WHERE or ASK's group, ignoring comments, IRIs, literals and nested expressions. */
function findWhereIndex(sparql: string): number {
  // This scans boundaries, not highlighting tokens: comparison '<' is not an IRI,
  // and escaped punctuation in prefixed names must not open strings or comments.
  const iri = /<(?:[^\u0000-\u0020<>"{}|^`\\]|\\u[\da-fA-F]{4}|\\U[\da-fA-F]{8})*>/y;
  const word = /[?$]?(?:\\[^\r\n]|[^ \t\r\n{}()[\]<>"';,=!?$+*/|&^#\\])+/y;
  let depth = 0;
  let ask = false;
  for (let offset = 0; offset < sparql.length;) {
    const char = sparql[offset];
    if (char === "#") {
      while (offset < sparql.length && !/[\r\n]/.test(sparql[offset])) offset += 1;
      continue;
    }
    if (char === '"' || char === "'") {
      const delimiter = sparql.startsWith(char.repeat(3), offset) ? char.repeat(3) : char;
      offset += delimiter.length;
      while (offset < sparql.length) {
        if (sparql[offset] === "\\") offset += 2;
        else if (sparql.startsWith(delimiter, offset)) {
          offset += delimiter.length;
          break;
        } else offset += 1;
      }
      continue;
    }
    iri.lastIndex = offset;
    if (char === "<" && iri.test(sparql)) {
      offset = iri.lastIndex;
      continue;
    }
    word.lastIndex = offset;
    const token = word.exec(sparql);
    if (token) {
      if (depth === 0) {
        const keyword = token[0].toUpperCase();
        if (keyword === "ASK") ask = true;
        if (keyword === "WHERE") return offset;
      }
      offset = word.lastIndex;
      continue;
    }
    if (char === "{" && depth === 0 && ask) return offset;
    if (char === "{" || char === "(") depth += 1;
    if (char === "}" || char === ")") depth -= 1;
    offset += 1;
  }
  return -1;
}
