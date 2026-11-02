"use client";

// [OPUS-4.8] sq-xoxu — the live /surface/full-text playground: a tiny RDF corpus + a
// `text:`-predicate SELECT (both editable) → BM25-ranked literal hits, run entirely in your
// tab via the separate, lazy-loaded W-text wasm bundle (TextSearch.query / .indexStats).
// The default is the classic "quick brown fox" example from skills/full-text-search/SKILL.md.
// Nothing is sent to a server.

import * as React from "react";
import { useHydrated } from "@/lib/use-hydrated";
import { Play, Loader2, Search, CheckCircle2 } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { cn } from "@/lib/utils";
import {
  prewarmText,
  sparqTextQuery,
  sparqTextIndexStats,
  type TextIndexStats,
} from "@/lib/sparq-text-wasm";
import { resultCells } from "@/lib/text-results";
import type { SparqlResults } from "@/lib/sparq-wasm";
import { TEXT_EXAMPLES, type TextExample } from "@/data/text-examples";

type EngineState = "cold" | "warming" | "ready" | "error";

type RunState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "results"; results: SparqlResults; stats: TextIndexStats; ms: number }
  | { kind: "error"; message: string };

const DEFAULT = TEXT_EXAMPLES[0];

export function TextPlayground() {
  const [example, setExample] = React.useState<TextExample>(DEFAULT);
  const [data, setData] = React.useState(DEFAULT.data);
  const [query, setQuery] = React.useState(DEFAULT.query);
  const [engineState, setEngine] = React.useState<EngineState>("cold");
  const hydrated = useHydrated();
  const engine = hydrated && engineState === "cold" ? "warming" : engineState;
  const [state, setState] = React.useState<RunState>({ kind: "idle" });

  // Pre-warm the (separate) full-text wasm bundle on mount so the first "Search" pays no
  // cold start. A failure flips the indicator; the first query retries the load.
  React.useEffect(() => {
    let cancelled = false;
    prewarmText()
      .then(() => {
        if (!cancelled) setEngine("ready");
      })
      .catch((e) => {
        if (cancelled) return;
        setEngine("error");
        toast.error("Full-text engine failed to load", {
          description: e instanceof Error ? e.message : String(e),
        });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const selectExample = React.useCallback((id: string) => {
    const ex = TEXT_EXAMPLES.find((e) => e.id === id);
    if (!ex) return;
    setExample(ex);
    setData(ex.data);
    setQuery(ex.query);
    setState({ kind: "idle" });
  }, []);

  const run = React.useCallback(async () => {
    setState({ kind: "running" });
    const t0 = performance.now();
    try {
      // Index footprint + the ranked hits, both over the current corpus. indexStats is the
      // bundle's index-build-memory surface (the risk the bead flags); we show it alongside.
      const [stats, results] = await Promise.all([
        sparqTextIndexStats(data, example.format),
        sparqTextQuery(data, query, example.format),
      ]);
      setEngine("ready");
      setState({
        kind: "results",
        results,
        stats,
        ms: Math.round(performance.now() - t0),
      });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setState({ kind: "error", message });
      toast.error("Full-text query failed", { description: message });
    }
  }, [data, query, example.format]);

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
        <CardTitle className="flex items-center gap-2 text-base">
          <Search className="size-4 text-primary" />
          Live BM25 full-text search
        </CardTitle>
        <EngineIndicator engine={engine} />
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap gap-1.5">
          {TEXT_EXAMPLES.map((ex) => (
            <Button
              key={ex.id}
              variant={example.id === ex.id ? "default" : "outline"}
              size="sm"
              onClick={() => selectExample(ex.id)}
              title={ex.description}
            >
              {ex.label}
            </Button>
          ))}
        </div>

        <p className="text-sm text-muted-foreground">{example.description}</p>

        <div className="grid gap-4 lg:grid-cols-2">
          <Field
            id="text-corpus"
            label="Corpus (RDF)"
            help={`format: ${example.format}`}
            value={data}
            onChange={setData}
            rows={7}
          />
          <Field
            id="text-query"
            label="text: SPARQL query"
            help="text:matches / matchesAny / phrase / near / slop / score"
            value={query}
            onChange={setQuery}
            rows={7}
          />
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <Button
            onClick={() => void run()}
            disabled={state.kind === "running"}
          >
            {state.kind === "running" ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <Play className="size-4" />
            )}
            Search
          </Button>
          <p aria-live="polite" className="text-xs text-muted-foreground">
            {state.kind === "running"
              ? "Indexing + querying on the wasm engine…"
              : state.kind === "results"
                ? `${state.results.results.bindings.length} hit${state.results.results.bindings.length === 1 ? "" : "s"} · ${state.ms} ms`
                : "Edit the corpus or query, then Search."}
          </p>
        </div>

        {state.kind === "error" && (
          <pre className="overflow-x-auto rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">
            {state.message}
          </pre>
        )}

        {state.kind === "results" && (
          <>
            <IndexStats stats={state.stats} />
            <ResultsTable results={state.results} />
          </>
        )}
      </CardContent>
    </Card>
  );
}

function Field({
  id,
  label,
  help,
  value,
  onChange,
  rows,
}: {
  id: string;
  label: string;
  help: string;
  value: string;
  onChange: (v: string) => void;
  rows: number;
}) {
  return (
    <div className="space-y-1.5">
      <div className="flex items-baseline justify-between">
        <label htmlFor={id} className="text-xs font-medium text-muted-foreground">
          {label}
        </label>
        <span className="text-[11px] text-muted-foreground">{help}</span>
      </div>
      <textarea
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        rows={rows}
        spellCheck={false}
        className="w-full resize-y rounded-lg border bg-muted/40 p-3 font-mono text-[12.5px] leading-relaxed text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
      />
    </div>
  );
}

function IndexStats({ stats }: { stats: TextIndexStats }) {
  const kib = (stats.heapBytes / 1024).toFixed(1);
  return (
    <div className="flex flex-wrap gap-2" data-testid="text-index-stats">
      <Badge variant="muted" className="font-mono text-[11px]">
        {stats.docs} indexed literal{stats.docs === 1 ? "" : "s"}
      </Badge>
      <Badge variant="muted" className="font-mono text-[11px]">
        {stats.tokens} distinct token{stats.tokens === 1 ? "" : "s"}
      </Badge>
      <Badge variant="muted" className="font-mono text-[11px]">
        ≈ {kib} KiB heap
      </Badge>
      {stats.hasPositions && (
        <Badge variant="muted" className="font-mono text-[11px]">
          positions on
        </Badge>
      )}
    </div>
  );
}

function ResultsTable({ results }: { results: SparqlResults }) {
  const { vars, rows } = resultCells(results);
  if (rows.length === 0) {
    return (
      <p className="rounded-lg border bg-muted/30 p-3 text-sm text-muted-foreground">
        No literals matched — every term filters the set to empty (an empty result is not an
        error: a query whose tokens match nothing yields zero rows).
      </p>
    );
  }
  return (
    <div className="overflow-x-auto rounded-lg border" data-testid="text-results">
      <table className="w-full border-collapse font-mono text-[12.5px]">
        <thead>
          <tr className="bg-muted/40 text-left text-muted-foreground">
            {vars.map((v) => (
              <th key={v} className="border-b px-3 py-1.5 font-medium">
                ?{v}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, ri) => (
            <tr key={ri}>
              {row.map((cell, ci) => (
                <td
                  key={ci}
                  className={cn(
                    "border-b px-3 py-1.5 text-foreground",
                    cell === "" && "text-muted-foreground",
                  )}
                >
                  {cell === "" ? "—" : cell}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function EngineIndicator({ engine }: { engine: EngineState }) {
  if (engine === "ready") {
    return (
      <Badge variant="success" aria-live="polite">
        <CheckCircle2 className="size-3" /> Engine ready
      </Badge>
    );
  }
  if (engine === "error") {
    return (
      <Badge variant="warning" aria-live="polite">
        Engine failed — retries on search
      </Badge>
    );
  }
  return (
    <Badge variant="muted" aria-live="polite">
      <Loader2 className="size-3 animate-spin" /> Engine loading…
    </Badge>
  );
}
