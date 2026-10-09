"use client";

// [OPUS-4.8] sq-4r4b — the interactive Solid (user, app)-pair demo.
//
// Pick an (agent, client) pair → run a SPARQL query over the SAME Pod → see the
// access-controlled result set that pair gets. The headline insight: identical query,
// identical Pod, DIFFERENT rows — because WAC/ACP restricts the named graphs each
// requester may read. The restriction runs LIVE in your tab: the wasm engine evaluates
// the query rewritten with the pair's `FROM NAMED <authorized-graphs>` set (sparq-solid's
// `rewrite_for`). The access-control DECISION (which graphs) is the materialized
// sparq-solid output, precomputed at build time — see the honesty section on the page.
//
// [OPUS-4.8] sq-p6p7 (#797/#549) — the query is now EDITABLE. The user can rewrite the
// demo query and re-run it for both pairs; the per-pair `FROM NAMED <authorized-graphs>`
// restriction is injected around WHATEVER query they type, so the access-control eval
// re-runs over their own SPARQL. The default query is preserved as the reset target.
// Results render generically from the result `head.vars` (so a custom projection / ASK
// works), with the original `?graph ?s ?p ?o` shape still getting the rich per-graph diff.

import * as React from "react";
import {
  Play,
  Loader2,
  User,
  AppWindow,
  FileLock2,
  FileText,
  Globe,
  Users,
  ShieldOff,
  KeyRound,
  ArrowRight,
  RotateCcw,
} from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { cn } from "@/lib/utils";
import {
  loadSparq,
  formatTerm,
  termValue,
  resultVars,
  resultRows,
  isAskResult,
  askValue,
  type SparqlResults,
  type WasmStore,
} from "@/lib/sparq-wasm";
import { SparqlEditor } from "@/components/sparql-editor";
import { rewriteForGraphs } from "@/lib/solid-acl";
import {
  SESSIONS,
  SHARED_QUERY,
  POD_NQUADS,
  POD_DOCS,
  ACL_GRANTS,
  DOC_BY_IRI,
  type PodSession,
  type PodDoc,
} from "@/data/solid-pod";

interface RunResult {
  /** The full parsed SPARQL-JSON result (SELECT bindings OR an ASK boolean). */
  results: SparqlResults;
  ms: number;
  rewritten: string;
}

/** The SELECT solution rows of a run (empty for an ASK), via the shared helper. */
function runRows(r: RunResult): SparqlResults["results"]["bindings"] {
  return resultRows(r.results);
}

type RunState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "running" }
  | { kind: "done"; result: RunResult }
  | { kind: "error"; message: string };

function shortGraph(iri: string): string {
  return iri.replace("https://alice.pod.example/", "…/");
}

export function SolidPairsDemo() {
  const [sessionId, setSessionId] = React.useState<string>(SESSIONS[0].id);
  const [compareId, setCompareId] = React.useState<string>(SESSIONS[3].id);
  // [OPUS-4.8] sq-p6p7 — the user-editable query, seeded with the demo default.
  const [query, setQuery] = React.useState<string>(SHARED_QUERY);
  const [state, setState] = React.useState<RunState>({ kind: "idle" });
  const [compareState, setCompareState] = React.useState<RunState>({ kind: "idle" });
  const storeRef = React.useRef<WasmStore | null>(null);

  const session = SESSIONS.find((s) => s.id === sessionId)!;
  const compare = SESSIONS.find((s) => s.id === compareId)!;
  const isDefaultQuery = query === SHARED_QUERY;

  const ensureStore = React.useCallback(async (): Promise<WasmStore> => {
    if (storeRef.current) return storeRef.current;
    const Store = await loadSparq();
    // loadDataset preserves the named graphs (one per document) — essential, because
    // the whole access-control model is graph-level.
    const store = Store.loadDataset(POD_NQUADS, "nquads");
    storeRef.current = store;
    return store;
  }, []);

  // [OPUS-4.8] sq-p6p7 — runs WHATEVER query the user typed for a pair, after injecting
  // that pair's `FROM NAMED <authorized-graphs>` restriction (sparq-solid's rewrite_for).
  // So the access-control eval re-runs over the custom query exactly as over the default.
  const runFor = React.useCallback(
    async (s: PodSession, sparql: string): Promise<RunResult> => {
      const store = await ensureStore();
      const rewritten = rewriteForGraphs(sparql, s.authorizedGraphs);
      const t0 = performance.now();
      const json = store.query(rewritten);
      const ms = performance.now() - t0;
      const parsed = JSON.parse(json) as SparqlResults;
      return { results: parsed, ms, rewritten };
    },
    [ensureStore],
  );

  const run = React.useCallback(async () => {
    // A custom query is just text — guard the empty/whitespace case before touching wasm.
    if (query.trim().length === 0) {
      const message = "Enter a SPARQL query to run.";
      setState({ kind: "error", message });
      setCompareState({ kind: "idle" });
      toast.error("Empty query", { description: message });
      return;
    }
    try {
      setState({ kind: "loading" });
      setCompareState({ kind: "loading" });
      // A bad custom query throws inside the wasm engine — caught below and shown as a
      // friendly inline message (per pair) instead of crashing the page.
      const [a, b] = await Promise.all([
        runFor(session, query),
        runFor(compare, query),
      ]);
      setState({ kind: "done", result: a });
      setCompareState({ kind: "done", result: b });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setState({ kind: "error", message });
      setCompareState({ kind: "error", message });
      toast.error("Query failed", { description: message });
    }
  }, [runFor, session, compare, query]);

  const resetQuery = React.useCallback(() => {
    setQuery(SHARED_QUERY);
  }, []);

  // Re-run automatically when a selector changes, if we've already run once. (Editing the
  // query does NOT auto-run — the user drives that with the Run button.)
  const hasRun = state.kind === "done" || state.kind === "error";
  React.useEffect(() => {
    if (hasRun) void run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId, compareId]);

  const busy = state.kind === "loading" || state.kind === "running";

  return (
    <div className="space-y-6">
      {/* The query — editable, but identical for both pairs on a given run. */}
      <Card>
        <CardHeader>
          <CardTitle className="flex flex-wrap items-center justify-between gap-2 text-base">
            <span>1 · One query, one Pod</span>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-7 gap-1 px-2 text-xs"
              onClick={resetQuery}
              disabled={isDefaultQuery}
            >
              <RotateCcw className="size-3" aria-hidden="true" />
              Reset to demo query
            </Button>
          </CardTitle>
          <CardDescription>
            This query runs for <em>both</em> (user, app) pairs, against the same
            four-document Pod. <strong>Edit it and re-run</strong> — only{" "}
            <strong>who is asking, from which app</strong> changes the rows that come
            back, because each pair&rsquo;s authorized named-graph set is injected as a{" "}
            <code className="font-mono">FROM NAMED</code> restriction around whatever
            you type.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          {/* [OPUS-4.8] sq-p6p7 — editable query via the existing SparqlEditor (overlay
              highlighting + missing-prefix help); no new editor dependency. */}
          <SparqlEditor
            value={query}
            onChange={setQuery}
            rows={9}
            ariaLabel="SPARQL query for the Solid access-control demo"
            id="solid-pairs-query"
          />
          <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
            <KeyRound
              className="mt-0.5 size-3.5 shrink-0 text-primary"
              aria-hidden="true"
            />
            <span>
              Access control still applies to your query: it is evaluated only over the
              named graphs the pair may read. Wrap patterns in{" "}
              <code className="font-mono">GRAPH ?g {`{ … }`}</code> to read across the
              authorized documents — a query against the default graph alone returns
              nothing here, because a Pod&rsquo;s data lives in its per-document named
              graphs (and that is the fail-closed default).
            </span>
          </p>
          <PodMap />
        </CardContent>
      </Card>

      {/* The (user, app) selectors. */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            2 · Pick two (user, app) pairs to compare
          </CardTitle>
          <CardDescription>
            The session key is exactly sparq-solid&rsquo;s{" "}
            <code className="font-mono">Session {`{ agent, client }`}</code>. Pick a
            pair on each side, run, and watch the result sets diverge.
          </CardDescription>
        </CardHeader>
        <CardContent className="grid gap-4 md:grid-cols-2">
          <SessionPicker
            legend="Pair A"
            value={sessionId}
            onChange={setSessionId}
            accent="primary"
          />
          <SessionPicker
            legend="Pair B"
            value={compareId}
            onChange={setCompareId}
            accent="muted"
          />
          <div className="md:col-span-2">
            <Button onClick={run} disabled={busy}>
              {busy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Play className="size-4" aria-hidden="true" />
              )}
              {state.kind === "loading"
                ? "Loading engine…"
                : isDefaultQuery
                  ? "Run the demo query for both pairs"
                  : "Run your query for both pairs"}
            </Button>
          </div>
        </CardContent>
      </Card>

      {/* Side-by-side results. */}
      <div className="grid gap-4 lg:grid-cols-2">
        <SessionResult
          session={session}
          state={state}
          accent="primary"
          otherRows={
            compareState.kind === "done"
              ? runRows(compareState.result)
              : undefined
          }
        />
        <SessionResult
          session={compare}
          state={compareState}
          accent="muted"
          otherRows={state.kind === "done" ? runRows(state.result) : undefined}
        />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------

function SessionPicker({
  legend,
  value,
  onChange,
  accent,
}: {
  legend: string;
  value: string;
  onChange: (id: string) => void;
  accent: "primary" | "muted";
}) {
  return (
    <fieldset className="space-y-2 rounded-xl border p-3">
      <legend
        className={cn(
          "px-1 text-xs font-semibold uppercase tracking-wide",
          accent === "primary" ? "text-primary" : "text-muted-foreground",
        )}
      >
        {legend}
      </legend>
      {SESSIONS.map((s) => (
        <label
          key={s.id}
          className={cn(
            "flex cursor-pointer items-start gap-2 rounded-lg p-2 text-sm transition-colors",
            value === s.id ? "bg-muted ring-1 ring-foreground/15" : "hover:bg-muted/50",
          )}
        >
          <input
            type="radio"
            name={`session-${legend}`}
            value={s.id}
            checked={value === s.id}
            onChange={() => onChange(s.id)}
            className="mt-1 accent-[var(--primary)]"
          />
          <span className="space-y-0.5">
            <span className="flex flex-wrap items-center gap-1.5 font-medium">
              <User className="size-3.5 text-muted-foreground" aria-hidden="true" />
              {s.user}
              <ArrowRight className="size-3 text-muted-foreground" aria-hidden="true" />
              <AppWindow className="size-3.5 text-muted-foreground" aria-hidden="true" />
              {s.app}
            </span>
            <span className="block text-xs text-muted-foreground">{s.scenario}</span>
          </span>
        </label>
      ))}
    </fieldset>
  );
}

const SENS_ICON: Record<PodDoc["sensitivity"], React.ComponentType<{ className?: string }>> = {
  public: Globe,
  shared: Users,
  private: FileLock2,
};

function PodMap() {
  return (
    <div className="space-y-2">
      <p className="text-xs font-medium text-muted-foreground">
        The Pod — one named graph per document:
      </p>
      <div className="grid gap-2 sm:grid-cols-2">
        {POD_DOCS.map((d) => {
          const Icon = SENS_ICON[d.sensitivity];
          return (
            <div
              key={d.iri}
              className="flex items-start gap-2 rounded-lg border bg-muted/30 p-2.5 text-sm"
            >
              <Icon className="mt-0.5 size-4 text-muted-foreground" aria-hidden="true" />
              <div className="min-w-0">
                <div className="flex items-center gap-1.5">
                  <span className="font-medium">{d.label}</span>
                  <Badge
                    variant={
                      d.sensitivity === "public"
                        ? "success"
                        : d.sensitivity === "shared"
                          ? "warning"
                          : "muted"
                    }
                    className="h-4 px-1.5 text-[10px]"
                  >
                    {d.sensitivity}
                  </Badge>
                </div>
                <div className="truncate font-mono text-[11px] text-muted-foreground">
                  {shortGraph(d.iri)}
                </div>
                <div className="text-xs text-muted-foreground">{d.about}</div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------

function SessionResult({
  session,
  state,
  accent,
  otherRows,
}: {
  session: PodSession;
  state: RunState;
  accent: "primary" | "muted";
  otherRows?: SparqlResults["results"]["bindings"];
}) {
  const result = state.kind === "done" ? state.result : null;
  const visibleGraphs = session.authorizedGraphs;
  const empty = visibleGraphs.length === 0;

  // The SELECT solution rows of this run (empty for an ASK answer).
  const rows = result ? runRows(result) : [];
  // The per-graph diff is only meaningful when the projection carries a ?graph column
  // (the default demo query does). For a custom query without it, fall back to a plain
  // row-count delta so the comparison still degrades gracefully.
  const hasGraphVar =
    !!result && resultVars(result.results).includes("graph");
  const otherGraphs = otherRows ? graphSet(otherRows) : null;
  const thisGraphs = hasGraphVar ? graphSet(rows) : new Set<string>();

  return (
    <Card
      className={cn(
        accent === "primary" ? "ring-1 ring-primary/30" : "ring-1 ring-foreground/10",
      )}
    >
      <CardHeader className="space-y-1.5">
        <CardTitle className="flex flex-wrap items-center gap-1.5 text-base">
          <User className="size-4 text-muted-foreground" aria-hidden="true" />
          {session.user}
          <span className="text-muted-foreground">·</span>
          <AppWindow className="size-4 text-muted-foreground" aria-hidden="true" />
          {session.app}
        </CardTitle>
        <CardDescription className="font-mono text-[11px]">
          agent={session.userWebId ?? "(anonymous)"} · client={session.appId ?? "(any)"}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {/* Authorized graphs (the FROM NAMED set) + the fail-closed property. */}
        <div className="space-y-1.5">
          <div className="flex items-center gap-1.5 text-xs font-semibold">
            <KeyRound className="size-3.5 text-primary" aria-hidden="true" />
            Authorized graphs (FROM NAMED)
          </div>
          {empty ? (
            <p className="flex items-center gap-1.5 rounded-lg bg-muted/50 p-2 text-xs text-muted-foreground">
              <ShieldOff className="size-3.5" aria-hidden="true" />
              None — fail-closed. The query is restricted to a guaranteed-absent
              graph, so it returns nothing.
            </p>
          ) : (
            <ul className="space-y-1">
              {visibleGraphs.map((g) => {
                const doc = DOC_BY_IRI[g];
                return (
                  <li key={g} className="flex items-center gap-1.5 text-xs">
                    <span className="inline-block size-1.5 rounded-full bg-[var(--success)]" />
                    <span className="font-mono">{shortGraph(g)}</span>
                    {doc && <span className="text-muted-foreground">— {doc.label}</span>}
                  </li>
                );
              })}
            </ul>
          )}
          {/* The hidden graphs — what this pair CANNOT see, and that it can't tell they exist. */}
          <HiddenGraphs visible={visibleGraphs} />
        </div>

        {/* The "why" — the grants that produced this set. */}
        <WhyGrants session={session} />

        {/* The actual result rows from the live engine. */}
        <ResultRows state={state} session={session} />

        {/* The diff line vs the other pair. ASK answers have no rows to diff. */}
        {result && !isAskResult(result.results) && otherGraphs && (
          <DiffSummary
            thisCount={rows.length}
            otherCount={otherRows!.length}
            extra={[...thisGraphs].filter((g) => !otherGraphs.has(g))}
          />
        )}
      </CardContent>
    </Card>
  );
}

function graphSet(rows: SparqlResults["results"]["bindings"]): Set<string> {
  const s = new Set<string>();
  for (const r of rows) {
    const g = termValue(r["graph"]);
    if (g) s.add(g);
  }
  return s;
}

function HiddenGraphs({ visible }: { visible: string[] }) {
  const hidden = POD_DOCS.filter((d) => !visible.includes(d.iri));
  if (hidden.length === 0) return null;
  return (
    <details className="text-xs text-muted-foreground">
      <summary className="cursor-pointer select-none">
        {hidden.length} graph{hidden.length === 1 ? "" : "s"} invisible to this pair
      </summary>
      <ul className="mt-1 space-y-0.5 pl-3">
        {hidden.map((d) => (
          <li key={d.iri} className="flex items-center gap-1.5">
            <ShieldOff className="size-3" aria-hidden="true" />
            <span className="line-through opacity-70">{d.label}</span>
            <span className="opacity-60">(indistinguishable from absent)</span>
          </li>
        ))}
      </ul>
    </details>
  );
}

function WhyGrants({ session }: { session: PodSession }) {
  const grants = ACL_GRANTS.filter((g) => session.authorizedGraphs.includes(g.docIri));
  return (
    <details className="rounded-lg border bg-muted/20 p-2 text-xs">
      <summary className="cursor-pointer select-none font-semibold">
        Why these graphs? {grants.length} WAC/ACP grant{grants.length === 1 ? "" : "s"} matched
      </summary>
      <div className="mt-2 space-y-2">
        {grants.length === 0 && (
          <p className="text-muted-foreground">
            No grant matched this (agent, client) pair on any document beyond the
            public profile — fail-closed default.
          </p>
        )}
        {grants.map((g) => {
          const doc = DOC_BY_IRI[g.docIri];
          return (
            <div key={g.docIri} className="space-y-1">
              <div className="flex flex-wrap items-center gap-1.5">
                <Badge variant={g.system === "ACP" ? "default" : "muted"} className="h-4 px-1.5 text-[10px]">
                  {g.system}
                </Badge>
                <span className="font-medium">{doc?.label}</span>
                <span className="text-muted-foreground">
                  → {g.subject}, {g.client}
                </span>
              </div>
              <pre className="overflow-x-auto rounded bg-background/60 p-2 font-mono text-[10.5px] leading-snug text-muted-foreground">
                {g.rule}
              </pre>
            </div>
          );
        })}
      </div>
    </details>
  );
}

function ResultRows({ state, session }: { state: RunState; session: PodSession }) {
  if (state.kind === "error") {
    return (
      <pre className="overflow-x-auto rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">
        {state.message}
      </pre>
    );
  }
  if (state.kind === "idle") {
    return (
      <p className="rounded-lg border bg-muted/20 p-3 text-xs text-muted-foreground">
        Run the query to see what this pair gets back.
      </p>
    );
  }
  if (state.kind === "loading" || state.kind === "running") {
    return (
      <p className="flex items-center gap-2 rounded-lg border bg-muted/20 p-3 text-xs text-muted-foreground">
        <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
        Running on the wasm engine…
      </p>
    );
  }

  // [OPUS-4.8] sq-p6p7 — render generically so a custom query works. ASK answers show a
  // boolean; SELECT answers render a table whose columns come from the result head.vars
  // (the default ?graph ?s ?p ?o is just one such projection). CONSTRUCT/DESCRIBE return
  // no SPARQL-JSON bindings here, so they surface as "no solutions" — noted in the UI.
  const results = state.result.results;
  if (isAskResult(results)) {
    const value = askValue(results) === true;
    return (
      <div className="space-y-1.5">
        <ResultHeader rowCount={null} ms={state.result.ms} />
        <div
          className={cn(
            "rounded-lg p-3 text-sm font-medium",
            value
              ? "bg-[color-mix(in_oklch,var(--success)_15%,transparent)] text-[var(--success)]"
              : "bg-muted text-muted-foreground",
          )}
        >
          ASK → {value ? "true" : "false"}
        </div>
      </div>
    );
  }

  const vars = resultVars(results);
  const rows = resultRows(results);
  return (
    <div className="space-y-1.5">
      <ResultHeader rowCount={rows.length} ms={state.result.ms} />
      {rows.length === 0 ? (
        <p className="rounded-lg border bg-muted/30 p-3 text-xs text-muted-foreground">
          No solutions. {session.authorizedGraphs.length === 0
            ? "Fail-closed: this pair has no authorized graphs, so the Pod looks empty to it."
            : "This pair's authorized graphs held no matching triples (a query against the default graph alone returns nothing — wrap patterns in GRAPH ?g)."}
        </p>
      ) : (
        <div className="max-h-72 overflow-auto rounded-lg border">
          <table className="w-full text-left text-[12px]">
            <thead className="sticky top-0 bg-muted/80 backdrop-blur">
              <tr>
                {vars.map((v) => (
                  <th key={v} className="px-2 py-1.5 font-medium">
                    ?{v}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, i) => (
                <tr key={i} className="border-t">
                  {vars.map((v) => (
                    <td
                      key={v}
                      className="px-2 py-1 font-mono text-[11px]"
                    >
                      {compact(formatTerm(row[v]))}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function ResultHeader({
  rowCount,
  ms,
}: {
  rowCount: number | null;
  ms: number;
}) {
  return (
    <div
      className="flex items-center justify-between text-xs font-semibold"
      aria-live="polite"
    >
      <span className="flex items-center gap-1.5">
        <FileText className="size-3.5 text-primary" aria-hidden="true" />
        Result set
      </span>
      <span className="tabular text-muted-foreground">
        {rowCount !== null && `${rowCount} row${rowCount === 1 ? "" : "s"} · `}
        {ms.toFixed(1)} ms · live
      </span>
    </div>
  );
}

function compact(s: string): string {
  return s
    .replace("https://alice.pod.example/", "…/")
    .replace("http://xmlns.com/foaf/0.1/", "foaf:")
    .replace("http://schema.org/", "schema:")
    .replace("http://www.w3.org/2001/XMLSchema#", "xsd:");
}

function DiffSummary({
  thisCount,
  otherCount,
  extra,
}: {
  thisCount: number;
  otherCount: number;
  extra: string[];
}) {
  const delta = thisCount - otherCount;
  return (
    <div className="rounded-lg bg-primary/5 p-2.5 text-xs ring-1 ring-primary/20">
      <span className="font-semibold">vs. the other pair: </span>
      {delta === 0 && extra.length === 0 ? (
        <span className="text-muted-foreground">same {thisCount} rows.</span>
      ) : (
        <span className="text-muted-foreground">
          {delta > 0 ? `+${delta}` : delta} rows
          {extra.length > 0 && (
            <>
              {" "}
              — this pair additionally sees{" "}
              <strong className="text-foreground">
                {extra.map((g) => DOC_BY_IRI[g]?.label ?? shortGraph(g)).join(", ")}
              </strong>
            </>
          )}
          .
        </span>
      )}
    </div>
  );
}
