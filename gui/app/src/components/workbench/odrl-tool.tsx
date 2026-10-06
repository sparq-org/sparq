"use client";

// [FABLE-5] sq-ixc3.15 — the ODRL policy tool: author/validate a policy, evaluate a request,
// and preview the access-controlled result set per requester NEXT TO the ungated rows.
//
// The whole round-trip runs in ONE native command (`odrl_preview`, gui/src-tauri/src/odrl.rs):
// parse the Turtle ODRL policy → evaluate the (party, action, target) request per requester →
// materialize through the odrl-bridge → run the SAME SPARQL query ungated AND per requester
// through PodStore's fail-closed per-session named-graph gating. NATIVE-ONLY: the ODRL stack
// is not in the in-tab wasm bundle, so the hosted web build degrades honestly (WEB_ODRL_MESSAGE,
// run disabled) — never a fabricated decision.
//
// Follows the GUI workbench translation rule (research/gui-design.md §A.4/§A.5): marketing
// chrome CUT, live-result rendering only, honest error messaging. FAIL-CLOSED UX invariants:
//   - A malformed policy renders the deny-everything banner with the parser's verbatim reason;
//     every gated pane shows the REAL zero-row result the empty auth index produced.
//   - A requester's pane lists the named graphs the policy HID versus the ungated pane, so a
//     prohibition visibly flips a previously visible graph to hidden.
//   - No result is ever synthesized in the webview; every row comes from the native engine.

import * as React from "react";
import { Scale, Loader2, Play } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { toolById, TIER_META } from "@/data/tools";
import { useEngine } from "@/lib/engine-context";
import { isTauriRuntime, nativeOdrlPreview } from "@/lib/tauri-ipc";
import {
  WEB_ODRL_MESSAGE,
  ODRL_ACTIONS,
  SAMPLE_DATASET_TRIG,
  SAMPLE_POLICY_TTL,
  SAMPLE_TARGET,
  SAMPLE_REQUESTERS,
  SAMPLE_QUERY,
  hiddenGraphs,
  describeMalformedPolicy,
  type OdrlPreviewResult,
  type OdrlPane,
} from "@/lib/odrl";
import { isAskResult, termValue, type SparqlResults } from "@sparq/client";

// [FABLE-5] sq-qgkwy.2 — the override lives in the sibling `.meta.ts` (eagerly bundled for the
// rail/tab honesty read path) so THIS panel module can stay behind a lazy dynamic import().
// Re-exported here so the panel file remains the tool's single import surface.
export { ODRL_TOOL_OVERRIDE } from "@/components/workbench/odrl-tool.meta";

// ---------------------------------------------------------------------------
// State shapes.
// ---------------------------------------------------------------------------

type RunState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "result"; preview: OdrlPreviewResult }
  | { kind: "error"; message: string };

type DatasetSource = "sample" | "store";

/** Parse a pane's SPARQL-JSON, or `null` when it is not a SELECT document. */
function parseSelect(json: string): SparqlResults | null {
  try {
    const parsed = JSON.parse(json) as SparqlResults;
    return isAskResult(parsed) ? null : parsed;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// Small render helpers.
// ---------------------------------------------------------------------------

function LabeledTextarea({
  label,
  value,
  onChange,
  hook,
  rows,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  hook: string;
  rows: number;
}) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      <textarea
        {...{ [hook]: "" }}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        rows={rows}
        spellCheck={false}
        aria-label={label}
        className="w-full resize-y rounded-md border bg-background p-2 font-mono text-xs outline-none focus:ring-2 focus:ring-ring"
      />
    </div>
  );
}

/** A compact bindings table for one pane (fail-closed: renders exactly what came back). */
function PaneTable({ results }: { results: SparqlResults }) {
  const vars = results.head?.vars ?? [];
  const rows = results.results?.bindings ?? [];
  if (rows.length === 0) {
    return <p className="px-2 py-1.5 text-xs text-muted-foreground">(0 rows)</p>;
  }
  return (
    <table className="w-full text-left text-xs">
      <thead className="border-b bg-card">
        <tr>
          {vars.map((v) => (
            <th key={v} className="px-2 py-1 font-medium text-muted-foreground">
              ?{v}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row, ri) => (
          <tr key={ri} className="border-b">
            {vars.map((v) => (
              <td key={v} className="px-2 py-1 font-mono">
                {termValue(row[v]) ?? ""}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** One requester's gated pane: decision, hidden-graph diff, and the gated rows. */
function RequesterPane({
  pane,
  ungated,
  index,
}: {
  pane: OdrlPane;
  ungated: SparqlResults | null;
  index: number;
}) {
  const gated = parseSelect(pane.results_json);
  const hidden = ungated && gated ? hiddenGraphs(ungated, gated) : [];
  const suffix = index === 0 ? "a" : index === 1 ? "b" : String(index);
  return (
    <div
      className="flex min-w-0 flex-1 flex-col rounded-md border"
      {...{ [`data-odrl-pane-${suffix}`]: "" }}
    >
      <div className="flex items-center gap-2 border-b bg-card px-2 py-1.5">
        <span className="min-w-0 flex-1 truncate font-mono text-xs" title={pane.requester}>
          {pane.requester}
        </span>
        <Badge
          variant={pane.allow ? "success" : "muted"}
          {...{ [`data-odrl-decision-${suffix}`]: pane.allow ? "allow" : "deny" }}
        >
          {pane.allow ? "Allow" : "Deny"}
        </Badge>
      </div>
      {(pane.matched_rules.length > 0 || pane.unmet_constraints.length > 0) && (
        <div className="space-y-0.5 border-b px-2 py-1.5 text-[11px] text-muted-foreground">
          {pane.matched_rules.map((r) => (
            <p key={r} className="truncate" title={r}>
              matched: <span className="font-mono">{r}</span>
            </p>
          ))}
          {pane.unmet_constraints.map((c) => (
            <p key={c}>unmet: {c}</p>
          ))}
        </div>
      )}
      {hidden.length > 0 && (
        <p
          className="border-b px-2 py-1.5 text-[11px] text-[var(--warning)]"
          {...{ [`data-odrl-hidden-${suffix}`]: "" }}
        >
          Hidden vs ungated: {hidden.map((g) => `<${g}>`).join(", ")}
        </p>
      )}
      <div className="min-h-0 overflow-auto">
        {gated ? (
          <PaneTable results={gated} />
        ) : (
          <pre className="whitespace-pre-wrap p-2 font-mono text-[11px]">{pane.results_json}</pre>
        )}
      </div>
      {pane.bridge_notes.length > 0 && (
        <details className="border-t px-2 py-1 text-[11px] text-muted-foreground">
          <summary className="cursor-pointer">Bridge notes ({pane.bridge_notes.length})</summary>
          <ul className="space-y-0.5 pt-1">
            {pane.bridge_notes.map((n, i) => (
              <li key={i} className="break-all font-mono">
                {n}
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main panel.
// ---------------------------------------------------------------------------

export function OdrlTool() {
  const native = isTauriRuntime();
  const { snapshotStore } = useEngine();

  const [policy, setPolicy] = React.useState(SAMPLE_POLICY_TTL);
  const [datasetSource, setDatasetSource] = React.useState<DatasetSource>("sample");
  const [sampleDataset, setSampleDataset] = React.useState(SAMPLE_DATASET_TRIG);
  const [action, setAction] = React.useState(ODRL_ACTIONS[0].iri);
  const [target, setTarget] = React.useState(SAMPLE_TARGET);
  const [requesterA, setRequesterA] = React.useState<string>(SAMPLE_REQUESTERS[0]);
  const [requesterB, setRequesterB] = React.useState<string>(SAMPLE_REQUESTERS[1]);
  const [query, setQuery] = React.useState(SAMPLE_QUERY);
  const [run, setRun] = React.useState<RunState>({ kind: "idle" });

  // toolById (not tool-panel's resolveTool) to avoid an import cycle with the registry; the
  // override does not change the tier, so the base read is the honest one.
  const tool = toolById("odrl");
  const tier = tool ? TIER_META[tool.tier] : null;

  const onRun = React.useCallback(() => {
    // The live-store snapshot is N-Quads; the editable sample is TriG.
    const dataset =
      datasetSource === "sample" ? sampleDataset : (snapshotStore() ?? "");
    const format = datasetSource === "sample" ? "trig" : "nquads";
    setRun({ kind: "running" });
    void nativeOdrlPreview({
      dataset,
      format,
      policy,
      action,
      target,
      requesters: [requesterA, requesterB],
      query,
    }).then(
      (preview) => {
        // `null` = not in the desktop shell (the button is disabled there, but stay honest).
        if (preview === null) setRun({ kind: "error", message: WEB_ODRL_MESSAGE });
        else setRun({ kind: "result", preview });
      },
      (err) =>
        setRun({
          kind: "error",
          message: err instanceof Error ? err.message : String(err),
        }),
    );
  }, [
    datasetSource,
    sampleDataset,
    snapshotStore,
    policy,
    action,
    target,
    requesterA,
    requesterB,
    query,
  ]);

  return (
    <div className="flex h-full flex-col overflow-auto">
      {/* Operational header: icon + verb + the honesty tier dot. */}
      <div className="flex items-center gap-2 border-b bg-card px-3 py-1.5">
        <Scale className="size-4 text-primary" aria-hidden />
        <h2 className="text-sm font-semibold">Policies (ODRL)</h2>
        {tier && (
          <span
            className="flex items-center gap-1.5 text-[11px] text-muted-foreground"
            data-odrl-tier=""
          >
            <span className={cn("size-2 rounded-full", tier.dot)} aria-hidden />
            {tier.label}
          </span>
        )}
        <Button
          size="sm"
          onClick={onRun}
          disabled={!native || run.kind === "running"}
          data-odrl-run=""
          className="ml-auto"
        >
          {run.kind === "running" ? <Loader2 className="animate-spin" /> : <Play />}
          Evaluate + preview
        </Button>
      </div>

      {/* Honest web-build degradation: the ODRL stack is not in the wasm bundle. */}
      {!native && (
        <p
          data-odrl-web-note=""
          className="border-b bg-card px-3 py-2 text-xs text-[var(--warning)]"
        >
          {WEB_ODRL_MESSAGE}
        </p>
      )}

      {/* Authoring: policy + dataset. */}
      <div className="grid gap-3 p-3 md:grid-cols-2">
        <LabeledTextarea
          label="ODRL policy (Turtle)"
          value={policy}
          onChange={setPolicy}
          hook="data-odrl-policy"
          rows={12}
        />
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-3 text-xs">
            <span className="font-medium text-muted-foreground">Dataset</span>
            <label className="flex items-center gap-1">
              <input
                type="radio"
                name="odrl-dataset-source"
                checked={datasetSource === "sample"}
                onChange={() => setDatasetSource("sample")}
                data-odrl-source-sample=""
              />
              Sample (two named graphs)
            </label>
            <label className="flex items-center gap-1">
              <input
                type="radio"
                name="odrl-dataset-source"
                checked={datasetSource === "store"}
                onChange={() => setDatasetSource("store")}
                data-odrl-source-store=""
              />
              Live store snapshot
            </label>
          </div>
          {datasetSource === "sample" ? (
            <LabeledTextarea
              label="Sample dataset (TriG)"
              value={sampleDataset}
              onChange={setSampleDataset}
              hook="data-odrl-dataset"
              rows={9}
            />
          ) : (
            <p className="rounded-md border bg-card p-2 text-xs text-muted-foreground">
              The whole live workspace store (every named graph) is snapshotted to the native
              engine at run time. Gating decides per NAMED graph — data in the default graph is
              not addressable by an ODRL target.
            </p>
          )}
        </div>
      </div>

      {/* Request form: action / target / the two requesters. */}
      <div className="grid gap-3 px-3 pb-3 md:grid-cols-4">
        <div className="flex flex-col gap-1">
          <label htmlFor="odrl-action" className="text-xs font-medium text-muted-foreground">
            Action
          </label>
          <select
            id="odrl-action"
            data-odrl-action=""
            value={action}
            onChange={(e) => setAction(e.target.value)}
            className="rounded-md border bg-background px-2 py-1.5 font-mono text-xs outline-none focus:ring-2 focus:ring-ring"
          >
            {ODRL_ACTIONS.map((a) => (
              <option key={a.iri} value={a.iri}>
                odrl:{a.label}
              </option>
            ))}
          </select>
        </div>
        {(
          [
            ["Target (named graph IRI)", target, setTarget, "data-odrl-target"],
            ["Requester A (WebID)", requesterA, setRequesterA, "data-odrl-requester-a"],
            ["Requester B (WebID)", requesterB, setRequesterB, "data-odrl-requester-b"],
          ] as const
        ).map(([label, value, set, hook]) => (
          <div key={hook} className="flex flex-col gap-1">
            <span className="text-xs font-medium text-muted-foreground">{label}</span>
            <input
              {...{ [hook]: "" }}
              value={value}
              onChange={(e) => set(e.target.value)}
              spellCheck={false}
              aria-label={label}
              className="rounded-md border bg-background px-2 py-1.5 font-mono text-xs outline-none focus:ring-2 focus:ring-ring"
            />
          </div>
        ))}
      </div>

      {/* The SAME query every pane runs. */}
      <div className="px-3 pb-3">
        <LabeledTextarea
          label="SPARQL query (run ungated + per requester through the gated view)"
          value={query}
          onChange={setQuery}
          hook="data-odrl-query"
          rows={4}
        />
      </div>

      {/* Results. */}
      <div className="flex min-h-0 flex-1 flex-col gap-3 border-t p-3">
        {run.kind === "idle" && (
          <p className="text-sm text-muted-foreground">
            Evaluate the request and preview the gated result set per requester.
          </p>
        )}
        {run.kind === "running" && <p className="text-sm text-muted-foreground">Running…</p>}
        {run.kind === "error" && (
          <pre
            data-odrl-error=""
            className="whitespace-pre-wrap font-mono text-xs text-destructive"
          >
            {run.message}
          </pre>
        )}
        {run.kind === "result" && <PreviewResult preview={run.preview} />}
      </div>
    </div>
  );
}

function PreviewResult({ preview }: { preview: OdrlPreviewResult }) {
  const ungated = parseSelect(preview.ungated_json);
  return (
    <>
      {/* Policy validation status — malformed ⇒ the deny-everything banner, reason verbatim. */}
      {preview.policy_ok ? (
        <p className="text-xs text-muted-foreground" data-odrl-policy-ok="">
          Policy OK — {preview.permissions} permission(s), {preview.prohibitions}{" "}
          prohibition(s).
          {preview.refused &&
            " REFUSED: the policy's odrl:conflict strategy cannot be honored — nothing was materialized (fail-closed)."}
        </p>
      ) : (
        <pre
          data-odrl-policy-error=""
          className="whitespace-pre-wrap rounded-md border border-destructive/50 bg-card p-2 font-mono text-xs text-destructive"
        >
          {describeMalformedPolicy(preview.policy_error ?? "unknown parse error")}
        </pre>
      )}

      {/* The two gated panes, side by side. */}
      <div className="flex flex-col gap-3 lg:flex-row">
        {preview.panes.map((pane, i) => (
          <RequesterPane key={pane.requester + i} pane={pane} ungated={ungated} index={i} />
        ))}
      </div>

      {/* The ungated pane: what the raw dataset answers with no gating at all. */}
      <div className="rounded-md border" data-odrl-pane-ungated="">
        <p className="border-b bg-card px-2 py-1.5 text-xs font-medium text-muted-foreground">
          Ungated (no access control)
        </p>
        <div className="overflow-auto">
          {ungated ? (
            <PaneTable results={ungated} />
          ) : (
            <pre className="whitespace-pre-wrap p-2 font-mono text-[11px]">
              {preview.ungated_json}
            </pre>
          )}
        </div>
      </div>
    </>
  );
}
