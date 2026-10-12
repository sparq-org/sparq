// [OPUS-4.8] sq-1s2.1.3 — the comprehensive SPARQL → ZK gate-cost catalog on the in-site
// /benchmarks/zk page. Renders the canonical catalog (src/data/zk-sparql-catalog.generated
// .json, synced from bench/zk-compose/sparql_feature_catalog.json) as a coverage table
// grouped by SPARQL-1.1 feature area: query shape → ZK circuit member(s) → circuit_size
// gate count → coverage status. NO gate count is hard-coded here — every number is read
// from the catalog via the typed accessor, so the page auto-reflects the canonical data.
//
// HONESTY (load-bearing):
//   - Gate counts are DETERMINISTIC circuit-SIZE metrics (`bb gates -s ultra_honk`), NOT
//     performance/throughput. The header + each tooltip frame them as such.
//   - High-gate rows are flagged in TWO honest categories: HIGH_GATE_blake3_binding (the
//     value-hook reduction TARGET) vs HIGH_GATE_lattice (large scan/join lattice corners —
//     big for a different reason). They are visually distinct.
//   - "NO ZK CIRCUIT YET (gap)" rows are greyed and labelled "not yet ZK-provable"; they
//     carry no gate number (never fabricated).
//   - The value-hook before→after (17,416 → ~3,200) is a PROJECTION, surfaced from the
//     catalog's own self-labelled `projected_after` ESTIMATE string and rendered as
//     projected-pending-audit (bb gates re-measurement + external audit, CR-G8 / sq-qhy4),
//     NEVER as an achieved result.
//
// PRIVACY-CLAIMS GATE (sq-qhy4): the v1 composition verifier is NOT externally audited and
// is research-grade only — a passing proof is not a soundness or privacy guarantee. The
// banner surfaces that caveat verbatim. (This file is scanned by check-privacy-claims.sh.)
import { CircleAlert, FlaskConical, Layers, ShieldQuestion } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import {
  ZK_SPARQL_CATALOG,
  ZK_SPARQL_SUMMARY,
  ZK_SPARQL_GATE_TOOL,
  ZK_SPARQL_BB_VERSION,
  ZK_SPARQL_NARGO_VERSION,
  blake3ReductionProjection,
  type CatalogEntry,
  type CatalogStatus,
} from "@/data/zk-sparql-catalog";

/** Coarse SPARQL feature-area grouping, inferred from the catalog query id (QNN_…). */
type FeatureGroup =
  | "Basic graph patterns"
  | "Numeric & typed FILTER"
  | "Join & property paths"
  | "Composition (OPTIONAL / UNION / subquery / …)"
  | "Aggregation, BIND & negation"
  | "Hidden-credential primitives";

function groupOf(id: string): FeatureGroup {
  if (/^Q0[12]_/.test(id)) return "Basic graph patterns";
  if (/^Q0[3-9]_filter/.test(id)) return "Numeric & typed FILTER";
  if (/^Q1[2-8]_/.test(id) || /_join_/.test(id) || /_path_/.test(id))
    return "Join & property paths";
  if (/^Q(10|11|15|20|22)_/.test(id))
    return "Composition (OPTIONAL / UNION / subquery / …)";
  if (/^Q(19|21|23)_/.test(id)) return "Aggregation, BIND & negation";
  return "Hidden-credential primitives";
}

const GROUP_ORDER: FeatureGroup[] = [
  "Basic graph patterns",
  "Numeric & typed FILTER",
  "Join & property paths",
  "Composition (OPTIONAL / UNION / subquery / …)",
  "Aggregation, BIND & negation",
  "Hidden-credential primitives",
];

function StatusBadge({ status }: { status: CatalogStatus }) {
  if (status === "covered")
    return (
      <Badge variant="success" className="font-normal">
        covered
      </Badge>
    );
  if (status === "partial")
    return (
      <Badge variant="muted" className="font-normal">
        partial
      </Badge>
    );
  return (
    <Badge variant="outline" className="font-normal text-muted-foreground">
      no circuit yet
    </Badge>
  );
}

/** The high-gate category chip — distinguishes the two honest reasons a row is big. */
function GateFlag({ entry }: { entry: CatalogEntry }) {
  if (entry.flag === "HIGH_GATE_blake3_binding")
    return (
      <Badge
        variant="warning"
        className="gap-1 font-normal"
        title="High gate count driven by the blake3 token-binding — the value-hook reduction TARGET (projection only, pending bb-gates re-measurement + external audit)."
      >
        <FlaskConical aria-hidden />
        blake3-binding · reduction target
      </Badge>
    );
  if (entry.flag === "HIGH_GATE_lattice")
    return (
      <Badge
        variant="muted"
        className="gap-1 font-normal"
        title="High gate count at a scan/join (k,n,r)/(na,nb) lattice corner — large for a different reason than blake3-binding; not a value-hook target."
      >
        <Layers aria-hidden />
        scan/join lattice corner
      </Badge>
    );
  return null;
}

/** A gate count, or an honest em-dash for partial/gap rows (never a fabricated number). */
function gates(n: number | null): string {
  return n == null ? "—" : n.toLocaleString();
}

function CatalogRow({ entry }: { entry: CatalogEntry }) {
  const isGap = entry.status === "gap";
  return (
    // [FABLE-5] sq-mcsbp — WCAG 2.1 AA color-contrast: a row-wide `opacity-60` on gap
    // rows composited EVERY descendant down below the AA 4.5:1 threshold (axe measured
    // ~#5a686d / #687073 on #081116 for the code, the "not yet ZK-provable" italics and
    // the status badge). Opacity affects contrast for all children, so it cannot coexist
    // with AA text. Signal the gap row with a subtle tinted background + left rail instead
    // of dimming the text — the text keeps its AA-compliant colours (set on each element).
    <tr
      className={`border-b align-top last:border-0 hover:bg-muted/30 ${
        isGap ? "bg-muted/20 border-l-2 border-l-foreground/25" : ""
      }`}
    >
      <td className="px-3 py-2.5">
        <span className="block text-[13px] font-medium">{entry.feature}</span>
        {/* [FABLE-5] text-foreground/70 (not muted-foreground) clears WCAG AA 4.5:1 on the dark card bg; muted-foreground was 3.29:1 here (color-contrast, sq-0rbfn) */}
        <code className="mt-0.5 block whitespace-pre-wrap break-words font-mono text-[11.5px] text-foreground/70">
          {entry.sparql}
        </code>
        {isGap && (
          <span className="mt-1 inline-block text-[11px] italic text-foreground/70">
            not yet ZK-provable
          </span>
        )}
      </td>
      <td className="px-3 py-2.5">
        {entry.members.length === 0 ? (
          <span className="text-[12px] text-muted-foreground">
            {entry.status === "partial"
              ? "verifier-side / desugars to a covered primitive"
              : "—"}
          </span>
        ) : (
          <ul className="space-y-0.5">
            {entry.members.map((m) => (
              <li key={m} className="font-mono text-[11.5px]">
                {m}
                {entry.perMember && entry.perMember[m] != null && (
                  <span className="ml-1.5 tabular-nums text-muted-foreground">
                    {entry.perMember[m].toLocaleString()}
                  </span>
                )}
              </li>
            ))}
          </ul>
        )}
      </td>
      <td className="px-3 py-2.5 text-right">
        <span className="font-mono tabular-nums">{gates(entry.circuitSize)}</span>
        {entry.range && entry.range[0] !== entry.range[1] && (
          <span className="block text-[11px] text-muted-foreground">
            range {entry.range[0].toLocaleString()}–
            {entry.range[1].toLocaleString()}
          </span>
        )}
      </td>
      <td className="px-3 py-2.5">
        <div className="flex flex-col items-start gap-1">
          <StatusBadge status={entry.status} />
          <GateFlag entry={entry} />
        </div>
      </td>
    </tr>
  );
}

export function ZkSparqlCatalog() {
  const proj = blake3ReductionProjection();
  // Group the catalog rows in a stable feature-area order; preserve catalog order within.
  const grouped = GROUP_ORDER.map((g) => ({
    group: g,
    rows: ZK_SPARQL_CATALOG.filter((e) => groupOf(e.id) === g),
  })).filter((s) => s.rows.length > 0);

  return (
    <section className="space-y-4">
      <header className="space-y-1.5">
        <h2 className="text-lg font-semibold">
          SPARQL feature → ZK gate-cost catalog
        </h2>
        <p className="measure text-sm text-muted-foreground">
          A coverage map across SPARQL&nbsp;1.1: for each query shape, which ZK circuit
          member(s) it compiles to today and that member&rsquo;s circuit size. The numbers
          are <strong>deterministic</strong>{" "}
          <code className="font-mono">{ZK_SPARQL_GATE_TOOL}</code> circuit-size metrics (bb{" "}
          <code className="font-mono">{ZK_SPARQL_BB_VERSION}</code>, nargo{" "}
          <code className="font-mono">{ZK_SPARQL_NARGO_VERSION}</code>) joined from the
          regression-gated gate snapshot — <strong>not</strong> a throughput or wall-clock
          measurement. Gaps carry no number (never fabricated).
        </p>
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <Badge variant="muted" className="font-normal">
            {ZK_SPARQL_SUMMARY.total_queries} queries
          </Badge>
          <Badge variant="success" className="font-normal">
            {ZK_SPARQL_SUMMARY.covered} covered
          </Badge>
          <Badge variant="muted" className="font-normal">
            {ZK_SPARQL_SUMMARY.partial} partial
          </Badge>
          <Badge variant="outline" className="font-normal">
            {ZK_SPARQL_SUMMARY.gaps} no circuit yet
          </Badge>
        </div>
      </header>

      {/* Honesty banner — the privacy-claims gate (the verifier is NOT externally audited). */}
      <div className="rounded-lg bg-[color-mix(in_oklch,var(--warning)_10%,transparent)] p-3 text-sm ring-1 ring-[var(--warning)]/25">
        <p className="flex items-start gap-2 text-muted-foreground">
          <CircleAlert className="mt-0.5 size-4 shrink-0 text-[var(--warning)]" />
          <span>
            <strong className="text-foreground">
              Research-grade, not externally audited.
            </strong>{" "}
            These are gate-count (circuit-size) figures, not a performance benchmark and
            not an audited cryptographic guarantee. The v1 composition verifier is
            research-grade and has <strong>not</strong> been externally audited (bead
            sq-qhy4); a covered row is not a soundness or privacy guarantee to a relying
            party.
          </span>
        </p>
      </div>

      {/* Legend for the two high-gate categories + the gap state. */}
      <div className="grid gap-2 text-xs sm:grid-cols-3">
        <div className="flex items-start gap-2 rounded-lg border bg-card p-2.5">
          <FlaskConical className="mt-0.5 size-3.5 shrink-0 text-[var(--warning)]" aria-hidden />
          <span className="text-muted-foreground">
            <strong className="text-foreground">blake3-binding</strong> — the numeric-FILTER
            family. High because of the blake3 token-binding; the value-hook{" "}
            <strong>reduction target</strong>.
          </span>
        </div>
        <div className="flex items-start gap-2 rounded-lg border bg-card p-2.5">
          <Layers className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" aria-hidden />
          <span className="text-muted-foreground">
            <strong className="text-foreground">scan/join lattice corner</strong> — big for
            a different reason (the (k,n,r)/(na,nb) lattice), not a value-hook target.
          </span>
        </div>
        <div className="flex items-start gap-2 rounded-lg border bg-card p-2.5">
          <ShieldQuestion className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" aria-hidden />
          <span className="text-muted-foreground">
            <strong className="text-foreground">no circuit yet</strong> — greyed; the
            feature is <strong>not yet ZK-provable</strong>. No gate number.
          </span>
        </div>
      </div>

      {/* The grouped coverage table. */}
      <div tabIndex={0} className="overflow-x-auto rounded-lg ring-1 ring-foreground/10">
        <table className="w-full border-collapse text-sm">
          <thead>
            <tr className="border-b bg-muted/40 text-left">
              <th className="px-3 py-2 font-medium">SPARQL feature / query shape</th>
              <th className="px-3 py-2 font-medium">
                ZK circuit member(s){" "}
                <span className="font-normal text-muted-foreground">(per-member gates)</span>
              </th>
              <th className="px-3 py-2 text-right font-medium">
                Gates{" "}
                <span className="font-normal text-muted-foreground">(circuit_size)</span>
              </th>
              <th className="px-3 py-2 font-medium">Status</th>
            </tr>
          </thead>
          {grouped.map((s) => (
            <tbody key={s.group}>
              <tr className="bg-muted/20">
                <th
                  colSpan={4}
                  className="px-3 py-1.5 text-left text-[11px] font-semibold uppercase tracking-wide text-muted-foreground/90"
                >
                  {s.group}
                </th>
              </tr>
              {s.rows.map((e) => (
                <CatalogRow key={e.id} entry={e} />
              ))}
            </tbody>
          ))}
        </table>
      </div>

      {/* The value-hook PROJECTION — explicitly projected, never achieved. */}
      {proj && (
        <div className="rounded-lg border border-dashed bg-muted/20 p-4 text-sm">
          <p className="mb-1.5 flex items-center gap-2 font-medium">
            <FlaskConical className="size-4 text-[var(--warning)]" aria-hidden />
            Value-hook reduction target (projection — not yet measured)
          </p>
          <p className="measure text-muted-foreground">
            The numeric-FILTER family currently measures{" "}
            <span className="font-mono tabular-nums text-foreground">
              {proj.measuredCeiling.toLocaleString()}
            </span>{" "}
            gates (driven by the blake3 token-binding, gate-identical across digit count).
            The field-native value-hook encoding is a <strong>projected</strong> reduction
            {proj.projectedGates && (
              <>
                {" "}
                to{" "}
                <span className="font-mono text-foreground">
                  ~{proj.projectedGates}
                </span>
              </>
            )}{" "}
            gates — an{" "}
            <strong className="text-foreground">
              estimate that must be re-measured with{" "}
              <code className="font-mono">bb gates</code>
            </strong>
            ; it is <strong>not</strong> an achieved result, and lands only after the
            external audit (CR-G8 / sq-qhy4).
            {proj.floorMember && proj.floorCircuitSize != null && (
              <>
                {" "}
                For context, the raw-compare floor member{" "}
                <code className="font-mono text-foreground">{proj.floorMember}</code>{" "}
                (no token-binding) measures{" "}
                <span className="font-mono tabular-nums text-foreground">
                  {proj.floorCircuitSize.toLocaleString()}
                </span>{" "}
                gates — a measured lower bound on the achievable size.
              </>
            )}
          </p>
          <p className="mt-1.5 text-[11px] text-muted-foreground">
            Projection string from the canonical catalog:{" "}
            <span className="italic">&ldquo;{proj.projectedRaw}&rdquo;</span>
          </p>
        </div>
      )}

      <p className="text-xs text-muted-foreground">
        Source: <code className="font-mono">bench/zk-compose/sparql_feature_catalog.json</code>{" "}
        (regenerated by <code className="font-mono">scripts/sparql_catalog.py</code>; every
        covered <code className="font-mono">circuit_size</code> is joined from
        <code className="font-mono"> gate_count_snapshot.json</code> and regression-gated,
        so it cannot drift).
      </p>
    </section>
  );
}
