"use client";

// [OPUS-4.8] sq-tp1m (#757) — the INFERENCE tool: a real, working panel (no longer a stub) for
// the per-workspace entailment regime. It drives the SAME engine + workspace state as the
// compact toolbar selector (InferenceControl), and additionally explains the active regime and
// shows the MEASURED base / closure / entailed triple counts the engine's forward-chaining
// reasoner (sparq-reason, via the tier-b W-reason wasm bundle) produced over the live store.
//
// [sq-glo5r] — N3 RULES section: when the workspace is in "n3" mode, this tool additionally
// shows the per-workspace N3 rule documents (list + enable/disable + remove), a drag-drop /
// browse bulk-upload zone (.n3 / .ttl files), and a collapsible "Author a rule" pane with a
// syntax-highlighting editor + name input + "Add rule" button.
//
// HONESTY: reasoning is REAL (not a captured walkthrough), but it is a QUERY-TIME regime — it
// never mutates the persisted store, and named graphs are folded into the default graph for
// entailment. RDFS + OWL 2 RL are wired; N3 derives GROUND triples only — formula / quoted-graph
// conclusions are dropped (the in-tab ground-triple store cannot hold formula terms).

import * as React from "react";
import { Brain, Info, Plus, Trash2, ChevronDown, ChevronUp } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useEngine } from "@/lib/engine-context";
import { useWorkspace } from "@/lib/workspace-context";
import { INFERENCE_MODE_META } from "@/lib/reason-wasm";
import { toolById, TIER_META } from "@/data/tools";
import { InferenceControl } from "@/components/workbench/inference-control";
import { ProofPanel, type ExplainTarget } from "@/components/workbench/proof-panel";
import { WorkbenchTurtleEditor } from "@/components/workbench/turtle-editor";
import { Dropzone } from "@/components/workbench/dropzone";
import type { IngestResult } from "@/lib/file-ingest";
import type { InferredFact } from "@/lib/inferred-facts";

/** One labelled measured count in the stats strip. */
function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex flex-col rounded-md border bg-card px-3 py-2">
      <span className="tabular text-lg font-semibold">{value.toLocaleString()}</span>
      <span className="text-[11px] uppercase tracking-wide text-muted-foreground">{label}</span>
    </div>
  );
}

// Accepted file extensions for N3 rule documents.
const N3_ACCEPT = [".n3", ".ttl"];

/**
 * [sq-glo5r] The N3 rules management panel: list of attached rule documents (enable / remove),
 * a bulk-upload dropzone, and a collapsible inline-authoring pane.
 */
function N3RulesPanel() {
  const { rulesDocs, addRulesDocs, removeRulesDoc, setRulesDocEnabled } = useWorkspace();

  // --- Authoring pane state ---
  const [authorOpen, setAuthorOpen] = React.useState(false);
  const [authorText, setAuthorText] = React.useState("");
  const [authorName, setAuthorName] = React.useState("");

  // --- Dropzone upload handler ---
  const handleFiles = React.useCallback(
    (result: IngestResult) => {
      if (result.accepted.length === 0) return;
      void addRulesDocs(result.accepted.map((f) => ({ name: f.name, text: f.text })));
    },
    [addRulesDocs],
  );

  // Hidden file input for Playwright setInputFiles() — exercises the same addRulesDocs path.
  const hiddenInputRef = React.useRef<HTMLInputElement>(null);
  const handleHiddenInput = React.useCallback(
    async (e: React.ChangeEvent<HTMLInputElement>) => {
      const files = Array.from(e.target.files ?? []);
      if (hiddenInputRef.current) hiddenInputRef.current.value = "";
      if (files.length === 0) return;
      const read: Array<{ name: string; text: string }> = [];
      for (const file of files) {
        try {
          read.push({ name: file.name, text: await file.text() });
        } catch {
          /* skip unreadable files */
        }
      }
      if (read.length > 0) void addRulesDocs(read);
    },
    [addRulesDocs],
  );

  // --- Add authored rule ---
  const handleAddRule = React.useCallback(() => {
    const name = authorName.trim() || "rule";
    const text = authorText.trim();
    if (!text) return;
    void addRulesDocs([{ name, text }]);
    setAuthorText("");
    setAuthorName("");
  }, [authorName, authorText, addRulesDocs]);

  return (
    <div className="space-y-4" data-n3-rules>
      <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        N3 Rules ({rulesDocs.length})
      </h3>

      {/* Rules list */}
      {rulesDocs.length > 0 ? (
        <div className="space-y-1">
          {rulesDocs.map((doc) => (
            <div
              key={doc.addedAt}
              data-n3-rule-row={String(doc.addedAt)}
              className="flex items-center gap-2 rounded-md border bg-background px-3 py-2"
            >
              <input
                type="checkbox"
                data-n3-rule-toggle
                aria-label={`Enable rule "${doc.name}"`}
                checked={doc.enabled !== false}
                onChange={(e) => void setRulesDocEnabled(doc.addedAt, e.target.checked)}
                className="size-3.5 accent-primary"
              />
              <span className="min-w-0 flex-1 truncate text-xs font-medium" title={doc.name}>
                {doc.name}
              </span>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="size-6 shrink-0 text-muted-foreground hover:text-destructive"
                aria-label={`Remove rule "${doc.name}"`}
                data-n3-rule-remove
                onClick={() => void removeRulesDoc(doc.addedAt)}
              >
                <Trash2 className="size-3.5" aria-hidden />
              </Button>
            </div>
          ))}
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          No rules attached yet — upload .n3 / .ttl files below or author one inline.
        </p>
      )}

      {/* Bulk upload dropzone */}
      <div className="relative" data-n3-dropzone>
        <Dropzone
          onFiles={handleFiles}
          accept={N3_ACCEPT}
          multiple
          label="Drop .n3 or .ttl rule files to add them"
          hint=".n3 · .ttl"
          browseLabel="Browse rule files…"
        />
        {/* Stable hidden input for Playwright setInputFiles() — mirrors data-global-drop-input
            (sq-eydh9): sr-only + a real aria-label, never aria-hidden on a focusable control. */}
        <input
          ref={hiddenInputRef}
          type="file"
          multiple
          accept={N3_ACCEPT.join(",")}
          data-n3-file-input
          aria-label="Select N3 rule files to add"
          className="sr-only"
          onChange={handleHiddenInput}
        />
      </div>

      {/* Scope note */}
      <p className="text-[11px] text-muted-foreground">
        Derived ground triples only — formula / quoted-graph conclusions are dropped and not
        visible in query results.
      </p>

      {/* Author a rule (collapsible) */}
      <div className="rounded-md border bg-background">
        <button
          type="button"
          onClick={() => setAuthorOpen((v) => !v)}
          className="flex w-full items-center justify-between px-3 py-2 text-xs font-medium"
          aria-expanded={authorOpen}
        >
          <span className="flex items-center gap-1.5">
            <Plus className="size-3.5" aria-hidden /> Author a rule
          </span>
          {authorOpen ? (
            <ChevronUp className="size-3.5 text-muted-foreground" aria-hidden />
          ) : (
            <ChevronDown className="size-3.5 text-muted-foreground" aria-hidden />
          )}
        </button>
        {authorOpen ? (
          <div className="space-y-2 border-t px-3 pb-3 pt-2">
            {/* Name input */}
            <div className="flex items-center gap-2">
              <label htmlFor="n3-author-name" className="shrink-0 text-[11px] text-muted-foreground">
                Name
              </label>
              <input
                id="n3-author-name"
                type="text"
                data-n3-author-name
                value={authorName}
                onChange={(e) => setAuthorName(e.target.value)}
                placeholder="my-rule"
                className="flex-1 rounded border bg-background px-2 py-1 text-xs outline-none focus:ring-1 focus:ring-primary"
              />
            </div>
            {/* Turtle/N3 editor */}
            <div
              data-n3-author-textarea
              className="h-32 overflow-hidden rounded border bg-background"
            >
              <WorkbenchTurtleEditor
                value={authorText}
                onChange={setAuthorText}
                ariaLabel="N3 rule editor"
                id="n3-author-textarea"
                placeholder={`@prefix ex: <http://example.org/> .\n{ ?s a ex:A } => { ?s a ex:B . }`}
              />
            </div>
            <Button
              type="button"
              size="sm"
              disabled={!authorText.trim()}
              onClick={handleAddRule}
              data-n3-add-rule
            >
              <Plus className="mr-1 size-3.5" aria-hidden />
              Add rule
            </Button>
          </div>
        ) : null}
      </div>
    </div>
  );
}

const ENTAILED_FACT_PAGE_SIZE = 50;

/** [GPT-5.6] Browse the exact closure-added set and open why() on any retained fact. */
function EntailedFactsBrowser({
  facts,
  onExplain,
}: {
  facts: readonly InferredFact[];
  onExplain: (target: ExplainTarget) => void;
}) {
  const [visibleCount, setVisibleCount] = React.useState(ENTAILED_FACT_PAGE_SIZE);

  const [previousFacts, setPreviousFacts] = React.useState(facts);
  if (previousFacts !== facts) {
    setPreviousFacts(facts);
    setVisibleCount(ENTAILED_FACT_PAGE_SIZE);
  }

  const visibleFacts = facts.slice(0, visibleCount);
  return (
    <section className="space-y-2" data-entailed-facts-browser>
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Entailed facts
        </h3>
        <span className="text-[11px] text-muted-foreground">
          {facts.length.toLocaleString()} closure additions
        </span>
      </div>
      {facts.length === 0 ? (
        <p className="rounded-md border bg-background p-3 text-xs text-muted-foreground">
          This regime added no facts over the asserted store.
        </p>
      ) : (
        <div className="overflow-hidden rounded-md border bg-background">
          <ol className="max-h-72 divide-y overflow-auto" aria-label="Entailed facts">
            {visibleFacts.map((fact) => (
              <li
                key={fact.key}
                className="flex items-start gap-2 px-3 py-1.5"
                data-entailed-fact
              >
                <code
                  className="min-w-0 flex-1 overflow-x-auto whitespace-nowrap text-[11px]"
                  title={fact.ntriples}
                >
                  {fact.ntriples}
                </code>
                <button
                  type="button"
                  className="shrink-0 rounded border border-transparent bg-primary/10 px-1.5 py-0 font-mono text-[10px] text-primary hover:bg-primary/20"
                  onClick={() => onExplain(fact)}
                  aria-label={`Explain inferred fact: ${fact.ntriples}`}
                  title="Explain this inferred fact"
                  data-entailed-fact-explain
                >
                  why?
                </button>
              </li>
            ))}
          </ol>
          {visibleFacts.length < facts.length ? (
            <div className="border-t px-3 py-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => setVisibleCount((count) => count + ENTAILED_FACT_PAGE_SIZE)}
                data-entailed-facts-more
              >
                Show more
              </Button>
            </div>
          ) : null}
        </div>
      )}
      <p className="text-[11px] text-muted-foreground">
        Closure additions only — asserted facts are excluded. Choose why? to inspect one witness
        derivation.
      </p>
    </section>
  );
}

export function InferenceTool() {
  const { inferenceStatus, status, entailedTripleFacts } = useEngine();
  const { inference } = useWorkspace();
  // [GPT-5.6] The status object is the reactive signal that a new exact closure cache is ready.
  const entailedFacts = React.useMemo(
    () => (inferenceStatus.kind === "ready" ? (entailedTripleFacts() ?? []) : []),
    [inferenceStatus, entailedTripleFacts],
  );
  const [explainTarget, setExplainTarget] = React.useState<ExplainTarget | null>(null);

  const [previousInference, setPreviousInference] = React.useState(inference);
  if (previousInference !== inference) {
    setPreviousInference(inference);
    setExplainTarget(null);
  }
  const tool = toolById("inference");
  const tier = tool ? TIER_META[tool.tier] : null;
  const modeMeta = INFERENCE_MODE_META[inference];

  return (
    <div className="relative h-full overflow-auto">
      <div className="mx-auto max-w-2xl space-y-5 p-6">
        {/* Header. */}
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <Brain className="size-5 text-primary" />
            <h2 className="text-lg font-semibold">Inference</h2>
            {tier ? (
              <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <span className={`size-2 rounded-full ${tier.dot}`} aria-hidden />
                {tier.label}
              </span>
            ) : null}
          </div>
          <p className="text-sm text-muted-foreground">
            Apply an RDFS / OWL 2 RL entailment regime or N3 rules to your queries over the live
            store. The engine forward-chains the deductive closure with the real{" "}
            <code className="rounded bg-muted px-1 py-0.5 text-[11px]">sparq-reason</code> reasoner,
            so a query can match an <em>entailed</em> triple, not just an asserted one. The choice
            is saved per workspace.
          </p>
        </div>

        {/* The selector (shared with the query toolbar). */}
        <div className="rounded-lg border bg-background p-4">
          <InferenceControl />
          <p className="mt-3 text-xs text-muted-foreground">{modeMeta.blurb}</p>
        </div>

        {/* N3 rules panel (when N3 mode is active). */}
        {inference === "n3" ? (
          <div className="rounded-lg border bg-background p-4">
            <N3RulesPanel />
          </div>
        ) : null}

        {/* Live measured stats when a regime is active + ready. */}
        {inferenceStatus.kind === "ready" ? (
          <div className="space-y-2" data-inference-stats>
            <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
              Closure over the live store
            </h3>
            <div className="grid grid-cols-3 gap-2">
              <Stat label="Asserted" value={inferenceStatus.info.baseTriples} />
              <Stat label="After reasoning" value={inferenceStatus.info.closureTriples} />
              <Stat label="Entailed" value={inferenceStatus.info.entailed} />
            </div>
            <p className="text-[11px] text-muted-foreground">
              Measured counts (distinct triples; named graphs folded into the default graph for
              entailment). Reasoning runs at query time and does not change the stored data.
            </p>
          </div>
        ) : inferenceStatus.kind === "loading" ? (
          <p className="text-sm text-muted-foreground">Materialising the closure…</p>
        ) : inferenceStatus.kind === "error" ? (
          <div className="rounded-md border border-[var(--warning)]/40 bg-[var(--warning)]/5 p-3 text-xs text-muted-foreground">
            The reasoning bundle could not run: {inferenceStatus.message} It is a separate wasm
            bundle that must be synced into this build — rebuild it with{" "}
            <code className="rounded bg-muted px-1 py-0.5">npm run build:reason-wasm</code> in{" "}
            <code className="rounded bg-muted px-1 py-0.5">js/</code>. Until then, queries fail while
            inference is on — turn inference Off to query the asserted data.
          </div>
        ) : status.kind !== "ready" ? (
          <p className="text-sm text-muted-foreground">Waiting for the engine to warm…</p>
        ) : (
          <p className="text-sm text-muted-foreground">
            Inference is off — queries run over the asserted data. Pick RDFS, OWL 2 RL, or N3
            above to reason over the live store.
          </p>
        )}

        {/* [GPT-5.6] RDFox-style browser over the exact closure-minus-base set. */}
        {inferenceStatus.kind === "ready" ? (
          <EntailedFactsBrowser facts={entailedFacts} onExplain={setExplainTarget} />
        ) : null}

        {/* Honest caveats. */}
        <div className="space-y-1.5 rounded-md border bg-muted/30 p-3 text-xs text-muted-foreground">
          <div className="flex items-center gap-1.5 font-medium text-foreground">
            <Info className="size-3.5" /> How this works
          </div>
          <ul className="list-disc space-y-1 pl-5">
            <li>
              Real forward-chaining: OWL 2 RL includes RDFS. The closure is (re)materialised when
              you change data (import / update) or switch regime.
            </li>
            <li>
              Query-time only: reasoning never mutates the persisted store — turning inference off
              returns you to the asserted data exactly.
            </li>
            <li>
              N3 rules are attached to this workspace — derived ground triples appear in queries
              while N3 is on; formula conclusions cannot live in the ground-triple store and are
              dropped. Named graphs are folded into the default graph for N3 reasoning (same as
              RDFS / OWL-RL).
            </li>
          </ul>
          <div className="pt-1">
            <Badge variant="outline">sq-tp1m</Badge>
            <Badge variant="outline" className="ml-1">sq-glo5r</Badge>
          </div>
        </div>
      </div>
      {explainTarget ? (
        <ProofPanel target={explainTarget} onClose={() => setExplainTarget(null)} />
      ) : null}
    </div>
  );
}
