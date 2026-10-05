"use client";

// [OPUS-4.8] sq-tp1m (#757) — the per-workspace INFERENCE toggle + the workspace⇄engine sync.
//
// The selector chooses the entailment regime (Off / RDFS / OWL 2 RL / N3) for the ACTIVE workspace;
// the choice is PERSISTED on the workspace (survives a restart) and applied to queries by the
// engine's REAL forward-chaining reasoner (sparq-reason, via the tier-b W-reason wasm bundle) —
// never a mock. Reasoning is a query-time regime: it never mutates the persisted store, only what
// queries see (an entailed triple can match). The live status pill shows the measured
// entailed-triple count, a loading spinner, or an honest "reasoner unavailable" when the bundle
// was not synced into this build.

import * as React from "react";
import { Brain, Loader2, AlertTriangle, Sparkles } from "lucide-react";
import { WORKSPACE_INFERENCE_MODES, type WorkspaceInferenceMode } from "@sparq/client";

import { cn } from "@/lib/utils";
import { useEngine } from "@/lib/engine-context";
import { useWorkspace } from "@/lib/workspace-context";
import { INFERENCE_MODE_META } from "@/lib/reason-wasm";

/**
 * Keep the ENGINE's active inference regime in lockstep with the active workspace's PERSISTED
 * choice. Renders null; mounted once in the workbench shell so the sync runs regardless of which
 * tool tab is focused — a restored workspace's regime is applied as soon as it loads.
 */
export function InferenceModeBridge() {
  const { inference } = useWorkspace();
  const { setInferenceMode } = useEngine();
  React.useEffect(() => {
    setInferenceMode(inference);
  }, [inference, setInferenceMode]);
  return null;
}

/**
 * [sq-glo5r] Keep the ENGINE's N3 rules cache in lockstep with the active workspace's persisted
 * rules docs. Renders null; mounted once alongside {@link InferenceModeBridge} so the N3 closure
 * is rebuilt whenever rules change, regardless of the focused tool tab.
 */
export function N3RulesBridge() {
  const { rulesDocs } = useWorkspace();
  const { setN3Rules } = useEngine();
  React.useEffect(() => {
    setN3Rules(rulesDocs);
  }, [rulesDocs, setN3Rules]);
  return null;
}

/** A live status pill for the active regime: entailed-triple count / loading / honest error. */
export function InferenceStatusPill() {
  const { inferenceStatus } = useEngine();
  if (inferenceStatus.kind === "off") return null;
  if (inferenceStatus.kind === "loading") {
    return (
      <span
        className="flex items-center gap-1 text-[11px] text-muted-foreground"
        data-inference-status="loading"
      >
        <Loader2 className="size-3 animate-spin" /> reasoning…
      </span>
    );
  }
  if (inferenceStatus.kind === "error") {
    return (
      <span
        className="flex items-center gap-1 text-[11px] text-destructive"
        title={inferenceStatus.message}
        data-inference-status="error"
      >
        <AlertTriangle className="size-3" /> reasoner unavailable
      </span>
    );
  }
  const { entailed, closureTriples } = inferenceStatus.info;
  return (
    <span
      className="flex items-center gap-1 text-[11px] text-primary"
      title={`Forward-chained closure over the live store — ${closureTriples.toLocaleString()} triples (${entailed.toLocaleString()} entailed)`}
      data-inference-status="ready"
    >
      <Sparkles className="size-3" /> +{entailed.toLocaleString()} entailed
    </span>
  );
}

/**
 * The compact per-workspace INFERENCE selector for the query toolbar: Off / RDFS / OWL 2 RL / N3.
 * Reads the persisted mode from the workspace and the live reasoning status from the engine;
 * applies the choice to the engine immediately AND persists it on the workspace.
 */
export function InferenceControl({ className }: { className?: string }) {
  const { inference, setInference } = useWorkspace();
  const { setInferenceMode } = useEngine();

  const choose = React.useCallback(
    (mode: WorkspaceInferenceMode) => {
      // Apply to the engine immediately, then persist on the workspace (best-effort).
      setInferenceMode(mode);
      void setInference(mode);
    },
    [setInferenceMode, setInference],
  );

  return (
    <div className={cn("flex items-center gap-1.5", className)} data-inference-control>
      <Brain className="size-3.5 text-muted-foreground" aria-hidden />
      <span className="text-[11px] font-medium text-muted-foreground">Inference</span>
      <div
        className="flex items-center rounded-md border bg-background p-0.5"
        role="group"
        aria-label="Inference mode"
      >
        {WORKSPACE_INFERENCE_MODES.map((m) => {
          const meta = INFERENCE_MODE_META[m];
          const active = inference === m;
          return (
            <button
              key={m}
              type="button"
              data-inference-mode={m}
              aria-pressed={active}
              onClick={() => choose(m)}
              title={meta.blurb}
              className={cn(
                "rounded px-1.5 py-0.5 text-[11px]",
                active
                  ? "bg-primary/10 font-medium text-primary"
                  : "text-muted-foreground hover:bg-accent/40",
              )}
            >
              {meta.short}
            </button>
          );
        })}
      </div>
      <InferenceStatusPill />
    </div>
  );
}
