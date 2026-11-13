// Public snapshot of the API and Graph ingest ratchets at json-ld-api 8654ac2
// and json-ld-framing 3bf782b. Full-manifest floors include 1.0 compatibility
// entries; the separate EARL summary includes only applicable 1.1 entries.

export interface JsonLdConformanceLane {
  id: "toRdf" | "toRdf-ingest" | "expand" | "flatten" | "compact" | "frame" | "fromRdf" | "html" | "remote-doc";
  label: string;
  component: string;
  floor: number;
  total: number;
  failureCeiling?: number;
}

export const jsonLdEarlReport = {
  passed: 1295,
  failed: 1,
  total: 1296,
  toRdfPassed: 448,
  toRdfTotal: 448,
} as const;

export const jsonLdConformanceLanes = [
  { id: "toRdf", label: "Processor API: toRdf", component: "sparq-jsonld (1.1); oxjsonld (1.0-only compatibility)", floor: 454, total: 467 },
  { id: "toRdf-ingest", label: "Store ingest: toRdf", component: "sparq-core Graph over oxjsonld", floor: 429, total: 467, failureCeiling: 38 },
  { id: "expand", label: "Expansion", component: "sparq-jsonld", floor: 381, total: 385 },
  { id: "flatten", label: "Flattening", component: "sparq-jsonld", floor: 55, total: 58 },
  { id: "compact", label: "Compaction", component: "sparq-jsonld", floor: 244, total: 246 },
  { id: "frame", label: "Framing", component: "sparq-jsonld", floor: 92, total: 92 },
  { id: "fromRdf", label: "RDF to JSON-LD", component: "sparq-jsonld", floor: 52, total: 53 },
  { id: "html", label: "HTML / XHTML documents", component: "sparq-jsonld with html enabled", floor: 49, total: 50 },
  { id: "remote-doc", label: "Remote documents", component: "sparq-jsonld with manifest responses", floor: 18, total: 18 },
] as const satisfies readonly JsonLdConformanceLane[];
