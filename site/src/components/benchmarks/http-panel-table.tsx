"use client";

// [OPUS-4.8] sq-7d3dj.34.3 — the canonical HTTP-mode panel (SP2Bench-HTTP / WatDiv-HTTP):
// EVERY engine measured in the SAME HTTP-server mode (SPARQL 1.1 over HTTP via a shared
// adapter), so this closes the CLI-vs-HTTP measurement-mode asymmetry of the CLI matrix
// above. Each engine cell shows the full-request time (primary) with time-to-first-byte
// (TTFB) beneath it. A connection-regime toggle switches keep-alive (the steady-state
// server measure, default) ⇄ fresh-connect (a new TCP connection per request).
//
// HONESTY (load-bearing, per the perf-dominance mandate + the empirical-honesty rule):
//   * The fastest count-checked value per row is emphasised REGARDLESS of engine — so where
//     a competitor leads (SP2Bench complex-shape queries; oxigraph/qlever first-byte TTFB
//     on large SELECTs) the competitor is highlighted, never sparq by default. No spin.
//   * Per-query solution-COUNT cross-check (✓ / DIFF) is shown; a DIFF row is excluded from
//     the win/loss count (timing not comparable) and its cells are not emphasised.
//   * The summary reports count-checked wins AND losses honestly (SP2Bench is BEHIND on ~half
//     the queries — shown as losses, not hidden).
import * as React from "react";

import { Badge } from "@/components/ui/badge";
import { fmtNum } from "@/lib/fmt-num";
import type {
  CompetitiveSummary,
  SameBoxComparison,
  SameBoxRow,
} from "@/data/benchmarks";

type Regime = "keep-alive" | "fresh-connect";

// Pull the (full, ttfb) value maps for a row in the selected connection regime. Keep-alive
// uses `values` / `values_ttfb`; fresh-connect uses the `*_fresh*` twins. Missing twins
// degrade to an empty map so a cell renders "n/a", never a fabricated number.
function regimeValues(
  row: SameBoxRow,
  regime: Regime,
): { full: Record<string, number | null>; ttfb: Record<string, number | null> } {
  if (regime === "fresh-connect") {
    return { full: row.values_fresh ?? {}, ttfb: row.values_fresh_ttfb ?? {} };
  }
  return { full: row.values, ttfb: row.values_ttfb ?? {} };
}

// The fastest engine id for a value map, considering ONLY count-checked rows and numeric
// positive values. Returns null on a DIFF row (count disagreed → not comparable) or when no
// engine produced a value — so nothing is emphasised in those cases.
function fastestId(
  row: SameBoxRow,
  vals: Record<string, number | null>,
): string | null {
  if (row.count_match === false) return null;
  let best = Infinity;
  let id: string | null = null;
  for (const [eng, v] of Object.entries(vals)) {
    if (typeof v === "number" && v > 0 && v < best) {
      best = v;
      id = eng;
    }
  }
  return id;
}

export function HttpPanelTable({
  comparison,
  summary,
}: {
  comparison: SameBoxComparison;
  summary?: CompetitiveSummary;
}) {
  const engines = comparison.engines;
  const primary: Regime =
    comparison.connection?.primary === "fresh-connect" ? "fresh-connect" : "keep-alive";
  const [regime, setRegime] = React.useState<Regime>(primary);
  const modesMeasured = comparison.connection?.modes_measured ?? ["keep-alive"];
  const canToggle = modesMeasured.includes("fresh-connect");

  const sb = summary && summary.kind === "same-box" ? summary : null;

  return (
    <div className="space-y-2">
      {/* Same-mode framing + the honest win/loss line (never spun). */}
      {sb ? (
        <p className="text-sm text-muted-foreground">
          All {engines.length} engines are measured in the{" "}
          <strong className="text-foreground">same HTTP-server mode</strong> (SPARQL 1.1 over
          HTTP via one shared adapter), so this panel closes the CLI-vs-HTTP measurement-mode
          asymmetry of the matrix above. Across{" "}
          <strong className="text-foreground">{sb.total}</strong> quer
          {sb.total === 1 ? "y" : "ies"} whose solution <em>count cross-checked</em>, sparq
          was fastest on <strong className="text-foreground">{sb.wins}</strong>
          {sb.losses > 0 ? (
            <>
              {" "}
              and a competitor was faster on{" "}
              <strong className="text-foreground">{sb.losses}</strong>
            </>
          ) : null}{" "}
          (fastest competitor ranged{" "}
          <strong className="text-foreground">
            {sb.min}×–{sb.max}×
          </strong>{" "}
          sparq&rsquo;s full-request time, median{" "}
          <strong className="text-foreground">{sb.median}×</strong>).
          {sb.diffQueries.length > 0 && (
            <>
              {" "}
              {sb.diffQueries.join(", ")}{" "}
              {sb.diffQueries.length === 1 ? "was" : "were"} excluded from the win/loss count
              because engines disagreed on the solution count.
            </>
          )}
        </p>
      ) : null}

      {/* Engine measurement-mode legend + the connection-regime toggle. */}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <ul className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
          {engines.map((e) => (
            <li key={e.id} className="flex items-center gap-1">
              <span className="font-medium text-foreground">{e.label}</span>
              {e.mode ? <span title={e.mode}>· HTTP</span> : null}
              {e.status === "failed" ? (
                <span className="font-medium text-[var(--warning)]">· failed</span>
              ) : null}
            </li>
          ))}
        </ul>
        {canToggle && (
          <div
            role="group"
            aria-label="connection regime"
            className="inline-flex items-center rounded-md ring-1 ring-foreground/15 text-xs"
          >
            {(["keep-alive", "fresh-connect"] as Regime[]).map((r) => (
              <button
                key={r}
                type="button"
                aria-pressed={regime === r}
                onClick={() => setRegime(r)}
                className={
                  "px-2.5 py-1 font-medium transition-colors first:rounded-l-md last:rounded-r-md " +
                  (regime === r
                    ? "bg-primary/10 text-primary"
                    : "text-muted-foreground hover:bg-muted/60")
                }
              >
                {r}
                {r === primary ? " ·default" : ""}
              </button>
            ))}
          </div>
        )}
      </div>

      <div tabIndex={0} className="overflow-x-auto rounded-lg ring-1 ring-foreground/10">
        <table className="w-full border-collapse text-sm">
          <thead>
            <tr className="border-b bg-muted/40 text-left">
              <th className="px-3 py-2 font-medium">Query</th>
              <th className="px-3 py-2 text-right font-medium">Rows</th>
              {engines.map((e) => (
                <th
                  key={e.id}
                  className="px-3 py-2 text-right font-medium"
                  title={[e.mode, e.env].filter(Boolean).join(" — ")}
                >
                  {e.label}
                  {e.version ? (
                    <code className="ml-1 text-[11px] font-normal text-muted-foreground">
                      {e.version}
                    </code>
                  ) : null}
                </th>
              ))}
              <th
                className="px-3 py-2 text-center font-medium"
                title="engines agree on the solution count"
              >
                counts
              </th>
            </tr>
          </thead>
          <tbody>
            {comparison.rows.map((r) => {
              const { full, ttfb } = regimeValues(r, regime);
              const fastFull = fastestId(r, full);
              const fastTtfb = fastestId(r, ttfb);
              return (
                <tr key={r.query} className="border-b last:border-0 hover:bg-muted/30">
                  <td className="px-3 py-2 font-mono align-top">{r.query}</td>
                  <td className="px-3 py-2 text-right font-mono tabular-nums align-top text-muted-foreground">
                    {fmtNum(r.rows)}
                  </td>
                  {engines.map((e) => {
                    const failed = e.status === "failed";
                    const fv = full[e.id];
                    const tv = ttfb[e.id];
                    return (
                      <td
                        key={e.id}
                        className="px-3 py-2 text-right font-mono tabular-nums align-top"
                      >
                        {failed ? (
                          <span
                            className="text-muted-foreground"
                            title={e.failure || "engine failed to load"}
                          >
                            failed
                          </span>
                        ) : fv == null ? (
                          <span
                            className="text-muted-foreground"
                            title="engine did not produce a timing for this query"
                          >
                            n/a
                          </span>
                        ) : (
                          <div className="flex flex-col items-end leading-tight">
                            <span
                              className={
                                fastFull === e.id
                                  ? "font-semibold text-foreground"
                                  : undefined
                              }
                              title={
                                fastFull === e.id
                                  ? "fastest full-request on this count-checked query"
                                  : undefined
                              }
                            >
                              {fmtNum(fv)} {r.unit}
                            </span>
                            <span
                              className={
                                "text-[11px] " +
                                (fastTtfb === e.id
                                  ? "font-semibold text-foreground"
                                  : "text-muted-foreground")
                              }
                              title={
                                fastTtfb === e.id
                                  ? "earliest first byte (TTFB) on this count-checked query"
                                  : "time to first byte"
                              }
                            >
                              TTFB {tv == null ? "—" : fmtNum(tv)}
                            </span>
                          </div>
                        )}
                      </td>
                    );
                  })}
                  <td
                    className="px-3 py-2 text-center font-mono align-top"
                    title={
                      r.count_match === false
                        ? "engines disagreed on the solution count for this query — timing not comparable"
                        : r.count_match
                          ? "all engines that produced a count agree (and match the expected rows)"
                          : "count not cross-checked"
                    }
                  >
                    {r.count_match == null ? "—" : r.count_match ? "✓" : "DIFF"}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {/* TTFB / streaming honesty callout — a genuine axis where sparq is BEHIND. */}
      <p className="text-xs text-muted-foreground">
        <Badge variant="muted" className="mr-1 align-middle">
          TTFB
        </Badge>
        time-to-first-byte is shown beneath each full-request time. On a large SELECT an
        engine can win end-to-end (full-request) while another delivers its first byte
        earlier — a distinct first-byte-latency axis. Where a competitor&rsquo;s TTFB or
        full-request is fastest, that competitor&rsquo;s value is the emphasised one.
      </p>

      <p className="text-xs text-muted-foreground">
        {comparison.canonical === true ? (
          <strong className="text-foreground">Canonical</strong>
        ) : (
          "Non-canonical"
        )}{" "}
        same-box HTTP panel · {comparison.scale} · min-of-{comparison.iters} per regime ·{" "}
        {comparison.env.host_class}
        {comparison.env.quiet_box ? " (quiet box)" : ""} · git{" "}
        <code className="text-[11px]">{comparison.git_commit}</code> · counts cross-checked
        engine-vs-engine and vs expected rows.
        {comparison.connection?.note ? " " + comparison.connection.note : ""}
        {comparison.combine ? " " + comparison.combine : ""}
      </p>
    </div>
  );
}
