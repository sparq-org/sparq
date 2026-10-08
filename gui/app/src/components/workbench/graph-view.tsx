"use client";

// [OPUS-4.8] sq-ixc3.12 — the GRAPH result view: a dependency-free SVG node-link visualisation
// of a CONSTRUCT/DESCRIBE result graph (the engine answers those as flat N-Triples). This is
// exactly what research/gui-design.md §A.4 names as something the GUI adds that the static site
// "fundamentally cannot": a graph visualisation of results. The site's REPL only ever serialises
// the graph to Turtle/N-Triples; here it renders.
//
// DEPENDENCY-FREE on purpose: no d3 / cytoscape / vis-network. A force simulation or a heavy
// graph lib would be a large SSR-incompatible dependency (cytoscape is ~350 KB min, d3-force
// pulls a tree of modules) for a view that, for a CONSTRUCT/DESCRIBE result, is small. Instead we
// use the `parseNTriples` reshaper already in `@sparq/client` and a deterministic radial layout
// (no animation, no physics, no RNG → stable + diffable). Nodes are subjects/objects; directed
// edges are predicates. IRIs are abbreviated with the common prefixes for legibility. For very
// large graphs we cap the rendered triples and say so (the full result still lives in the
// N-Triples / Turtle views + export).

import * as React from "react";

import {
  parseNTriples,
  type RdfStatement,
  type RdfTerm,
} from "@sparq/client";

// [FABLE-5] sq-ixc3.20 — canonical triple identity (the inferred-edge membership test) +
// the clicked-triple shape the proof panel consumes.
import { tripleKeyOfTerms } from "@/lib/inferred-facts";
import { abbreviateIri } from "@/lib/iri-label";
import type { ExplainTarget } from "@/components/workbench/proof-panel";

/**
 * [FABLE-5] sq-ixc3.20 — the optional INFERRED-fact affordance: when the active inference
 * regime has a materialised closure, `keys` holds the canonical keys of every triple the
 * closure ADDED (exact set difference, engine-context `entailedTripleKeys`), and an edge
 * whose statement is in that set renders dashed in the primary colour and becomes a
 * click-to-explain target (`onExplain` opens the why() proof panel). Asserted edges are
 * untouched — no affordance, exactly as the honesty contract requires.
 */
export interface InferredAffordance {
  keys: ReadonlySet<string>;
  onExplain: (t: ExplainTarget) => void;
}

/** The max triples we lay out before capping (a render bound, labelled — not a result bound). */
const MAX_TRIPLES = 200;

/** A short, human label for a term (the node/edge caption). */
function termLabel(t: RdfTerm): string {
  switch (t.kind) {
    case "iri":
      return abbreviateIri(t.value);
    case "bnode":
      return `_:${t.label}`;
    case "literal":
      return t.value.length > 24 ? `"${t.value.slice(0, 23)}…"` : `"${t.value}"`;
    case "triple":
      return "« triple »";
  }
}

/** A stable identity key for a term (so the same node merges across triples). */
function termKey(t: RdfTerm): string {
  return t.nt;
}

interface LaidOutNode {
  key: string;
  label: string;
  isLiteral: boolean;
  x: number;
  y: number;
}

interface LaidOutEdge {
  from: string;
  to: string;
  label: string;
}

/** Deterministic radial layout: collect distinct nodes, place them on a circle in first-seen
 *  order, and connect them with the predicate-labelled directed edges. No physics, no RNG. */
function layout(statements: RdfStatement[], size: number): {
  nodes: LaidOutNode[];
  edges: LaidOutEdge[];
} {
  const order: string[] = [];
  const meta = new Map<string, { label: string; isLiteral: boolean }>();
  const note = (t: RdfTerm) => {
    const k = termKey(t);
    if (!meta.has(k)) {
      order.push(k);
      meta.set(k, { label: termLabel(t), isLiteral: t.kind === "literal" });
    }
  };
  const edges: LaidOutEdge[] = [];
  for (const st of statements) {
    note(st.s);
    note(st.o);
    edges.push({ from: termKey(st.s), to: termKey(st.o), label: termLabel(st.p) });
  }

  const cx = size / 2;
  const cy = size / 2;
  const radius = size / 2 - 64;
  const n = order.length;
  const nodes: LaidOutNode[] = order.map((k, i) => {
    // A single node sits at the centre; otherwise spread evenly around the circle starting at
    // the top (−90°) so the layout reads clockwise.
    const angle = n <= 1 ? 0 : (i / n) * 2 * Math.PI - Math.PI / 2;
    const x = n <= 1 ? cx : cx + radius * Math.cos(angle);
    const y = n <= 1 ? cy : cy + radius * Math.sin(angle);
    const m = meta.get(k)!;
    return { key: k, label: m.label, isLiteral: m.isLiteral, x, y };
  });
  return { nodes, edges };
}

/**
 * Render a CONSTRUCT/DESCRIBE result's N-Triples document as an SVG node-link graph. Stateless +
 * deterministic; safe in a static export and in the Tauri webview.
 */
export function GraphView({
  ntriples,
  inferred,
}: {
  ntriples: string;
  /** [FABLE-5] sq-ixc3.20 — mark + explain inferred edges (absent = no affordance). */
  inferred?: InferredAffordance | null;
}) {
  const { statements, total } = React.useMemo(() => {
    // `parseNTriples` returns `{ statements, passthrough }`; we only graph the parsed triples.
    const { statements: all } = parseNTriples(ntriples);
    return { statements: all.slice(0, MAX_TRIPLES), total: all.length };
  }, [ntriples]);

  const SIZE = 520;
  const { nodes, edges } = React.useMemo(() => layout(statements, SIZE), [statements]);

  // [FABLE-5] sq-ixc3.20 — per-edge inferredness. `layout` pushes exactly ONE edge per
  // statement in order, so edge i ↔ statement i; membership in the closure-added key set
  // is the affordance gate (an asserted edge can never test true).
  const inferredFlags = React.useMemo(() => {
    if (!inferred || inferred.keys.size === 0) return null;
    const flags = statements.map((st) => inferred.keys.has(tripleKeyOfTerms(st.s, st.p, st.o)));
    return flags.some(Boolean) ? flags : null;
  }, [statements, inferred]);
  const posByKey = React.useMemo(() => {
    const m = new Map<string, LaidOutNode>();
    for (const node of nodes) m.set(node.key, node);
    return m;
  }, [nodes]);

  if (total === 0) {
    return (
      <p className="p-3 text-sm text-muted-foreground" data-result-view="graph">
        Empty graph — the template produced no triples.
      </p>
    );
  }

  const truncated = total > statements.length;

  return (
    <div className="flex h-full flex-col" data-result-view="graph">
      {/* [FABLE-5] sq-ixc3.20 — the inferred-edge legend, only when something IS inferred. */}
      {inferredFlags && (
        <p
          className="border-b bg-card px-3 py-1 text-[11px] text-muted-foreground"
          data-graph-inferred-legend
        >
          <span className="font-medium text-primary">Dashed edges are inferred</span> by the
          active inference regime (not asserted) — click one to see its derivation.
        </p>
      )}
      {truncated && (
        <p className="border-b bg-warning/10 px-3 py-1 text-[11px] text-muted-foreground">
          Showing the first {statements.length.toLocaleString()} of{" "}
          {total.toLocaleString()} triples (graph-view render cap). The full graph is in the
          N-Triples / Turtle views and the export.
        </p>
      )}
      <div className="min-h-0 flex-1 overflow-auto p-3">
        <svg
          viewBox={`0 0 ${SIZE} ${SIZE}`}
          className="mx-auto h-auto w-full max-w-[560px]"
          role="img"
          aria-label="Node-link graph of the result triples"
        >
          <defs>
            <marker
              id="sq-arrow"
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

          {/* Edges first (under the nodes). */}
          {edges.map((e, i) => {
            const a = posByKey.get(e.from);
            const b = posByKey.get(e.to);
            if (!a || !b) return null;
            const mx = (a.x + b.x) / 2;
            const my = (a.y + b.y) / 2;
            // [FABLE-5] sq-ixc3.20 — an INFERRED edge (in the closure-added set) renders
            // dashed in the primary colour and is a click-to-explain target. Colour is not
            // the only signal: the dash pattern + the legend + the tooltip carry it too.
            const isInferred = inferredFlags?.[i] === true;
            const st = statements[i];
            const onExplain =
              isInferred && inferred
                ? () => inferred.onExplain({ s: st.s.nt, p: st.p.nt, o: st.o.nt })
                : undefined;
            return (
              <g
                key={i}
                {...(isInferred
                  ? {
                      role: "button",
                      tabIndex: 0,
                      className: "cursor-pointer focus:outline-1 focus:outline-primary",
                      onClick: onExplain,
                      onKeyDown: (ev: React.KeyboardEvent) => {
                        if (ev.key === "Enter" || ev.key === " ") {
                          ev.preventDefault();
                          onExplain?.();
                        }
                      },
                      "data-entailed-edge": true,
                      "aria-label": `Inferred: ${e.label} — press Enter to see why`,
                    }
                  : {})}
              >
                {isInferred && (
                  <title>Inferred (not asserted) — click to see the derivation</title>
                )}
                {/* A wide invisible hit line so a 1.25px inferred edge is actually clickable. */}
                {isInferred && (
                  <line
                    x1={a.x}
                    y1={a.y}
                    x2={b.x}
                    y2={b.y}
                    stroke="transparent"
                    strokeWidth={10}
                  />
                )}
                <line
                  x1={a.x}
                  y1={a.y}
                  x2={b.x}
                  y2={b.y}
                  stroke={isInferred ? "var(--primary)" : "var(--border)"}
                  strokeWidth={1.25}
                  strokeDasharray={isInferred ? "4 3" : undefined}
                  markerEnd="url(#sq-arrow)"
                />
                <text
                  x={mx}
                  y={my}
                  className={isInferred ? "fill-primary" : "fill-muted-foreground"}
                  fontSize={8}
                  textAnchor="middle"
                  dy={-2}
                >
                  {e.label}
                </text>
              </g>
            );
          })}

          {/* Nodes. Literals render as rounded rects, resources as circles. */}
          {nodes.map((node) =>
            node.isLiteral ? (
              <g key={node.key}>
                <rect
                  x={node.x - 36}
                  y={node.y - 10}
                  width={72}
                  height={20}
                  rx={4}
                  fill="var(--accent)"
                  stroke="var(--border)"
                />
                <text
                  x={node.x}
                  y={node.y}
                  className="fill-accent-foreground"
                  fontSize={9}
                  textAnchor="middle"
                  dominantBaseline="central"
                >
                  {node.label}
                </text>
              </g>
            ) : (
              <g key={node.key}>
                <circle cx={node.x} cy={node.y} r={6} fill="var(--primary)" />
                <text
                  x={node.x}
                  y={node.y - 10}
                  className="fill-foreground"
                  fontSize={9}
                  textAnchor="middle"
                >
                  {node.label}
                </text>
              </g>
            ),
          )}
        </svg>
      </div>
    </div>
  );
}
