import type { Metadata } from "next";
import { FileCode2 } from "lucide-react";

import { SurfaceContent } from "@/components/surface-content";
import { withBasePath } from "@/lib/base-path";

export const metadata: Metadata = {
  title: "SPARQL 1.1 / 1.2",
  description:
    "SELECT / ASK / CONSTRUCT / UPDATE, property paths, RDF 1.2 triple terms, aggregates, subqueries — the sparq SPARQL engine, live in your tab.",
};

// [OPUS-4.8] sq-vw3ax.8 — condensed to the scan-first template: one-line lead, ≤5
// capabilities, single-sentence runs note + reproduce, caveats behind <details>, and the
// universal README + SKILL.md links. The long-form algebra walk-through lives in the SKILL.
export default function SparqlSurfacePage() {
  return (
    <SurfaceContent
      icon={FileCode2}
      title="SPARQL 1.1 / 1.2"
      statement="The full query language — SELECT, ASK, CONSTRUCT, UPDATE — over the sparq engine."
      tier="live"
      intro={
        <p>
          sparq evaluates SPARQL 1.1/1.2 over an in-memory graph with a
          worst-case-optimal join engine; the same engine compiled to wasm ships as{" "}
          <code className="font-mono text-foreground">@sparq-org/sparq</code>, so you can
          run real queries in your tab with no server round-trip.
        </p>
      }
      capabilities={[
        {
          title: "All four query forms",
          body: "SELECT / ASK / CONSTRUCT / DESCRIBE, returning bindings, a boolean, or a constructed graph.",
        },
        {
          title: "SPARQL Update",
          body: "INSERT / DELETE / DELETE…WHERE and graph management, mutating the store in place.",
        },
        {
          title: "Property paths",
          body: "Sequence, alternative, inverse, and the transitive *, +, ? operators.",
        },
        {
          title: "RDF 1.2 triple terms",
          body: "Triple terms in object position, written <<( s p o )>>, with the triple-term functions.",
        },
        {
          title: "Aggregates & subqueries",
          body: "COUNT / SUM / AVG / MIN / MAX / GROUP_CONCAT, GROUP BY / HAVING, nested SELECT, plus EXPLAIN / EXPLAIN ANALYZE.",
        },
      ]}
      runsNote="Live in your browser tab — the lean wasm bundle carries the parser, triplestore and SPARQL engine, so queries run against the sample graph with no network round-trip."
      reproduce="cargo test -p sparq-engine"
      caveat={
        <p>
          REGEX / REPLACE are compiled out of the lean wasm bundle to keep it small
          (available in the native build). The query-budget deadline (wall-clock
          timeout) is native-only — the wasm bundle enforces a row cap instead — and{" "}
          <code className="font-mono">EXPLAIN ANALYZE</code> reports exact per-operator
          row counts but zero wall-times under wasm32 (no monotonic clock).
        </p>
      }
      readmeHref="https://github.com/sparq-org/sparq/tree/main/crates/sparq-engine"
      skillHref="https://github.com/sparq-org/sparq/blob/main/skills/sparql-query/SKILL.md"
      links={[{ href: withBasePath("/app/"), label: "Open the SPARQL workbench", external: true }]}
    />
  );
}
