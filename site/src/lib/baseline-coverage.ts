// [OPUS-4.8] sq-vw3ax.12 — the competitor-baseline COVERAGE matrix, derived live from the
// real same-box gathers. This is the honest, non-fabricating half of "the benchmarks page is
// missing many competitor baselines": instead of a missing competitor being silently absent,
// the matrix states, for each recognised SPARQL query suite × each recognised engine, whether
// a same-box baseline actually EXISTS (with its host/commit/measurement-mode), was ATTEMPTED
// but the engine failed to load, or is PENDING a canonical-host (EC2) gather.
//
// HONESTY (load-bearing):
//   * A "measured" cell is emitted ONLY when the engine produced ≥1 real timing in a committed
//     same_box_comparisons entry for that suite — never asserted from a catalog.
//   * Blazegraph / GraphDB / RDF4J, and the LUBM / BSBM query suites, have NO same-box gather
//     yet, so every one of their cells is "pending" — the honest state, not a fabricated ratio.
//   * The catalog of TARGET engines/suites is the set the benchmarks bead (sq-vw3ax.12) tracks;
//     the numbers themselves remain gated on a maintainer-greenlit canonical EC2 gather. When
//     that gather lands and writes a new same_box_comparisons entry (e.g. a `bsbm` suite, or a
//     `blazegraph` engine column), the corresponding cells flip to "measured" automatically —
//     no further site change, mirroring the SAMEBOX_SUITE_FAMILY seam in data/benchmarks.ts.
//
// This module is deliberately DEPENDENCY-FREE (type-only imports, erased at transpile) so the
// derivation is unit-testable without pulling in the ~1.3 MB generated snapshot.
import type { SameBoxComparison } from "@/data/benchmarks";

export interface CoverageEngine {
  id: string;
  label: string;
}

export interface CoverageSuite {
  id: string;
  label: string;
}

export type CoverageCell =
  | {
      state: "measured";
      canonical: boolean;
      host: string;
      gitCommit: string;
      // "cli" = in-process CLI only, "http" = HTTP SPARQL adapter only, "both" = measured in
      // both modes across the suite's gathers. Surfaced so the CLI-vs-HTTP asymmetry is honest.
      mode: "cli" | "http" | "both";
    }
  | { state: "failed"; note: string }
  | { state: "pending" };

export interface CoverageMatrix {
  engines: CoverageEngine[];
  suites: CoverageSuite[];
  // cells[suiteId][engineId]
  cells: Record<string, Record<string, CoverageCell>>;
  measuredCount: number;
  totalCount: number;
}

// The recognised SPARQL query suites the benchmarks bead tracks (SP2Bench / WatDiv / LUBM /
// BSBM). Suite ids match the same_box_comparisons `suite` field the ingest emits.
export const COVERAGE_SUITES: CoverageSuite[] = [
  { id: "sp2b", label: "SP2Bench" },
  { id: "watdiv", label: "WatDiv" },
  { id: "lubm", label: "LUBM" },
  { id: "bsbm", label: "BSBM" },
];

// The recognised competitor engines the bead tracks. Ids match the engine `id` the gather
// records. Oxigraph / QLever / Apache Jena (Fuseki) / Virtuoso have same-box SP2Bench + WatDiv
// gathers today; Blazegraph / GraphDB / RDF4J do not yet — their cells render "pending".
export const COVERAGE_ENGINES: CoverageEngine[] = [
  { id: "oxigraph", label: "Oxigraph" },
  { id: "qlever", label: "QLever" },
  { id: "fuseki", label: "Apache Jena (Fuseki)" },
  { id: "virtuoso", label: "Virtuoso" },
  { id: "blazegraph", label: "Blazegraph" },
  { id: "graphdb", label: "GraphDB" },
  { id: "rdf4j", label: "RDF4J" },
];

// All committed same-box gathers relevant to a coverage suite — the CLI matrix (`<suite>`) and
// its HTTP-mode twin (`<suite>-http`), unioned so an engine measured in EITHER mode counts.
function comparisonsForSuite(
  comparisons: SameBoxComparison[],
  suiteId: string,
): SameBoxComparison[] {
  const ids = [suiteId, `${suiteId}-http`];
  return comparisons.filter((c) => ids.includes(c.suite.toLowerCase()));
}

// Did `engineId` produce at least one real (positive) timing in this comparison's rows?
function producedTiming(c: SameBoxComparison, engineId: string): boolean {
  return c.rows.some((r) => {
    const v = r.values[engineId];
    return typeof v === "number" && v > 0;
  });
}

function modeOf(mode: string | undefined): "cli" | "http" {
  return /http/i.test(mode || "") ? "http" : "cli";
}

// Derive the full coverage matrix from the committed same-box comparisons. Pure — the caller
// (data/benchmarks.ts competitorCoverage) passes COMPETITORS.same_box_comparisons.
export function deriveCoverage(
  comparisons: SameBoxComparison[],
  suites: CoverageSuite[] = COVERAGE_SUITES,
  engines: CoverageEngine[] = COVERAGE_ENGINES,
): CoverageMatrix {
  const cells: Record<string, Record<string, CoverageCell>> = {};
  let measuredCount = 0;

  for (const suite of suites) {
    cells[suite.id] = {};
    const comps = comparisonsForSuite(comparisons, suite.id);
    for (const eng of engines) {
      let measured = false;
      let failed = false;
      let canonical = false;
      let host = "";
      let gitCommit = "";
      let failNote = "";
      const modes = new Set<"cli" | "http">();

      for (const c of comps) {
        const e = c.engines.find((x) => x.id === eng.id);
        if (!e) continue;
        if (producedTiming(c, eng.id)) {
          measured = true;
          if (c.canonical) canonical = true;
          if (c.env?.host_class) host = c.env.host_class;
          if (c.git_commit) gitCommit = c.git_commit;
          modes.add(modeOf(e.mode));
        } else if (e.status === "failed") {
          failed = true;
          if (!failNote) failNote = e.failure || "engine failed to load";
        }
      }

      let cell: CoverageCell;
      if (measured) {
        const mode: "cli" | "http" | "both" =
          modes.size === 2 ? "both" : modes.has("http") ? "http" : "cli";
        cell = { state: "measured", canonical, host, gitCommit, mode };
        measuredCount += 1;
      } else if (failed) {
        cell = { state: "failed", note: failNote };
      } else {
        cell = { state: "pending" };
      }
      cells[suite.id][eng.id] = cell;
    }
  }

  return {
    engines,
    suites,
    cells,
    measuredCount,
    totalCount: suites.length * engines.length,
  };
}
