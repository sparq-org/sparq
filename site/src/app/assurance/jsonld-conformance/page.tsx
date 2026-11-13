// Static public scoreboard for processor API and store ingest conformance.
import type { Metadata } from "next";
import { CheckCircle2, FlaskConical } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { jsonLdConformanceLanes, jsonLdEarlReport } from "@/data/jsonld-conformance";

export const metadata: Metadata = {
  title: "JSON-LD processor API and store ingest conformance",
  description: "Separate W3C JSON-LD measurements for the sparq-jsonld processor API and sparq-core Graph ingest over oxjsonld.",
};

export default function JsonLdConformancePage() {
  return (
    <div className="space-y-10">
      <header className="space-y-3">
        <div className="flex items-center gap-3">
          <span className="flex size-11 items-center justify-center rounded-xl bg-primary/10 text-primary">
            <FlaskConical className="size-5" aria-hidden />
          </span>
          <div>
            <h1 className="text-2xl font-semibold">JSON-LD 1.1 conformance floors</h1>
            <p className="text-sm text-muted-foreground">
              Processor API and store ingest measured separately against pinned W3C suites.
            </p>
          </div>
        </div>
        <Badge variant="outline">W3C JSON-LD 1.1</Badge>
      </header>

      <div className="max-w-3xl space-y-3 text-sm text-muted-foreground">
        <p>
          The JSON-LD 1.1 EARL report measures the <strong>sparq-jsonld processor API</strong>:
          expand, compact, flatten, frame, toRdf and fromRdf. It reports{" "}
          {jsonLdEarlReport.passed}/{jsonLdEarlReport.total} applicable cases passing,
          including {jsonLdEarlReport.toRdfPassed}/{jsonLdEarlReport.toRdfTotal} toRdf cases.
          It does not measure store ingest.
        </p>
        <p>
          Store ingest uses <strong>sparq-core Graph over oxjsonld</strong> and has its own{" "}
          <code>toRdf-ingest</code> ratchet, excluded from EARL. The switch to native ingest is
          tracked in{" "}
          <a href="https://github.com/sparq-org/sparq/issues/7342" className="text-primary underline underline-offset-4">
            #7342
          </a>.
          Until that switch lands, both paths have separate results below.
        </p>
      </div>

      <p className="max-w-3xl rounded-xl border border-amber-500/30 bg-amber-500/5 p-4 text-sm text-muted-foreground">
        <strong className="text-foreground">Measured floors, not conformance claims.</strong>{" "}
        Full-manifest floors below retain any 1.0-only compatibility entries. The toRdf
        runner uses sparq-jsonld for 1.1 cases and oxjsonld for 1.0-only compatibility cases. EARL omits 1.0-only entries and counts every
        other unexecuted entry as failed. A lane below its total is not claimed conformant.
        Store ingest runs all 467 toRdf entries without skips, using input and base only;
        unsupported options and remote loading are not emulated, and negatives check
        rejection rather than exact W3C error codes.
      </p>

      <section aria-labelledby="lanes-heading" className="space-y-4">
        <h2 id="lanes-heading" className="text-lg font-semibold">
          Processor API and store ingest ratchets
        </h2>
        <ul className="grid gap-3 sm:grid-cols-2">
          {jsonLdConformanceLanes.map((lane) => {
            const complete = lane.floor === lane.total;
            return (
              <li key={lane.id} className="rounded-xl border bg-card p-4">
                <div className="flex items-start justify-between gap-4">
                  <div>
                    <h3 className="font-semibold">{lane.label}</h3>
                    <p className="text-xs text-muted-foreground">{lane.component}</p>
                    <code className="text-xs text-muted-foreground">{lane.id}</code>
                  </div>
                  <div className="text-right">
                    <p className="text-xl font-semibold tabular-nums" aria-label={`${lane.floor} passes out of ${lane.total}`}>
                      {lane.floor}/{lane.total}
                    </p>
                    <p className="text-xs text-muted-foreground">measured floor</p>
                  </div>
                </div>
                {lane.id === "toRdf-ingest" && (
                  <p className="mt-3 text-xs text-muted-foreground">
                    Excluded from EARL; failure ceiling {lane.failureCeiling}.
                  </p>
                )}
                {complete ? (
                  <p className="mt-3 flex items-center gap-1.5 text-xs text-muted-foreground">
                    <CheckCircle2 className="size-3.5 text-[var(--success)]" aria-hidden />
                    Full score at the pinned revision
                  </p>
                ) : (
                  <p className="mt-3 text-xs text-muted-foreground">Not claimed conformant</p>
                )}
              </li>
            );
          })}
        </ul>
      </section>
    </div>
  );
}

