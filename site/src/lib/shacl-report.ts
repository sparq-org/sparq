// [OPUS-4.8] sq-egy6 — pure, framework-free helpers for rendering a SHACL
// validation report in the live /surface/shacl playground. Kept separate from the
// React component so they can be unit-tested under node:test without a DOM.

import { termToNTriples } from "@sparq/client";

import type { ShaclReport, ShaclResult } from "./sparq-wasm";

const PREFIXES: [string, string][] = [
  ["http://www.w3.org/ns/shacl#", "sh:"],
  ["http://www.w3.org/2001/XMLSchema#", "xsd:"],
  ["http://www.w3.org/1999/02/22-rdf-syntax-ns#", "rdf:"],
  ["http://www.w3.org/2000/01/rdf-schema#", "rdfs:"],
  ["http://example.org/", "ex:"],
];

/**
 * Compacts a full IRI (optionally wrapped in `<>` as a term string) to a CURIE
 * using the well-known SHACL/XSD/example prefixes. Anything outside those
 * namespaces is returned unchanged. Blank-node (`_:b0`) and literal term strings
 * pass through untouched.
 */
export function shortenIri(s: string): string {
  const inner = s.startsWith("<") && s.endsWith(">") ? s.slice(1, -1) : s;
  for (const [ns, prefix] of PREFIXES) {
    if (inner.startsWith(ns)) {
      return prefix + inner.slice(ns.length);
    }
  }
  return s;
}

/** The bare local name of a SHACL constraint-component IRI, e.g.
 *  `…#DatatypeConstraintComponent` -> `DatatypeConstraintComponent`. */
export function componentName(iri: string): string {
  const hash = iri.lastIndexOf("#");
  return hash >= 0 ? iri.slice(hash + 1) : iri;
}

/** The bare local name of a severity IRI, e.g. `…#Violation` -> `Violation`. */
export function severityName(iri: string): string {
  return componentName(iri);
}

/**
 * A one-line, human summary of the report: either "Conforms" or the violation
 * count. Mirrors the conformance flag the bead asks the page to surface.
 */
export function reportSummary(report: ShaclReport): string {
  if (report.conforms) return "Conforms — no violations.";
  const n = report.results.length;
  return `Does not conform — ${n} ${n === 1 ? "violation" : "violations"}.`;
}

/** A message as a Turtle string literal, via the shared sparq-client writer (#6719). */
function turtleString(s: string): string {
  return termToNTriples({ type: "literal", value: s });
}

/** An engine-reported IRI as `<iri>`; the shared writer throws if it cannot be written as-is. */
function turtleIri(iri: string): string {
  return termToNTriples({ type: "uri", value: iri });
}

/**
 * Renders the report as a W3C `sh:ValidationReport` Turtle graph — the per-result
 * vocabulary (`sh:result`, `sh:focusNode`, `sh:resultPath`, `sh:value`,
 * `sh:resultSeverity`, `sh:sourceConstraintComponent`, `sh:sourceShape`,
 * `sh:resultMessage`). This is a faithful client-side serialisation of the JSON the
 * wasm `validate` binding returns; the canonical Turtle serialiser is in
 * `sparq-shacl` (`ValidationReport::to_turtle`).
 */
export function reportToTurtle(report: ShaclReport): string {
  const lines: string[] = [
    "@prefix sh:   <http://www.w3.org/ns/shacl#> .",
    "@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .",
    "",
    "[] a sh:ValidationReport ;",
    `  sh:conforms ${report.conforms ? "true" : "false"}`,
  ];
  if (report.results.length === 0) {
    lines[lines.length - 1] += " .";
    return lines.join("\n") + "\n";
  }
  lines[lines.length - 1] += " ;";
  report.results.forEach((r, i) => {
    const last = i === report.results.length - 1;
    lines.push("  sh:result [");
    lines.push("    a sh:ValidationResult ;");
    lines.push(`    sh:focusNode ${r.focusNode} ;`);
    if (r.path) lines.push(`    sh:resultPath ${r.path} ;`);
    if (r.value) lines.push(`    sh:value ${r.value} ;`);
    lines.push(`    sh:resultSeverity ${turtleIri(r.severity)} ;`);
    lines.push(
      `    sh:sourceConstraintComponent ${turtleIri(r.sourceConstraintComponent)} ;`,
    );
    lines.push(`    sh:sourceShape ${r.sourceShape} ;`);
    if (r.message) lines.push(`    sh:resultMessage ${turtleString(r.message)} ;`);
    // Trim the trailing ` ;` of the last property inside the blank node.
    lines[lines.length - 1] = lines[lines.length - 1].replace(/ ;$/, "");
    lines.push(last ? "  ] ." : "  ] ;");
  });
  return lines.join("\n") + "\n";
}

export type { ShaclReport, ShaclResult };
