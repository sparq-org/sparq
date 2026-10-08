// [OPUS-4.8] sq-vw3ax.10 — the PURE, framework-free logic behind the /try Graph view: turn a
// non-aggregate SELECT result set into a node-link graph (nodes = distinct RDF terms the engine
// bound, edges = the per-row relationships between adjacent bound columns). The React SVG renderer
// (repl-graph-view.tsx) draws whatever this returns; keeping the shape derivation here makes it
// unit-testable under `npm run test:unit` and framework-free.
//
// HONESTY (load-bearing, mirrors repl-result-cells.tsx). Every node is a real `SparqlTerm` the
// engine actually returned and every edge is a real (source, target) co-occurrence in one of the
// engine's OWN result rows — nothing is invented. `deriveGraph` returns `null` (the caller then
// offers no Graph view) unless the result is genuinely entity-relationship shaped: ≥2 columns, ≥1
// edge, and at least one RESOURCE (uri/bnode) node. That gate is what makes this the complement of
// the aggregate mini-viz (`deriveViz`): a pure label→number aggregate (e.g. `?name (SUM(?x))`, two
// literal columns) has no resource node, so it charts as bars and draws no graph — never both from
// one honest reading of the same rows.

import { formatTerm, type SparqlResults, type SparqlTerm } from "@sparq/client";
import { curie } from "./curie";
import { isGraphShaped, termKey, MAX_GRAPH_NODES } from "./result-graph-shape";

// Re-exported for the renderer/tests; it LIVES in result-graph-shape.ts (the light hero-chunk
// module) because the eligibility predicate replays the same cap to stay exact.
export { MAX_GRAPH_NODES };

export type NodeKind = "uri" | "literal" | "bnode" | "triple";

export interface GraphNode {
  /** Stable identity key across rows/columns (type + lexical value + datatype/lang). */
  id: string;
  /** The underlying engine term (kept verbatim for tooltips / raw display). */
  term: SparqlTerm;
  /** Display label — CURIE for IRIs, quoted for literals, `_:x` for blank nodes. */
  label: string;
  kind: NodeKind;
  /** The projection variables this term appeared under (first-seen order), for legend/labelling. */
  vars: string[];
}

export interface GraphEdge {
  /** Source node id. */
  source: string;
  /** Target node id. */
  target: string;
  /** The relationship label — the target column's variable name (how the edge reads: a→[?var]). */
  label: string;
  /** How many result rows produced this exact (source, target, label) edge. */
  count: number;
}

export interface ResultGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
  /** True when the full result had more than {@link MAX_GRAPH_NODES} distinct terms (nodes capped). */
  truncated: boolean;
  /** The full distinct-term count before capping (so the note can say "N of M"). */
  totalNodes: number;
}

/** The human-facing node label for a term (display only; raw value stays on `node.term`). */
function nodeLabel(t: SparqlTerm): string {
  if (t.type === "uri") return curie(t.value);
  if (t.type === "bnode") return `_:${t.value}`;
  if (t.type === "triple") return formatTerm(t);
  return `"${t.value}"`;
}

/**
 * Derive a node-link graph from a SELECT result, or `null` when the result is not
 * entity-relationship shaped (see the module header for the honesty gate). Deterministic: node and
 * edge order follow first appearance in `head.vars` × row order, so the same result always yields
 * the same graph (and the same layout downstream).
 */
export function deriveGraph(results: SparqlResults): ResultGraph | null {
  // Precondition gate — the SAME cheap predicate the hero uses to decide whether to offer the
  // toggle. It exactly replays this function's capped admission (shared termKey + MAX_GRAPH_NODES,
  // same scan order), so the two can never disagree about what is "graph-shaped" — including at the
  // cap boundary (see result-graph-shape.ts). The edge/resource checks below are therefore
  // defensive only.
  if (!isGraphShaped(results)) return null;
  const vars = results.head?.vars ?? [];
  const rows = results.results?.bindings ?? [];

  const nodes = new Map<string, GraphNode>();
  const allKeys = new Set<string>(); // distinct-term count INCLUDING terms dropped past the cap
  let droppedAny = false;

  // Register a term as a node (capped at MAX_GRAPH_NODES) and return its id, or null if it was
  // dropped because the cap is already full and this is a new term.
  const register = (t: SparqlTerm, v: string): string | null => {
    const id = termKey(t);
    allKeys.add(id);
    const existing = nodes.get(id);
    if (existing) {
      if (!existing.vars.includes(v)) existing.vars.push(v);
      return id;
    }
    if (nodes.size >= MAX_GRAPH_NODES) {
      droppedAny = true;
      return null;
    }
    nodes.set(id, {
      id,
      term: t,
      label: nodeLabel(t),
      kind: t.type,
      vars: [v],
    });
    return id;
  };

  // Edges: for each row, connect consecutive BOUND columns (in `head.vars` order). Unbound cells
  // are skipped so an OPTIONAL that did not match never breaks the chain. The edge label is the
  // TARGET column's variable name, which reads as "source —[?target]→ target".
  const edgeMap = new Map<string, GraphEdge>();
  for (const row of rows) {
    const bound: { v: string; id: string | null }[] = [];
    for (const v of vars) {
      const t = row[v];
      if (t) bound.push({ v, id: register(t, v) });
    }
    for (let i = 0; i + 1 < bound.length; i++) {
      const a = bound[i];
      const b = bound[i + 1];
      if (a.id === null || b.id === null) continue; // an endpoint was dropped past the cap
      if (a.id === b.id) continue; // skip a self-relationship (a row binding both cols to one term)
      const key = JSON.stringify([a.id, b.id, b.v]);
      const existing = edgeMap.get(key);
      if (existing) existing.count += 1;
      else edgeMap.set(key, { source: a.id, target: b.id, label: b.v, count: 1 });
    }
  }

  const edges = [...edgeMap.values()];
  if (edges.length === 0) return null;

  // The entity-relationship gate: at least one RESOURCE node (uri/bnode). A pure literal→literal
  // result (a label→number aggregate) has none, so it draws no graph — the mini-viz handles it.
  const hasResource = [...nodes.values()].some((n) => n.kind !== "literal");
  if (!hasResource) return null;

  return {
    nodes: [...nodes.values()],
    edges,
    truncated: droppedAny,
    totalNodes: allKeys.size,
  };
}

export interface Point {
  x: number;
  y: number;
}

/**
 * Deterministic circular layout: place `n` nodes evenly on a circle inscribed in a `width`×`height`
 * box (first node at the top, clockwise). No force simulation and no randomness — stable across
 * renders and unit-testable. Suits the small (≤{@link MAX_GRAPH_NODES}) graphs the compact view draws.
 */
export function circularLayout(
  n: number,
  width: number,
  height: number,
  margin: number,
): Point[] {
  const cx = width / 2;
  const cy = height / 2;
  const r = Math.max(0, Math.min(width, height) / 2 - margin);
  if (n <= 0) return [];
  if (n === 1) return [{ x: cx, y: cy }];
  const pts: Point[] = [];
  for (let i = 0; i < n; i++) {
    const angle = -Math.PI / 2 + (2 * Math.PI * i) / n;
    pts.push({ x: cx + r * Math.cos(angle), y: cy + r * Math.sin(angle) });
  }
  return pts;
}
