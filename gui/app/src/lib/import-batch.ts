// (sq-810a0) [FABLE-5] — the batch-import mode sequencer shared by the Import drawer's WEB and
// NATIVE multi-file loops (import-drawer.tsx).
//
// THE BUG THIS FIXES (GPT-5.6 GUI review, adversarially verified): the loops decided the
// per-file import mode by ARRAY INDEX — `i === 0 ? mode : "add"` — so the selected `replace`
// was pinned to file[0]. If file[0] FAILED to parse (its `importRdf` throws before any store
// mutation — engine-context.tsx decodes at step 1, mutates only after a successful decode) but a
// later file SUCCEEDED, the `replace` was silently never applied: the old dataset survived and
// the later file was merged in as `add`, contradicting the user's replace intent.
//
// THE FIX: apply the selected mode to the FIRST file that actually imports SUCCESSFULLY, and
// `add` for every file after that first success. A `replace` is therefore only ever "spent" on a
// file that really cleared+loaded the store; if the first file(s) fail, the `replace` carries
// forward to the first one that works. Modelled here as a pure, DOM-free sequencer so it is
// unit-testable (import-batch.test.ts) with a mock importer and no React/WASM.

import type { ImportMode } from "@/lib/engine-context";

/** Per-item outcome of a batch import. Mirrors the drawer's `FileItemStatus`. */
export type BatchItemStatus =
  | { kind: "ok"; added: number }
  | { kind: "error"; message: string };

/** The result of importing a single item under a resolved mode. */
export interface BatchImportResult {
  added: number;
}

/**
 * The resolved per-file mode for the file at batch position `index`, given whether a `replace`
 * has ALREADY been applied to an earlier, successful file.
 *
 * - selected `add`      → always `add` (nothing to sequence).
 * - selected `replace`  → `replace` until (and including) the first SUCCESSFUL import, then `add`.
 *
 * `replacedAlready` MUST reflect a *successful* replace only — see {@link runImportBatch}. A
 * failed file does not consume the replace, so the next file inherits it.
 */
export function nextFileMode(selected: ImportMode, replacedAlready: boolean): ImportMode {
  return !replacedAlready && selected === "replace" ? "replace" : "add";
}

/**
 * Summary flags describing what the batch actually did — used to build honest completion
 * feedback, so a user who asked to REPLACE is told when the replace was NOT applied.
 */
export interface BatchSummary {
  okCount: number;
  errCount: number;
  totalAdded: number;
  /** The user selected `replace`. */
  replaceRequested: boolean;
  /** A `replace` was actually applied to some file (i.e. some file succeeded under replace). */
  replaceApplied: boolean;
}

/**
 * Sequence a batch import with the correct first-successful-replace semantics.
 *
 * `importOne(item, mode)` performs the real import (decode + store mutation) for one item under
 * the resolved `mode`; it must REJECT on a per-file failure without mutating the store — exactly
 * how `importRdf`/`runImport` behave (a parse error throws at decode, before any store write).
 *
 * `onStatus(key, status)` is called after each item settles so the drawer can light up each row
 * incrementally. `keyOf` maps an item to its status-map key (filename / path).
 *
 * Returns the per-item status map plus a {@link BatchSummary}. Guarantees:
 *  - the selected mode is applied to the FIRST successful import, `add` thereafter;
 *  - a failing item never consumes the `replace` — it carries to the next item;
 *  - a single failure NEVER aborts the batch (invariant from sq-eydh9).
 */
export async function runImportBatch<T>(
  items: readonly T[],
  selected: ImportMode,
  keyOf: (item: T) => string,
  importOne: (item: T, mode: ImportMode) => Promise<BatchImportResult>,
  onStatus?: (key: string, status: BatchItemStatus, statuses: Record<string, BatchItemStatus>) => void,
): Promise<{ statuses: Record<string, BatchItemStatus>; summary: BatchSummary }> {
  const statuses: Record<string, BatchItemStatus> = {};
  let replaced = false;
  let totalAdded = 0;

  for (const item of items) {
    const key = keyOf(item);
    // Resolve the mode from the SUCCESS-tracked `replaced` flag, not the array index.
    const fileMode = nextFileMode(selected, replaced);
    try {
      const result = await importOne(item, fileMode);
      statuses[key] = { kind: "ok", added: result.added };
      totalAdded += result.added;
      // Only a SUCCESSFUL replace consumes the replace; everything after is `add`.
      if (fileMode === "replace") replaced = true;
    } catch (err) {
      // A single-file failure MUST NOT abort the batch (invariant from sq-eydh9); the replace is
      // NOT consumed, so it carries forward to the next file.
      statuses[key] = {
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      };
    }
    onStatus?.(key, statuses[key], statuses);
  }

  const okCount = Object.values(statuses).filter((s) => s.kind === "ok").length;
  const errCount = Object.values(statuses).filter((s) => s.kind === "error").length;

  return {
    statuses,
    summary: {
      okCount,
      errCount,
      totalAdded,
      replaceRequested: selected === "replace",
      replaceApplied: replaced,
    },
  };
}

/** One successfully imported document of a batch, as recorded in workspace history. */
export interface ImportedDocument {
  label: string;
  /** The serialisation the document was parsed as (`turtle`, `nquads`, ...). */
  format: string;
  /** Byte length of the imported document (`ImportResult.bytes`). */
  bytes: number;
}

/**
 * The single workspace-history entry for a web multi-file batch (#6257): the successful
 * documents' REAL byte total, their shared format (or `mixed` when they differ), and a label that
 * is the file's own name for a one-file batch, else the count of files actually imported.
 * Returns `null` when nothing imported.
 */
export function summariseImportedBatch(
  docs: readonly ImportedDocument[],
): ImportedDocument | null {
  if (docs.length === 0) return null;
  if (docs.length === 1) return { ...docs[0] };
  const formats = new Set(docs.map((d) => d.format));
  return {
    label: `${docs.length} files`,
    format: formats.size === 1 ? docs[0].format : "mixed",
    bytes: docs.reduce((sum, d) => sum + d.bytes, 0),
  };
}
