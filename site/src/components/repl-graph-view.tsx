"use client";

// [OPUS-4.8] sq-vw3ax.10 — the Graph view: a node-link rendering of a non-aggregate SELECT result.
// It is the entity-relationship complement of the aggregate mini-viz (repl-result-cells.tsx):
// where a label→number result charts as bars, a result that links resources renders here as a
// node-link diagram. Consumed by the home hero runner (hero-runner.tsx) as a Table | Graph toggle.
//
// HONESTY (load-bearing). Every node is a real term the engine bound and every edge is a real
// co-occurrence of two terms in one of the engine's OWN result rows (see result-graph.ts). Nothing
// is invented; the shape derivation returns null unless the result is genuinely graph-shaped, so the
// toggle only appears when there is a real graph to draw. Layout is deterministic (a fixed circular
// placement, no force sim, no randomness) so the same result always draws the same picture.
//
// DEPENDENCY-FREE by design: a hand-rolled SVG, no graph library — the home route has a first-load
// bundle budget (sq-vw3ax.9) and this ships inside the already-lazy hero-runner chunk.

import * as React from "react";

import { termValue, type SparqlResults } from "@/lib/sparq-wasm";
import {
  circularLayout,
  deriveGraph,
  MAX_GRAPH_NODES,
  type GraphNode,
  type NodeKind,
} from "@/lib/result-graph";

const VIEW_W = 620;
const VIEW_H = 380;
const MARGIN = 96; // room around the circle for outward-radiating node labels
const NODE_R = 7;

/** The fill token per term kind — reuses the syntax-highlight chart tokens so it reads like the table. */
const KIND_FILL: Record<NodeKind, string> = {
  uri: "var(--chart-1)", // IRI (teal)
  literal: "var(--chart-4)", // literal (string token)
  bnode: "var(--muted-foreground)", // blank node
  triple: "var(--chart-2)", // RDF 1.2 triple term
};

const KIND_LABEL: Record<NodeKind, string> = {
  uri: "Resource",
  literal: "Literal",
  bnode: "Blank node",
  triple: "Triple term",
};

/** Truncate a long node label for the compact view; the full value stays in the `<title>` tooltip. */
function clip(label: string, max = 22): string {
  return label.length > max ? label.slice(0, max - 1) + "…" : label;
}

/**
 * The node-link Graph view. Returns `null` when the result has no graph-able shape (the caller must
 * itself gate on {@link deriveGraph} to decide whether to offer the toggle at all, so this is a
 * defensive fallback). Presentational only — it draws what {@link deriveGraph} extracted.
 */
export function ResultGraphView({ results }: { results: SparqlResults }) {
  const graph = React.useMemo(() => deriveGraph(results), [results]);
  if (!graph) return null;

  const { nodes, edges, truncated, totalNodes } = graph;
  const points = circularLayout(nodes.length, VIEW_W, VIEW_H, MARGIN);
  const index = new Map<string, number>(nodes.map((n, i): [string, number] => [n.id, i]));
  const cx = VIEW_W / 2;
  const cy = VIEW_H / 2;

  // Which kinds actually appear — the legend shows only those.
  const presentKinds = (["uri", "literal", "bnode"] as NodeKind[]).filter((k) =>
    nodes.some((n) => n.kind === k),
  );

  const summary = `Node-link graph of ${nodes.length} node${
    nodes.length === 1 ? "" : "s"
  } and ${edges.length} relationship${edges.length === 1 ? "" : "s"} from the result.`;

  return (
    <figure className="m-0 px-3 py-3" data-hero-graph-view>
      <svg
        viewBox={`0 0 ${VIEW_W} ${VIEW_H}`}
        className="w-full"
        style={{ height: "auto", maxHeight: "22rem" }}
        role="img"
        aria-label={summary}
      >
        <title>{summary}</title>
        <defs>
          <marker
            id="sq-graph-arrow"
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="7"
            markerHeight="7"
            orient="auto-start-reverse"
          >
            <path d="M 0 0 L 10 5 L 0 10 z" fill="var(--muted-foreground)" />
          </marker>
        </defs>

        {/* Edges (drawn under the nodes). A slight perpendicular bow separates a↔b reverse pairs. */}
        <g>
          {edges.map((e, i) => {
            const si = index.get(e.source);
            const ti = index.get(e.target);
            if (si === undefined || ti === undefined) return null;
            const a = points[si];
            const b = points[ti];
            const dx = b.x - a.x;
            const dy = b.y - a.y;
            const len = Math.hypot(dx, dy) || 1;
            const ux = dx / len;
            const uy = dy / len;
            // Trim endpoints to the node boundary (and leave a gap at the target for the arrowhead).
            const sx = a.x + ux * NODE_R;
            const sy = a.y + uy * NODE_R;
            const ex = b.x - ux * (NODE_R + 6);
            const ey = b.y - uy * (NODE_R + 6);
            // Bow the curve; the side depends on id order so a→b and b→a don't overlap.
            const bow = (e.source < e.target ? 1 : -1) * Math.min(28, len * 0.14);
            const mx = (sx + ex) / 2 - uy * bow;
            const my = (sy + ey) / 2 + ux * bow;
            const label = e.count > 1 ? `${e.label} ×${e.count}` : e.label;
            return (
              <g key={`${e.source}-${e.target}-${e.label}-${i}`}>
                <path
                  d={`M ${sx.toFixed(1)} ${sy.toFixed(1)} Q ${mx.toFixed(1)} ${my.toFixed(
                    1,
                  )} ${ex.toFixed(1)} ${ey.toFixed(1)}`}
                  fill="none"
                  stroke="var(--border)"
                  strokeWidth={1.25}
                  markerEnd="url(#sq-graph-arrow)"
                />
                <text
                  x={mx.toFixed(1)}
                  y={my.toFixed(1)}
                  textAnchor="middle"
                  dominantBaseline="central"
                  className="font-mono"
                  fontSize="9"
                  fill="var(--muted-foreground)"
                >
                  {clip(label, 16)}
                </text>
              </g>
            );
          })}
        </g>

        {/* Nodes + radially-placed labels. */}
        <g>
          {nodes.map((n: GraphNode, i) => {
            const p = points[i];
            // Push the label outward from the circle centre so labels fan out, not overlap.
            const ldx = p.x - cx;
            const ldy = p.y - cy;
            const llen = Math.hypot(ldx, ldy) || 1;
            const lx = p.x + (ldx / llen) * (NODE_R + 6);
            const ly = p.y + (ldy / llen) * (NODE_R + 6);
            const anchor = ldx >= 0 ? "start" : "end";
            return (
              <g key={n.id}>
                <circle
                  cx={p.x}
                  cy={p.y}
                  r={NODE_R}
                  fill={KIND_FILL[n.kind]}
                  stroke="var(--card)"
                  strokeWidth={1.5}
                >
                  <title>{termValue(n.term)}</title>
                </circle>
                <text
                  x={lx.toFixed(1)}
                  y={ly.toFixed(1)}
                  textAnchor={anchor}
                  dominantBaseline="central"
                  className="font-mono"
                  fontSize="11"
                  fill="var(--foreground)"
                >
                  {clip(n.label)}
                  <title>{termValue(n.term)}</title>
                </text>
              </g>
            );
          })}
        </g>
      </svg>

      <figcaption className="mt-1 flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
        {/* Legend — only the kinds present in this result. */}
        <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
          {presentKinds.map((k) => (
            <span key={k} className="inline-flex items-center gap-1.5">
              <span
                aria-hidden
                className="inline-block size-2.5 rounded-full"
                style={{ backgroundColor: KIND_FILL[k] }}
              />
              {KIND_LABEL[k]}
            </span>
          ))}
        </span>
        <span className="font-mono">
          {nodes.length} node{nodes.length === 1 ? "" : "s"} · {edges.length} edge
          {edges.length === 1 ? "" : "s"}
        </span>
        {truncated && (
          <span className="font-mono text-[var(--warning)]">
            showing first {MAX_GRAPH_NODES} of {totalNodes} nodes
          </span>
        )}
      </figcaption>
    </figure>
  );
}
