// [OPUS-4.8] sq-8thu — the canonical site IA, derived from the feature-showcase
// design doc (research/feature-showcase-site-design.md §2). One source for the
// sidebar nav AND the landing surface grid. `tier` drives the honesty badge.
//
// [OPUS-4.8] sq-vw3ax.2 — re-grouped ONCE here into the 5 capability THEMES from the
// website-redesign record (research/website-redesign.md §2). Every consumer — the
// sidebar nav, the Home theme grid, the future /capabilities gallery, and the Cmd-K
// command palette — derives from this single GROUPS source, so one edit restructures
// all of them. This commit is a STRUCTURAL regroup only: no surface, blurb, tier, or
// route was added, removed, or reworded — only their grouping (and per-theme metadata)
// changed.

import type { LucideIcon } from "lucide-react";
import {
  Database,
  FileCode2,
  Boxes,
  Brain,
  ShieldCheck,
  Search,
  Sparkles,
  Binary,
  MapPin,
  Radio,
  Lock,
  Network,
  Server,
  Terminal,
  Code2,
  Info,
  Waypoints,
  FileLock2,
} from "lucide-react";

/** Live-execution tier — drives the honesty badge colour + label. */
export type Tier =
  | "live" // (a) shipped wasm, in your tab today
  | "live-new-wasm" // (b) a new wasm bundle (portability spike first)
  | "live-bbjs" // (c) 3rd-party WASM (bb.js UltraHonk proving)
  | "live-sim" // (d) faithful in-tab JS simulation
  | "hosted" // (e) hosted sparq-server
  | "native" // built, opt-in native Rust crate; linked from the static site
  | "walkthrough" // (e) captured-I/O replay (different host / native-only)
  | "soon"; // not yet built — honest placeholder

// [GPT-5.6] sq-vw3ax.15 — compact native-only rows carry their proof-of-existence inline:
// one copyable API line plus direct crate and Agent Skill links. Keeping this metadata beside
// the surface makes /capabilities, Home, and Cmd-K update from the same IA source.
export interface NativeSurfaceMeta {
  snippet: string;
  readme: string;
  skill: string;
}

export interface Surface {
  slug: string;
  href: string;
  title: string;
  blurb: string;
  tier: Tier;
  icon: LucideIcon;
  built?: boolean; // does a real page exist yet?
  native?: NativeSurfaceMeta;
}

export interface SurfaceGroup {
  /** Stable anchor id — `/capabilities#<id>` + the Home theme grid + Cmd-K group key. */
  id: string;
  label: string;
  /** One-line theme description (the capability the surfaces under it deliver). */
  description: string;
  surfaces: Surface[];
}

export const TIER_LABEL: Record<Tier, string> = {
  live: "Live in your tab",
  "live-new-wasm": "Live (new wasm)",
  "live-bbjs": "Live via bb.js",
  "live-sim": "Live simulation",
  hosted: "Hosted",
  native: "Native crate",
  walkthrough: "Walkthrough",
  soon: "Coming soon",
};

export const TIER_VARIANT: Record<
  Tier,
  "success" | "warning" | "muted" | "default"
> = {
  live: "success",
  "live-new-wasm": "success",
  "live-bbjs": "success",
  "live-sim": "default",
  hosted: "warning",
  native: "muted",
  walkthrough: "muted",
  soon: "muted",
};

export const FLAGSHIPS: Surface[] = [
  {
    slug: "zk-car-hire",
    href: "/showcase/zk-car-hire",
    title: "ZK cross-credential car-hire",
    blurb:
      "Prove you may hire a car — age ≥ 25, valid non-revoked licence, same holder across two credentials — without revealing your documents.",
    tier: "live-bbjs",
    icon: ShieldCheck,
    built: true,
  },
  {
    slug: "mpc-100k",
    href: "/showcase/mpc-100k",
    title: "MPC £100k secure threshold",
    blurb:
      "Four flatmates learn only whether their combined income clears a £100k threshold — no salary, and not even the exact total, is revealed.",
    tier: "live-sim",
    icon: Lock,
    built: true,
  },
  {
    slug: "solid-pairs",
    href: "/showcase/solid-pairs",
    title: "Solid (user, app)-pair result sets",
    blurb:
      "One Pod, one query — different result sets per (agent, client) pair, enforced by the engine's FROM NAMED dataset restriction, live in your tab.",
    tier: "live",
    icon: Network,
    built: true,
  },
];

// [OPUS-4.8] sq-vw3ax.2 — the 5 capability THEMES (research/website-redesign.md §2).
// Surfaces are mapped to themes by what they DO, keeping every existing surface, route,
// blurb and tier verbatim:
//   1. Query & data      — SPARQL + formats/JS-WASM/geo + native analytics/canon/Arrow
//   2. Reason & validate — inference + SHACL + native PROV-O lineage
//   3. Search & GenAI    — full-text BM25, vector, GenAI/NLQ
//   4. Privacy (ZK / MPC)— ZK query proofs + MPC federation (the Solid flagship sits in
//      /showcase; it is surfaced via FLAGSHIPS, not duplicated as a /surface row here)
//   5. Serve & embed     — HTTP/MCP servers, CLI, Python, streaming RSP-QL
// The redesign's aspirational extra rows became beads, not fabricated nav entries: the
// federation row landed via sq-vw3ax.13 (a captured-output demo row under Serve & embed);
// structural-similarity remains unbuilt and is still NOT invented here.
export const GROUPS: SurfaceGroup[] = [
  {
    id: "query-data",
    label: "Query & data",
    // [OPUS-4.8] sq-4hiqe — the in-tab workbench now lives at /app (the /try REPL was removed);
    // this description is user-facing copy, so it stays clean (no route/bead cross-reference).
    description:
      "Query, exchange, and analyse RDF — SPARQL 1.1/1.2, data formats, canonicalization, and graph algorithms.",
    surfaces: [
      {
        slug: "sparql",
        href: "/surface/sparql",
        title: "SPARQL 1.1 / 1.2",
        blurb: "SELECT / ASK / CONSTRUCT / UPDATE, property paths, RDF 1.2 triple terms.",
        tier: "live",
        icon: FileCode2,
        built: true,
      },
      {
        slug: "data-formats",
        href: "/surface/data-formats",
        title: "Data formats",
        blurb: "Turtle / N-Triples / N-Quads / TriG + compressed ingest.",
        tier: "live",
        icon: Database,
        built: true,
      },
      {
        slug: "javascript-wasm",
        href: "/surface/javascript-wasm",
        title: "JavaScript / WASM",
        blurb: "The @sparq-org/sparq browser & Node API — streaming cursors, match, applyDelta.",
        tier: "live",
        icon: Boxes,
        built: true,
      },
      {
        slug: "geosparql",
        href: "/surface/geosparql",
        title: "GeoSPARQL",
        blurb: "geof: functions + sf*/eh*/rcc8* + R-tree GeoIndex with a map overlay.",
        // [OPUS-4.8] sq-ndaz: tier-e. sparq-geo is an opt-in native crate (the core engine
        // + lean wasm bundle carry zero geometry code; the server exposes geof: only behind
        // its non-default `geo` feature), and the static Pages site has no backend — so the
        // honest surface is a captured-output walkthrough (the real London–Paris distance
        // < 400 km query, within-polygon spatial join, topology-property rewrite, and R-tree
        // GeoIndex metres, all answer-exact), not a live hosted endpoint.
        tier: "walkthrough",
        icon: MapPin,
        built: true,
      },
      // [GPT-5.6] sq-vw3ax.15 — shipped native capabilities stay compact: no dense surface
      // page, only an honest native tier, one API line, and direct code/docs links.
      {
        slug: "graph-analytics",
        href: "https://github.com/sparq-org/sparq/tree/main/crates/sparq-algos",
        title: "Graph analytics",
        blurb:
          "PageRank, centrality, communities, SCCs, and topological order over a directed RDF projection.",
        tier: "native",
        icon: Network,
        native: {
          snippet: "let ranks = sparq_algos::pagerank(&graph, Default::default());",
          readme: "https://github.com/sparq-org/sparq/tree/main/crates/sparq-algos",
          skill: "https://github.com/sparq-org/sparq/blob/main/skills/graph-analytics/SKILL.md",
        },
      },
      {
        slug: "rdf-canon",
        href: "https://github.com/sparq-org/sparq/tree/main/crates/sparq-canon",
        title: "RDFC-1.0 canonicalization",
        blurb:
          "Canonical blank-node labelling for stable N-Quads, hashing, signing, and graph isomorphism.",
        tier: "native",
        icon: Binary,
        native: {
          snippet: "let canonical = sparq_canon::canonicalize(&quads)?;",
          readme: "https://github.com/sparq-org/sparq/tree/main/crates/sparq-canon",
          skill: "https://github.com/sparq-org/sparq/blob/main/skills/rdf-canon/SKILL.md",
        },
      },
      {
        slug: "arrow-columnar",
        href: "https://github.com/sparq-org/sparq/tree/main/crates/sparq-arrow",
        title: "Arrow export",
        blurb:
          "Faithful SPARQL SELECT interchange through Arrow RecordBatch, Parquet, and Arrow IPC.",
        tier: "native",
        icon: Database,
        native: {
          snippet: "let batch = sparq_arrow::to_record_batch(&result)?;",
          readme: "https://github.com/sparq-org/sparq/tree/main/crates/sparq-arrow",
          skill: "https://github.com/sparq-org/sparq/blob/main/skills/arrow-columnar/SKILL.md",
        },
      },
    ],
  },
  {
    id: "reason-validate",
    label: "Reason & validate",
    description:
      "Derive, trace, validate, and govern RDF — RDFS / OWL 2 RL / N3 closure, PROV-O lineage, SHACL, and ODRL usage-control policy decisions.",
    surfaces: [
      {
        slug: "inference",
        href: "/surface/inference",
        title: "Inference",
        blurb: "RDFS / OWL 2 RL / N3 closure + proof trees.",
        tier: "live-new-wasm",
        icon: Brain,
        built: true,
      },
      {
        slug: "shacl",
        href: "/surface/shacl",
        title: "SHACL",
        blurb: "SHACL Core + SHACL-SPARQL → W3C validation report.",
        tier: "live",
        icon: ShieldCheck,
        built: true,
      },
      {
        // [GPT-5.6] sq-vw3ax.15 — PROV-O is a built opt-in crate, not an in-tab demo.
        slug: "prov-lineage",
        href: "https://github.com/sparq-org/sparq/tree/main/crates/sparq-prov",
        title: "PROV-O lineage",
        blurb:
          "PROV-O derivations for CONSTRUCT, DESCRIBE, UPDATE, and optional reasoner proof trees.",
        tier: "native",
        icon: Waypoints,
        native: {
          snippet: "let derivation = sparq_prov::derive_construct(&graph, query, config)?;",
          readme: "https://github.com/sparq-org/sparq/tree/main/crates/sparq-prov",
          skill: "https://github.com/sparq-org/sparq/blob/main/skills/prov-lineage/SKILL.md",
        },
      },
      {
        slug: "policy",
        href: "/surface/policy",
        title: "ODRL usage control",
        blurb:
          "W3C ODRL 2.2 permission / prohibition / duty → fail-closed Permit / Deny, with purpose / recipient / dateTime / count constraints and deny-overrides.",
        // [OPUS-4.8] sq-vw3ax.14: walkthrough tier. sparq-policy is an OPT-IN native crate
        // (publish = false, dependency-of-nothing; `evaluate` is pure Rust, no I/O) that the
        // lean wasm bundles never carry, and the static Pages site has no backend — so the
        // honest surface REPLAYS the real evaluate() decision for each (policy, request) pair
        // (each verdict pinned by a named crate test; src/lib/policy.ts), not a live in-tab run.
        // A live evaluator would need a dedicated sparq-policy wasm bundle (a separate spike).
        tier: "walkthrough",
        icon: FileLock2,
        built: true,
      },
    ],
  },
  {
    id: "search-genai",
    label: "Search & GenAI",
    description:
      "Find and generate over RDF — BM25 full-text, vector k-NN, and a natural-language → SPARQL loop.",
    surfaces: [
      {
        slug: "full-text",
        href: "/surface/full-text",
        title: "Full-text",
        blurb: "BM25 text search via magic predicates.",
        tier: "live-new-wasm",
        icon: Search,
        built: true,
      },
      {
        slug: "vector",
        href: "/surface/vector",
        title: "Vector",
        blurb: "Embedding store + cosine top-k (HNSW / DiskANN) + k-NN inside SPARQL.",
        // [OPUS-4.8] sq-dwdm: tier-e. sparq-vectors is an opt-in native crate (nothing in
        // the workspace or the lean wasm bundle depends on it, and the vec: magic predicate
        // sits behind the non-default vec-predicate feature), and the static Pages site has
        // no backend — so the honest surface is a captured-output walkthrough (the real
        // Usain Bolt label-embedding run + real vec:nearest / vec:search result tables,
        // answer-exact backend), not a live hosted endpoint.
        tier: "walkthrough",
        icon: Binary,
        built: true,
      },
      {
        slug: "genai",
        href: "/surface/genai",
        title: "GenAI / NLQ",
        blurb: "Schema-card / VoID introspection + natural-language → SPARQL loop.",
        // [OPUS-4.8] sq-3was: tier-e. sparq-nlq is an opt-in native crate (it can pull a
        // model behind a trait), not in the lean wasm bundle, and the static Pages site
        // has no backend — so the honest surface is a captured-output walkthrough (real
        // schema card + real executed result tables, scripted-fixture model step), not a
        // live hosted endpoint.
        tier: "walkthrough",
        icon: Sparkles,
        built: true,
      },
    ],
  },
  {
    id: "privacy",
    label: "Privacy (ZK / MPC)",
    description:
      "Answer queries without revealing the data — zero-knowledge query proofs and threshold MPC federation. Research-grade: the v1 verifier is not externally audited.",
    surfaces: [
      {
        slug: "zk",
        href: "/surface/zk",
        title: "ZK query proofs",
        blurb: "Commitments, BGP + FILTER, issuer attestation, revocation.",
        tier: "live-bbjs",
        icon: Lock,
        built: true,
      },
      {
        slug: "mpc",
        href: "/surface/mpc",
        title: "MPC federation",
        blurb: "Federated SPARQL across distrusting holders (Shamir, threshold).",
        tier: "live-sim",
        icon: Network,
        built: true,
      },
    ],
  },
  {
    id: "serve-embed",
    label: "Serve & embed",
    description:
      "Run sparq as a service or embed it — HTTP, MCP, CLI, Python, and streaming RSP-QL.",
    surfaces: [
      {
        slug: "http-server",
        href: "/surface/http-server",
        title: "HTTP server",
        blurb: "SPARQL 1.1 Protocol endpoint, GSP, /metrics, WS/SSE subscriptions.",
        // [OPUS-4.8] sq-rnwc: tier-e. No backend behind the static Pages site, so the
        // honest surface is a captured curl + SSE-frame walkthrough (incl. a live
        // subscription firing on a committed UPDATE), not a live hosted endpoint.
        tier: "walkthrough",
        icon: Server,
        built: true,
      },
      {
        slug: "streaming-rsp",
        href: "/surface/streaming-rsp",
        title: "Streaming RSP",
        blurb: "RSP-QL windows (sliding / tumbling, R/I/DSTREAM).",
        tier: "live-new-wasm",
        icon: Radio,
        built: true,
      },
      {
        slug: "federation",
        href: "/surface/federation",
        title: "Federation",
        blurb:
          "SERVICE + multi-source federation — cost-based source selection (HiBISCuS/CostFed), brTPF/TPF streaming client, SSRF-guarded transport.",
        // [FABLE-5] sq-vw3ax.13: walkthrough tier. All three federation crates are
        // opt-in NATIVE code (sparq-engine-service's SERVICE transport sits behind the
        // engine's non-default `service` feature and its HTTP stack is compiled out on
        // wasm32; sparq-fedclient and sparq-fedplan are standalone opt-in members no
        // wasm bundle depends on), and the static Pages site has no backend — so the
        // honest surface replays REAL captured sparq-fedplan planner output (the
        // committed capture harness's verbatim selection + plans), not a live run.
        tier: "walkthrough",
        icon: Waypoints,
        built: true,
      },
      {
        // [GPT-5.6] sq-vw3ax.15 — the MCP front door is native and read-only by default.
        slug: "mcp-server",
        href: "https://github.com/sparq-org/sparq/tree/main/crates/sparq-mcp",
        title: "MCP server",
        blurb:
          "Read-only-by-default MCP tools for SPARQL, schema introspection, stats, prefixes, and VoID.",
        tier: "native",
        icon: Server,
        native: {
          snippet: "let mut server = sparq_mcp::McpServer::new(graph);",
          readme: "https://github.com/sparq-org/sparq/tree/main/crates/sparq-mcp",
          skill: "https://github.com/sparq-org/sparq/blob/main/skills/agent-tools/SKILL.md",
        },
      },
      {
        slug: "cli",
        href: "/surface/cli",
        title: "CLI",
        blurb: "sparq-cli — query / reason / build / query-mmap.",
        tier: "walkthrough",
        icon: Terminal,
      },
      {
        slug: "python",
        href: "/surface/python",
        title: "Python",
        blurb: "sparq pyo3 bindings.",
        tier: "walkthrough",
        icon: Code2,
      },
    ],
  },
];

// [OPUS-4.8] sq-vw3ax.2 — /about is a UTILITY destination (the "what runs where" matrix),
// not a capability theme, so it is kept OUT of GROUPS (which is now strictly the 5 themes)
// and exported separately. Consumers that previously walked GROUPS for the About entry
// (the sidebar, the About-page table) read this instead, so no route is lost.
export const ABOUT_SURFACE: Surface = {
  slug: "about",
  href: "/about",
  title: "About",
  blurb: 'Architecture and the honest "what runs where" matrix.',
  tier: "live",
  icon: Info,
  built: true,
};

/** Every capability surface across the 5 themes (excludes the /about utility page). */
export const ALL_SURFACES: Surface[] = GROUPS.flatMap((g) => g.surfaces);

/** Capability surfaces + the /about utility page — the complete navigable surface set. */
export const ALL_SURFACES_WITH_ABOUT: Surface[] = [...ALL_SURFACES, ABOUT_SURFACE];

/** [GPT-5] issue #6144 — crates.io inventory mapped to the capability catalogue.
 *
 * Support crates intentionally share their user-facing capability instead of acquiring a
 * duplicate marketing page. Keep this exhaustive for workspace packages whose Cargo manifest
 * does not set `publish = false`. scripts/tests/test_published_crate_catalogue.py validates
 * both this mapping and the mdBook catalogue against the manifests and known surface slugs.
 */
export const PUBLISHED_CRATE_SURFACE = {
  "sparq-algos": "graph-analytics",
  "sparq-arrow": "arrow-columnar",
  "sparq-canon": "rdf-canon",
  "sparq-cli": "cli",
  "sparq-core": "sparql",
  "sparq-engine": "sparql",
  "sparq-engine-serialize": "sparql",
  "sparq-engine-service": "federation",
  "sparq-fedplan": "federation",
  "sparq-forms": "shacl",
  "sparq-geo": "geosparql",
  "sparq-hdt": "data-formats",
  "sparq-http3": "http-server",
  "sparq-introspect": "genai",
  "sparq-jsonld": "data-formats",
  "sparq-mcp": "mcp-server",
  "sparq-nlq": "genai",
  "sparq-policy": "policy",
  "sparq-reason": "inference",
  "sparq-reason-el": "inference",
  "sparq-reason-ql": "inference",
  "sparq-rsp": "streaming-rsp",
  "sparq-secprop-vocab": "zk",
  "sparq-serve": "http-server",
  "sparq-server": "http-server",
  "sparq-shacl": "shacl",
  "sparq-shaclc": "shacl",
  "sparq-sim": "genai",
  "sparq-solid": "solid-pairs",
  "sparq-substrate": "sparql",
  "sparq-terse": "genai",
  "sparq-text": "full-text",
  "sparq-trust": "solid-pairs",
  "sparq-vc": "zk",
  "sparq-vectors": "vector",
  "sparq-wrapper": "sparql",
  "sparq-zk": "zk",
} as const;
