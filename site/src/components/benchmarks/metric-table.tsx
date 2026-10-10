// [OPUS-4.8] sq-vjn4 — the per-suite metrics table (sparq absolute numbers, smaller is
// better). Pure presentational; no fabricated competitor columns — same-box competitor
// numbers are surfaced via the same-box table + the live summary, references separately.
import { fmtNum } from "@/lib/fmt-num";
import type { MetricRow } from "@/data/benchmarks";

export function MetricTable({ rows }: { rows: MetricRow[] }) {
  return (
    <div tabIndex={0} className="overflow-x-auto rounded-lg ring-1 ring-foreground/10">
      <table className="w-full border-collapse text-sm">
        <thead>
          <tr className="border-b bg-muted/40 text-left">
            <th className="px-3 py-2 font-medium">Benchmark</th>
            <th className="px-3 py-2 text-right font-medium">sparq</th>
            <th className="px-3 py-2 text-left font-medium">Unit</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.name} className="border-b last:border-0 hover:bg-muted/30">
              <td className="px-3 py-2">
                <span className="block">{r.label}</span>
                <code className="text-[11px] text-muted-foreground">{r.name}</code>
              </td>
              <td className="px-3 py-2 text-right font-mono tabular-nums">
                {fmtNum(r.value)}
              </td>
              <td className="px-3 py-2 text-muted-foreground">{r.unit}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
