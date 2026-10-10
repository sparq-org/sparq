"use client";

// [FABLE-5] sq-ixc3.15 — pure helpers + fixtures for the ODRL policy tool.
//
// The ODRL usage-control stack (the sparq-policy evaluator + sparq-solid's read rewrite)
// is NOT in the in-tab wasm bundle, so the tool is NATIVE-ONLY: the desktop shell
// runs the whole round-trip in one Rust command (`odrl_preview`, gui/src-tauri/src/odrl.rs) and
// the hosted web build degrades HONESTLY with the message below — never a fabricated decision.
//
// Everything here is a PURE function or a constant (no React, no IPC) so the pane-diff logic
// and the fail-closed messaging are unit-testable in the node test runner.

import type { SparqlResults } from "@sparq/client";

/**
 * One requester's pane — byte-for-byte the Rust `OdrlPane` (gui/src-tauri/src/odrl.rs): the
 * ODRL decision for the explicit request, the policy's read verdict per named graph
 * (`bridge_notes`, a historical name), and the gated result set (SPARQL 1.1 JSON) over the
 * graphs that verdict grants.
 */
export interface OdrlPane {
  requester: string;
  allow: boolean;
  matched_rules: string[];
  unmet_constraints: string[];
  bridge_notes: string[];
  results_json: string;
}

/**
 * The whole preview — byte-for-byte the Rust `OdrlPreview`. `policy_ok === false` means the
 * policy did not parse: NOTHING is granted (deny-everything, fail-closed) and
 * `policy_error` carries the verbatim parse error as the visible reason.
 */
export interface OdrlPreviewResult {
  policy_ok: boolean;
  policy_error: string | null;
  permissions: number;
  prohibitions: number;
  refused: boolean;
  ungated_json: string;
  panes: OdrlPane[];
}

/**
 * The honest web-build degradation message (the tools.ts taxonomy's "native-only" framing):
 * the ODRL evaluator + enforcement store are not in the in-tab wasm bundle, and pretending
 * otherwise would fabricate an access decision. Shown instead of running.
 */
export const WEB_ODRL_MESSAGE =
  "The ODRL policy tool is native-only: the in-tab WASM bundle ships neither the ODRL " +
  "evaluator (sparq-policy) nor the usage-control enforcement store (sparq-solid). Run this " +
  "tool in the sparq desktop app, where the whole evaluate-and-preview round-trip executes " +
  "on the native engine with fail-closed named-graph gating.";

/** The ODRL actions the request form offers (label → full IRI). */
export const ODRL_ACTIONS: ReadonlyArray<{ label: string; iri: string }> = [
  { label: "read", iri: "http://www.w3.org/ns/odrl/2/read" },
  { label: "modify", iri: "http://www.w3.org/ns/odrl/2/modify" },
  { label: "append", iri: "http://www.w3.org/ns/odrl/2/append" },
  { label: "delete", iri: "http://www.w3.org/ns/odrl/2/delete" },
];

/**
 * The self-contained sample dataset (TriG, TWO named graphs) the tool prefills so the gating
 * demo works out of the box: per-session visibility is decided PER NAMED GRAPH, so the data
 * must be graph-structured and the sample policy must target these graph IRIs.
 */
export const SAMPLE_DATASET_TRIG = `@prefix ex: <http://example.org/> .
ex:public {
  ex:doc1 ex:title "Public report" .
}
ex:secret {
  ex:doc2 ex:title "Secret memo" .
}
`;

/**
 * The sample Turtle ODRL policy: read permitted on BOTH graphs for anyone, with a prohibition
 * scoped to alice on the secret graph — so the prohibition VISIBLY flips a previously visible
 * named graph to hidden in alice's preview pane while bob keeps it.
 */
export const SAMPLE_POLICY_TTL = `@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix ex: <http://example.org/> .

ex:policy1 a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target ex:public ] ;
  odrl:permission [ odrl:action odrl:read ; odrl:target ex:secret ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target ex:secret ;
                     odrl:assignee ex:alice ] .
`;

/** The default explicit request target (the graph the sample prohibition contests). */
export const SAMPLE_TARGET = "http://example.org/secret";

/** The two default requesters the two-pane preview compares. */
export const SAMPLE_REQUESTERS: readonly [string, string] = [
  "http://example.org/alice",
  "http://example.org/bob",
];

/**
 * The SAME query every pane runs. `GRAPH ?g` because PodStore's default graph is EMPTY by
 * design (spec-conformant fail-closed default) — gating decides per named graph.
 */
export const SAMPLE_QUERY = `SELECT ?g ?s ?title WHERE {
  GRAPH ?g { ?s <http://example.org/title> ?title }
}`;

/**
 * The distinct values a variable binds in a SPARQL results document, in first-seen order.
 * Used to list the named graphs (`?g`) each pane can see.
 */
export function boundValues(results: SparqlResults, variable: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const row of results.results?.bindings ?? []) {
    const value = row[variable]?.value;
    if (typeof value === "string" && !seen.has(value)) {
      seen.add(value);
      out.push(value);
    }
  }
  return out;
}

/**
 * The named graphs visible UNGATED but hidden in a requester's gated pane — the tool's
 * headline diff ("what did the policy hide from this requester?"). Pure set difference over
 * the `?g` bindings, so an empty result honestly means "nothing was hidden".
 */
export function hiddenGraphs(
  ungated: SparqlResults,
  gated: SparqlResults,
  variable = "g",
): string[] {
  const visible = new Set(boundValues(gated, variable));
  return boundValues(ungated, variable).filter((g) => !visible.has(g));
}

/**
 * The visible fail-closed banner for a malformed policy: the deny-everything consequence
 * FIRST, then the parser's verbatim reason (never paraphrased, never hidden).
 */
export function describeMalformedPolicy(reason: string): string {
  return (
    "Malformed policy — denying everything (fail-closed). Nothing is granted; every " +
    `requester sees zero rows. Parser: ${reason}`
  );
}
