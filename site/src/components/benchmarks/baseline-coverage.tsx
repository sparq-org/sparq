// [OPUS-4.8] sq-vw3ax.12 — the competitor-baseline COVERAGE matrix on the SPARQL /benchmarks
// page. One row per recognised competitor engine, one column per recognised SPARQL query
// suite; each cell states — from REAL data only — whether a same-box baseline exists, was
// attempted but the engine failed to load, or is still pending a canonical-host gather.
//
// HONESTY (load-bearing): this component renders ONLY what deriveCoverage computed from the
// committed same-box gathers (data/benchmarks.ts competitorCoverage). A "pending" cell is the
// honest state for an engine/suite with no gather yet (Blazegraph / GraphDB / RDF4J, and the
// LUBM / BSBM query suites) — the canonical EC2 measurement is maintainer-greenlight-gated and
// is NEVER fabricated here. Presentational only: the matrix arrives as a prop.
import { Check, Hourglass, TriangleAlert } from "lucide-react";

import type { CoverageCell, CoverageMatrix } from "@/lib/baseline-coverage";

function modeLabel(mode: "cli" | "http" | "both"): string {
  return mode === "both"
    ? "CLI + HTTP"
    : mode === "http"
      ? "HTTP adapter"
      : "in-process CLI";
}

function CellContent({ cell }: { cell: CoverageCell }) {
  if (cell.state === "measured") {
    return (
      <span
        className="inline-flex items-center gap-1 text-[var(--success-on-tint)]"
        title={`Same-box ${cell.canonical ? "canonical " : ""}baseline — ${modeLabel(cell.mode)}${cell.host ? ` · ${cell.host}` : ""}${cell.gitCommit ? ` · git ${cell.gitCommit}` : ""}`}
      >
        <Check className="size-3.5" aria-hidden />
        <span className="text-xs">
          {cell.canonical ? "canonical" : "same-box"}
        </span>
      </span>
    );
  }
  if (cell.state === "failed") {
    return (
      <span
        className="inline-flex items-center gap-1 text-[color-mix(in_oklch,var(--warning)_80%,var(--foreground))]"
        title={cell.note}
      >
        <TriangleAlert className="size-3.5" aria-hidden />
        <span className="text-xs">failed</span>
      </span>
    );
  }
  return (
    <span
      className="inline-flex items-center gap-1 text-muted-foreground"
      title="No same-box baseline gathered yet — requires a maintainer-greenlit canonical-host (EC2) measurement; never fabricated"
    >
      <Hourglass className="size-3.5" aria-hidden />
      <span className="text-xs">pending</span>
    </span>
  );
}

export function BaselineCoverageMatrix({ matrix }: { matrix: CoverageMatrix }) {
  return (
    <section className="space-y-3">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold">Competitor-baseline coverage</h2>
        <p className="measure text-sm text-muted-foreground">
          Which recognised engines have a <strong className="text-foreground">same-box</strong>{" "}
          baseline on which recognised SPARQL query suite —{" "}
          <strong className="text-foreground">
            {matrix.measuredCount}/{matrix.totalCount}
          </strong>{" "}
          cells measured. A <em>pending</em> cell has no gather yet; those numbers require a
          canonical-host (EC2) measurement on the same hardware and are never fabricated. When a
          gather lands, the cell fills in automatically.
        </p>
      </div>
      <div className="overflow-x-auto rounded-lg ring-1 ring-foreground/10">
        <table className="w-full border-collapse text-sm">
          <thead>
            <tr className="border-b bg-muted/40 text-left">
              <th className="px-3 py-2 font-medium">Engine</th>
              {matrix.suites.map((s) => (
                <th key={s.id} className="px-3 py-2 text-center font-medium">
                  {s.label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {matrix.engines.map((e) => (
              <tr key={e.id} className="border-b last:border-0 hover:bg-muted/30">
                <td className="px-3 py-2 font-medium">{e.label}</td>
                {matrix.suites.map((s) => (
                  <td key={s.id} className="px-3 py-2 text-center">
                    <CellContent cell={matrix.cells[s.id][e.id]} />
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="text-xs text-muted-foreground">
        <span className="inline-flex items-center gap-1 text-[var(--success-on-tint)]">
          <Check className="size-3" aria-hidden /> same-box baseline exists
        </span>{" "}
        · <span className="italic">canonical</span> = dedicated quiet-box gather ·{" "}
        <span className="inline-flex items-center gap-1 text-[color-mix(in_oklch,var(--warning)_80%,var(--foreground))]">
          <TriangleAlert className="size-3" aria-hidden /> engine attempted but failed to load
        </span>{" "}
        ·{" "}
        <span className="inline-flex items-center gap-1">
          <Hourglass className="size-3" aria-hidden /> not yet measured (canonical gather
          pending)
        </span>
        . Hover a cell for its host / commit / measurement-mode provenance.
      </p>
    </section>
  );
}
