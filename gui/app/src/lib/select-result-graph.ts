// [SONNET-4.6] #3602 — pure SELECT-result to node-link graph derivation for the workbench.

import {
  COMMON_PREFIXES,
  formatTerm,
  termValue,
  type SparqlResults,
  type SparqlTerm,
} from "@sparq/client";

export const MAX_SELECT_GRAPH_NODES = 24;

export type SelectGraphNodeKind = SparqlTerm["type"];

export interface SelectGraphNode {
  id: string;
  label: string;
  kind: SelectGraphNodeKind;
  value: string;
}

export interface SelectGraphEdge {
  source: string;
  target: string;
  label: string;
  count: number;
}

export interface SelectResultGraph {
  nodes: SelectGraphNode[];
  edges: SelectGraphEdge[];
  totalNodes: number;
  truncated: boolean;
}

function termKey(term: SparqlTerm): string {
  return JSON.stringify([
    term.type,
    term.value,
    term.type === "literal" ? (term.datatype ?? "") : "",
    term.type === "literal" ? (term["xml:lang"] ?? "") : "",
  ]);
}

function iriLabel(iri: string): string {
  for (const { prefix, iri: namespace } of COMMON_PREFIXES) {
    if (iri.startsWith(namespace)) return `${prefix}:${iri.slice(namespace.length)}`;
  }
  const cut = Math.max(iri.lastIndexOf("#"), iri.lastIndexOf("/"));
  return cut >= 0 && cut < iri.length - 1 ? iri.slice(cut + 1) : iri;
}

function termLabel(term: SparqlTerm): string {
  if (term.type === "uri") return iriLabel(term.value);
  if (term.type === "bnode") return `_:${term.value}`;
  if (term.type === "triple") return formatTerm(term);
  return `"${term.value}"`;
}

/** Derive real nodes and row co-occurrence edges from an entity-shaped SELECT result. */
export function deriveSelectResultGraph(results: SparqlResults): SelectResultGraph | null {
  const vars = results.head?.vars ?? [];
  const rows = results.results?.bindings ?? [];
  if (vars.length < 2 || rows.length === 0) return null;

  const nodes = new Map<string, SelectGraphNode>();
  const allNodes = new Set<string>();
  const edges = new Map<string, SelectGraphEdge>();

  const register = (term: SparqlTerm): string | null => {
    const id = termKey(term);
    allNodes.add(id);
    if (!nodes.has(id)) {
      if (nodes.size >= MAX_SELECT_GRAPH_NODES) return null;
      nodes.set(id, { id, label: termLabel(term), kind: term.type, value: termValue(term) ?? "" });
    }
    return id;
  };

  for (const row of rows) {
    const bound = vars.flatMap((variable) => {
      const term = row[variable];
      return term ? [{ variable, id: register(term) }] : [];
    });
    for (let index = 0; index + 1 < bound.length; index += 1) {
      const source = bound[index];
      const target = bound[index + 1];
      if (!source.id || !target.id || source.id === target.id) continue;
      const key = JSON.stringify([source.id, target.id, target.variable]);
      const existing = edges.get(key);
      if (existing) existing.count += 1;
      else {
        edges.set(key, {
          source: source.id,
          target: target.id,
          label: target.variable,
          count: 1,
        });
      }
    }
  }

  if (edges.size === 0 || ![...nodes.values()].some((node) => node.kind !== "literal")) {
    return null;
  }
  return {
    nodes: [...nodes.values()],
    edges: [...edges.values()],
    totalNodes: allNodes.size,
    truncated: allNodes.size > nodes.size,
  };
}

export function circularSelectGraphLayout(count: number, size: number, margin: number) {
  if (count <= 0) return [];
  const center = size / 2;
  if (count === 1) return [{ x: center, y: center }];
  const radius = Math.max(0, center - margin);
  return Array.from({ length: count }, (_, index) => {
    const angle = -Math.PI / 2 + (2 * Math.PI * index) / count;
    return { x: center + radius * Math.cos(angle), y: center + radius * Math.sin(angle) };
  });
}
