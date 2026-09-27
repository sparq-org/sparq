// [OPUS-4.8] sq-vjn4 — the in-site benchmark data layer. Consumes the committed
// snapshot (src/data/benchmarks.generated.json, refreshed by scripts/sync-benchmarks.mjs
// from the benchmark-data branch + bench/dashboard/{metric-labels,competitors}.json) and
// derives the structures the /benchmarks pages render: a per-TYPE (family) grouping for
// the left sidebar, per-suite grouping for the SPARQL page's collapsible groups, and the
// LIVE-COMPUTED competitive summary.
//
// HONESTY (load-bearing, per the bead + the empirical-honesty rule):
//   * Every metric is smaller-is-better (github-action-benchmark customSmallerIsBetter).
//   * A competitive speedup is computed ONLY from REAL competitor numbers that exist in
//     the data, MEASURED ON THE SAME BOX as the sparq number it is divided against:
//       - SHACL: competitors.values (pyshacl + jena-shacl) gathered same-machine vs the
//         same sparq SHACL metrics in the series.
//       - SP2Bench: competitors.same_box_comparisons (sparq vs Oxigraph/QLever on one
//         ephemeral box) — but Oxigraph/QLever rows are currently null, so NO speedup is
//         computed there; it shows "competitor baseline pending".
//   * Cross-machine `references` numbers (QLever on an M1, EYE DeepTaxonomy) are surfaced
//     SEPARATELY and never divided into a same-box speedup.
//   * A group with NO same-box competitor baseline shows sparq's absolute numbers + an
//     honest "competitor baseline pending" note — never a fabricated ratio.
//   * The CI series runs on a GitHub-hosted runner; the SHACL/SP2Bench gathers ran on an
//     ephemeral AWS Graviton box (NON-CANONICAL). Absolute µs across host classes are not
//     directly comparable — every speedup is labelled with its gather provenance.

import data from "./benchmarks.generated.json";
import { deriveCoverage } from "@/lib/baseline-coverage";
import type { CoverageMatrix } from "@/lib/baseline-coverage";
export {
  COVERAGE_ENGINES,
  COVERAGE_SUITES,
} from "@/lib/baseline-coverage";
export type {
  CoverageCell,
  CoverageEngine,
  CoverageMatrix,
  CoverageSuite,
} from "@/lib/baseline-coverage";

export interface Bench {
  name: string;
  value: number;
  unit: string;
}

export interface MetricLabel {
  label: string;
  suite: string;
  dataset?: string;
  query?: string;
  mode?: string;
  regime?: string;
  unit?: string;
}

export interface CompetitorEngine {
  id: string;
  label: string;
  version: string;
  env?: string;
  source?: string;
}

export interface ReferenceBaseline {
  suite: string;
  engine: string;
  version: string;
  env: string;
  scale: string;
  metric: string;
  value: number | null;
  unit: string;
  regime: string;
  source: string;
  caveat: string;
}

export interface SameBoxRow {
  query: string;
  unit: string;
  rows: number;
  values: Record<string, number | null>;
  count_match: boolean | null;
  // [SONNET-4.6] A row may use another workload's count oracle or a distinct
  // corpus variant; the table surfaces both caveats beside the query.
  sparq_oracle_workload?: string;
  corpus_variant?: string;
  // [FABLE-5] sq-7d3dj.34 — present on HTTP/TTFB-panel suites (e.g. "sp2b-http") only:
  // `values` is then the keep-alive full-request best; these carry the TTFB and
  // fresh-connect twins (same best-of-gathers rule; see the entry's `connection` note).
  values_ttfb?: Record<string, number | null>;
  values_fresh?: Record<string, number | null>;
  values_fresh_ttfb?: Record<string, number | null>;
}

// [OPUS-4.8] sq-1sa9r — one engine column in a same-box comparison. `mode` labels the
// measurement path (CLI in-process vs HTTP SPARQL adapter) — a real asymmetry that must be
// surfaced. `status`/`failure` carry a whole-engine failure (e.g. fuseki load timeout) so
// the column renders "failed", never blank.
export interface SameBoxEngine {
  id: string;
  label: string;
  version: string;
  env?: string;
  mode?: string;
  status?: string; // "ok" | "failed"
  failure?: string;
}

export interface SameBoxComparison {
  suite: string;
  scale: string;
  iters: number;
  git_commit: string;
  gathered_at_utc: string;
  source: string;
  // [OPUS-4.8] sq-1sa9r — true for a dedicated quiet-box canonical gather (recorded as
  // `canonical` in the envelope). Absent/false = an ephemeral non-canonical ordering gather.
  canonical?: boolean;
  combine?: string; // how duplicate gathers were combined (e.g. best-of)
  count_crosscheck_note?: string;
  // [FABLE-5] sq-7d3dj.34 — HTTP/TTFB-panel entries only: which connection regimes were
  // measured (keep-alive AND fresh-connect) + which one `values` reports.
  connection?: { modes_measured: string[]; primary: string; note: string };
  profile?: string;
  env: {
    host_class: string;
    quiet_box: boolean;
    gathered_at_utc?: string;
    cpu_model?: string;
    kernel?: string;
    note?: string;
  };
  engines: SameBoxEngine[];
  rows: SameBoxRow[];
}

interface CompetitorsData {
  schema_version?: number;
  gathered_at_utc?: string;
  gathered_by?: string;
  engines: CompetitorEngine[];
  oxigraph_compare_note?: string;
  values: Record<string, Record<string, number>>;
  references: ReferenceBaseline[];
  same_box_comparisons: SameBoxComparison[];
}

// [OPUS-4.8] sq-hsyg — one commit's worth of the github-action-benchmark series. `date` is
// the epoch-ms the series carries (github-action-benchmark writes a number).
export interface HistoryEntry {
  commit: string | null;
  date: number | null;
  benches: Bench[];
}

interface Snapshot {
  generatedAt: string;
  source: string;
  latest: HistoryEntry;
  // sq-hsyg: trailing window of commits (oldest -> newest), latest === history[last].
  history?: HistoryEntry[];
  labels: Record<string, MetricLabel>;
  competitors: CompetitorsData;
}

const SNAPSHOT = data as unknown as Snapshot;

export const LATEST = SNAPSHOT.latest;
export const SOURCE = SNAPSHOT.source;
export const GENERATED_AT = SNAPSHOT.generatedAt;
// History is optional in the schema (a pre-sq-hsyg JSON, or a fork without the
// benchmark-data branch); fall back to a single-point series so trend charts still render
// one marker and never crash on an empty/absent history.
export const HISTORY: HistoryEntry[] =
  Array.isArray(SNAPSHOT.history) && SNAPSHOT.history.length
    ? SNAPSHOT.history
    : [SNAPSHOT.latest];
const LABELS = SNAPSHOT.labels;
export const COMPETITORS = SNAPSHOT.competitors;

// ---- metric-label lookup (mirror of dashboard.js lookup/labelFor/suiteFor) ----------

function stem(name: string): string {
  return name.replace(/_us$/, "");
}

export function lookup(name: string): MetricLabel | null {
  return LABELS[stem(name)] || LABELS[name] || null;
}

function humanize(name: string): string {
  return name
    .replace(/_us$/, " (µs)")
    .replace(/_s$/, " (s)")
    .replace(/_/g, " ")
    .trim();
}

export function labelFor(name: string): string {
  const rec = lookup(name);
  return rec && rec.label ? rec.label : humanize(name);
}

export function suiteFor(name: string): string {
  const rec = lookup(name);
  if (rec && rec.suite) return rec.suite;
  return "Other";
}

export function unitFor(name: string, fallback: string): string {
  const rec = lookup(name);
  return (rec && rec.unit) || fallback;
}

// ---- top-level capability FAMILIES (mirror of dashboard.js FAMILIES, label-driven) ---
// One entry per benchmark TYPE — drives the left sidebar. `suites` are matched against
// suiteFor() (normalised lower-case); `prefixes` are a structural fallback on the raw
// metric name's leading token. Order is the display order. A family with no data is KEPT
// (shows "not yet reported") so coverage is honest, never fabricated.
//
// [OPUS-4.8] sq-p744: this list MUST stay in lock-step with the standalone
// bench/dashboard/dashboard.js FAMILIES so the in-site Benchmarks section is not a strict
// subset of the published dashboard. Full coverage = Core, SPARQL, SHACL, GeoSPARQL,
// Full-Text, Vector/ANN, Reasoning, plus the capability families ZK / Solid / HDT / RSP /
// GenAI / GPU (the latter six currently have NO data in benchmarks.generated.json — they
// are stdout/criterion benches not yet wired into the CI customSmallerIsBetter feed — and
// render "not yet reported" until a ci-bench hook lands; that harness work is tracked
// separately, NOT fabricated here).

export interface Family {
  key: string;
  title: string;
  blurb: string;
  suites: string[];
  prefixes: string[];
}

export const FAMILIES: Family[] = [
  {
    key: "sparql",
    title: "SPARQL query suites",
    blurb:
      "Recognised public SPARQL benchmarks — LUBM, WatDiv, SP2Bench, BSBM, DBPSB — plus operator micro-benchmarks and the synthetic qlever-style suite.",
    suites: [
      "sp2bench",
      "watdiv",
      "lubm (reasoning)",
      "bsbm",
      "dbpsb",
      "synthetic (qlever-style)",
      "operators",
    ],
    prefixes: [
      "sp2b",
      "watdiv",
      "lubm",
      "bsbm",
      "dbpsb",
      "op",
      "q01",
      "q02",
      "q03",
      "q04",
      "q05",
      "q06",
      "q07",
      "q08",
      "q09",
      "q10",
      "q11",
      "q12",
    ],
  },
  {
    key: "shacl",
    title: "SHACL validation",
    blurb:
      "SHACL Core + SHACL-SPARQL validation latency over the LUBM(1) ABox × five hand-authored shape graphs, with same-box pySHACL + Apache Jena baselines.",
    suites: ["shacl validation", "shacl"],
    prefixes: ["shacl"],
  },
  {
    key: "geo",
    title: "GeoSPARQL",
    blurb:
      "geof: spatial functions + R-tree GeoIndex over a fixed ~100k CRS84 point corpus — within / nearest-k / topology compliance.",
    suites: ["geosparql", "geo"],
    prefixes: ["geo"],
  },
  {
    key: "fts",
    title: "Full-Text search",
    blurb:
      "BM25 full-text search via magic predicates over a synthetic 100k-literal corpus, plus index footprint.",
    suites: ["full-text", "fulltext", "fts", "text"],
    prefixes: ["text", "fts"],
  },
  {
    key: "vector",
    title: "Vector / ANN",
    blurb:
      "Approximate-nearest-neighbour recall deficits (HNSW / Vamana / PQ) vs exact kNN.",
    suites: ["vector", "vector / ann", "vectors", "ann"],
    prefixes: ["vector", "vectors", "vec", "ann", "knn", "hnsw", "diskann"],
  },
  {
    key: "reasoning",
    title: "Reasoning (N3 · RDFS · OWL-RL)",
    blurb:
      "Forward-closure materialization — the Deep Taxonomy depth series — with an EYE cross-engine reference.",
    suites: [
      "deep taxonomy",
      "deeptax",
      "deep-taxonomy",
      "deeptaxonomy",
      "reasoning",
      "inference",
    ],
    prefixes: ["deeptax", "deep", "owl", "infer", "incremental", "reason"],
  },
  {
    key: "core",
    title: "Core (load · dict · memory)",
    blurb:
      "End-to-end load (parse + index build), dictionary footprint, and per-triple store memory.",
    suites: ["pipeline", "memory / size"],
    prefixes: ["load", "parse", "store", "dict", "wasm", "rdfs"],
  },
  // [OPUS-4.8] sq-p744 (in-site capability-families gap; sq-vjn4) — register the capability
  // families the standalone bench/dashboard already covers (dashboard.js FAMILIES) so the
  // in-site Benchmarks section is no longer a strict subset. These are stdout/criterion
  // benches today that do NOT yet emit into the github-action-benchmark customSmallerIsBetter
  // CI feed, so their LATEST data is currently empty — every family below KEEPS its sidebar
  // entry + page and renders "not yet reported" / a "soon" badge (the same honest graceful-
  // degradation the existing empty families use), never a fabricated row. When a ci-bench hook
  // starts emitting (e.g. `zk_commit_us`, `hdt_load_s`, `rsp_window_us`, …) the metric lands
  // here automatically via these suites/prefixes — no further site change. Suites/prefixes are
  // copied from the standalone dashboard so the two surfaces bucket identically.
  {
    key: "zk",
    title: "Zero-knowledge (commit · trace · circuit)",
    // HONESTY (privacy-claims gate, sq-qhy4): the v1 ZK query-proof verifier is research-grade
    // and has NOT been externally audited; any figures here are indicative micro-benchmarks of
    // the commitment / trace / circuit machinery, NOT an externally-verified soundness claim.
    blurb:
      "Commitment, trace-capture and circuit micro-benchmarks for the research-grade ZK estate (commit · canonicalisation · trace · circuit gates). Research-grade only — the v1 verifier is not externally audited, so these are indicative engineering numbers, not an audited cryptographic guarantee.",
    suites: ["zk", "zk-trace", "zk trace", "zk-compose", "zk compose", "zero-knowledge"],
    prefixes: ["zk", "zktrace", "zkcompose", "poseidon2", "rdfc10", "commitment"],
  },
  {
    key: "solid",
    title: "Solid / access control (WAC · ACP)",
    blurb:
      "Solid access-control evaluation latency — Web Access Control (WAC) and Access Control Policy (ACP) decision micro-benchmarks.",
    suites: ["solid", "wac", "acp", "access control"],
    prefixes: ["solid", "wac", "acp"],
  },
  {
    key: "hdt",
    title: "HDT archive ingest",
    blurb:
      "HDT (Header-Dictionary-Triples) archive ingest — decode + id-translation throughput when loading .hdt archives into a sparq Graph.",
    suites: ["hdt"],
    prefixes: ["hdt"],
  },
  // [FABLE-5] sq-hmd7l.28 — the SPARQL 1.1 Update parity axis (PSS LDP-CRUD stream). Its
  // canonical same-box gather (suite `pss-update-parity`, competitors Fuseki / Oxigraph over
  // an HTTP update endpoint) surfaces here via SAMEBOX_SUITE_FAMILY even before a CI-feed
  // `update_*` metric emits, so the competitor columns render as soon as the gather merges.
  {
    key: "update",
    title: "SPARQL Update (LDP-CRUD parity)",
    blurb:
      "SPARQL 1.1 Update latency over a Solid-pod LDP-CRUD stream (INSERT / DELETE / DROP), with same-box Apache Jena Fuseki + Oxigraph baselines; the post-workload quad count is cross-checked engine-vs-engine before any latency row is trusted.",
    suites: ["update", "sparql update", "pss-update-parity", "pss update"],
    prefixes: ["update", "pss"],
  },
  {
    key: "rsp",
    title: "RDF Stream Processing (continuous queries)",
    blurb:
      "RDF Stream Processing — continuous-query and windowed-evaluation micro-benchmarks over streaming RDF.",
    suites: ["rsp", "rdf stream processing", "streaming"],
    prefixes: ["rsp", "stream", "window"],
  },
  {
    key: "genai",
    title: "GenAI (similarity · introspection)",
    blurb:
      "Generative-AI-adjacent micro-benchmarks — embedding similarity and graph introspection helpers.",
    suites: ["similarity", "introspect", "introspection", "genai"],
    prefixes: ["sim", "similarity", "introspect"],
  },
  {
    key: "gpu",
    title: "GPU kernels (experimental)",
    blurb:
      "Experimental GPU-kernel micro-benchmarks — an exploratory surface, reported as-is.",
    suites: ["gpu"],
    prefixes: ["gpu"],
  },
];

// Capability families are prefix-matched BEFORE core/sparql so a generic `q*`/`op*` token
// never mis-buckets them. Mirrors dashboard.js `kind: 'capability'`. [OPUS-4.8] sq-p744
const CAPABILITY_KEYS = new Set([
  "shacl",
  "geo",
  "fts",
  "vector",
  "reasoning",
  "zk",
  "solid",
  "hdt",
  "update",
  "rsp",
  "genai",
  "gpu",
]);

export function familyOf(name: string): Family {
  const suite = suiteFor(name).toLowerCase();
  const token = name.toLowerCase().split("_")[0];
  for (const f of FAMILIES) {
    if (f.suites.includes(suite)) return f;
  }
  // capability families before core/sparql so a `q`/`op` prefix never mis-buckets them
  for (const f of FAMILIES) {
    if (CAPABILITY_KEYS.has(f.key) && f.prefixes.includes(token)) return f;
  }
  for (const f of FAMILIES) {
    if (!CAPABILITY_KEYS.has(f.key) && f.prefixes.includes(token)) return f;
  }
  if (/^(q\d+|op)_/.test(name.toLowerCase())) {
    return FAMILIES.find((f) => f.key === "sparql")!;
  }
  return FAMILIES.find((f) => f.key === "core")!;
}

// ---- per-family + per-suite views -----------------------------------------------------

export interface MetricRow {
  name: string;
  label: string;
  value: number;
  unit: string;
  suite: string;
}

function benchesForFamily(key: string): Bench[] {
  return LATEST.benches.filter((b) => familyOf(b.name).key === key);
}

export function familyMetricCount(key: string): number {
  return benchesForFamily(key).length;
}

export interface SuiteGroup {
  suite: string;
  rows: MetricRow[];
  summary: CompetitiveSummary;
}

// Rows for a family, grouped by their fine-grained suite, sorted by label. Each suite
// group carries its live-computed competitive summary.
export function suiteGroupsForFamily(key: string): SuiteGroup[] {
  const bySuite: Record<string, MetricRow[]> = {};
  for (const b of benchesForFamily(key)) {
    const suite = suiteFor(b.name);
    (bySuite[suite] = bySuite[suite] || []).push({
      name: b.name,
      label: labelFor(b.name),
      value: b.value,
      unit: b.unit,
      suite,
    });
  }
  return Object.keys(bySuite)
    .sort()
    .map((suite) => {
      const rows = bySuite[suite].sort((a, b) =>
        a.label < b.label ? -1 : a.label > b.label ? 1 : 0,
      );
      return { suite, rows, summary: competitiveSummary(suite, rows) };
    });
}

// Full assembly for a family's suite groups: rows + live summary + any same-box table +
// any cross-machine references — the shape the collapsible SuiteGroup component renders.
export interface FullSuiteGroup {
  suite: string;
  rows: MetricRow[];
  summary: CompetitiveSummary;
  sameBox?: SameBoxComparison;
  // [OPUS-4.8] sq-7d3dj.34.3 — the canonical HTTP-mode panel (all engines over HTTP,
  // full-request + TTFB, keep-alive + fresh-connect) for this suite, plus its own honest
  // same-mode wins/losses summary. Present only for suites with a `<base>-http` gather.
  httpSameBox?: SameBoxComparison;
  httpSummary?: CompetitiveSummary;
  references: ReferenceBaseline[];
  // sq-hsyg: per-suite trend series (one per metric) + scaling families (size-parametrised).
  trends: TrendSeries[];
  scaling: ScalingFamily[];
}

export function fullSuiteGroupsForFamily(key: string): FullSuiteGroup[] {
  // Derive the family's trend + scaling once, then slice per suite (cheap; the series count
  // is small). Trend/scaling carry a `suite` so the per-suite filter is exact.
  const allTrends = trendSeriesForFamily(key);
  const allScaling = scalingFamiliesForFamily(key);
  const groups = suiteGroupsForFamily(key).map((g) => {
    const httpSameBox = httpSameBoxFor(g.suite);
    return {
      suite: g.suite,
      rows: g.rows,
      summary: g.summary,
      sameBox: sameBoxFor(g.suite),
      httpSameBox,
      // Computed server-side (no fabrication client-side) — same honesty path as the CLI
      // matrix, so a SP2Bench HTTP loss shows as a loss, never spun.
      httpSummary: httpSameBox ? summarizeSameBox(httpSameBox) : undefined,
      references: referencesForSuite(g.suite),
      trends: allTrends.filter((t) => t.suite === g.suite),
      scaling: allScaling.filter((s) => s.suite === g.suite),
    };
  });

  // [FABLE-5] sq-hmd7l.28 — surface same-box comparisons whose FAMILY is this one but which
  // have NO CI-feed metric yet (so suiteGroupsForFamily produced no group for them). Without
  // this a canonical fts/geo/hdt/update/materialize gather would land in competitors.json and
  // stay INVISIBLE on the site (the group list is bench-driven). We synthesize a group per
  // such comparison carrying the same-box table + its honest live summary, zero metric rows.
  // A `-http` twin is NOT synthesized standalone: it attaches to its base via httpSameBoxFor.
  const alreadyRendered = new Set<string>();
  for (const g of groups) {
    if (g.sameBox) alreadyRendered.add(g.sameBox.suite.toLowerCase());
    if (g.httpSameBox) alreadyRendered.add(g.httpSameBox.suite.toLowerCase());
  }
  const extra: FullSuiteGroup[] = [];
  for (const c of COMPETITORS.same_box_comparisons || []) {
    if (familyForComparison(c) !== key) continue;
    const sid = c.suite.toLowerCase();
    if (alreadyRendered.has(sid)) continue;
    if (sid.endsWith("-http")) continue; // attaches to a base group, not standalone
    alreadyRendered.add(sid);
    const httpSameBox = httpSameBoxFor(c.suite);
    if (httpSameBox) alreadyRendered.add(httpSameBox.suite.toLowerCase());
    extra.push({
      suite: sameBoxDisplaySuite(c),
      rows: [],
      summary: summarizeSameBox(c),
      sameBox: c,
      httpSameBox,
      httpSummary: httpSameBox ? summarizeSameBox(httpSameBox) : undefined,
      references: referencesForSuite(c.suite),
      trends: [],
      scaling: [],
    });
  }
  return [...groups, ...extra];
}

// A human display label for a comparison-only suite group header (the raw suite id like
// `pss-update-parity` is machine-ish; render a friendlier title, provenance stays in the table).
function sameBoxDisplaySuite(c: SameBoxComparison): string {
  const map: Record<string, string> = {
    fts: "Full-text search — same-box vs Jena-text",
    geo: "GeoSPARQL — same-box vs Jena GeoSPARQL",
    hdt: "HDT decode — same-box vs hdt-cpp",
    "pss-update-parity": "SPARQL Update — same-box parity",
    "materialize-competitors": "Materialisation — same-box vs Jena / VLog / Nemo",
  };
  return map[c.suite.toLowerCase()] || `${c.suite} — same-box comparison`;
}

// ---- LIVE competitive summary (the load-bearing honesty computation) ------------------
// For a given suite, compute the speedup of sparq vs the NEXT-BEST competitor across the
// suite's benchmarks, using ONLY same-box competitor numbers. Returns a discriminated
// union so the UI can render an honest state for every case.

export type CompetitiveSummary =
  | {
      kind: "speedup";
      // per-benchmark ratios competitor/sparq (>1 means sparq faster)
      min: number;
      max: number;
      median: number;
      n: number; // number of benchmarks contributing a real ratio
      competitor: string; // the next-best (fastest) competitor used per benchmark
      engines: string[]; // competitor labels present in the comparison
      provenance: string; // host/scale/quiet-box provenance string (NON-CANONICAL)
      nonCanonical: boolean;
    }
  | {
      // [OPUS-4.8] sq-1sa9r — a CANONICAL (or non-canonical) same-box multi-engine gather.
      // We deliberately do NOT headline a single "N× faster" multiplier here: the engines are
      // measured in DIFFERENT modes (sparq/Oxigraph in-process via the CLI; Fuseki/Virtuoso/
      // QLever over an HTTP SPARQL adapter), so on small result sets the absolute ratios carry
      // per-query harness/transport overhead as much as engine speed. Instead we report the
      // honest, count-checked "fastest on W of N" plus the raw range, heavily caveated.
      kind: "same-box";
      competitors: string[]; // competitor labels that produced >=1 timing
      cliCompetitors: string[]; // in-process CLI competitors (excl. sparq), e.g. Oxigraph
      httpEngines: string[]; // HTTP-adapter engines, e.g. Fuseki / Virtuoso / QLever
      failedEngines: string[]; // labels of engines that failed to load (e.g. Fuseki)
      total: number; // count-cross-checked queries compared
      wins: number; // queries where sparq was the fastest engine
      losses: number; // queries where a competitor was faster
      median: number; // median (fastest-competitor / sparq) over the compared queries
      min: number;
      max: number;
      diffQueries: string[]; // queries excluded because engines disagreed on the count
      canonical: boolean;
      host: string;
      scale: string;
      gitCommit: string;
    }
  | {
      kind: "pending";
      // a same-box comparison EXISTS for this suite but competitor cells are null
      reason: string;
      provenance?: string;
    }
  | {
      kind: "sparq-only";
      // no same-box competitor data at all for this suite
      reason: string;
    };

// median of a sorted-or-unsorted numeric array
function median(xs: number[]): number {
  const s = [...xs].sort((a, b) => a - b);
  const mid = Math.floor(s.length / 2);
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

const round1 = (x: number) => Math.round(x * 10) / 10;

// Find the same_box_comparisons entry whose suite matches (case-insensitive). The
// fuzzy match deliberately does NOT match the `<base>-http` panel suites (e.g. "sp2b-http"),
// so a CLI suite group ("SP2Bench") binds to the CLI matrix ("sp2b"), not the HTTP panel.
function sameBoxFor(suite: string): SameBoxComparison | undefined {
  const norm = suite.toLowerCase();
  return (COMPETITORS.same_box_comparisons || []).find(
    (c) =>
      c.suite.toLowerCase() === norm ||
      norm.includes(c.suite.toLowerCase()) ||
      c.suite.toLowerCase().includes(norm.split(" ")[0]),
  );
}

// [OPUS-4.8] sq-7d3dj.34.3 — find the canonical HTTP-mode panel (`<base>-http`) for a CLI
// suite group, so the SP2Bench / WatDiv groups can render the same-mode HTTP full-request +
// TTFB panel BELOW the CLI matrix. We strip the "-http" suffix and reuse the same fuzzy
// suite match ("sp2b-http" → "sp2b" matches the "SP2Bench" group). Returns undefined when
// no HTTP panel exists for the suite, so groups without one are unaffected.
function httpSameBoxFor(suite: string): SameBoxComparison | undefined {
  const norm = suite.toLowerCase();
  return (COMPETITORS.same_box_comparisons || []).find((c) => {
    const cs = c.suite.toLowerCase();
    if (!cs.endsWith("-http")) return false;
    const base = cs.slice(0, -"-http".length);
    return norm === base || norm.includes(base) || base.includes(norm.split(" ")[0]);
  });
}

// [FABLE-5] sq-hmd7l.28 — map a same_box_comparison's `suite` id (as the ingest emits it) to
// the capability FAMILY whose page should render it. This is the seam that lets a new-axis
// canonical gather surface on the site WITHOUT hand-editing the page: the ingest writes the
// entry into competitors.json under a known suite id, and this table routes it to a family.
// A comparison whose suite is not listed (a brand-new axis) still renders on the SPARQL-suite
// pages via the existing fuzzy sameBoxFor() bind, so it is never dropped silently.
const SAMEBOX_SUITE_FAMILY: { match: RegExp; family: string }[] = [
  { match: /^fts$/i, family: "fts" },
  { match: /^geo$/i, family: "geo" },
  { match: /^hdt$/i, family: "hdt" },
  { match: /^pss-update-parity$/i, family: "update" },
  { match: /^update/i, family: "update" },
  { match: /^materialize/i, family: "reasoning" },
  { match: /^parse/i, family: "core" },
];

// The family a same_box_comparison belongs to (or null if it is a SPARQL-suite comparison
// already surfaced by the per-suite fuzzy bind — sp2b / watdiv and their -http twins).
function familyForComparison(c: SameBoxComparison): string | null {
  for (const { match, family } of SAMEBOX_SUITE_FAMILY) {
    if (match.test(c.suite)) return family;
  }
  return null;
}

export function competitiveSummary(
  suite: string,
  rows: MetricRow[],
): CompetitiveSummary {
  // (1) SHACL-style: competitors.values keyed by metric stem, gathered same-machine
  //     against the SAME sparq metrics in the series. Compute per-metric ratios.
  const valueRatios: number[] = [];
  const valueEngines = new Set<string>();
  let usedNextBest = "";
  for (const r of rows) {
    const cell =
      COMPETITORS.values[r.name] || COMPETITORS.values[stem(r.name)] || null;
    if (!cell) continue;
    // next-best competitor = the FASTEST (smallest) competitor number for this metric
    let best = Infinity;
    let bestEngine = "";
    for (const [eng, v] of Object.entries(cell)) {
      valueEngines.add(eng);
      if (typeof v === "number" && v > 0 && v < best) {
        best = v;
        bestEngine = eng;
      }
    }
    if (best !== Infinity && r.value > 0) {
      valueRatios.push(best / r.value);
      if (!usedNextBest) usedNextBest = bestEngine;
    }
  }
  if (valueRatios.length > 0) {
    return {
      kind: "speedup",
      min: round1(Math.min(...valueRatios)),
      max: round1(Math.max(...valueRatios)),
      median: round1(median(valueRatios)),
      n: valueRatios.length,
      competitor: "next-best competitor",
      engines: engineLabels([...valueEngines]),
      provenance:
        "same-box vs " +
        engineLabels([...valueEngines]).join(" / ") +
        " on an ephemeral AWS Graviton box (NON-CANONICAL; for cross-engine ordering)",
      nonCanonical: true,
    };
  }

  // (2) same_box_comparisons (SP2Bench / WatDiv): per-query rows with a sparq value +
  //     competitor values measured on ONE box. HONESTY (sq-1sa9r): we compute the ratio ONLY
  //     from rows whose solution COUNT cross-checked across engines (count_match !== false —
  //     a disagreeing count means a competitor computed a DIFFERENT answer, so its timing is
  //     not comparable). We also record the mode asymmetry (in-process CLI vs HTTP adapter)
  //     and any failed engine so the UI can caveat, never a bald "N× faster" headline.
  const sb = sameBoxFor(suite);
  if (sb) return summarizeSameBox(sb);

  // (3) nothing comparable on the same box.
  return {
    kind: "sparq-only",
    reason:
      "No same-box competitor baseline has been gathered for this suite yet — sparq's absolute numbers are shown below.",
  };
}

// [FABLE-5]-authored data, this UI by [OPUS-4.8] sq-7d3dj.34.3 — the honest same-box
// summary for ONE comparison (CLI matrix OR HTTP-mode panel). Extracted from
// competitiveSummary so both the CLI cross-engine table and the HTTP/TTFB panel share the
// identical count-checked wins/losses computation (no fabrication; a query whose solution
// count disagreed across engines is EXCLUDED from the ratio, never spun into a win). The
// ratio is fastest-competitor / sparq per query; wins = queries where sparq was fastest.
export function summarizeSameBox(sb: SameBoxComparison): CompetitiveSummary {
  const ratios: number[] = [];
  const diffQueries: string[] = [];
  const engines = new Set<string>();
  let wins = 0;
  let losses = 0;
  for (const row of sb.rows) {
    const sparq = row.values["sparq"];
    if (typeof sparq !== "number" || sparq <= 0) continue;
    if (row.count_match === false) {
      diffQueries.push(row.query);
      continue; // engines disagreed on the count — timing not comparable
    }
    let best = Infinity;
    for (const [eng, v] of Object.entries(row.values)) {
      if (eng === "sparq") continue;
      if (typeof v === "number" && v > 0) {
        engines.add(eng);
        if (v < best) best = v;
      }
    }
    if (best !== Infinity) {
      const ratio = best / sparq;
      ratios.push(ratio);
      if (ratio > 1) wins += 1;
      else if (ratio < 1) losses += 1;
    }
  }
  // Label + mode grouping comes from THIS comparison's own engines (not the top-level
  // registry), so a competitor absent from the registry still gets its proper label, and
  // the CLI-vs-HTTP asymmetry is classified from each engine's recorded `mode`. In the
  // HTTP-mode panel every engine's mode is HTTP, so cliCompetitors is empty and httpEngines
  // is all — that is the point: the panel measures every engine in the SAME mode.
  const isHttp = (e: { mode?: string }) => /http/i.test(e.mode || "");
  const isCli = (e: { mode?: string }) => /in-process/i.test(e.mode || "");
  const failedEngines = sb.engines
    .filter((e) => e.status === "failed")
    .map((e) => e.label);
  const competitors = sb.engines
    .filter((e) => e.id !== "sparq" && engines.has(e.id))
    .map((e) => e.label);
  const cliCompetitors = sb.engines
    .filter((e) => e.id !== "sparq" && isCli(e))
    .map((e) => e.label);
  const httpEngines = sb.engines.filter((e) => isHttp(e)).map((e) => e.label);
  if (ratios.length > 0) {
    return {
      kind: "same-box",
      competitors,
      cliCompetitors,
      httpEngines,
      failedEngines,
      total: ratios.length,
      wins,
      losses,
      median: round1(median(ratios)),
      min: round1(Math.min(...ratios)),
      max: round1(Math.max(...ratios)),
      diffQueries,
      canonical: sb.canonical === true,
      host: sb.env.host_class || "quiet box",
      scale: sb.scale,
      gitCommit: sb.git_commit,
    };
  }
  // a same-box gather EXISTS but every competitor cell is null → honest pending
  return {
    kind: "pending",
    reason:
      "A same-box " +
      sb.suite +
      " comparison was run, but competitor timings were not captured this run, so no comparison can be computed yet.",
    provenance: sb.env.host_class,
  };
}

function engineLabels(ids: string[]): string[] {
  return ids.map((id) => {
    const e = COMPETITORS.engines.find((x) => x.id === id);
    return e ? e.label : id;
  });
}

// External-reference (cross-machine) baselines for a suite — shown SEPARATELY, never
// divided into a same-box speedup. The featured-suite key the dashboard uses.
export function referencesForSuite(suite: string): ReferenceBaseline[] {
  const norm = suite.toLowerCase();
  return (COMPETITORS.references || []).filter(
    (r) =>
      r.suite.toLowerCase() === norm ||
      norm.includes(r.suite.toLowerCase()) ||
      r.suite.toLowerCase().includes(norm.split(" ")[0]),
  );
}

export function sameBoxComparison(suite: string): SameBoxComparison | undefined {
  return sameBoxFor(suite);
}

// Number-formatter matching dashboard.js fmtNum (no fabrication, just display).
// [FABLE-5] sq-qgkwy.1 — moved to src/lib/fmt-num.ts and RE-EXPORTED here for server-side
// callers only. "use client" components must import it from "@/lib/fmt-num" directly:
// importing any VALUE from THIS module drags the full ~1.3 MB generated snapshot into the
// browser bundle (it double-shipped /benchmarks/[type]'s data as a ~762 KB raw page chunk
// on top of the prerendered HTML). Guarded by test/benchmarks-data-server-only.test.mjs.
export { fmtNum } from "@/lib/fmt-num";

// ---- TREND series (sq-hsyg; mirror of dashboard.js trendPoints) -----------------------
// Per-metric history points {date, value} across the committed window, oldest → newest.
// Points are only emitted for commits that actually carry the metric — no fabricated gaps.

export interface TrendPoint {
  date: number | null; // epoch-ms (as the series carries it)
  commit: string | null;
  value: number;
}

export interface TrendSeries {
  name: string;
  label: string;
  unit: string;
  suite: string;
  points: TrendPoint[];
}

function trendPointsFor(name: string): TrendPoint[] {
  const pts: TrendPoint[] = [];
  for (const entry of HISTORY) {
    const b = entry.benches.find((x) => x.name === name);
    if (b) {
      pts.push({
        date: entry.date == null ? null : Number(entry.date),
        commit: entry.commit,
        value: b.value,
      });
    }
  }
  return pts;
}

// Trend series for every metric in a family, ordered by suite then label (same order the
// metric tables use). A metric with a single point is kept (degenerate single-marker chart).
export function trendSeriesForFamily(key: string): TrendSeries[] {
  const series = benchesForFamily(key).map((b) => ({
    name: b.name,
    label: labelFor(b.name),
    unit: b.unit,
    suite: suiteFor(b.name),
    points: trendPointsFor(b.name),
  }));
  return series.sort((a, b) =>
    a.suite !== b.suite
      ? a.suite < b.suite
        ? -1
        : 1
      : a.label < b.label
        ? -1
        : a.label > b.label
          ? 1
          : 0,
  );
}

// ---- SCALING families (sq-hsyg; mirror of dashboard.js sizeAxisOf/buildScalingFamilies) -
// For SIZE-PARAMETRISED metrics (Deep Taxonomy depth, WatDiv scale factor, …) the harness
// encodes the size in the metric NAME (e.g. `deeptax_d10000_closure_s`). We split that size
// token out so two sizes of the same query share a `base` and form one scaling family, then
// plot the LATEST value vs the size axis. No fabricated points — only real metrics appear.

interface SizeAxis {
  base: string;
  axisLabel: string;
  axis: number;
}

const SIZE_TOKENS: { re: RegExp; label: string }[] = [
  { re: /_sf(\d+)([km]?)(?![0-9])/i, label: "scale factor" },
  { re: /_depth(\d+)(?![0-9])/i, label: "depth" },
  { re: /_d(\d+)(?![0-9])/i, label: "depth" },
  { re: /_scale(\d+)([km]?)(?![0-9])/i, label: "scale" },
  { re: /_size(\d+)([km]?)(?![0-9])/i, label: "size" },
];

function sizeAxisOf(name: string): SizeAxis | null {
  for (const t of SIZE_TOKENS) {
    const m = name.match(t.re);
    if (m) {
      let mag = parseInt(m[1], 10);
      const suffix = (m[2] || "").toLowerCase();
      if (suffix === "k") mag *= 1000;
      else if (suffix === "m") mag *= 1_000_000;
      // Removing the token leaves a placeholder `_`; collapse repeats + trim so the displayed
      // base stem renders cleanly (`deeptax_closure_s`). Two sizes still share one base.
      const base = name
        .replace(m[0], "_")
        .replace(/_{2,}/g, "_")
        .replace(/^_|_$/g, "");
      return { base, axisLabel: t.label, axis: mag };
    }
  }
  return null;
}

export interface ScalingPoint {
  axis: number;
  value: number;
  name: string;
  unit: string;
}

export interface ScalingFamily {
  base: string;
  label: string;
  suite: string;
  axisLabel: string;
  unit: string;
  points: ScalingPoint[];
}

// Group the LATEST commit's benches in a family into scaling families keyed by `base`. Each
// family's points are sorted ascending by the size axis. A family with a single point is kept
// (single marker). Families with NO size token never appear. Sorted by suite then label.
export function scalingFamiliesForFamily(key: string): ScalingFamily[] {
  const fams: Record<string, ScalingFamily> = {};
  for (const b of benchesForFamily(key)) {
    const s = sizeAxisOf(b.name);
    if (!s) continue;
    const fam =
      fams[s.base] ||
      (fams[s.base] = {
        base: s.base,
        label: labelFor(b.name).replace(/\s*\([^()]*\)\s*$/, "").trim(),
        suite: suiteFor(b.name),
        axisLabel: s.axisLabel,
        unit: b.unit,
        points: [],
      });
    fam.points.push({ axis: s.axis, value: b.value, name: b.name, unit: b.unit });
  }
  return Object.values(fams)
    .map((f) => {
      f.points.sort((a, b) => a.axis - b.axis);
      return f;
    })
    .sort((a, b) =>
      a.suite !== b.suite
        ? a.suite < b.suite
          ? -1
          : 1
        : a.label < b.label
          ? -1
          : a.label > b.label
            ? 1
            : 0,
    );
}

// ---- competitor-baseline COVERAGE matrix (sq-vw3ax.12) --------------------------------
// The honest at-a-glance answer to "which named competitors are (not) yet baselined on which
// SPARQL suite". Derived live from the committed same-box gathers — a cell is "measured" only
// when the engine produced a real timing; the missing engines/suites render "pending" (a
// canonical-host gather is greenlight-gated, never fabricated here). Server-side only (reads
// the snapshot); pass the result to the presentational component as a prop.
export function competitorCoverage(): CoverageMatrix {
  return deriveCoverage(COMPETITORS.same_box_comparisons || []);
}

// A one-line headline for a suite's collapsed group header.
export function summaryHeadline(suite: string, s: CompetitiveSummary): string {
  if (s.kind === "speedup") {
    const range =
      s.min === s.max
        ? `${s.min}×`
        : `${s.min}×–${s.max}× (median ${s.median}×)`;
    return `${range} faster than ${s.competitor} across ${suite}`;
  }
  if (s.kind === "same-box") {
    return `${s.canonical ? "canonical " : ""}same-box: sparq fastest on ${s.wins}/${s.total} count-checked queries (vs ${s.competitors.length} competitor${s.competitors.length === 1 ? "" : "s"})`;
  }
  if (s.kind === "pending") return "competitor baseline pending";
  return "sparq absolute numbers (no competitor baseline yet)";
}
