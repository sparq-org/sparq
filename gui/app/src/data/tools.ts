// [OPUS-4.8] sq-ixc3.9 — the operational TOOLS taxonomy for the left-rail TOOLS list.
//
// Per research/gui-design.md §A.2: each tool is a VERB the user opens as a tab to RUN over the
// live store — NOT a marketing page that describes the feature. Each carries an honesty-TIER
// dot inherited from the engine's existing tier taxonomy (site/src/data/surfaces.ts) so a
// `walkthrough`/`soon` capability is never silently dressed up as `live`.
//
// The Query (sq-ixc3.9) and SHACL (sq-ixc3.11) tools are working tabs that RUN over the live
// store with the marketing chrome cut. Every tool whose capability is not genuinely in the
// GUI's in-tab wasm bundle (`--features shacl,jsonld,serialize-rdf,scs`) stays an honest STUB
// the later phases fill (sq-ixc3.12 the query workbench multi-view; sq-ixc3.13 the import
// drawer; sq-zeai the native-only vector/geo wasm spike). A stub tab states plainly what it
// will do and its current tier — it does not fabricate a result.

import type { LucideIcon } from "lucide-react";
import {
  Database,
  Network as GraphIcon,
  ShieldCheck,
  FilePenLine,
  Brain,
  Search,
  Binary,
  MapPin,
  Radio,
  Lock,
  Users,
  Server,
  Scale,
  Gauge,
} from "lucide-react";

/**
 * Live-execution tier — the honesty dot. Mirrors the engine's existing tier taxonomy
 * (site/src/data/surfaces.ts) so the GUI inherits the same honest "what runs where" framing.
 *   live           — shipped wasm, in-tab today
 *   live-new-wasm  — a new wasm bundle (portability spike first)
 *   live-native    — LIVE on the desktop app's native engine; honestly unavailable in the
 *                    hosted web build (the capability is not in the wasm bundle)
 *   live-bbjs      — 3rd-party WASM (bb.js UltraHonk proving); NOT externally audited
 *   live-sim       — faithful in-tab JS simulation; NOT the native protocol
 *   walkthrough    — captured-I/O replay (native-only / different host)
 *   soon           — not yet built; honest placeholder
 */
export type Tier =
  | "live"
  | "live-new-wasm"
  | "live-native"
  | "live-bbjs"
  | "live-sim"
  | "walkthrough"
  | "soon";

/**
 * [FABLE-5] sq-5lyme — rail grouping for the later honesty regroup (sq-1odi9): `working` tools
 * have a live panel today; `coming-soon` tools are honest stubs. Data-only for now — nothing
 * renders the group yet.
 */
export type ToolGroup = "working" | "coming-soon";

export interface ToolDef {
  /** Stable id — the tab key + the Cmd-K (later) index key. */
  id: string;
  /** The VERB shown in the rail + the tab label. */
  label: string;
  /** One-line operational description (what running this tool DOES over the live store). */
  blurb: string;
  tier: Tier;
  icon: LucideIcon;
  /** Does a working tab exist yet, or is this an honest stub the later phases fill? */
  built: boolean;
  /** Rail group (sq-5lyme): working panel vs coming-soon honest stub. Not yet rendered. */
  group: ToolGroup;
}

/**
 * [FABLE-5] sq-5lyme — an optional honesty-metadata override a per-tool PANEL FILE exports
 * (see the tool-panel registry), merged over the base `ToolDef` by `applyToolOverride`. This
 * is the seam that lets a later tool bead flip its tier/copy/group INSIDE its own panel file
 * when the working panel lands — never by editing this shared file. OMIT fields you do not
 * override (an explicit `undefined` field would clobber the base value).
 */
export interface ToolOverride {
  tier?: Tier;
  blurb?: string;
  built?: boolean;
  group?: ToolGroup;
}

/** Merge a panel file's override (if any) over the base tool definition. */
export function applyToolOverride(tool: ToolDef, override?: ToolOverride): ToolDef {
  return override ? { ...tool, ...override } : tool;
}

/** The TOOLS list, in the rail order from research/gui-design.md §A.2. */
export const TOOLS: ToolDef[] = [
  {
    id: "query",
    label: "Query",
    blurb: "Run SPARQL 1.1/1.2 over the live store — SELECT/ASK/CONSTRUCT/DESCRIBE/UPDATE.",
    tier: "live",
    icon: Database,
    built: true,
    group: "working",
  },
  {
    id: "plan",
    label: "Plan",
    // [FABLE-5] sq-ixc3.19 — the visual query-plan explorer: EXPLAIN / EXPLAIN ANALYZE as a
    // navigable operator tree (per-operator time / cardinality / q-error heat), over the
    // in-tab store, the desktop native engine (real wall times), or a server endpoint;
    // plus the this-workbench live query monitor with endpoint Kill.
    blurb:
      "Explore EXPLAIN / EXPLAIN ANALYZE as an operator tree — est vs actual rows, q-error heat, per-operator time — and monitor/kill in-flight queries.",
    tier: "live",
    icon: Gauge,
    built: true,
    group: "working",
  },
  {
    id: "graph-view",
    label: "Graph view",
    blurb: "Node-link visualisation of CONSTRUCT/DESCRIBE results over the live store.",
    tier: "soon",
    icon: GraphIcon,
    built: false,
    group: "coming-soon",
  },
  {
    id: "shacl",
    label: "SHACL",
    blurb: "Validate the live store against SHACL shapes; W3C report with sh:detail.",
    tier: "live",
    icon: ShieldCheck,
    built: true,
    group: "working",
  },
  {
    id: "forms",
    label: "Forms",
    // [GPT-5.6] sq-lsp7k.1.2 — shared renderer is live; edits remain drafts until F4.
    blurb: "View and edit resources through DASH widgets selected from SHACL shapes; consumes FormDescription JSON.",
    tier: "live",
    icon: FilePenLine,
    built: true,
    group: "working",
  },
  {
    id: "inference",
    label: "Inference",
    // [SONNET-4.6] sq-bfimm — N3 joins the existing per-workspace RDFS / OWL 2 RL
    // entailment selector with persisted rule documents and ground-triple closure.
    blurb: "Per-workspace RDFS / OWL 2 RL / N3 entailment — forward-chain the closure so queries match entailed triples.",
    tier: "live-new-wasm",
    icon: Brain,
    built: true,
    group: "working",
  },
  {
    id: "full-text",
    label: "Full-text",
    blurb: "BM25 full-text search via magic predicates over the live store.",
    tier: "live-new-wasm",
    icon: Search,
    built: false,
    group: "coming-soon",
  },
  {
    id: "vector",
    label: "Vector",
    blurb: "kNN over embeddings (vec:nearest); native-only, not yet in the wasm bundle.",
    tier: "walkthrough",
    icon: Binary,
    built: false,
    group: "coming-soon",
  },
  {
    id: "geosparql",
    label: "GeoSPARQL",
    blurb: "Spatial joins + topology (geof:); native-only, not yet in the wasm bundle.",
    tier: "walkthrough",
    icon: MapPin,
    built: false,
    group: "coming-soon",
  },
  {
    id: "streaming",
    label: "Streaming",
    blurb: "RSP-QL windowed stream processing with R/I/DSTREAM tick view.",
    tier: "live-new-wasm",
    icon: Radio,
    built: false,
    group: "coming-soon",
  },
  {
    id: "zk",
    label: "ZK proofs",
    blurb:
      "Commit → prove → verify a query result in-tab (UltraHonk). Research-grade; NOT externally audited.",
    tier: "live-bbjs",
    icon: Lock,
    built: false,
    group: "coming-soon",
  },
  {
    id: "mpc",
    label: "MPC",
    blurb:
      "Multi-party threshold over additive shares. In-tab JS illustration; NOT the native protocol, NOT live MPC.",
    tier: "live-sim",
    icon: Users,
    built: false,
    group: "coming-soon",
  },
  {
    id: "odrl",
    label: "Policies",
    // [FABLE-5] sq-ixc3.15 — the ODRL usage-control tool. NATIVE-ONLY: the sparq-policy
    // evaluator + sparq-solid enforcement store are not in the in-tab wasm bundle, so the
    // hosted web build degrades honestly (the panel shows the native-only message).
    blurb:
      "Author + evaluate an ODRL policy; preview the access-gated result set per requester next to the ungated rows.",
    tier: "live-native",
    icon: Scale,
    built: false,
    group: "coming-soon",
  },
  {
    id: "server",
    label: "Server",
    blurb:
      "Connect to a running sparq-server (SPARQL 1.1 Protocol) — endpoint mode, subscriptions, health.",
    tier: "walkthrough",
    icon: Server,
    built: false,
    group: "coming-soon",
  },
];

/** Tier → honesty-dot colour class + short label, for the rail dots and tab headers. */
export const TIER_META: Record<Tier, { dot: string; label: string }> = {
  live: { dot: "bg-[var(--success)]", label: "Live in-tab" },
  "live-new-wasm": { dot: "bg-[var(--success)]", label: "Live (new wasm)" },
  "live-native": { dot: "bg-primary", label: "Live (desktop native only)" },
  "live-bbjs": { dot: "bg-primary", label: "Live (bb.js); not audited" },
  "live-sim": { dot: "bg-[var(--warning)]", label: "In-tab simulation" },
  walkthrough: { dot: "bg-muted-foreground", label: "Walkthrough (native-only)" },
  soon: { dot: "bg-muted-foreground", label: "Not yet built" },
};

export function toolById(id: string): ToolDef | undefined {
  return TOOLS.find((t) => t.id === id);
}
