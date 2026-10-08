"use client";

// [OPUS-4.8] sq-egy6 — the live /surface/shacl playground: two textareas (data +
// shapes) -> conformance flag + per-violation W3C report, run entirely in your tab
// via the shacl-enabled wasm bundle (Store.validate). The default example is the
// `ex:age "thirty"` datatype violation from skills/shacl-validation/SKILL.md.

import * as React from "react";
import {
  Play,
  Loader2,
  ShieldCheck,
  CheckCircle2,
  XCircle,
  FileCode2,
} from "lucide-react";
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
import { RdfHighlight } from "@/components/rdf-highlight";
import {
  prewarmSparqWhenIdle,
  loadSparq,
  loadIntoStore,
  matchQuads,
  sparqShaclValidate,
  sparqParseShaclCompact,
  type ShaclReport,
  type SparqlBinding,
  type SparqlTerm,
} from "@/lib/sparq-wasm";
import {
  componentName,
  reportSummary,
  reportToTurtle,
  severityName,
  shortenIri,
} from "@/lib/shacl-report";
import {
  shapesToCompact,
  type ScsPrefix,
  type ScsTerm,
  type ScsTriple,
} from "@/lib/shacl-compact";
import { SHACL_EXAMPLES } from "@/data/shacl-examples";

type RunState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "report"; report: ShaclReport; ms: number }
  | { kind: "error"; message: string };

// [OPUS-4.8] sq-pvg2 (#796) — the shapes-graph -> SHACL Compact Syntax render, kept
// separate from the validation RunState so the compact view does not depend on a
// validation run (it serialises the SHAPES textarea, not the data graph).
type CompactState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "done"; text: string; unsupported: string[] }
  | { kind: "error"; message: string };

type EngineState = "cold" | "warming" | "ready" | "error";
type View = "results" | "turtle" | "compact";

// [OPUS-4.8] sq-pyn7 (#796) — the SHAPES-input syntax. "turtle" feeds the textarea text to
// the validator unchanged; "compact" reads it as SHACL Compact Syntax (SCS) and parses it to
// a Turtle shapes graph via the wasm `Store.parseShaclCompact` binding FIRST, so the user can
// author shapes in the terser compact notation and still validate / round-trip them. This is
// the SCS *input* direction, the counterpart of the `shapesToCompact` Turtle→SCS serialiser.
type ShapesMode = "turtle" | "compact";

// Turtle `@prefix p: <iri> .` OR SPARQL-style `PREFIX p: <iri>` declarations. The
// shared `declaredPrefixBindings` only matches the SPARQL form; the SHACL Turtle
// shapes need this @-aware pass to recover the user's prefix labels for the compact
// output. Best-effort over the text (an unrecovered prefix only means the IRI is
// emitted in `<…>` form, never wrong output).
const PREFIX_RE = /(?:^|\s)(?:@?prefix|PREFIX)\s+([A-Za-z][\w.-]*)?:\s*<([^>]*)>/gi;

function extractPrefixes(turtle: string): ScsPrefix[] {
  const out: ScsPrefix[] = [];
  const seen = new Set<string>();
  for (const m of turtle.matchAll(PREFIX_RE)) {
    const prefix = m[1] ?? "";
    if (seen.has(prefix)) continue;
    seen.add(prefix);
    out.push({ prefix, iri: m[2] });
  }
  return out;
}

// Map a SPARQL-JSON term (the `{type,value,datatype,xml:lang}` shape the wasm SELECT
// yields) to the serializer's structural `ScsTerm`.
function toScsTerm(t: SparqlTerm): ScsTerm {
  if (t.type === "uri") return { termType: "NamedNode", value: t.value };
  if (t.type === "bnode") return { termType: "BlankNode", value: t.value };
  if (t.type === "triple") {
    throw new Error("SHACL Compact Syntax cannot express an RDF 1.2 triple term");
  }
  return {
    termType: "Literal",
    value: t.value,
    datatype: t.datatype,
    language: t["xml:lang"],
  };
}

const DEFAULT = SHACL_EXAMPLES[0];

export function ShaclPlayground() {
  const [data, setData] = React.useState(DEFAULT.data);
  const [shapes, setShapes] = React.useState(DEFAULT.shapes);
  const [shapesMode, setShapesMode] = React.useState<ShapesMode>("turtle");
  const [state, setState] = React.useState<RunState>({ kind: "idle" });
  const [compact, setCompact] = React.useState<CompactState>({ kind: "idle" });
  const [engine, setEngine] = React.useState<EngineState>("cold");
  const [activeExample, setActiveExample] = React.useState<string>(DEFAULT.id);
  const [view, setView] = React.useState<View>("results");

  // [OPUS-4.8] sq-pyn7 (#796) — resolve the SHAPES textarea to a Turtle shapes graph: in
  // "turtle" mode it is the text unchanged; in "compact" mode it is SCS, parsed to Turtle via
  // the wasm `Store.parseShaclCompact` binding (a clear error is surfaced if the loaded bundle
  // lacks the `scs` feature, or if the SCS fails to parse). Memoised on `shapes`/`shapesMode`.
  const resolveShapesTurtle = React.useCallback(async (): Promise<string> => {
    if (shapesMode === "turtle") return shapes;
    return sparqParseShaclCompact(shapes);
  }, [shapes, shapesMode]);

  // [OPUS-4.8] sq-4296 (#935 / #981) — pre-warm the wasm engine on the next browser-IDLE
  // slot, not synchronously during mount, so the ~300 kB+ engine wasm never blocks the
  // initial page paint. The first "Validate" still awaits the memoised `loadSparq()`, so an
  // interaction before the idle warm-up completes joins the in-flight load (no cold-start
  // double-load, no call into an uninitialised wasm). A failure resets the indicator.
  React.useEffect(() => {
    let cancelled = false;
    setEngine("warming");
    const handle = prewarmSparqWhenIdle({
      onReady: () => {
        if (!cancelled) setEngine("ready");
      },
      onError: (e) => {
        if (cancelled) return;
        setEngine("error");
        toast.error("Engine failed to load", {
          description: e instanceof Error ? e.message : String(e),
        });
      },
    });
    return () => {
      cancelled = true;
      handle.cancel();
    };
  }, []);

  const run = React.useCallback(async () => {
    setState({ kind: "running" });
    try {
      const t0 = performance.now();
      // [OPUS-4.8] sq-pyn7 (#796) — when the shapes editor is in Compact mode the SCS text is
      // parsed to a Turtle shapes graph FIRST (via the wasm binding), then validated exactly
      // as a hand-written Turtle shapes graph would be — the SCS input round-trips losslessly
      // through the same `validate` path.
      const shapesTurtle = await resolveShapesTurtle();
      const report = await sparqShaclValidate(data, shapesTurtle, "turtle");
      const ms = performance.now() - t0;
      setEngine("ready");
      setState({ kind: "report", report, ms });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setState({ kind: "error", message });
      toast.error("Validation failed", { description: message });
    }
  }, [data, resolveShapesTurtle]);

  // [OPUS-4.8] sq-pvg2 (#796) — render the SHAPES graph in SHACL Compact Syntax.
  // Parses the shapes Turtle with the same wasm engine, pulls every triple out via
  // `matchQuads`, and serialises with the dependency-free `shapesToCompact`. Switches
  // the report view to the Compact tab so the result is visible immediately.
  const renderCompact = React.useCallback(async () => {
    setView("compact");
    setCompact({ kind: "running" });
    try {
      // [OPUS-4.8] sq-pyn7 (#796) — resolve to Turtle first so the compact render works in
      // BOTH input modes: in Compact mode this parses SCS → Turtle, then re-serialises Turtle
      // → SCS, exercising the full round-trip; in Turtle mode it serialises the text directly.
      const shapesTurtle = await resolveShapesTurtle();
      const Store = await loadSparq();
      const store = loadIntoStore(Store, shapesTurtle, "turtle");
      const rows: SparqlBinding[] = matchQuads(store);
      const triples: ScsTriple[] = rows.flatMap((row) => {
        const { s, p, o } = row;
        if (!s || !p || !o) return [];
        return [
          { subject: toScsTerm(s), predicate: toScsTerm(p), object: toScsTerm(o) },
        ];
      });
      // Recover the prefix labels from the resolved Turtle (it carries the `@prefix`/PREFIX
      // declarations in both modes — the wasm parser emits them for the SCS-derived graph too).
      const { text, unsupported } = shapesToCompact(triples, extractPrefixes(shapesTurtle));
      setEngine("ready");
      setCompact({ kind: "done", text, unsupported });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setCompact({ kind: "error", message });
      toast.error("Could not render compact syntax", { description: message });
    }
  }, [resolveShapesTurtle]);

  const selectExample = React.useCallback((id: string) => {
    const ex = SHACL_EXAMPLES.find((e) => e.id === id);
    if (!ex) return;
    setData(ex.data);
    setShapes(ex.shapes);
    // [OPUS-4.8] sq-pyn7 (#796) — an example may carry its shapes as SCS (`shapesMode:
    // "compact"`); switch the shapes-input toggle to match so it parses, not validates raw.
    setShapesMode(ex.shapesMode ?? "turtle");
    setActiveExample(id);
    setState({ kind: "idle" });
    setCompact({ kind: "idle" });
  }, []);

  const busy = state.kind === "running";
  const compactBusy = compact.kind === "running";

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
        <CardTitle className="flex items-center gap-2 text-base">
          <ShieldCheck className="size-4 text-primary" />
          Live SHACL validator
        </CardTitle>
        <EngineIndicator engine={engine} />
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex flex-wrap gap-1.5">
          {SHACL_EXAMPLES.map((ex) => (
            <Button
              key={ex.id}
              variant={activeExample === ex.id ? "default" : "outline"}
              size="sm"
              onClick={() => selectExample(ex.id)}
              title={ex.description}
            >
              {ex.label}
            </Button>
          ))}
        </div>

        <div className="grid gap-3 md:grid-cols-2">
          <div className="space-y-1.5">
            <label
              htmlFor="shacl-data"
              className="text-xs font-medium text-muted-foreground"
            >
              Data graph (Turtle)
            </label>
            <textarea
              id="shacl-data"
              value={data}
              spellCheck={false}
              onChange={(e) => setData(e.target.value)}
              rows={12}
              className="w-full resize-y rounded-lg border bg-muted/40 p-3 font-mono text-[12.5px] leading-relaxed outline-none focus-visible:ring-3 focus-visible:ring-ring/40"
            />
          </div>
          <div className="space-y-1.5">
            <div className="flex items-center justify-between gap-2">
              <label
                htmlFor="shacl-shapes"
                className="text-xs font-medium text-muted-foreground"
              >
                Shapes graph ({shapesMode === "compact" ? "Compact syntax" : "Turtle"})
              </label>
              <ShapesModeToggle mode={shapesMode} onChange={setShapesMode} />
            </div>
            <textarea
              id="shacl-shapes"
              value={shapes}
              spellCheck={false}
              onChange={(e) => setShapes(e.target.value)}
              rows={12}
              className="w-full resize-y rounded-lg border bg-muted/40 p-3 font-mono text-[12.5px] leading-relaxed outline-none focus-visible:ring-3 focus-visible:ring-ring/40"
              aria-describedby={shapesMode === "compact" ? "shacl-shapes-scs-hint" : undefined}
            />
            {shapesMode === "compact" && (
              <p
                id="shacl-shapes-scs-hint"
                className="text-[11px] text-muted-foreground"
              >
                SHACL Compact Syntax — parsed to a shapes graph via the wasm{" "}
                <code className="font-mono">Store.parseShaclCompact</code> binding before
                validation.
              </p>
            )}
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <Button onClick={run} disabled={busy || compactBusy}>
            {busy ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <Play className="size-4" />
            )}
            Validate
          </Button>
          <Button
            variant="outline"
            onClick={renderCompact}
            disabled={busy || compactBusy}
            title="Render the shapes graph in SHACL Compact Syntax"
          >
            {compactBusy ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <FileCode2 className="size-4" />
            )}
            Compact syntax
          </Button>
          {(state.kind === "report" || compact.kind === "done") && (
            <ViewTabs
              view={view}
              onChange={setView}
              hasReport={state.kind === "report"}
              hasCompact={compact.kind === "done"}
            />
          )}
          <p aria-live="polite" className="text-xs text-muted-foreground">
            {state.kind === "report" &&
              `${state.report.results.length} result${state.report.results.length === 1 ? "" : "s"} · ${state.ms.toFixed(1)} ms`}
            {state.kind === "running" && "Validating on the wasm engine…"}
            {compact.kind === "running" && "Rendering compact syntax…"}
            {state.kind === "idle" &&
              compact.kind === "idle" &&
              engine === "warming" &&
              "Pre-warming the wasm engine…"}
          </p>
        </div>

        <ResultPanel state={state} compact={compact} view={view} />
      </CardContent>
    </Card>
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
        Engine failed — retries on validate
      </Badge>
    );
  }
  return (
    <Badge variant="muted" aria-live="polite">
      <Loader2 className="size-3 animate-spin" /> Engine loading…
    </Badge>
  );
}

// [OPUS-4.8] sq-pyn7 (#796) — the shapes-input syntax toggle: Turtle (the default; the text
// is fed to the validator unchanged) vs Compact (the text is SHACL Compact Syntax, parsed to
// a Turtle shapes graph via the wasm `Store.parseShaclCompact` binding before validation).
function ShapesModeToggle({
  mode,
  onChange,
}: {
  mode: ShapesMode;
  onChange: (m: ShapesMode) => void;
}) {
  const options: { value: ShapesMode; label: string }[] = [
    { value: "turtle", label: "Turtle" },
    { value: "compact", label: "Compact" },
  ];
  return (
    <div
      role="radiogroup"
      aria-label="Shapes input syntax"
      className="inline-flex rounded-lg border bg-muted/40 p-0.5"
    >
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={mode === o.value}
          onClick={() => onChange(o.value)}
          className={cn(
            "rounded-md px-2 py-0.5 text-[11px] font-medium transition-colors outline-none focus-visible:ring-3 focus-visible:ring-ring/40",
            mode === o.value
              ? "bg-background text-foreground shadow-sm"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function ViewTabs({
  view,
  onChange,
  hasReport,
  hasCompact,
}: {
  view: View;
  onChange: (v: View) => void;
  hasReport: boolean;
  hasCompact: boolean;
}) {
  const tabs: { value: View; label: string }[] = [
    ...(hasReport
      ? ([
          { value: "results", label: "Results" },
          { value: "turtle", label: "W3C Turtle" },
        ] as const)
      : []),
    ...(hasCompact
      ? ([{ value: "compact", label: "Compact syntax" }] as const)
      : []),
  ];
  return (
    <div
      role="tablist"
      aria-label="Report view"
      className="inline-flex rounded-lg border bg-muted/40 p-0.5"
    >
      {tabs.map((t) => (
        <button
          key={t.value}
          type="button"
          role="tab"
          aria-selected={view === t.value}
          onClick={() => onChange(t.value)}
          className={cn(
            "rounded-md px-2.5 py-1 text-xs font-medium transition-colors outline-none focus-visible:ring-3 focus-visible:ring-ring/40",
            view === t.value
              ? "bg-background text-foreground shadow-sm"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}

function ResultPanel({
  state,
  compact,
  view,
}: {
  state: RunState;
  compact: CompactState;
  view: View;
}) {
  // The Compact tab is driven by the (validation-independent) compact render.
  if (view === "compact") {
    return <CompactPanel compact={compact} />;
  }

  if (state.kind === "error") {
    return (
      <pre className="overflow-x-auto rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">
        {state.message}
      </pre>
    );
  }
  if (state.kind !== "report") return null;
  const { report } = state;

  return (
    // [OPUS-4.8] sq-800o — stable e2e anchor: the validation report panel. The Playwright
    // SHACL spec waits on `[data-testid="shacl-report"]` (and reads `data-conforms` off the
    // banner below) rather than scraping copy, so it catches a regression where the report
    // never renders (e.g. the lost-receiver `__wbg_ptr` throw this bead fixes).
    <div className="space-y-3" data-testid="shacl-report">
      <ConformanceBanner conforms={report.conforms} summary={reportSummary(report)} />
      {view === "turtle" ? (
        <pre className="max-h-96 overflow-auto rounded-lg border bg-muted/40 p-3 font-mono text-[12.5px] leading-relaxed">
          {reportToTurtle(report)}
        </pre>
      ) : (
        <ResultList report={report} />
      )}
    </div>
  );
}

// [OPUS-4.8] sq-pvg2 (#796) — renders the shapes-graph -> SHACL Compact Syntax output,
// highlighted via the shared Turtle highlighter (the compact grammar's prefixes / IRIs
// / strings / numbers / comments tokenise compatibly). Any construct the compact
// grammar cannot carry (logical constraints, shape references) is listed explicitly
// rather than silently dropped, so the output never looks falsely complete.
function CompactPanel({ compact }: { compact: CompactState }) {
  if (compact.kind === "error") {
    return (
      <pre className="overflow-x-auto rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">
        {compact.message}
      </pre>
    );
  }
  if (compact.kind !== "done") {
    return (
      <p className="rounded-lg border bg-muted/30 p-3 text-sm text-muted-foreground">
        Rendering…
      </p>
    );
  }
  return (
    <div className="space-y-3">
      {compact.text ? (
        <RdfHighlight
          text={compact.text}
          className="max-h-96 overflow-auto rounded-lg border bg-muted/40 p-3 text-[12.5px] leading-relaxed"
          data-testid="scs-output"
        />
      ) : (
        <p className="rounded-lg border bg-muted/30 p-3 text-sm text-muted-foreground">
          No SHACL node shapes found in the shapes graph.
        </p>
      )}
      {compact.unsupported.length > 0 && (
        <div className="space-y-1.5 rounded-lg border border-[var(--warning)]/30 bg-[color-mix(in_oklch,var(--warning)_8%,transparent)] p-3">
          <Badge variant="warning">
            {compact.unsupported.length} not expressible in compact syntax
          </Badge>
          <ul className="space-y-1 text-xs text-muted-foreground">
            {compact.unsupported.map((u, i) => (
              <li key={i}>{u}</li>
            ))}
          </ul>
          <p className="text-xs text-muted-foreground">
            SHACL Compact Syntax has no form for logical constraints (sh:and /
            sh:or / sh:xone / sh:not) or shape references (sh:node), so those are
            listed here rather than dropped — the compact output stays honest
            about what it carries.
          </p>
        </div>
      )}
    </div>
  );
}

function ConformanceBanner({
  conforms,
  summary,
}: {
  conforms: boolean;
  summary: string;
}) {
  return (
    <div
      // [OPUS-4.8] sq-800o — expose conformance as a stable `data-conforms` boolean attr so the
      // e2e SHACL spec asserts `sh:conforms` without reading the localized banner copy.
      data-conforms={conforms ? "true" : "false"}
      className={cn(
        "flex items-center gap-2 rounded-lg p-3 text-sm font-medium",
        conforms
          ? "bg-[color-mix(in_oklch,var(--success)_15%,transparent)] text-[var(--success)]"
          : "bg-destructive/10 text-destructive",
      )}
    >
      {conforms ? (
        <CheckCircle2 className="size-4" aria-hidden="true" />
      ) : (
        <XCircle className="size-4" aria-hidden="true" />
      )}
      <span>
        <span className="font-mono">sh:conforms</span> = {conforms ? "true" : "false"}{" "}
        — {summary}
      </span>
    </div>
  );
}

function ResultList({ report }: { report: ShaclReport }) {
  if (report.results.length === 0) {
    return (
      <p className="rounded-lg border bg-muted/30 p-3 text-sm text-muted-foreground">
        No violations — every targeted node satisfies its shape.
      </p>
    );
  }
  return (
    <ul className="space-y-2">
      {report.results.map((r, i) => (
        <li
          key={i}
          // [OPUS-4.8] sq-800o — per-violation e2e anchor + the constraint component as a
          // stable `data-component` attr (the DatatypeConstraintComponent the default example
          // produces), so the spec asserts the violation without scraping the badge label.
          data-testid="shacl-violation"
          data-component={componentName(r.sourceConstraintComponent)}
          className="rounded-lg border bg-muted/30 p-3 text-sm"
        >
          <div className="mb-1.5 flex flex-wrap items-center gap-2">
            <Badge variant="warning" className="font-mono text-[11px]">
              {componentName(r.sourceConstraintComponent)}
            </Badge>
            <Badge variant="muted" className="text-[11px]">
              {severityName(r.severity)}
            </Badge>
          </div>
          {r.message && (
            <p className="mb-1.5 text-foreground">{r.message}</p>
          )}
          <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 font-mono text-[12px] text-muted-foreground">
            <dt>focus</dt>
            <dd className="break-all text-foreground">{shortenIri(r.focusNode)}</dd>
            {r.path && (
              <>
                <dt>path</dt>
                <dd className="break-all text-foreground">{shortenIri(r.path)}</dd>
              </>
            )}
            {r.value && (
              <>
                <dt>value</dt>
                <dd className="break-all text-foreground">{shortenIri(r.value)}</dd>
              </>
            )}
          </dl>
        </li>
      ))}
    </ul>
  );
}
