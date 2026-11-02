"use client";

// [OPUS-4.8] sq-ixc3.12 — the QUERY WORKBENCH: the default work surface (research/gui-design.md
// §A.4 + §A.5). A vertical split:
//   TOP    = the full-height SPARQL editor (the ported `WorkbenchSparqlEditor`, reusing the
//            `@sparq/client` tokenizer + prefix helper) + a thin ACTION ROW:
//            Run (⌘↵) · EXPLAIN · ANALYZE · Stop · a target chip (LOCAL / ENDPOINT).
//   BOTTOM = a tabbed RESULTS panel with FOUR co-resident views — Table · Graph · Raw JSON ·
//            N-Triples/Turtle — over the SAME run, plus CSV / TSV / JSON export.
// SELECT rows STREAM through the wasm cursor (`streamQueryRows` in the engine context) so a large
// result never materialises whole in JS; Stop cancels cooperatively between batches. Each run's
// MEASURED `performance.now()` latency is shown, labelled — never a benchmark claim.
//
// DISTINCT from the site /try (§A.5): NO hero / prose / dataset picker (datasets live in the left
// rail) — it is a dense, full-height operational tool over the PERSISTENT live store.
//
// E2E CONTRACT (gui/e2e/run-e2e.mjs asserts these stable hooks — kept faithful so the harness
// cannot silently drift): the editing <textarea id="repl-query">, a "Run query" button, the
// "Engine ready" status copy (top bar), and the result container's data-result-kind="select" +
// data-result-view="table" wrapping a <table> with the binding.

import * as React from "react";
import {
  Play,
  Loader2,
  Square,
  Network,
  Activity,
  Telescope,
  Gauge,
  History,
  ChevronFirst,
  ChevronLast,
  ChevronLeft,
  ChevronRight,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { downloadText } from "@/lib/download";
// [FABLE-5] sq-ixc3.19 — the shared EXPLAIN/ANALYZE operator-tree renderer (plan explorer).
import { PlanTree } from "@/components/workbench/plan-tree";
import {
  useEngine,
  DEFAULT_ROW_CAP,
  type QueryOutcome,
  type RunMode,
} from "@/lib/engine-context";
import {
  extractTable,
  createPageCache,
  resultsToCsv,
  resultsToTsv,
  formatSparqlJson,
  prettyTurtle,
  tokenizeTurtle,
  type SparqlResults,
  type TurtleToken,
} from "@sparq/client";
import { WorkbenchSparqlEditor } from "@/components/workbench/sparql-editor";
import { GraphView, type InferredAffordance } from "@/components/workbench/graph-view";
// [FABLE-5] sq-ixc3.20 — click-to-explain inferred facts: the why() proof-tree panel + the
// canonical triple identity that gates the affordance (membership in the closure-added set).
import { ProofPanel, type ExplainTarget } from "@/components/workbench/proof-panel";
import {
  inferredFactsMatchingKeys,
  termToNT,
  tripleKeyOfBindings,
  type InferredFact,
} from "@/lib/inferred-facts";
// [OPUS-4.8] sq-tp1m (#757) — the per-workspace inference (RDFS / OWL 2 RL) selector, in the
// action row so the active entailment regime is visible + controllable while querying.
import { InferenceControl } from "@/components/workbench/inference-control";
// [FABLE-5] sq-ixc3.14 — the federation allowlist editor + the honest run-location badge.
import { FederationControl, RunLocationBadge } from "@/components/workbench/federation-control";
import { useWorkspace } from "@/lib/workspace-context";
import { DEFAULT_QUERY } from "@/data/sample-graph";
// [FABLE-5] sq-ixc3.14 — the honesty-override type for QUERY_TOOL_OVERRIDE (sq-5lyme seam).
import type { ToolOverride } from "@/data/tools";
// [OPUS-4.8] sq-ixc3.10 — the Query tool contributes its operational verbs (run / EXPLAIN /
// EXPLAIN ANALYZE / re-run a recent query) to the Cmd-K spine while it is mounted.
import { useRegisterPaletteCommands } from "@/components/workbench/command-palette";
import { useWorkbench } from "@/components/workbench/workbench-context";
import {
  pushRecentQuery,
  previewQuery,
  type PaletteCommand,
  type RecentQuery,
} from "@/lib/palette-commands";

/** The co-resident result views the panel toggles between (all over the same run). */
type ResultView = "table" | "graph" | "json" | "ntriples";

// [SONNET-4.6] #3602 — SELECT graph rendering is optional workbench code. Keep it out of the
// initial GUI bundle and load it only when a user opens Graph for a SELECT result.
const SelectResultGraphView = React.lazy(() =>
  import("@/components/workbench/select-result-graph-view").then((module) => ({
    default: module.SelectResultGraphView,
  })),
);

const VIEWS: { id: ResultView; label: string }[] = [
  { id: "table", label: "Table" },
  { id: "graph", label: "Graph" },
  { id: "json", label: "Raw JSON" },
  { id: "ntriples", label: "N-Triples / Turtle" },
];

// ---------------------------------------------------------------------------
// View bodies.
// ---------------------------------------------------------------------------

/** The page-size choices the results table offers (rows rendered per page). */
const PAGE_SIZES = [50, 100, 250, 500] as const;
const DEFAULT_PAGE_SIZE = 100;

// [OPUS-4.8] sq-9w4t (#817) — paginated SELECT table. Only the VISIBLE page of rows is shaped
// into <td> cells (serialising/rendering the whole kept result is the per-render cost #817
// calls out), and a bounded read-ahead PAGE CACHE (createPageCache, ~2 pages ahead) warms the
// next page so ⏭ is instant. The cache is rebuilt only when the underlying result or page size
// changes — paging itself never re-extracts the table. NOTE this paginates over the rows the
// engine ALREADY streamed into JS; demand-driven query EVALUATION up to the current page needs
// a pull-iterator exec model the engine lacks today (gated — see results.ts / gui-design.md).
function SelectTable({
  results,
  inferred,
}: {
  results: SparqlResults;
  /** [FABLE-5] sq-ixc3.20 — mark + explain inferred rows (absent = no affordance). */
  inferred?: InferredAffordance | null;
}) {
  const table = React.useMemo(() => extractTable(results), [results]);
  const [pageSize, setPageSize] = React.useState<number>(DEFAULT_PAGE_SIZE);
  const [page, setPage] = React.useState(0);

  // One cache per (result, pageSize). Reset to the first page whenever either changes so a new
  // run / resize never strands the view on an out-of-range page.
  const cache = React.useMemo(
    () => createPageCache(table, pageSize, 2),
    [table, pageSize],
  );
  const [previousCache, setPreviousCache] = React.useState(cache);
  if (previousCache !== cache) {
    setPreviousCache(cache);
    setPage(0);
  }

  // [FABLE-5] sq-ixc3.20 — a SELECT row is a solution, not inherently a triple; it earns
  // the why() affordance ONLY when its first three bound terms ARE a triple the active
  // closure ADDED (exact membership in the entailed set — never a heuristic — so an
  // asserted fact, or a row that is not a fact at all, can never show the affordance).
  const rowTarget = React.useCallback(
    (absRow: number): ExplainTarget | null => {
      if (!inferred || table.vars.length < 3) return null;
      const b = results.results?.bindings?.[absRow];
      if (!b) return null;
      const s = b[table.vars[0]];
      const p = b[table.vars[1]];
      const o = b[table.vars[2]];
      if (!s || !p || !o) return null;
      if (!inferred.keys.has(tripleKeyOfBindings(s, p, o))) return null;
      return { s: termToNT(s), p: termToNT(p), o: termToNT(o) };
    },
    [inferred, table, results],
  );
  const showWhyColumn = inferred != null && table.vars.length >= 3;

  if (table.vars.length === 0) {
    return <p className="p-3 text-sm text-muted-foreground">No projected variables.</p>;
  }

  // `cache.get` clamps + warms the read-ahead window; render only its returned slice.
  const current = cache.get(page);
  const { rows, totalRows, totalPages, startRow } = current;
  const firstRow = totalRows === 0 ? 0 : startRow + 1;
  const lastRow = startRow + rows.length;
  const atFirst = current.page <= 0;
  const atLast = current.page >= totalPages - 1;

  return (
    <div className="flex h-full flex-col" data-result-view="table">
      <div className="min-h-0 flex-1 overflow-auto">
        <table className="w-full border-collapse text-sm">
          {/* [OPUS-4.8] sq-vw3ax (#820 redesign) — teal-capped sticky header (the proposal's IDE
              result rhythm). The token colour follows the theme. */}
          <thead className="sq-result-head sticky top-0">
            <tr>
              {table.vars.map((v) => (
                <th
                  key={v}
                  className="border-b px-3 py-1.5 text-left font-mono text-xs font-semibold"
                >
                  ?{v}
                </th>
              ))}
              {/* [FABLE-5] sq-ixc3.20 — the why? affordance column (inference active only). */}
              {showWhyColumn && (
                <th className="w-14 border-b px-2 py-1.5" aria-label="Explain inferred facts" />
              )}
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 ? (
              <tr>
                <td
                  colSpan={table.vars.length + (showWhyColumn ? 1 : 0)}
                  className="px-3 py-3 text-center text-muted-foreground"
                >
                  0 rows
                </td>
              </tr>
            ) : (
              rows.map((row, i) => {
                // [FABLE-5] sq-ixc3.20 — non-null ONLY for a row whose s/p/o is a triple
                // the closure added: asserted facts get NO affordance (the honesty gate).
                const target = showWhyColumn ? rowTarget(startRow + i) : null;
                return (
                  <tr
                    key={startRow + i}
                    className="odd:bg-muted/30"
                    {...(target ? { "data-inferred-row": true } : {})}
                  >
                    {row.map((cell, j) => (
                      <td key={j} className="border-b px-3 py-1 font-mono text-xs">
                        {cell}
                      </td>
                    ))}
                    {showWhyColumn && (
                      <td className="border-b px-2 py-1 text-right">
                        {target && (
                          <button
                            type="button"
                            className="rounded border border-transparent bg-primary/10 px-1.5 py-0 font-mono text-[10px] text-primary hover:bg-primary/20"
                            onClick={() => inferred!.onExplain(target)}
                            title="This fact is inferred (not asserted) — see its derivation"
                            data-explain-trigger
                          >
                            why?
                          </button>
                        )}
                      </td>
                    )}
                  </tr>
                );
              })
            )}
          </tbody>
        </table>
      </div>
      {/* Pager: only shown when there is more than one page of kept rows. */}
      {totalRows > 0 && (
        <div
          className="flex items-center gap-2 border-t bg-card px-3 py-1 text-[11px] text-muted-foreground"
          data-result-pager
        >
          <span className="tabular" data-pager-range>
            {firstRow.toLocaleString()}–{lastRow.toLocaleString()} of{" "}
            {totalRows.toLocaleString()}
          </span>
          <div className="ml-auto flex items-center gap-1">
            <label className="flex items-center gap-1">
              <span>Rows / page</span>
              <select
                className="rounded border bg-background px-1 py-0.5 text-[11px]"
                value={pageSize}
                onChange={(e) => setPageSize(Number(e.target.value))}
                aria-label="Rows per page"
              >
                {PAGE_SIZES.map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
              </select>
            </label>
            <button
              className="rounded p-0.5 hover:bg-accent/40 disabled:opacity-30"
              onClick={() => setPage(0)}
              disabled={atFirst}
              title="First page"
              aria-label="First page"
            >
              <ChevronFirst className="size-3.5" />
            </button>
            <button
              className="rounded p-0.5 hover:bg-accent/40 disabled:opacity-30"
              onClick={() => setPage((p) => p - 1)}
              disabled={atFirst}
              title="Previous page"
              aria-label="Previous page"
            >
              <ChevronLeft className="size-3.5" />
            </button>
            <span className="tabular px-1" data-pager-page>
              Page {(current.page + 1).toLocaleString()} / {totalPages.toLocaleString()}
            </span>
            <button
              className="rounded p-0.5 hover:bg-accent/40 disabled:opacity-30"
              onClick={() => setPage((p) => p + 1)}
              disabled={atLast}
              title="Next page"
              aria-label="Next page"
            >
              <ChevronRight className="size-3.5" />
            </button>
            <button
              className="rounded p-0.5 hover:bg-accent/40 disabled:opacity-30"
              onClick={() => setPage(totalPages - 1)}
              disabled={atLast}
              title="Last page"
              aria-label="Last page"
            >
              <ChevronLast className="size-3.5" />
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

const TURTLE_TOKEN_CLASS: Record<TurtleToken["type"], string> = {
  keyword: "sq-tok-keyword",
  variable: "sq-tok-variable",
  iri: "sq-tok-iri",
  prefixed: "sq-tok-prefixed",
  string: "sq-tok-string",
  number: "sq-tok-number",
  comment: "sq-tok-comment",
  punctuation: "",
  plain: "",
};

/** [GPT-5.6] Highlight one RDF text fragment without changing its line/triple identity. */
function RdfTokens({ text }: { text: string }) {
  const tokens = React.useMemo(() => tokenizeTurtle(text), [text]);
  return tokens.map((t, i) => {
    const cls = TURTLE_TOKEN_CLASS[t.type];
    return cls ? (
      <span key={i} className={cls}>
        {t.text}
      </span>
    ) : (
      <React.Fragment key={i}>{t.text}</React.Fragment>
    );
  });
}

/** A highlighted pretty-Turtle document. */
function RdfDocument({ ntriples }: { ntriples: string }) {
  const text = React.useMemo(() => {
    try {
      return prettyTurtle(ntriples);
    } catch {
      // The serialiser never throws by design, but degrade to the raw form if it ever does.
      return ntriples;
    }
  }, [ntriples]);
  return (
    <pre className="overflow-auto whitespace-pre p-3 font-mono text-xs">
      <RdfTokens text={text} />
    </pre>
  );
}

/** [GPT-5.6] One raw result line plus its exact inferred membership, if any. */
interface NTriplesLine {
  text: string;
  fact: InferredFact | null;
}

/** [GPT-5.6] Raw N-Triples keeps one statement per line, so why() can be attached precisely. */
function RawNTriplesDocument({
  lines,
  onExplain,
}: {
  lines: readonly NTriplesLine[];
  onExplain?: (target: ExplainTarget) => void;
}) {
  return (
    <div className="min-w-max py-2 font-mono text-xs" data-ntriples-lines>
      {lines.map(({ text, fact }, index) => (
        <div
          key={index}
          className={cn(
            "flex min-h-5 items-start gap-2 px-3",
            fact && "border-l-2 border-primary bg-primary/5 pl-2.5",
          )}
          {...(fact ? { "data-inferred-line": true } : {})}
        >
          <pre className="min-w-0 flex-1 whitespace-pre">
            <RdfTokens text={text} />
          </pre>
          {fact && onExplain ? (
            <button
              type="button"
              className="sticky right-2 shrink-0 rounded border border-transparent bg-primary/10 px-1.5 py-0 text-[10px] text-primary hover:bg-primary/20"
              onClick={() => onExplain(fact)}
              aria-label={`Explain inferred fact: ${fact.ntriples}`}
              title="This fact is inferred (not asserted) — see its derivation"
              data-explain-trigger
              data-ntriples-explain
            >
              why?
            </button>
          ) : null}
        </div>
      ))}
    </div>
  );
}

/** The N-Triples / Turtle view, with a pretty-Turtle ⇄ raw-N-Triples toggle. */
function GraphTextView({
  ntriples,
  inferred,
}: {
  ntriples: string;
  /** [GPT-5.6] Exact inferred-line membership + proof-panel opener. */
  inferred?: InferredAffordance | null;
}) {
  const [pretty, setPretty] = React.useState(true);
  const lines = React.useMemo<readonly NTriplesLine[]>(() => {
    return ntriples.split("\n").map((text) => ({
      text,
      fact: inferred ? (inferredFactsMatchingKeys(text, inferred.keys)[0] ?? null) : null,
    }));
  }, [ntriples, inferred]);
  const hasInferredLines = lines.some((line) => line.fact !== null);
  if (!ntriples.trim()) {
    return <p className="p-3 text-sm text-muted-foreground">(empty graph)</p>;
  }
  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-1 border-b bg-card px-3 py-1">
        {[
          { id: true, label: "Turtle" },
          { id: false, label: "N-Triples" },
        ].map((opt) => (
          <button
            key={String(opt.id)}
            type="button"
            onClick={() => setPretty(opt.id)}
            aria-pressed={pretty === opt.id}
            className={cn(
              "rounded px-2 py-0.5 text-[11px]",
              pretty === opt.id
                ? "bg-primary/10 font-medium text-primary"
                : "text-muted-foreground hover:bg-accent/40",
            )}
          >
            {opt.label}
          </button>
        ))}
        {hasInferredLines ? (
          <span
            className="ml-auto text-[10px] text-muted-foreground"
            data-text-inferred-hint
          >
            {pretty
              ? "Switch to N-Triples to explain inferred facts."
              : "Marked lines are inferred — choose why? for a derivation."}
          </span>
        ) : null}
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        {pretty ? (
          <RdfDocument ntriples={ntriples} />
        ) : (
          <RawNTriplesDocument lines={lines} onExplain={inferred?.onExplain} />
        )}
      </div>
    </div>
  );
}

function ExportRow({ results }: { results: SparqlResults }) {
  return (
    <div className="ml-auto flex items-center gap-1">
      {(
        [
          ["CSV", "sparql-results.csv", "text/csv", resultsToCsv],
          ["TSV", "sparql-results.tsv", "text/tab-separated-values", resultsToTsv],
          [
            "JSON",
            "sparql-results.json",
            "application/sparql-results+json",
            formatSparqlJson,
          ],
        ] as const
      ).map(([label, file, mime, fn]) => (
        <Button
          key={label}
          variant="outline"
          size="sm"
          className="h-6 px-2 text-[11px]"
          onClick={() => downloadText(file, fn(results), mime)}
          title={`Download the kept rows as ${label}`}
        >
          {label}
        </Button>
      ))}
    </div>
  );
}

function ResultBody({
  outcome,
  view,
  inferred,
}: {
  outcome: QueryOutcome;
  view: ResultView;
  /** [GPT-5.6] sq-l54uy — the inferred-fact affordance (table + graph + N-Triples views). */
  inferred?: InferredAffordance | null;
}) {
  if (outcome.kind === "error") {
    return (
      <pre
        className="overflow-auto whitespace-pre-wrap p-3 font-mono text-xs text-destructive"
        data-result-kind="error"
      >
        {outcome.message}
      </pre>
    );
  }
  if (outcome.kind === "cancelled") {
    return (
      <p className="p-3 text-sm text-muted-foreground" data-result-kind="cancelled">
        Run stopped.
      </p>
    );
  }
  if (outcome.kind === "explain") {
    // [FABLE-5] sq-ixc3.19 — the structured plan renders as the navigable operator tree
    // (per-operator est/actual rows, q-error heat, time); a lean bundle without the
    // explain-json binding still gets the text plan verbatim (never a synthesised tree).
    if (outcome.tree) {
      return (
        <div data-result-kind="explain">
          <PlanTree
            tree={outcome.tree}
            analyzed={outcome.mode === "analyze"}
            source={outcome.source ?? "wasm"}
          />
        </div>
      );
    }
    return (
      <pre
        className="overflow-auto whitespace-pre p-3 font-mono text-xs"
        data-result-kind="explain"
      >
        {outcome.plan || "(no plan)"}
      </pre>
    );
  }
  if (outcome.kind === "ask") {
    return (
      <div className="p-3 text-sm" data-result-kind="ask">
        {view === "json" ? (
          <pre className="overflow-auto font-mono text-xs">{outcome.rawJson}</pre>
        ) : (
          <span className="font-mono">
            ASK → <strong>{String(outcome.value)}</strong>
          </span>
        )}
      </div>
    );
  }
  if (outcome.kind === "graph") {
    if (view === "graph") return <GraphView ntriples={outcome.ntriples} inferred={inferred} />;
    if (view === "table") {
      return (
        <p className="p-3 text-sm text-muted-foreground" data-result-kind="graph">
          This is a graph result (CONSTRUCT / DESCRIBE). Use the Graph or N-Triples / Turtle view.
        </p>
      );
    }
    if (view === "json") {
      return (
        <pre
          className="overflow-auto whitespace-pre p-3 font-mono text-xs"
          data-result-kind="graph"
          data-result-view="json"
        >
          {outcome.ntriples || "(empty graph)"}
        </pre>
      );
    }
    return (
      <div className="h-full" data-result-kind="graph">
        <GraphTextView ntriples={outcome.ntriples} inferred={inferred} />
      </div>
    );
  }
  if (outcome.kind === "update") {
    return (
      <div className="p-3 text-sm" data-result-kind="update">
        Update applied. Store now holds{" "}
        <span className="tabular font-medium">{outcome.sizeAfter.toLocaleString()}</span> quads.
      </div>
    );
  }
  // select
  return (
    <div className="flex h-full flex-col" data-result-kind="select">
      {outcome.truncated && (
        <p className="border-b bg-warning/10 px-3 py-1 text-[11px] text-muted-foreground">
          Showing the first {outcome.rowCount.toLocaleString()} of{" "}
          {outcome.totalRows.toLocaleString()} rows (display cap{" "}
          {DEFAULT_ROW_CAP.toLocaleString()}). The full result streamed through the engine; the
          CSV / TSV / JSON export covers the kept rows.
        </p>
      )}
      <div className="min-h-0 flex-1 overflow-auto">
        {view === "table" ? (
          <SelectTable results={outcome.results} inferred={inferred} />
        ) : view === "json" ? (
          <pre className="overflow-auto p-3 font-mono text-xs" data-result-view="json">
            {outcome.rawJson}
          </pre>
        ) : view === "graph" ? (
          <React.Suspense
            fallback={<p className="p-3 text-sm text-muted-foreground">Loading Graph view…</p>}
          >
            <SelectResultGraphView results={outcome.results} />
          </React.Suspense>
        ) : (
          <p className="p-3 text-sm text-muted-foreground">
            N-Triples / Turtle applies to CONSTRUCT / DESCRIBE results.
          </p>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// The workbench.
// ---------------------------------------------------------------------------

// [FABLE-5] sq-ixc3.14 — honesty override (the sq-5lyme seam: copy flips live in the panel
// file that earns them, never by editing data/tools.ts). The Query tool now also executes
// federated SERVICE queries — on the DESKTOP's native engine, gated by the per-workspace
// egress allowlist; the browser build labels that half native-only instead of pretending.
export const QUERY_TOOL_OVERRIDE: ToolOverride = {
  blurb:
    "Run SPARQL 1.1/1.2 over the live store — SELECT/ASK/CONSTRUCT/DESCRIBE/UPDATE, plus " +
    "federated SERVICE on the desktop's native engine (allowlist-gated, fail-closed).",
};

export function QueryWorkbench() {
  // [OPUS-4.8] sq-ixc3.10/.12 — EXPLAIN is the canonical run(query, { mode }) path (no standalone
  // explain() method); the Cmd-K verbs and the EXPLAIN/ANALYZE buttons both drive it.
  const { run, status, entailedTripleKeys, inferenceMode, inferenceStatus } = useEngine();
  const workbench = useWorkbench();
  // [OPUS-4.8] sq-lcd6e — the editor text round-trips through the active workspace so a saved
  // query survives a reload / workspace switch (the persisted editor state was never restored
  // before). `workspace` starts null (restore is async); we seed the editor from it once, on the
  // first restore AND on every workspace-id change, and write the text back (debounced) below.
  const { workspace, setEditorQuery, recordUpdateSnapshot } = useWorkspace();
  const [query, setQuery] = React.useState(workspace?.editor.query ?? DEFAULT_QUERY);
  // The id of the workspace whose editor text is currently loaded — guards the write-back from
  // firing (and clobbering the saved query) before the restore has hydrated the editor.
  const [loadedWorkspaceId, setLoadedWorkspaceId] = React.useState(workspace?.id ?? null);
  const [outcome, setOutcome] = React.useState<QueryOutcome | null>(null);
  const [view, setView] = React.useState<ResultView>("table");
  const [running, setRunning] = React.useState(false);
  const [runLatencyMs, setRunLatencyMs] = React.useState<number | null>(null);
  const abortRef = React.useRef<AbortController | null>(null);
  // [OPUS-4.8] sq-ixc3.10 — a session-only ring of recently-run queries for the palette.
  const [recentQueries, setRecentQueries] = React.useState<RecentQuery[]>([]);
  // [FABLE-5] sq-ixc3.20 — the inferred fact whose why() proof panel is open, or null.
  const [explainTarget, setExplainTarget] = React.useState<ExplainTarget | null>(null);

  // [FABLE-5] sq-ixc3.20 — the inferred-fact affordance for the results views: live only
  // while the ACTIVE regime's closure is materialised (`inferenceStatus` is the reactive
  // signal; `entailedTripleKeys` the key-guarded set) and something was actually entailed.
  const inferred = React.useMemo<InferredAffordance | null>(() => {
    if (inferenceMode === "off" || inferenceStatus.kind !== "ready") return null;
    const keys = entailedTripleKeys();
    if (!keys || keys.size === 0) return null;
    return { keys, onExplain: setExplainTarget };
  }, [inferenceMode, inferenceStatus, entailedTripleKeys]);

  // A regime change invalidates an open proof (it was derived under the previous regime).
  const [previousInferenceMode, setPreviousInferenceMode] = React.useState(inferenceMode);
  if (previousInferenceMode !== inferenceMode) {
    setPreviousInferenceMode(inferenceMode);
    setExplainTarget(null);
  }

  const recordRecent = React.useCallback((q: string) => {
    setRecentQueries((prev) => pushRecentQuery(prev, q, Date.now()));
  }, []);

  // Hydrate once per workspace id, preserving in-progress edits within the same workspace.
  if (workspace && loadedWorkspaceId !== workspace.id) {
    setLoadedWorkspaceId(workspace.id);
    setQuery(workspace.editor.query);
  }

  // Write back only after restore has hydrated the editor, so the default cannot overwrite it.
  React.useEffect(() => {
    if (loadedWorkspaceId === null) return;
    const handle = setTimeout(() => {
      void setEditorQuery(query);
    }, 400);
    return () => clearTimeout(handle);
  }, [loadedWorkspaceId, query, setEditorQuery]);

  // [OPUS-4.8] sq-ixc3.10/.12 — the SINGLE run path: plain run, EXPLAIN (plan only), or EXPLAIN
  // ANALYZE (plan + run), each surfaced as a { kind } outcome through the same RunResult pipeline.
  // The Cmd-K spine verbs and the EXPLAIN/ANALYZE toolbar buttons both call this with a mode.
  const onRun = React.useCallback(
    async (mode: RunMode = "run") => {
      // A run already in flight: ignore (Stop is the way to cancel).
      if (abortRef.current) return;
      recordRecent(query);
      const controller = new AbortController();
      abortRef.current = controller;
      setRunning(true);
      try {
        const result = await run(query, { mode, signal: controller.signal });
        setOutcome(result.outcome);
        setRunLatencyMs(result.latencyMs);
        // (sq-7gdfp) — snapshot the live store after a successful SPARQL UPDATE so INSERT/DELETE
        // data survives a page reload. A failed update yields outcome.kind === "error" and never
        // reaches here, so the snapshot is never taken on failure.
        if (result.outcome.kind === "update") {
          void recordUpdateSnapshot();
        }
        // Pick the most useful default view for the result shape.
        if (result.outcome.kind === "graph") {
          if (view === "table" || view === "json") setView("graph");
        } else if (result.outcome.kind === "select" || result.outcome.kind === "ask") {
          if (view === "graph" || view === "ntriples") setView("table");
        }
      } finally {
        abortRef.current = null;
        setRunning(false);
      }
    },
    [run, query, view, recordRecent, recordUpdateSnapshot],
  );

  const onStop = React.useCallback(() => {
    abortRef.current?.abort();
  }, []);

  // ⌘/Ctrl-Enter runs the query (the keyboard-first spine; the full Cmd-K palette is sq-ixc3.10).
  const onKeyDown = React.useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        void onRun("run");
      }
    },
    [onRun],
  );

  const ready = status.kind === "ready";
  const canExport = outcome?.kind === "select";

  // [OPUS-4.8] sq-ixc3.10 — register the Query tool's operational commands on the Cmd-K spine. A
  // palette verb focuses the Query tab first (so the result lands where the user looks), then acts.
  const paletteCommands = React.useMemo<PaletteCommand[]>(() => {
    const focusQuery = () => workbench?.openTool("query");
    return [
      {
        id: "query.run",
        group: "Actions",
        title: "Run query",
        blurb: "Execute the editor's SPARQL over the live store",
        keywords: ["run", "execute", "query", "go"],
        icon: Play,
        disabled: !ready || running,
        run: () => {
          focusQuery();
          void onRun();
        },
      },
      {
        id: "query.explain",
        group: "Actions",
        title: "Run EXPLAIN",
        blurb: "Show the query plan without executing it",
        keywords: ["explain", "plan", "planner", "optimize"],
        icon: Telescope,
        disabled: !ready || running,
        run: () => {
          focusQuery();
          void onRun("explain");
        },
      },
      {
        id: "query.analyze",
        group: "Actions",
        title: "Run EXPLAIN ANALYZE",
        blurb: "Plan + execute with a per-operator trace (SELECT/ASK)",
        keywords: ["analyze", "analyse", "trace", "profile", "explain"],
        icon: Gauge,
        disabled: !ready || running,
        run: () => {
          focusQuery();
          void onRun("analyze");
        },
      },
      ...recentQueries.map((r): PaletteCommand => {
        const preview = previewQuery(r.query);
        return {
          id: `query.recent.${r.ranAt}`,
          group: "Recent queries",
          title: preview,
          keywords: ["recent", "history", "query", preview],
          icon: History,
          run: () => {
            focusQuery();
            setQuery(r.query);
          },
        };
      }),
    ];
  }, [ready, running, recentQueries, onRun, workbench]);

  useRegisterPaletteCommands("query", paletteCommands);

  return (
    <div className="flex h-full flex-col">
      {/* Editor pane (the bigger half). */}
      <div className="flex min-h-0 flex-[3] flex-col border-b">
        {/* Action row. */}
        <div className="flex items-center gap-2 border-b bg-card px-3 py-1.5">
          <span className="text-xs font-medium text-muted-foreground">SPARQL</span>
          {/* [FABLE-5] sq-ixc3.14 — the HONEST run-location badge: in-tab WASM for a plain
              query; the desktop's native engine (allowlist-gated) when SERVICE is detected;
              native-only labelling on the web build instead of pretending to federate. */}
          <RunLocationBadge query={query} />
          {/* [OPUS-4.8] sq-tp1m — the per-workspace inference regime (queries run with the chosen
              RDFS / OWL 2 RL entailment applied by the engine). */}
          <InferenceControl className="ml-2" />
          {/* [FABLE-5] sq-ixc3.14 — the per-workspace federation egress allowlist (fail-closed:
              SERVICE may dial ONLY these endpoints, enforced by the native engine). */}
          <FederationControl className="ml-1" />
          <div className="ml-auto flex items-center gap-1.5">
            <Button
              size="sm"
              onClick={() => void onRun("run")}
              disabled={!ready || running}
              className="sq-glow-btn border-transparent"
              title="Run the query against the live store (⌘↵)"
            >
              {running ? <Loader2 className="animate-spin" /> : <Play />}
              Run query
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => void onRun("explain")}
              disabled={!ready || running}
              title="Show the planner's chosen plan (does not execute)"
            >
              <Network className="size-3.5" />
              EXPLAIN
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => void onRun("analyze")}
              disabled={!ready || running}
              title="Execute and trace the plan (EXPLAIN ANALYZE)"
            >
              <Activity className="size-3.5" />
              ANALYZE
            </Button>
            <Button
              variant="destructive"
              size="sm"
              onClick={onStop}
              disabled={!running}
              title="Stop the current run"
            >
              <Square className="size-3.5" />
              Stop
            </Button>
            <span className="ml-1 text-[11px] text-muted-foreground">⌘↵</span>
          </div>
        </div>
        <WorkbenchSparqlEditor
          id="repl-query"
          value={query}
          onChange={setQuery}
          onKeyDown={onKeyDown}
        />
      </div>

      {/* Results pane. [FABLE-5] sq-ixc3.20 — `relative` hosts the proof-panel overlay. */}
      <div className="relative flex min-h-0 flex-[2] flex-col">
        <div className="flex items-center gap-1 border-b bg-card px-3 py-1">
          {VIEWS.map((v) => (
            <button
              key={v.id}
              onClick={() => setView(v.id)}
              className={cn(
                "rounded px-2 py-0.5 text-xs",
                view === v.id
                  ? "bg-primary/10 font-medium text-primary"
                  : "text-muted-foreground hover:bg-accent/40",
              )}
            >
              {v.label}
            </button>
          ))}
          {/* MEASURED per-run latency (performance.now), labelled — never a benchmark. */}
          {runLatencyMs !== null && (
            <span
              className="tabular ml-3 text-[11px] text-muted-foreground"
              title="Wall-clock latency of the last run (performance.now) — measured, not a benchmark"
            >
              {runLatencyMs.toFixed(1)} ms
            </span>
          )}
          {canExport && outcome.kind === "select" && <ExportRow results={outcome.results} />}
        </div>
        <div className="min-h-0 flex-1 overflow-auto">
          {/* [OPUS-4.8] sq-ixc3.10/.12 — EXPLAIN / ANALYZE plans render through ResultBody as the
              { kind: "explain" } outcome (data-result-kind="explain"), the same pipeline as every
              other run — there is no separate plan pane. */}
          {outcome === null ? (
            <p className="p-3 text-sm text-muted-foreground">
              {ready
                ? "Run a query to see results. SELECT rows stream so large results stay bounded; EXPLAIN / ANALYZE show the plan."
                : "Waiting for the engine to warm…"}
            </p>
          ) : (
            <ResultBody outcome={outcome} view={view} inferred={inferred} />
          )}
        </div>
        {/* [FABLE-5] sq-ixc3.20 — the why() proof-tree panel for the clicked inferred fact. */}
        {explainTarget && (
          <ProofPanel target={explainTarget} onClose={() => setExplainTarget(null)} />
        )}
      </div>
    </div>
  );
}
