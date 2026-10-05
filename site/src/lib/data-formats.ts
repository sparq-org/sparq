// [OPUS-4.8] sq-j50v — pure, framework-free helpers for the live /surface/data-formats
// demo: the four-format sample catalogue (including a TriG named-graph example) and the
// in-tab gzip round-trip (compress a UTF-8 string with the browser's native
// `CompressionStream("gzip")`, decode it back with `DecompressionStream("gzip")`).
//
// WHY THESE LIVE HERE (and are framework-free). The demo's parse step runs the real wasm
// Store (no mocks); but the *gzip ingest* path is plain Web-platform code: gzip is the one
// codec the browser decompresses natively (zstd reaches the LAZY-loaded decoder through
// `@sparq/client`; bzip2 is native-only — see the page's honest caveat), so
// the gzip round-trip demo is `gunzip(bytes)` -> `Store.load(text, fmt)`.
// Keeping the codec round-trip + the sample catalogue here (no React, no wasm) lets them
// unit-test directly under `node --test` (the Web Streams API is in Node ≥ 18).

/** The engine format string each picker entry loads through. */
export type DataFormat = "turtle" | "ntriples" | "nquads" | "trig" | "jsonld";

export interface FormatSample {
  /** Engine format string for `Store.load` / `Store.loadDataset`. */
  format: DataFormat;
  /** Human label shown in the picker. */
  label: string;
  /** Canonical file extension, for display. */
  ext: string;
  /** One-line description of the format. */
  blurb: string;
  /** A small, self-contained sample document in this exact format. */
  sample: string;
}

// N-Triples / N-Quads share the same four triples so the "same data, four syntaxes" point
// is visible; the quad formats add a named graph so the named-graph-preserving load path
// (loadDataset) is exercised. All four parse with the lean wasm bundle.

const TURTLE_SAMPLE = `@prefix ex:   <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .

ex:alice a foaf:Person ;
  foaf:name "Alice" ;
  foaf:knows ex:bob .

ex:bob a foaf:Person ;
  foaf:name "Bob"@en .
`;

const NTRIPLES_SAMPLE = `<http://example.org/alice> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://xmlns.com/foaf/0.1/Person> .
<http://example.org/alice> <http://xmlns.com/foaf/0.1/name> "Alice" .
<http://example.org/alice> <http://xmlns.com/foaf/0.1/knows> <http://example.org/bob> .
<http://example.org/bob> <http://xmlns.com/foaf/0.1/name> "Bob"@en .
`;

// N-Quads: the same triples, but two are placed in a named graph (4th term) — so a
// GRAPH ?g query against the loaded dataset returns them (loadDataset preserves graphs).
const NQUADS_SAMPLE = `<http://example.org/alice> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://xmlns.com/foaf/0.1/Person> .
<http://example.org/alice> <http://xmlns.com/foaf/0.1/name> "Alice" .
<http://example.org/alice> <http://xmlns.com/foaf/0.1/knows> <http://example.org/bob> <http://example.org/social> .
<http://example.org/bob> <http://xmlns.com/foaf/0.1/name> "Bob"@en <http://example.org/social> .
`;

// TriG: Turtle-with-named-graphs. The default graph holds the type assertions; the
// `ex:social` named graph holds the social links — the named-graph example the bead asks for.
const TRIG_SAMPLE = `@prefix ex:   <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .

ex:alice a foaf:Person .
ex:bob   a foaf:Person .

ex:social {
  ex:alice foaf:name "Alice" ;
    foaf:knows ex:bob .
  ex:bob foaf:name "Bob"@en .
}
`;

// [OPUS-4.8] sq-ixc3.1 — JSON-LD: the same Alice/Bob graph as a `@context`-abbreviated node
// array. Engine-side JSON-LD parsing is the site wasm bundle's opt-in `jsonld` feature
// (oxjsonld). Compact-IRI keys (`foaf:name`) resolve via the `@context`, `@type` is the rdf:type
// shorthand, and `@id` carries the node IRI — the linked-data structure the `@…`-keyword
// highlighting makes legible.
const JSONLD_SAMPLE = `{
  "@context": {
    "ex": "http://example.org/",
    "foaf": "http://xmlns.com/foaf/0.1/",
    "knows": { "@id": "foaf:knows", "@type": "@id" }
  },
  "@graph": [
    {
      "@id": "ex:alice",
      "@type": "foaf:Person",
      "foaf:name": "Alice",
      "knows": "ex:bob"
    },
    {
      "@id": "ex:bob",
      "@type": "foaf:Person",
      "foaf:name": { "@value": "Bob", "@language": "en" }
    }
  ]
}
`;

/** The five RDF formats, each with a live sample the picker seeds the textarea with. */
export const FORMAT_SAMPLES: FormatSample[] = [
  {
    format: "turtle",
    label: "Turtle",
    ext: ".ttl",
    blurb: "Prefixed, compact triple syntax — the most human-readable.",
    sample: TURTLE_SAMPLE,
  },
  {
    format: "ntriples",
    label: "N-Triples",
    ext: ".nt",
    blurb: "One fully-expanded triple per line — the line-parallel bulk format.",
    sample: NTRIPLES_SAMPLE,
  },
  {
    format: "nquads",
    label: "N-Quads",
    ext: ".nq",
    blurb: "N-Triples plus a 4th graph term — carries named graphs.",
    sample: NQUADS_SAMPLE,
  },
  {
    format: "trig",
    label: "TriG",
    ext: ".trig",
    blurb: "Turtle plus { … } named-graph blocks.",
    sample: TRIG_SAMPLE,
  },
  {
    format: "jsonld",
    label: "JSON-LD",
    ext: ".jsonld",
    blurb: "JSON for linked data — `@context`-mapped IRIs, `@id`, `@type`, `@graph`.",
    sample: JSONLD_SAMPLE,
  },
];

// [OPUS-4.8] sq-ixc3.1: JSON-LD can carry named graphs (`@graph` under an outer `@id`), so it
// joins nquads/trig as a dataset format routed through `loadDataset` (mirrors repl-dataset.ts).
/** Whether a format carries named graphs (so the demo must use `loadDataset`). */
const DATASET_FORMATS = new Set<DataFormat>(["nquads", "trig", "jsonld"]);

/** True for the quad-bearing formats (`nquads` / `trig`). */
export function isDatasetFormat(format: DataFormat): boolean {
  return DATASET_FORMATS.has(format);
}

/** The sample for a format, or `undefined` for an unknown format string. */
export function sampleFor(format: DataFormat): FormatSample | undefined {
  return FORMAT_SAMPLES.find((f) => f.format === format);
}

// ---------------------------------------------------------------------------
// gzip round-trip — the live compressed-ingest path (gzip is the one codec the
// browser decompresses natively; zstd is decoded by the shared lazy-loaded JS package
// path; bzip2 is native-only, see the page caveat).
// ---------------------------------------------------------------------------

// All byte buffers here are backed by a plain `ArrayBuffer` (never `SharedArrayBuffer`),
// so they satisfy `BlobPart` under TS 5.7's typed-array generics — hence the explicit
// `Uint8Array<ArrayBuffer>` annotation rather than the bare `Uint8Array` alias.

/** Drains a ReadableStream of Uint8Array chunks into one contiguous ArrayBuffer-backed buffer. */
async function collect(
  stream: ReadableStream<Uint8Array>,
): Promise<Uint8Array<ArrayBuffer>> {
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    total += value.byteLength;
  }
  const out = new Uint8Array(new ArrayBuffer(total));
  let offset = 0;
  for (const c of chunks) {
    out.set(c, offset);
    offset += c.byteLength;
  }
  return out;
}

/**
 * Gzip-compresses a UTF-8 string entirely in-tab via the native `CompressionStream`.
 * Used by the demo to MANUFACTURE a `.ttl.gz`-style payload from the sample text so the
 * decode side is a real gzip stream, not a canned blob.
 */
export async function gzipString(text: string): Promise<Uint8Array<ArrayBuffer>> {
  const cs = new CompressionStream("gzip");
  const stream = new Blob([text]).stream().pipeThrough(cs);
  return collect(stream as ReadableStream<Uint8Array>);
}

/**
 * Gunzips a gzip byte buffer back to a UTF-8 string via the native `DecompressionStream`.
 * This is the exact JS-side decode the compressed-ingest path performs before handing the
 * text to `Store.load` — gzip is decoded by the browser, no wasm/JS codec dependency.
 */
export async function gunzipToString(
  bytes: Uint8Array<ArrayBuffer>,
): Promise<string> {
  const ds = new DecompressionStream("gzip");
  const stream = new Blob([bytes]).stream().pipeThrough(ds);
  const out = await collect(stream as ReadableStream<Uint8Array>);
  return new TextDecoder().decode(out);
}

/** Formats a byte count compactly (B / KB) for the compression-ratio readout. */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  return `${(n / 1024).toFixed(1)} KB`;
}

/**
 * [SONNET-4.6] sq-su1oe (#820) — formats a byte count for unintrusive store-stats readouts
 * (B / KB / MB). {@link formatBytes} stops at KB (it serves the compression-ratio panel,
 * where inputs are small); an in-memory store footprint or an uploaded source file can reach
 * MB, so this carries the extra magnitude without over-precision. Negative or non-finite
 * inputs clamp to `0 B` so a bad reading never renders a fabricated figure.
 */
export function formatFootprintBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "0 B";
  if (n < 1024) return `${Math.round(n)} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}
