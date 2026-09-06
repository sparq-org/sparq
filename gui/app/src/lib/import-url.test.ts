// [OPUS-5] sq-ixc3.13 — unit tests for the shared URL ingest step (import-url.ts), the one path
// behind BOTH the Import drawer's URL tab and the left rail's Imports re-fetch action.
//
// THE PROPERTIES UNDER TEST:
//   1. Format AUTO-DETECT precedence — the served RDF `Content-Type` wins; otherwise the
//      archive's INNER document name decides (never the outer `.gz`/`.zip` container).
//   2. The load options the user chose (`mode`, `preserveGraphs`) and the source URL reach
//      `importRdf` unchanged — `preserveGraphs` is the named-graph-preserving contract.
//   3. The recorded `WorkspaceSourceMeta` is RE-FETCHABLE (carries the URL) and reports what the
//      ENGINE parsed (`result.format` / `result.bytes`), not the pre-parse guess.
//
// The fetch step is stubbed, so these run with no network and no WASM.
//
// Run via:   npm run test:unit   (gui/app)
import { test } from "node:test";
import assert from "node:assert/strict";

import { importUrlDocument } from "./import-url.js";
import type { FetchedRdfDocument } from "./file-decompress.js";
import type { ImportRequest, ImportResult } from "@/lib/engine-context";

/** A stub fetch result. `contentType: null` models a server that sent no usable media type. */
function fetched(
  text: string,
  contentType: string | null,
  effectiveName: string,
): FetchedRdfDocument {
  return { text, contentType, effectiveName, wasDecompressed: false };
}

/** Records the request it was handed and reports a fixed engine outcome. */
function recordingImporter(result: Partial<ImportResult> = {}) {
  const seen: ImportRequest[] = [];
  const importRdf = async (req: ImportRequest): Promise<ImportResult> => {
    seen.push(req);
    return {
      added: 3,
      storeSize: 7,
      loadedNatively: false,
      format: "nquads",
      bytes: 4096,
      ...result,
    };
  };
  return { seen, importRdf };
}

test("importUrlDocument: an RDF Content-Type decides the format", async () => {
  const { seen, importRdf } = recordingImporter();
  await importUrlDocument(
    "https://example.org/data",
    { mode: "add", preserveGraphs: true },
    {
      importRdf,
      // The name says Turtle, the server says TriG — the server wins.
      fetchDocument: async () => fetched("<a> <b> <c> .", "application/trig", "data.ttl"),
    },
  );
  assert.equal(seen[0].format, "trig");
});

test("importUrlDocument: with no usable Content-Type the INNER archive name decides", async () => {
  const { seen, importRdf } = recordingImporter();
  await importUrlDocument(
    "https://example.org/bundle.zip",
    { mode: "add", preserveGraphs: true },
    {
      importRdf,
      // Outer container is `.zip`; the decompressor reported the member as `data.nq`.
      fetchDocument: async () => fetched("<a> <b> <c> <g> .", "application/zip", "data.nq"),
    },
  );
  assert.equal(seen[0].format, "nquads", "the zip member, not the .zip container, sets the format");
});

test("importUrlDocument: mode, preserveGraphs and the URL reach the importer unchanged", async () => {
  const { seen, importRdf } = recordingImporter();
  await importUrlDocument(
    "  https://example.org/data.ttl  ",
    { mode: "replace", preserveGraphs: false },
    { importRdf, fetchDocument: async () => fetched("body", null, "data.ttl") },
  );
  const req = seen[0];
  assert.equal(req.kind, "url");
  assert.equal(req.mode, "replace");
  assert.equal(req.preserveGraphs, false);
  assert.equal(req.text, "body");
  // The URL is trimmed before it is fetched AND before it is recorded.
  assert.equal(req.url, "https://example.org/data.ttl");
});

test("importUrlDocument: the recorded source is re-fetchable and reports the ENGINE's result", async () => {
  const { importRdf } = recordingImporter({ format: "trig", bytes: 512, added: 9, storeSize: 20 });
  const outcome = await importUrlDocument(
    "https://example.org/data.ttl",
    { mode: "add", preserveGraphs: true },
    { importRdf, fetchDocument: async () => fetched("body", null, "data.ttl"), now: () => 1234 },
  );

  assert.equal(outcome.source.kind, "url");
  assert.equal(outcome.source.url, "https://example.org/data.ttl", "re-fetchable: carries the URL");
  // What the engine ACTUALLY parsed, not the `guessFormat("data.ttl") === "turtle"` guess.
  assert.equal(outcome.source.format, "trig");
  assert.equal(outcome.source.bytes, 512);
  assert.equal(outcome.source.importedAt, 1234);
  assert.equal(outcome.added, 9);
  assert.equal(outcome.storeSize, 20);
});

test("importUrlDocument: an empty URL is rejected before any fetch", async () => {
  let fetches = 0;
  const { importRdf } = recordingImporter();
  await assert.rejects(
    () =>
      importUrlDocument(
        "   ",
        { mode: "add", preserveGraphs: true },
        {
          importRdf,
          fetchDocument: async () => {
            fetches += 1;
            return fetched("body", null, "x.ttl");
          },
        },
      ),
    /Enter a URL/,
  );
  assert.equal(fetches, 0);
});

test("importUrlDocument: a fetch failure propagates and nothing is imported", async () => {
  const { seen, importRdf } = recordingImporter();
  await assert.rejects(
    () =>
      importUrlDocument(
        "https://example.org/gone.ttl",
        { mode: "add", preserveGraphs: true },
        {
          importRdf,
          fetchDocument: async () => {
            throw new Error("Fetch failed: HTTP 404 Not Found");
          },
        },
      ),
    /HTTP 404/,
  );
  assert.equal(seen.length, 0, "a failed fetch must not touch the store");
});
