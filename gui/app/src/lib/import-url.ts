// [OPUS-5] sq-ixc3.13 — the SHARED URL ingest step: fetch (+ decompress), auto-detect the RDF
// serialisation, load into the live store, and build the re-fetchable `WorkspaceSourceMeta` the
// workspace records.
//
// It has two call sites and they MUST agree, which is why it lives here rather than inline:
//   * the Import drawer's URL tab (the first fetch), and
//   * the left rail's Imports subgroup RE-FETCH action (pulling a recorded URL source again).
// A divergence between them would mean the same URL parsed as one format on import and another
// on refresh.
//
// Kept DOM-free and React-free so it is unit-testable (import-url.test.ts) with a stubbed fetch
// and a mock importer — no network, no WASM.

import { fetchRdfDocument, type FetchedRdfDocument } from "./file-decompress.js";
import { formatFromContentType, guessFormat, urlLabel } from "./rdf-format.js";

import type { ImportMode, ImportRequest, ImportResult } from "@/lib/engine-context";
import type { WorkspaceSourceMeta } from "@sparq/client";

/** How the document should land in the live store. */
export interface UrlImportOptions {
  /** REPLACE the store, or merge into it. */
  mode: ImportMode;
  /** Preserve named graphs (route quad-bearing formats through the dataset loader). */
  preserveGraphs: boolean;
}

/** The collaborators this step needs. Only `importRdf` is required in production. */
export interface UrlImportDeps {
  /** Bring the fetched body into the live store (engine-context's `importRdf`). */
  importRdf: (req: ImportRequest) => Promise<ImportResult>;
  /** Override the fetch+decompress step (tests stub it; production uses `fetchRdfDocument`). */
  fetchDocument?: (url: string) => Promise<FetchedRdfDocument>;
  /** Epoch-ms clock, injected so a test can pin `importedAt`. */
  now?: () => number;
}

/** What a completed URL import reports back to the drawer / rail. */
export interface UrlImportOutcome {
  /** The re-fetchable workspace source to record (carries the URL it was pulled from). */
  source: WorkspaceSourceMeta;
  /** Quads this import added. */
  added: number;
  /** Total quads in the live store afterwards. */
  storeSize: number;
}

/**
 * Fetch `url`, load it into the live store, and describe the source for the workspace.
 *
 * The format is AUTO-DETECTED from the served `Content-Type` when it names an RDF media type,
 * falling back to the archive's INNER document name (`data.nq` for a `bundle.zip` carrying it) —
 * never the outer container's extension. The recorded source reports the format and byte count
 * the engine actually parsed, not the guess.
 *
 * Throws on an empty URL, a failed fetch, or a parse failure (the caller surfaces the message).
 */
export async function importUrlDocument(
  url: string,
  options: UrlImportOptions,
  deps: UrlImportDeps,
): Promise<UrlImportOutcome> {
  const target = url.trim();
  if (target === "") throw new Error("Enter a URL to fetch.");

  const fetchDocument = deps.fetchDocument ?? fetchRdfDocument;
  const now = deps.now ?? (() => Date.now());

  const document = await fetchDocument(target);
  const format =
    formatFromContentType(document.contentType) ?? guessFormat(document.effectiveName);
  const label = urlLabel(target);

  const result = await deps.importRdf({
    kind: "url",
    mode: options.mode,
    preserveGraphs: options.preserveGraphs,
    label,
    format,
    text: document.text,
    url: target,
  });

  return {
    source: {
      kind: "url",
      label,
      url: target,
      format: result.format,
      bytes: result.bytes,
      importedAt: now(),
    },
    added: result.added,
    storeSize: result.storeSize,
  };
}
