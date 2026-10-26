"use client";

// [GPT-5.6] sq-3eukz — operational Forms panel derived from the active live workspace. Focus,
// mode, shape, and store-epoch changes always request a whole new FormDescription from the
// runtime host; the browser never rebuilds groups or chooses widgets.
import * as React from "react";
import { Braces, FilePenLine, LoaderCircle } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { FormRenderer } from "@/components/workbench/form-renderer";
import { useEngine, type FormResource } from "@/lib/engine-context";
import {
  deriveForm,
  FormsBridgeUnavailableError,
  type DeriveFormResult,
} from "@/lib/forms-bridge";
import {
  parseFormDescription,
  termKey,
  termLabel,
  type FormDescription,
  type FormMode,
  type TermRef,
} from "@/lib/form-description";

interface DeriveSelection {
  focus: FormResource;
  mode: FormMode;
  shape?: TermRef;
}

type DeriveState =
  | { kind: "idle" }
  | { kind: "deriving" }
  | { kind: "ready"; source: DeriveFormResult["source"] }
  | { kind: "unavailable"; message: string }
  | { kind: "error"; message: string };

function resourceKey(resource: FormResource): string {
  return termKey(resource);
}

export function FormsTool() {
  const {
    status,
    storeEpoch,
    snapshotStore,
    listFormResources,
    wasmDeriveForm,
  } = useEngine();
  const [selection, setSelection] = React.useState<DeriveSelection | null>(null);
  const [description, setDescription] = React.useState<FormDescription | null>(null);
  const [descriptionOrigin, setDescriptionOrigin] = React.useState<"workspace" | "manual" | null>(
    null,
  );
  const [derivation, setDerivation] = React.useState<{
    selection: DeriveSelection | null;
    storeEpoch: number;
    state: DeriveState;
  } | null>(null);
  const [manualJson, setManualJson] = React.useState("");
  const [manualError, setManualError] = React.useState<string | null>(null);
  const requestGeneration = React.useRef(0);

  const ready = status.kind === "ready";

  // Every content epoch (workspace hydration, import, or update) replaces the candidate list.
  const candidates = React.useMemo(() => {
    try {
      return {
        storeEpoch,
        resources: ready ? listFormResources() : [],
        snapshot: ready ? snapshotStore() : null,
        error: null,
      };
    } catch (cause) {
      return {
        storeEpoch, resources: [], snapshot: null,
        error: cause instanceof Error ? cause.message : String(cause),
      };
    }
  }, [ready, storeEpoch, listFormResources, snapshotStore]);
  const { resources, snapshot } = candidates;
  const [previousCandidates, setPreviousCandidates] = React.useState<typeof candidates | null>(null);
  if (previousCandidates !== candidates) {
    setPreviousCandidates(candidates);
    if (ready) {
      const retained = selection
        ? resources.find((candidate) => resourceKey(candidate) === resourceKey(selection.focus))
        : undefined;
      setSelection(resources.length === 0
        ? null
        : retained && selection
          ? { ...selection, focus: retained }
          : { focus: resources[0], mode: selection?.mode ?? "edit" });
      if (resources.length === 0 && !candidates.error && descriptionOrigin === "workspace") {
        setDescription(null);
        setDescriptionOrigin(null);
      }
    }
  }
  let deriveState: DeriveState = { kind: "idle" };
  if (candidates.error !== null) {
    deriveState = { kind: "error", message: candidates.error };
  } else if (ready && selection !== null) {
    deriveState = snapshot === null ? {
      kind: "error", message: "The active workspace could not be serialized for form derivation.",
    }
    : derivation?.selection === selection && derivation.storeEpoch === candidates.storeEpoch
      ? derivation.state : { kind: "deriving" };
  }

  // This is the only operational derivation path. The exact same N-Quads snapshot is the data
  // and shapes input on both hosts, so a store epoch, focus, mode, or shape change replaces the
  // entire validated description returned by Tauri/wasm.
  React.useEffect(() => {
    if (!ready || selection === null || snapshot === null) return;

    const generation = ++requestGeneration.current;
    let cancelled = false;
    void deriveForm(
      {
        data: snapshot,
        shapes: snapshot,
        focus: selection.focus,
        mode: selection.mode,
        shape: selection.shape,
        format: "nquads",
      },
      wasmDeriveForm,
    )
      .then((result) => {
        if (cancelled || generation !== requestGeneration.current) return;
        setDescription(result.description);
        setDescriptionOrigin("workspace");
        setManualError(null);
        setDerivation({ selection, storeEpoch, state: { kind: "ready", source: result.source } });
      })
      .catch((cause: unknown) => {
        if (cancelled || generation !== requestGeneration.current) return;
        if (cause instanceof FormsBridgeUnavailableError) {
          setDerivation({
            selection, storeEpoch, state: { kind: "unavailable", message: cause.message },
          });
        } else {
          setDerivation({
            selection, storeEpoch,
            state: {
              kind: "error",
              message: cause instanceof Error ? cause.message : String(cause),
            },
          });
        }
      });

    return () => {
      cancelled = true;
    };
  }, [ready, selection, snapshot, storeEpoch, wasmDeriveForm]);

  const loadManualJson = () => {
    try {
      const parsed = parseFormDescription(manualJson);
      requestGeneration.current += 1;
      setSelection(null);
      setDescription(parsed);
      setDescriptionOrigin("manual");
      setManualError(null);
      setDerivation({ selection: null, storeEpoch, state: { kind: "idle" } });
    } catch (cause) {
      setManualError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const chooseFocus = (key: string) => {
    const focus = resources.find((candidate) => resourceKey(candidate) === key);
    if (!focus) {
      setSelection(null);
      return;
    }
    setSelection({ focus, mode: description?.mode ?? selection?.mode ?? "edit" });
  };

  const onModeRequest = (mode: FormMode) => {
    if (descriptionOrigin !== "workspace" || selection === null) return;
    setSelection({ ...selection, mode, shape: description?.shape });
  };

  const onShapeRequest = (shape: TermRef) => {
    if (descriptionOrigin !== "workspace" || selection === null) return;
    setSelection({ ...selection, mode: description?.mode ?? selection.mode, shape });
  };

  return (
    <div className="flex h-full flex-col" data-tool-panel="forms">
      <div className="flex shrink-0 items-center gap-2 border-b bg-card px-3 py-1.5">
        <FilePenLine className="size-3.5 text-muted-foreground" aria-hidden />
        <span className="text-xs font-medium text-muted-foreground">Shape-directed form</span>
        <Badge variant="outline" className="h-5 text-[10px]">
          Active workspace
        </Badge>
        {deriveState.kind === "ready" ? (
          <Badge variant="muted" className="h-5 text-[10px]" data-form-derive-source>
            {deriveState.source === "desktop" ? "Tauri" : "WASM"}
          </Badge>
        ) : null}
        <Badge variant="muted" className="h-5 text-[10px]">
          Draft only · F4 commits
        </Badge>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-3">
        <div className="mx-auto max-w-5xl space-y-3">
          <section className="rounded-md border bg-card p-3" data-form-workspace-source>
            <div className="flex flex-wrap items-end gap-2">
              <label className="min-w-64 flex-1 text-[10px] font-medium text-muted-foreground">
                Focus resource
                <select
                  className="mt-1 h-8 w-full rounded-md border bg-background px-2 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  value={selection ? resourceKey(selection.focus) : ""}
                  onChange={(event) => chooseFocus(event.currentTarget.value)}
                  aria-label="Focus resource"
                  disabled={!ready || resources.length === 0}
                  data-form-focus-picker
                >
                  <option value="">
                    {ready ? "Select a workspace resource…" : "Waiting for the workspace…"}
                  </option>
                  {resources.map((resource) => (
                    <option key={resourceKey(resource)} value={resourceKey(resource)}>
                      {resource.kind === "bnode" ? `_:${resource.value}` : termLabel(resource)}
                    </option>
                  ))}
                </select>
              </label>
              <p className="pb-1 text-[10px] text-muted-foreground">
                Data + shapes: current workspace snapshot
              </p>
            </div>

            {deriveState.kind === "deriving" ? (
              <p className="mt-2 flex items-center gap-2 text-xs text-muted-foreground" role="status">
                <LoaderCircle className="size-3.5 animate-spin" aria-hidden />
                Deriving the complete form from the active workspace…
              </p>
            ) : null}
            {deriveState.kind === "unavailable" ? (
              <p
                className="mt-2 rounded-md border border-warning/30 bg-warning/5 p-2 text-xs text-muted-foreground"
                role="status"
                data-form-unavailable
              >
                {deriveState.message}
              </p>
            ) : null}
            {deriveState.kind === "error" ? (
              <p className="mt-2 text-xs text-destructive" role="alert" data-form-derive-error>
                {deriveState.message}
              </p>
            ) : null}
            {ready && resources.length === 0 ? (
              <p className="mt-2 text-xs text-muted-foreground" data-form-no-resources>
                The active workspace has no subject resources to use as a form focus.
              </p>
            ) : null}
          </section>

          <details className="rounded-md border bg-card" data-form-json-source>
            <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-xs font-medium [&::-webkit-details-marker]:hidden">
              <Braces className="size-3.5 text-muted-foreground" aria-hidden />
              Manual: load FormDescription JSON
              <span className="ml-auto text-[10px] font-normal text-muted-foreground">
                Available even when no derivation bridge is installed
              </span>
            </summary>
            <div className="space-y-2 border-t p-3">
              <textarea
                value={manualJson}
                onChange={(event) => setManualJson(event.currentTarget.value)}
                className="h-48 w-full resize-y rounded-md border bg-background p-2 font-mono text-[10px] outline-none focus-visible:ring-2 focus-visible:ring-ring"
                aria-label="FormDescription JSON"
                placeholder="Paste snake_case FormDescription JSON…"
                spellCheck={false}
              />
              <div className="flex items-center gap-2">
                <Button type="button" size="sm" onClick={loadManualJson} data-load-form-json>
                  Render JSON
                </Button>
                {manualError ? (
                  <p role="alert" className="text-xs text-destructive">
                    {manualError}
                  </p>
                ) : null}
              </div>
            </div>
          </details>

          {description ? (
            <FormRenderer
              description={description}
              onDescriptionChange={setDescription}
              onModeRequest={descriptionOrigin === "workspace" ? onModeRequest : undefined}
              onShapeRequest={descriptionOrigin === "workspace" ? onShapeRequest : undefined}
            />
          ) : null}
        </div>
      </div>
    </div>
  );
}
