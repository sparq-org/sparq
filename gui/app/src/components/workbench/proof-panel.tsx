"use client";

// [FABLE-5] sq-ixc3.20 — the why() PROOF-TREE panel: click an INFERRED fact in the results
// views and see its derivation back to asserted source facts (the RDFox-console
// "explain an inferred fact" parity feature — no open-source engine ships this UI).
//
// The panel is a thin renderer: proof parsing / DAG→tree shaping / rule classification /
// term captions all live in lib/proof-view.ts where they are unit-tested. Rendering mirrors
// the plan explorer's collapsible tree idiom (plan-tree.tsx — the sq-ixc3.19 house pattern).
//
// HONESTY: the proof is ONE witness derivation (the reasoner's deterministic first), never
// presented as "the" unique derivation; leaves are labelled `asserted` and internal nodes
// carry the exact rule id that fired (`rdfs9`, `cax-sco`, `n3-rule-<i>`, …). A shared
// sub-derivation renders once and later occurrences say "see step #n" — the DAG is shown as
// a DAG, not silently duplicated. A triple the regime cannot explain (raced store edit,
// N3 existential blanks) states that plainly instead of fabricating a tree.

import * as React from "react";
import { ChevronRight, Loader2, X } from "lucide-react";

import { cn } from "@/lib/utils";
import { useEngine } from "@/lib/engine-context";
import {
  buildProofDisplay,
  ntTermLabel,
  parseProofJson,
  proofSummary,
  ruleLabel,
  type ProofDisplayNode,
  type ProofSummary,
  type ProofTreeJson,
} from "@/lib/proof-view";

/** The clicked triple: three N-Triples term strings (the lib/inferred-facts termToNT form). */
export interface ExplainTarget {
  s: string;
  p: string;
  o: string;
}

type PanelState =
  | { kind: "loading" }
  | { kind: "proof"; tree: ProofTreeJson; summary: ProofSummary }
  | { kind: "unentailed" }
  | { kind: "error"; message: string };

/** One triple caption: abbreviated s · p · o, with the full term strings on the title. */
function TripleCaption({ triple }: { triple: [string, string, string] }) {
  return (
    <span
      className="min-w-0 truncate font-mono"
      title={`${triple[0]} ${triple[1]} ${triple[2]} .`}
    >
      {ntTermLabel(triple[0])} <span className="text-muted-foreground">{ntTermLabel(triple[1])}</span>{" "}
      {ntTermLabel(triple[2])}
    </span>
  );
}

/** The badge colour per rule kind — shared tokens, colour never the only signal (the text
 *  label carries the truth). */
const KIND_BADGE: Record<ProofDisplayNode["kind"], string> = {
  asserted: "border-border text-muted-foreground",
  axiom: "border-border text-muted-foreground",
  rule: "border-transparent bg-primary/10 text-primary",
};

function ProofRow({
  node,
  depth,
  collapsed,
  onToggle,
}: {
  node: ProofDisplayNode;
  depth: number;
  collapsed: ReadonlySet<string>;
  onToggle: (key: string) => void;
}) {
  // Distinct DOM key per OCCURRENCE (a DAG node can appear once expanded + N times as a
  // reference); the collapse set keys on the occurrence path, not the node id.
  const [showFull, setShowFull] = React.useState(false);
  const key = `${depth}.${node.id}`;
  const hasChildren = node.children.length > 0;
  const isCollapsed = collapsed.has(key);
  return (
    <li role="treeitem" aria-expanded={hasChildren ? !isCollapsed : undefined}>
      <div
        className="flex items-center gap-1.5 px-3 py-0.5"
        style={{ paddingLeft: `${12 + depth * 14}px` }}
        data-proof-node
        data-proof-node-id={node.id}
        data-proof-rule={node.rule}
      >
        {hasChildren ? (
          <button
            type="button"
            aria-label={isCollapsed ? "Expand step" : "Collapse step"}
            onClick={() => onToggle(key)}
            className="shrink-0 text-muted-foreground hover:text-foreground"
          >
            <ChevronRight
              className={cn("size-3 transition-transform", !isCollapsed && "rotate-90")}
              aria-hidden
            />
          </button>
        ) : (
          <span className="inline-block size-3 shrink-0" aria-hidden />
        )}
        <span
          className={cn(
            "shrink-0 rounded border px-1 py-0 font-mono text-[10px]",
            KIND_BADGE[node.kind],
          )}
        >
          {node.kind === "asserted" ? "asserted" : ruleLabel(node.rule)}
        </span>
        {node.repeat ? (
          <span className="text-muted-foreground">
            <TripleCaption triple={node.conclusion} /> — see step #{node.id}
          </span>
        ) : (
          // The acceptance's "each leaf clickable to the triple": clicking the caption
          // toggles the FULL N-Triples line of this step's concluded triple.
          <button
            type="button"
            className="flex min-w-0 items-center text-left hover:underline"
            onClick={() => setShowFull((v) => !v)}
            title="Show the full triple"
            data-proof-triple
          >
            <TripleCaption triple={node.conclusion} />
          </button>
        )}
        <span className="ml-auto shrink-0 font-mono text-[10px] text-muted-foreground">
          #{node.id}
        </span>
      </div>
      {showFull && (
        <pre
          className="overflow-x-auto whitespace-pre px-3 py-1 font-mono text-[10px] text-muted-foreground"
          style={{ paddingLeft: `${30 + depth * 14}px` }}
          data-proof-full-triple
        >
          {node.conclusion[0]} {node.conclusion[1]} {node.conclusion[2]} .
        </pre>
      )}
      {hasChildren && !isCollapsed && (
        <ul role="group">
          {node.children.map((child, i) => (
            <ProofRow
              key={`${i}.${child.id}`}
              node={child}
              depth={depth + 1}
              collapsed={collapsed}
              onToggle={onToggle}
            />
          ))}
        </ul>
      )}
    </li>
  );
}

/**
 * The proof panel: fetches the why() proof for `target` under the active inference regime
 * and renders it as a collapsible derivation tree. Overlays the right side of the results
 * pane; ESC or the ✕ closes it.
 */
export function ProofPanel({ target, onClose }: { target: ExplainTarget; onClose: () => void }) {
  const { inferenceMode } = useEngine();
  const [request, setRequest] = React.useState({ target, inferenceMode, version: 0 });
  if (request.target !== target || request.inferenceMode !== inferenceMode) {
    setRequest({ target, inferenceMode, version: request.version + 1 });
  }
  return (
    <ProofPanelContent
      key={request.version}
      target={target}
      onClose={onClose}
    />
  );
}

function ProofPanelContent({ target, onClose }: { target: ExplainTarget; onClose: () => void }) {
  const { explainWhy, inferenceMode } = useEngine();
  const [state, setState] = React.useState<PanelState>({ kind: "loading" });
  const [collapsed, setCollapsed] = React.useState<ReadonlySet<string>>(new Set());

  React.useEffect(() => {
    let stale = false;
    explainWhy(target.s, target.p, target.o)
      .then((json) => {
        if (stale) return;
        const tree = parseProofJson(json);
        setState(
          tree === null
            ? { kind: "unentailed" }
            : { kind: "proof", tree, summary: proofSummary(tree) },
        );
      })
      .catch((err: unknown) => {
        if (stale) return;
        setState({
          kind: "error",
          message: err instanceof Error ? err.message : String(err),
        });
      });
    return () => {
      stale = true;
    };
  }, [explainWhy, target]);

  // ESC closes (a11y: same dialog idiom as the drawers).
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const toggle = React.useCallback((key: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const display = React.useMemo(
    () => (state.kind === "proof" ? buildProofDisplay(state.tree) : null),
    [state],
  );

  return (
    <aside
      className="absolute inset-y-0 right-0 z-10 flex w-[24rem] max-w-[85%] flex-col border-l bg-card text-xs shadow-lg"
      role="complementary"
      aria-label="Derivation of the selected inferred fact"
      data-proof-panel
    >
      <div className="flex items-center gap-2 border-b px-3 py-1.5">
        <span className="font-medium">Why is this fact entailed?</span>
        <button
          type="button"
          className="ml-auto rounded p-0.5 text-muted-foreground hover:bg-accent/40 hover:text-foreground"
          onClick={onClose}
          aria-label="Close the proof panel"
          data-proof-close
        >
          <X className="size-3.5" aria-hidden />
        </button>
      </div>
      <div className="border-b px-3 py-1.5" data-proof-target title={`${target.s} ${target.p} ${target.o} .`}>
        <TripleCaption triple={[target.s, target.p, target.o]} />
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {state.kind === "loading" && (
          <p className="flex items-center gap-2 p-3 text-muted-foreground" data-proof-loading>
            <Loader2 className="size-3.5 animate-spin" aria-hidden />
            Deriving… (the reasoner re-runs with proof tracing)
          </p>
        )}
        {state.kind === "error" && (
          <p className="p-3 text-destructive" data-proof-error>
            {state.message}
          </p>
        )}
        {state.kind === "unentailed" && (
          <p className="p-3 text-muted-foreground" data-proof-unentailed>
            The {inferenceMode.toUpperCase()} regime does not entail this triple over the
            current store (the store or rules may have changed since the result was produced
            — or, for N3, its derivation concludes a fresh existential blank node, which
            cannot be matched across runs). Re-run the query and try again.
          </p>
        )}
        {state.kind === "proof" && display && (
          <>
            <p className="border-b px-3 py-1.5 text-[11px] text-muted-foreground" data-proof-note>
              One derivation (a witness) — other derivations may exist. Leaves are{" "}
              <span className="font-mono">asserted</span> source facts; each step names the
              rule that fired. {state.summary.steps} steps · {state.summary.ruleFirings} rule{" "}
              firings · {state.summary.assertedLeaves} asserted facts.
            </p>
            <ul role="tree" aria-label="Derivation steps" className="py-1" data-proof-tree>
              <ProofRow node={display} depth={0} collapsed={collapsed} onToggle={toggle} />
            </ul>
          </>
        )}
      </div>
    </aside>
  );
}
