"use client";

// [SONNET-4.6] #3602 — node-link rendering for entity-shaped SELECT results in /app.

import * as React from "react";
import type { SparqlResults } from "@sparq/client";
import {
  circularSelectGraphLayout,
  deriveSelectResultGraph,
  MAX_SELECT_GRAPH_NODES,
  type SelectGraphNodeKind,
} from "@/lib/select-result-graph";

const SIZE = 520;
const NODE_RADIUS = 7;
const KIND_FILL: Record<SelectGraphNodeKind, string> = {
  uri: "var(--primary)",
  literal: "var(--accent-foreground)",
  bnode: "var(--muted-foreground)",
  triple: "var(--warning)",
};

function clip(value: string, max = 22): string {
  return value.length > max ? `${value.slice(0, max - 1)}…` : value;
}

export function SelectResultGraphView({ results }: { results: SparqlResults }) {
  const graph = React.useMemo(() => deriveSelectResultGraph(results), [results]);
  if (!graph) {
    return (
      <p className="p-3 text-sm text-muted-foreground" data-result-view="graph">
        This SELECT has no entity-relationship shape to draw. Use Table or Raw JSON instead.
      </p>
    );
  }

  const points = circularSelectGraphLayout(graph.nodes.length, SIZE, 72);
  const positions = new Map(graph.nodes.map((node, index) => [node.id, points[index]]));
  const summary = `Node-link graph of ${graph.nodes.length} nodes and ${graph.edges.length} relationships from the SELECT result.`;

  return (
    <div className="flex h-full flex-col" data-result-view="graph" data-select-result-graph>
      {graph.truncated && (
        <p className="border-b bg-warning/10 px-3 py-1 text-[11px] text-muted-foreground">
          Showing the first {MAX_SELECT_GRAPH_NODES} of {graph.totalNodes.toLocaleString()} nodes.
        </p>
      )}
      <div className="min-h-0 flex-1 overflow-auto p-3">
        <svg
          viewBox={`0 0 ${SIZE} ${SIZE}`}
          className="mx-auto h-auto w-full max-w-[560px]"
          role="img"
          aria-label={summary}
        >
          <title>{summary}</title>
          <defs>
            <marker
              id="sq-select-arrow"
              viewBox="0 0 10 10"
              refX="9"
              refY="5"
              markerWidth="6"
              markerHeight="6"
              orient="auto-start-reverse"
            >
              <path d="M 0 0 L 10 5 L 0 10 z" fill="var(--muted-foreground)" />
            </marker>
          </defs>
          {graph.edges.map((edge) => {
            const source = positions.get(edge.source);
            const target = positions.get(edge.target);
            if (!source || !target) return null;
            const label = edge.count > 1 ? `${edge.label} ×${edge.count}` : edge.label;
            return (
              <g key={`${edge.source}-${edge.target}-${edge.label}`}>
                <line
                  x1={source.x}
                  y1={source.y}
                  x2={target.x}
                  y2={target.y}
                  stroke="var(--border)"
                  strokeWidth={1.25}
                  markerEnd="url(#sq-select-arrow)"
                />
                <text
                  x={(source.x + target.x) / 2}
                  y={(source.y + target.y) / 2 - 3}
                  className="fill-muted-foreground"
                  fontSize={9}
                  textAnchor="middle"
                >
                  {clip(label, 16)}
                </text>
              </g>
            );
          })}
          {graph.nodes.map((node) => {
            const point = positions.get(node.id)!;
            return (
              <g key={node.id}>
                <circle
                  cx={point.x}
                  cy={point.y}
                  r={NODE_RADIUS}
                  fill={KIND_FILL[node.kind]}
                >
                  <title>{node.value}</title>
                </circle>
                <text
                  x={point.x}
                  y={point.y - 12}
                  className="fill-foreground"
                  fontSize={10}
                  textAnchor="middle"
                >
                  {clip(node.label)}
                  <title>{node.value}</title>
                </text>
              </g>
            );
          })}
        </svg>
      </div>
      <p className="border-t px-3 py-1 text-[11px] text-muted-foreground">
        {graph.nodes.length} node{graph.nodes.length === 1 ? "" : "s"} · {graph.edges.length}{" "}
        relationship{graph.edges.length === 1 ? "" : "s"}
      </p>
    </div>
  );
}
