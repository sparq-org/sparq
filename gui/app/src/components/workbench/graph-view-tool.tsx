"use client";

// (sq-lxomy) [SONNET-4.6] — Graph-view TOOL tab: working CONSTRUCT/DESCRIBE panel over the live
// store, using the already-shipped GraphView component (graph-view.tsx, sq-ixc3.12).
//
// The tool accepts any CONSTRUCT or DESCRIBE query typed in the compact textarea; the default is
// CONSTRUCT WHERE { ?s ?p ?o } LIMIT 100, which renders the live store (whatever the user has
// imported) as a node-link graph immediately on first Run. The result is fed directly to the
// existing <GraphView> component — this file imports it but DOES NOT EDIT it.
//
// INVARIANT: renders REAL query results from the live store (never a canned figure). The
// MAX_TRIPLES render cap is GraphView's own (and is labelled there — no perf claim here).
//
// OVERRIDE: flip tier/built/group via GRAPH_VIEW_TOOL_OVERRIDE — never by editing data/tools.ts.

import * as React from "react";
import { Play, Loader2, Network } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { useEngine, type QueryOutcome } from "@/lib/engine-context";
import { GraphView, type InferredAffordance } from "@/components/workbench/graph-view";
import { ProofPanel, type ExplainTarget } from "@/components/workbench/proof-panel";
import { WorkbenchSparqlEditor } from "@/components/workbench/sparql-editor";
import { graphViewRefusal } from "@/lib/query-form";

// [FABLE-5] sq-qgkwy.2 — the override lives in the sibling `.meta.ts` (eagerly bundled for the
// rail/tab honesty read path) so THIS panel module can stay behind a lazy dynamic import().
// Re-exported here so the panel file remains the tool's single import surface.
export { GRAPH_VIEW_TOOL_OVERRIDE } from "@/components/workbench/graph-view-tool.meta";

/** The default CONSTRUCT query the tool opens with — broad enough to show the sample graph. */
const DEFAULT_CONSTRUCT = `CONSTRUCT WHERE { ?s ?p ?o } LIMIT 100`;

export function GraphViewTool() {
  const { run, status, entailedTripleKeys, inferenceMode, inferenceStatus } = useEngine();
  const [query, setQuery] = React.useState(DEFAULT_CONSTRUCT);
  const [running, setRunning] = React.useState(false);
  const [outcome, setOutcome] = React.useState<QueryOutcome | null>(null);
  // [GPT-5.6] The standalone Graph tool owns its proof overlay just as QueryWorkbench does.
  const [explainTarget, setExplainTarget] = React.useState<ExplainTarget | null>(null);
  const abortRef = React.useRef<AbortController | null>(null);

  // [GPT-5.6] Exact closure-minus-base membership; an asserted edge never gains why().
  const inferred = React.useMemo<InferredAffordance | null>(() => {
    if (inferenceMode === "off" || inferenceStatus.kind !== "ready") return null;
    const keys = entailedTripleKeys();
    if (!keys || keys.size === 0) return null;
    return { keys, onExplain: setExplainTarget };
  }, [inferenceMode, inferenceStatus, entailedTripleKeys]);

  React.useEffect(() => {
    setExplainTarget(null);
  }, [inferenceMode]);

  const onRun = React.useCallback(async () => {
    // A run already in flight: ignore (no parallel runs for this tool).
    if (abortRef.current) return;
    // Refuse a non-graph form BEFORE executing it, so a typed UPDATE never mutates the store.
    const refusal = graphViewRefusal(query);
    if (refusal !== null) {
      setOutcome({ kind: "error", message: refusal });
      return;
    }
    const controller = new AbortController();
    abortRef.current = controller;
    setRunning(true);
    try {
      const result = await run(query, { signal: controller.signal });
      setOutcome(result.outcome);
    } finally {
      abortRef.current = null;
      setRunning(false);
    }
  }, [run, query]);

  // (sq-plqfs) [SONNET-4.6] ⌘/Ctrl-Enter fires Run from within the editor.
  const onKeyDown = React.useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        void onRun();
      }
    },
    [onRun],
  );

  const ready = status.kind === "ready";

  return (
    <div className="flex h-full flex-col" data-tool-panel="graph-view">
      {/* Action row */}
      <div className="flex items-center gap-2 border-b bg-card px-3 py-1.5">
        <Network className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="text-xs font-medium text-muted-foreground">Graph view</span>
        <Badge variant="outline" className="h-5 gap-1 text-[10px]" title="Where this query runs">
          LOCAL · in-tab WASM
        </Badge>
        <div className="ml-auto flex items-center gap-1.5">
          <Button
            size="sm"
            onClick={() => void onRun()}
            disabled={!ready || running}
            title="Run the CONSTRUCT or DESCRIBE over the live store"
            data-graph-view-run
          >
            {running ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />}
            Run
          </Button>
        </div>
      </div>

      {/* (sq-plqfs) Syntax-highlighting SPARQL editor — replaces the plain <textarea>.
          The id="graph-view-query" hook is preserved on the inner <textarea>
          (WorkbenchSparqlEditor passes it through). Fixed-height wrapper keeps the editor
          compact so the result graph fills most of the panel. */}
      <div className="flex h-28 flex-col border-b">
        <WorkbenchSparqlEditor
          id="graph-view-query"
          value={query}
          onChange={setQuery}
          onKeyDown={onKeyDown}
          ariaLabel="CONSTRUCT or DESCRIBE query"
        />
      </div>

      {/* Result area */}
      <div className="relative min-h-0 flex-1 overflow-auto" data-graph-view-result>
        {outcome === null ? (
          <p className="p-4 text-sm text-muted-foreground">
            {ready
              ? "Run a CONSTRUCT or DESCRIBE query to see the result as a node-link graph."
              : "Waiting for the engine to warm…"}
          </p>
        ) : outcome.kind === "graph" ? (
          <GraphView ntriples={outcome.ntriples} inferred={inferred} />
        ) : outcome.kind === "error" ? (
          <pre
            className="overflow-auto whitespace-pre-wrap p-3 font-mono text-xs text-destructive"
            data-result-kind="error"
          >
            {outcome.message}
          </pre>
        ) : outcome.kind === "cancelled" ? (
          <p className="p-4 text-sm text-muted-foreground">Run stopped.</p>
        ) : (
          <p className="p-4 text-sm text-muted-foreground">
            Use a{" "}
            <code className="rounded bg-muted px-1 py-0.5">CONSTRUCT</code> or{" "}
            <code className="rounded bg-muted px-1 py-0.5">DESCRIBE</code> query to generate a
            graph result that can be visualised here.
          </p>
        )}
        {/* [GPT-5.6] why() overlay for an inferred edge clicked in this standalone tool. */}
        {explainTarget && (
          <ProofPanel target={explainTarget} onClose={() => setExplainTarget(null)} />
        )}
      </div>
    </div>
  );
}
