// [GPT-5.6] sq-3eukz — runtime-selected adapters for deriving the Forms tool from the live
// workspace. Desktop uses the Tauri `derive_form` command; hosted web uses the optional
// sparq-wasm `Store.deriveForm` method. The adapter never reconstructs FormDescription content.

import { nativeDeriveForm, hasNativeForms } from "./tauri-ipc.js";
import {
  parseFormDescription,
  type FormDescription,
  type FormMode,
  type TermRef,
} from "./form-description.js";

/** The positional sparq-wasm `Store.deriveForm` contract, when that optional method exists. */
export type WasmDeriveForm = (
  data: string,
  shapes: string,
  focus: string,
  format: string,
  optionsJson: string,
) => string | null;

/** Everything a host needs to derive one whole FormDescription. */
export interface DeriveFormRequest {
  data: string;
  shapes: string;
  focus: TermRef;
  mode: FormMode;
  shape?: TermRef;
  format?: string;
}

export interface DeriveFormResult {
  description: FormDescription;
  source: "desktop" | "web";
}

/** Serialize only when a ready workspace has a selected focus to derive. */
export function formWorkspaceSnapshot(
  ready: boolean,
  selection: object | null,
  snapshotStore: () => string | null,
): { snapshot: string | null; error: string | null } {
  if (!ready || selection === null) return { snapshot: null, error: null };
  try {
    return { snapshot: snapshotStore(), error: null };
  } catch (cause) {
    return { snapshot: null, error: cause instanceof Error ? cause.message : String(cause) };
  }
}

/** Injectable adapter dependencies keep host selection testable before either bridge lands. */
export interface FormHostAdapters {
  desktopAvailable: boolean;
  desktopDerive: typeof nativeDeriveForm;
  webDerive: WasmDeriveForm;
}

/** Stable unavailable error so the tool can render a dedicated, explicit state. */
export class FormsBridgeUnavailableError extends Error {
  constructor() {
    super(
      "Form derivation is unavailable in this build. Load FormDescription JSON manually below.",
    );
    this.name = "FormsBridgeUnavailableError";
  }
}

function nodeString(term: TermRef): string {
  return term.kind === "bnode" ? `_:${term.value}` : term.value;
}

/** The exact snake_case options JSON consumed by sparq-wasm `deriveForm`. */
export function formOptionsJson(req: DeriveFormRequest): string {
  return JSON.stringify({
    mode: req.mode,
    ...(req.shape ? { shape: nodeString(req.shape) } : {}),
  });
}

/**
 * Route one request to exactly one runtime host and validate its snake_case response. Desktop
 * never silently falls through to wasm, and web never falls back to demo data.
 */
export async function deriveFormWith(
  req: DeriveFormRequest,
  adapters: FormHostAdapters,
): Promise<DeriveFormResult> {
  const format = req.format ?? "nquads";
  const focus = nodeString(req.focus);

  if (adapters.desktopAvailable) {
    const json = await adapters.desktopDerive({
      dataset: req.data,
      shapes: req.shapes,
      format,
      focus,
      mode: req.mode,
      shape: req.shape ? nodeString(req.shape) : undefined,
    });
    if (json === null) throw new FormsBridgeUnavailableError();
    return { description: parseFormDescription(json), source: "desktop" };
  }

  const json = await Promise.resolve(
    adapters.webDerive(req.data, req.shapes, focus, format, formOptionsJson(req)),
  );
  if (json === null) throw new FormsBridgeUnavailableError();
  return { description: parseFormDescription(json), source: "web" };
}

/** Select the runtime host and derive the complete replacement description. */
export async function deriveForm(
  req: DeriveFormRequest,
  webDerive: WasmDeriveForm,
): Promise<DeriveFormResult> {
  return deriveFormWith(req, {
    desktopAvailable: hasNativeForms(),
    desktopDerive: nativeDeriveForm,
    webDerive,
  });
}
