"use client";

// [OPUS-4.8] sq-5teq — the LIVE /surface/javascript-wasm demo. Four real, in-tab panels
// that EXERCISE the @sparq-org/sparq browser API against one seeded wasm Store (no mocks, no
// server) — each panel calls a distinct binding the page documents:
//
//   1. STREAMING CURSOR  — queryCursor(sparql, batchSize): pull self-contained SPARQL-JSON
//      batches one at a time (peak JS memory bounded by the batch size, not the result).
//   2. match() / countQuads() — RDF/JS triple-pattern lookup over the store via a generated
//      SELECT; countQuads() reads the count from the index without materialising terms.
//   3. count()            — a SELECT solution count with no JS-side binding materialisation.
//   4. applyDelta()       — an incremental O(batch) quad-level insert/delete through the
//      engine's delta overlay; the store size + a live re-query update in place.
//
// The store is rebuilt from the seed on mount and after a delta-reset, so every panel sees
// the same data and the demo is idempotent across reloads.

import * as React from "react";
import {
  Loader2,
  Play,
  CheckCircle2,
  Boxes,
  Search,
  Hash,
  GitCompareArrows,
  RotateCcw,
} from "lucide-react";

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
  loadSparq,
  loadIntoStore,
  prewarmSparqWhenIdle,
  formatTerm,
  matchQuads,
  countQuads,
  streamQueryRows,
  type CursorBatch,
  type MatchTerm,
  type SparqlResults,
  type WasmStore,
} from "@/lib/sparq-wasm";

// A small Turtle seed: 6 people in a tiny social graph, enough to show batching at
// batchSize 2 (multiple cursor pulls) yet trivially auditable by eye.
const SEED = `@prefix : <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .

:alice a foaf:Person ; foaf:name "Alice" ; foaf:age 34 ; foaf:knows :bob, :carol .
:bob   a foaf:Person ; foaf:name "Bob"   ; foaf:age 28 ; foaf:knows :alice .
:carol a foaf:Person ; foaf:name "Carol" ; foaf:age 41 ; foaf:knows :alice, :dave .
:dave  a foaf:Person ; foaf:name "Dave"  ; foaf:age 23 .
:erin  a foaf:Person ; foaf:name "Erin"  ; foaf:age 37 ; foaf:knows :carol .
:frank a foaf:Person ; foaf:name "Frank" ; foaf:age 52 .`;

const PEOPLE_QUERY =
  "PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n" +
  "SELECT ?name ?age WHERE { ?p foaf:name ?name ; foaf:age ?age } ORDER BY ?name";

const COUNT_QUERY =
  "PREFIX foaf: <http://xmlns.com/foaf/0.1/>\n" +
  "SELECT * WHERE { ?p foaf:knows ?o }";

// applyDelta operands (N-Quads): add Grace + a knows edge, then we can retract Frank.
const INSERT_NQUADS = [
  '<http://example.org/grace> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://xmlns.com/foaf/0.1/Person> .',
  '<http://example.org/grace> <http://xmlns.com/foaf/0.1/name> "Grace" .',
  '<http://example.org/grace> <http://xmlns.com/foaf/0.1/age> "29"^^<http://www.w3.org/2001/XMLSchema#integer> .',
  '<http://example.org/grace> <http://xmlns.com/foaf/0.1/knows> <http://example.org/alice> .',
].join("\n");

const DELETE_NQUADS = [
  '<http://example.org/frank> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://xmlns.com/foaf/0.1/Person> .',
  '<http://example.org/frank> <http://xmlns.com/foaf/0.1/name> "Frank" .',
  '<http://example.org/frank> <http://xmlns.com/foaf/0.1/age> "52"^^<http://www.w3.org/2001/XMLSchema#integer> .',
].join("\n");

type EngineState = "cold" | "warming" | "ready" | "error";

export function JavascriptWasmDemo() {
  const [engine, setEngine] = React.useState<EngineState>("cold");
  // The live store, rebuilt from SEED on mount and on reset; null until first build.
  const storeRef = React.useRef<WasmStore | null>(null);
  // Bumps whenever the store is rebuilt/mutated, so panels re-derive their read-outs.
  const [version, setVersion] = React.useState(0);

  const rebuild = React.useCallback(async () => {
    const Store = await loadSparq();
    setEngine("ready");
    storeRef.current = loadIntoStore(Store, SEED, "turtle");
    setVersion((v) => v + 1);
  }, []);

  // [OPUS-4.8] sq-4296 (#935 / #981) — pre-warm + seed on the next browser-IDLE slot (not
  // synchronously during mount) so the ~300 kB+ engine never blocks the initial page paint.
  // `rebuild` awaits the memoised loadSparq, so a user interaction before the idle warm-up
  // completes joins the in-flight load rather than calling into an uninitialised wasm.
  React.useEffect(() => {
    let cancelled = false;
    setEngine("warming");
    const handle = prewarmSparqWhenIdle({
      onReady: () => {
        if (!cancelled) void rebuild();
      },
      onError: () => {
        if (!cancelled) setEngine("error");
      },
    });
    return () => {
      cancelled = true;
      handle.cancel();
    };
  }, [rebuild]);

  const ready = engine === "ready" && storeRef.current !== null;

  return (
    <section className="space-y-4" aria-labelledby="js-wasm-demo-heading">
      <div className="flex items-center justify-between gap-2">
        <h2 id="js-wasm-demo-heading" className="text-xl font-semibold">
          Try it: drive the API in your tab
        </h2>
        <EngineIndicator engine={engine} />
      </div>
      <p className="text-sm text-muted-foreground">
        One seeded <code className="font-mono">Store</code> of six people; each panel
        below calls a different part of the <code className="font-mono">@sparq-org/sparq</code>{" "}
        surface against it — the real Rust engine, compiled to wasm, running here.
      </p>

      <StreamingCursorPanel store={storeRef} version={version} ready={ready} />
      <MatchPanel store={storeRef} version={version} ready={ready} />
      <CountPanel store={storeRef} version={version} ready={ready} />
      <DeltaPanel
        store={storeRef}
        version={version}
        ready={ready}
        onChanged={() => setVersion((v) => v + 1)}
        onReset={rebuild}
      />
    </section>
  );
}

interface PanelProps {
  store: React.RefObject<WasmStore | null>;
  version: number;
  ready: boolean;
}

// ---------------------------------------------------------------------------
// 1. Streaming cursor
// ---------------------------------------------------------------------------

function StreamingCursorPanel({ store, ready }: PanelProps) {
  const [batchSize, setBatchSize] = React.useState(2);
  const [batches, setBatches] = React.useState<CursorBatch[]>([]);
  const [meta, setMeta] = React.useState<{ rowCount: number; batchSize: number } | null>(
    null,
  );
  const [running, setRunning] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const run = React.useCallback(() => {
    const s = store.current;
    if (!s) return;
    setRunning(true);
    setError(null);
    setBatches([]);
    setMeta(null);
    try {
      const collected: CursorBatch[] = [];
      const m = streamQueryRows(s, PEOPLE_QUERY, batchSize, (b) => collected.push(b));
      setBatches(collected);
      setMeta({ rowCount: m.rowCount, batchSize: m.batchSize });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning(false);
    }
  }, [store, batchSize]);

  return (
    <Card>
      <CardHeader className="flex-row items-center gap-2 space-y-0">
        <CardTitle className="flex items-center gap-2 text-base">
          <Boxes className="size-4 text-primary" aria-hidden="true" />
          Streaming cursor — <code className="font-mono text-sm">queryCursor()</code>
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">
          Pull the result in self-contained batches of{" "}
          <code className="font-mono text-foreground">batchSize</code> rows — the consumer
          never holds more than one batch at a time. Lower the batch size to watch more
          pulls arrive.
        </p>
        <div className="flex flex-wrap items-center gap-3">
          <label htmlFor="batch-size" className="text-sm font-medium">
            batchSize
          </label>
          <input
            id="batch-size"
            type="range"
            min={1}
            max={6}
            value={batchSize}
            onChange={(e) => setBatchSize(Number(e.target.value))}
            className="accent-primary"
          />
          <span className="tabular text-sm font-semibold">{batchSize}</span>
          <Button onClick={run} disabled={!ready || running} size="sm">
            {running ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <Play className="size-4" aria-hidden="true" />
            )}
            Stream rows
          </Button>
          <p aria-live="polite" className="text-xs text-muted-foreground">
            {meta &&
              `${meta.rowCount} rows in ${batches.length} batch${
                batches.length === 1 ? "" : "es"
              } of ≤${meta.batchSize}`}
          </p>
        </div>
        {error && <ErrorBox message={error} />}
        {batches.length > 0 && (
          <ol className="space-y-2">
            {batches.map((b) => (
              <li
                key={b.index}
                className="rounded-lg border bg-muted/30 p-3"
              >
                <div className="mb-1.5 flex items-center gap-2 text-xs text-muted-foreground">
                  <Badge variant="muted" className="tabular">
                    batch {b.index}
                  </Badge>
                  <span className="tabular">
                    {b.rows.length} row{b.rows.length === 1 ? "" : "s"} · {b.cumulative}{" "}
                    cumulative
                  </span>
                </div>
                <ul className="flex flex-wrap gap-1.5">
                  {b.rows.map((r, i) => (
                    <li
                      key={i}
                      className="rounded-md bg-background px-2 py-1 font-mono text-[12px] ring-1 ring-foreground/10"
                    >
                      {formatTerm(r.name)} · {formatTerm(r.age)}
                    </li>
                  ))}
                </ul>
              </li>
            ))}
          </ol>
        )}
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// 2. match() / countQuads()
// ---------------------------------------------------------------------------

const MATCH_PRESETS: { label: string; s: string; p: string; o: string }[] = [
  { label: "all triples", s: "", p: "", o: "" },
  {
    label: "all foaf:knows edges",
    s: "",
    p: "<http://xmlns.com/foaf/0.1/knows>",
    o: "",
  },
  {
    label: "who knows :alice",
    s: "",
    p: "<http://xmlns.com/foaf/0.1/knows>",
    o: "<http://example.org/alice>",
  },
  {
    label: ":alice's triples",
    s: "<http://example.org/alice>",
    p: "",
    o: "",
  },
];

function MatchPanel({ store, version, ready }: PanelProps) {
  const [s, setS] = React.useState("");
  const [p, setP] = React.useState("<http://xmlns.com/foaf/0.1/knows>");
  const [o, setO] = React.useState("");
  const [result, setResult] = React.useState<{
    rows: SparqlResults["results"]["bindings"];
    count: number;
  } | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  const arg = (v: string): MatchTerm => (v.trim() ? v.trim() : null);

  const run = React.useCallback(() => {
    const store_ = store.current;
    if (!store_) return;
    setError(null);
    try {
      const rows = matchQuads(store_, arg(s), arg(p), arg(o));
      const count = countQuads(store_, arg(s), arg(p), arg(o));
      setResult({ rows, count });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [store, s, p, o]);

  // Re-run on a store change so the panel always reflects the live data (e.g. after a delta).
  React.useEffect(() => {
    if (ready && result) run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [version]);

  return (
    <Card>
      <CardHeader className="flex-row items-center gap-2 space-y-0">
        <CardTitle className="flex items-center gap-2 text-base">
          <Search className="size-4 text-primary" aria-hidden="true" />
          RDF/JS lookup —{" "}
          <code className="font-mono text-sm">match()</code> /{" "}
          <code className="font-mono text-sm">countQuads()</code>
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">
          A triple-pattern lookup: leave a position blank for a wildcard, or paste an
          N-Triples term (<code className="font-mono">&lt;iri&gt;</code> or{" "}
          <code className="font-mono">&quot;literal&quot;</code>).{" "}
          <code className="font-mono">countQuads()</code> reads the count from the index
          without materialising the terms.
        </p>
        <div className="flex flex-wrap gap-1.5">
          {MATCH_PRESETS.map((preset) => (
            <Button
              key={preset.label}
              variant="outline"
              size="sm"
              onClick={() => {
                setS(preset.s);
                setP(preset.p);
                setO(preset.o);
              }}
            >
              {preset.label}
            </Button>
          ))}
        </div>
        <div className="grid gap-2 sm:grid-cols-3">
          <TermInput label="subject" value={s} onChange={setS} />
          <TermInput label="predicate" value={p} onChange={setP} />
          <TermInput label="object" value={o} onChange={setO} />
        </div>
        <div className="flex items-center gap-3">
          <Button onClick={run} disabled={!ready} size="sm">
            <Play className="size-4" aria-hidden="true" />
            match()
          </Button>
          {result && (
            <Badge variant="muted" aria-live="polite" className="tabular">
              countQuads() = {result.count}
            </Badge>
          )}
        </div>
        {error && <ErrorBox message={error} />}
        {result && <QuadTable rows={result.rows} />}
      </CardContent>
    </Card>
  );
}

function TermInput({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
}) {
  const id = `match-${label}`;
  return (
    <div className="space-y-1">
      <label htmlFor={id} className="text-xs font-medium text-muted-foreground">
        {label}
      </label>
      <input
        id={id}
        value={value}
        spellCheck={false}
        placeholder="(any)"
        onChange={(e) => onChange(e.target.value)}
        className="w-full rounded-md border bg-muted/40 px-2 py-1.5 font-mono text-[12px] outline-none focus-visible:ring-3 focus-visible:ring-ring/40"
      />
    </div>
  );
}

function QuadTable({ rows }: { rows: SparqlResults["results"]["bindings"] }) {
  if (rows.length === 0) {
    return (
      <p className="rounded-lg border bg-muted/30 p-3 text-sm text-muted-foreground">
        No quads match this pattern.
      </p>
    );
  }
  const hasGraph = rows.some((r) => r.g);
  return (
    <div className="overflow-x-auto rounded-lg border">
      <table className="w-full text-left text-sm">
        <thead className="bg-muted/50">
          <tr>
            {["subject", "predicate", "object"].map((h) => (
              <th key={h} className="px-3 py-2 font-medium">
                {h}
              </th>
            ))}
            {hasGraph && <th className="px-3 py-2 font-medium">graph</th>}
          </tr>
        </thead>
        <tbody>
          {rows.slice(0, 20).map((r, i) => (
            <tr key={i} className="border-t">
              {(["s", "p", "o"] as const).map((k) => (
                <td
                  key={k}
                  className={cn(
                    "px-3 py-1.5 font-mono text-[12px]",
                    k === "o" && "break-all",
                  )}
                >
                  {formatTerm(r[k])}
                </td>
              ))}
              {hasGraph && (
                <td className="px-3 py-1.5 font-mono text-[12px] break-all text-muted-foreground">
                  {r.g ? formatTerm(r.g) : "—"}
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// ---------------------------------------------------------------------------
// 3. count()
// ---------------------------------------------------------------------------

function CountPanel({ store, version, ready }: PanelProps) {
  const [count, setCount] = React.useState<number | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  const run = React.useCallback(() => {
    const s = store.current;
    if (!s) return;
    setError(null);
    try {
      setCount(s.count(COUNT_QUERY));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [store]);

  React.useEffect(() => {
    if (ready && count !== null) run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [version]);

  return (
    <Card>
      <CardHeader className="flex-row items-center gap-2 space-y-0">
        <CardTitle className="flex items-center gap-2 text-base">
          <Hash className="size-4 text-primary" aria-hidden="true" />
          Solution count — <code className="font-mono text-sm">count()</code>
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">
          Count a SELECT&apos;s solutions without building any JS-side{" "}
          <code className="font-mono">Bindings</code> — read straight from the index where
          possible. Query:{" "}
          <code className="font-mono text-foreground">
            SELECT * WHERE {"{"} ?p foaf:knows ?o {"}"}
          </code>
        </p>
        <div className="flex items-center gap-3">
          <Button onClick={run} disabled={!ready} size="sm">
            <Play className="size-4" aria-hidden="true" />
            count()
          </Button>
          {count !== null && (
            <Badge variant="success" aria-live="polite" className="tabular">
              <CheckCircle2 className="size-3" aria-hidden="true" /> {count} solutions
            </Badge>
          )}
        </div>
        {error && <ErrorBox message={error} />}
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// 4. applyDelta()
// ---------------------------------------------------------------------------

function DeltaPanel({
  store,
  version,
  ready,
  onChanged,
  onReset,
}: PanelProps & { onChanged: () => void; onReset: () => Promise<void> }) {
  const [error, setError] = React.useState<string | null>(null);
  const [resetting, setResetting] = React.useState(false);

  // The live people count (default-graph triples is `size`; here we show distinct people).
  const people = React.useMemo(() => {
    const s = store.current;
    if (!ready || !s) return null;
    try {
      return s.count(
        "PREFIX foaf: <http://xmlns.com/foaf/0.1/> SELECT * WHERE { ?p a foaf:Person }",
      );
    } catch {
      return null;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [store, ready, version]);

  const size = ready ? (store.current?.size ?? null) : null;

  const apply = React.useCallback(
    (inserts: string, deletes: string) => {
      const s = store.current;
      if (!s) return;
      setError(null);
      try {
        s.applyDelta(inserts, deletes);
        onChanged();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [store, onChanged],
  );

  const reset = React.useCallback(async () => {
    setResetting(true);
    setError(null);
    try {
      await onReset();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setResetting(false);
    }
  }, [onReset]);

  return (
    <Card>
      <CardHeader className="flex-row items-center gap-2 space-y-0">
        <CardTitle className="flex items-center gap-2 text-base">
          <GitCompareArrows className="size-4 text-primary" aria-hidden="true" />
          Incremental delta — <code className="font-mono text-sm">applyDelta()</code>
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">
          Mutate the store in place with a quad-level delta — O(batch), no index rebuild.
          Watch the size and person count change; the lookup and count panels above
          re-derive from the same live store.
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="muted" className="tabular">
            {size ?? "—"} triples
          </Badge>
          <Badge variant="muted" className="tabular" aria-live="polite">
            {people ?? "—"} foaf:Person
          </Badge>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            onClick={() => apply(INSERT_NQUADS, "")}
            disabled={!ready}
            size="sm"
            variant="outline"
          >
            + insert :grace
          </Button>
          <Button
            onClick={() => apply("", DELETE_NQUADS)}
            disabled={!ready}
            size="sm"
            variant="outline"
          >
            − delete :frank
          </Button>
          <Button onClick={reset} disabled={!ready || resetting} size="sm" variant="ghost">
            {resetting ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <RotateCcw className="size-4" aria-hidden="true" />
            )}
            Reset to seed
          </Button>
        </div>
        {error && <ErrorBox message={error} />}
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Shared bits
// ---------------------------------------------------------------------------

function EngineIndicator({ engine }: { engine: EngineState }) {
  if (engine === "ready") {
    return (
      <Badge variant="success" aria-live="polite">
        <CheckCircle2 className="size-3" aria-hidden="true" /> Engine ready
      </Badge>
    );
  }
  if (engine === "error") {
    return (
      <Badge variant="warning" aria-live="polite">
        Engine failed to load
      </Badge>
    );
  }
  return (
    <Badge variant="muted" aria-live="polite">
      <Loader2 className="size-3 animate-spin" aria-hidden="true" /> Engine loading…
    </Badge>
  );
}

function ErrorBox({ message }: { message: string }) {
  return (
    <pre className="overflow-x-auto rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">
      {message}
    </pre>
  );
}
