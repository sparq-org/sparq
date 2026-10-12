// [OPUS-4.8] sq-vjn4 / sq-1sa9r — the same-box cross-engine comparison table (SP2Bench /
// WatDiv: sparq vs Oxigraph / Fuseki / Virtuoso / QLever on ONE box). HONESTY metadata is
// surfaced, never hidden:
//   * per-engine measurement MODE (in-process CLI vs HTTP SPARQL adapter) — an asymmetry;
//   * a whole-engine failure (e.g. Fuseki load timeout) renders "failed", never blank;
//   * per-query solution COUNT cross-check (✓ agree / DIFF) is shown;
//   * canonical vs non-canonical provenance (host / scale / commit) is labelled.
import { fmtNum } from "@/lib/fmt-num";
import type { SameBoxComparison } from "@/data/benchmarks";

export function SameBoxTable({ comparison }: { comparison: SameBoxComparison }) {
  const engines = comparison.engines;
  const canonical = comparison.canonical === true;
  return (
    <div className="space-y-2">
      {/* Measurement-mode legend — the CLI-vs-HTTP asymmetry, stated up front. */}
      <ul className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
        {engines.map((e) => (
          <li key={e.id} className="flex items-center gap-1">
            <span className="font-medium text-foreground">{e.label}</span>
            {e.mode ? <span>· {e.mode}</span> : null}
            {e.status === "failed" ? (
              <span className="font-medium text-[var(--warning)]">· failed</span>
            ) : null}
          </li>
        ))}
      </ul>
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
                  title={[e.mode, e.env, e.failure].filter(Boolean).join(" — ")}
                >
                  {e.label}
                  {e.version ? (
                    <code className="ml-1 text-[11px] font-normal text-muted-foreground">
                      {e.version}
                    </code>
                  ) : null}
                </th>
              ))}
              <th className="px-3 py-2 text-center font-medium" title="engines agree on the solution count">
                counts
              </th>
            </tr>
          </thead>
          <tbody>
            {comparison.rows.map((r) => (
              <tr key={r.query} className="border-b last:border-0 hover:bg-muted/30">
                <td className="px-3 py-2 font-mono">
                  {r.query}
                  {r.corpus_variant || r.sparq_oracle_workload ? (
                    <sup
                      className="ml-0.5 cursor-help text-muted-foreground"
                      title={[
                        r.corpus_variant,
                        r.sparq_oracle_workload
                          ? `count oracle: ${r.sparq_oracle_workload}`
                          : null,
                      ]
                        .filter(Boolean)
                        .join(" — ")}
                    >
                      †
                    </sup>
                  ) : null}
                </td>
                <td className="px-3 py-2 text-right font-mono tabular-nums text-muted-foreground">
                  {fmtNum(r.rows)}
                </td>
                {engines.map((e) => {
                  const v = r.values[e.id];
                  const failed = e.status === "failed";
                  return (
                    <td
                      key={e.id}
                      className="px-3 py-2 text-right font-mono tabular-nums"
                    >
                      {failed ? (
                        <span
                          className="text-muted-foreground"
                          title={e.failure || "engine failed to load"}
                        >
                          failed
                        </span>
                      ) : v == null ? (
                        <span
                          className="text-muted-foreground"
                          title="engine did not produce a timing for this query"
                        >
                          n/a
                        </span>
                      ) : (
                        `${fmtNum(v)} ${r.unit}`
                      )}
                    </td>
                  );
                })}
                <td
                  className="px-3 py-2 text-center font-mono"
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
            ))}
          </tbody>
        </table>
      </div>
      <p className="text-xs text-muted-foreground">
        {canonical ? (
          <strong className="text-foreground">Canonical</strong>
        ) : (
          "Non-canonical"
        )}{" "}
        same-box {comparison.suite} gather · {comparison.scale} · min-of-{comparison.iters}{" "}
        · {comparison.env.host_class}
        {comparison.env.quiet_box ? " (quiet box)" : ""} · git{" "}
        <code className="text-[11px]">{comparison.git_commit}</code> · counts cross-checked
        engine-vs-engine and vs expected rows.{" "}
        {canonical
          ? "Absolute cross-engine ratios still include measurement-mode overhead (in-process CLI vs HTTP adapter) — read for cross-engine ordering."
          : "NON-CANONICAL (for cross-engine ordering, not an absolute reference)."}{" "}
        {comparison.combine}
      </p>
    </div>
  );
}
