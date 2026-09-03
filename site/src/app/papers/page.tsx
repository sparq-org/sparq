// [OPUS-4.8] sq-gum8 — /papers index. One card per registered paper (data-driven off
// src/data/papers.ts), with a status badge, the contribution family, the honest evidence
// line, a link to the per-paper page, and a direct PDF link. An honest provenance line up
// top frames the whole section.
import type { Metadata } from "next";
import Link from "next/link";
import { ArrowRight, FileText, FileCode, Download, ScrollText } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  PAPERS,
  STATUS_LABEL,
  STATUS_VARIANT,
  FAMILY_LABEL,
} from "@/data/papers";
import { withBasePath } from "@/lib/base-path";

export const metadata: Metadata = {
  title: "Papers",
  description:
    "Academic papers generated from sparq contributions — each authored once in Typst, bound to verified deterministic or provenance-complete canonical timing evidence, and built into both an in-site render and a downloadable PDF.",
};

// basePath-aware PDF asset link. [OPUS-4.8] sq-9vw5 — env-switched (was hardcoded `/sparq`)
// so the PDF resolves under both the Pages `/sparq` prefix and the Tauri root-relative build.
function pdfHref(slug: string): string {
  return withBasePath(`/papers/${slug}.pdf`);
}

// [OPUS-4.8] sq-1scgk — the paper's single Typst source on GitHub (the authoring artifact the
// PDF + in-site render both compile from). Matches the site's source-link convention.
function sourceHref(source: string): string {
  return `https://github.com/sparq-org/sparq/blob/main/site/papers/${source}`;
}

export default function PapersIndexPage() {
  return (
    <div className="space-y-8">
      <header className="space-y-3">
        <div className="flex items-center gap-3">
          <span className="flex size-11 items-center justify-center rounded-xl bg-primary/10 text-primary">
            <ScrollText className="size-5" aria-hidden />
          </span>
          <div>
            <h1 className="text-2xl font-semibold">Papers</h1>
            <p className="text-sm text-muted-foreground">
              Academic write-ups of sparq contributions — single-source Typst, bound to live
              evidence, with a downloadable PDF.
            </p>
          </div>
        </div>
      </header>

      <section className="measure space-y-3 text-sm text-muted-foreground">
        <p>
          Each paper is authored once as a Typst source and built into both the in-site render
          below and a downloadable PDF, fed the same paper-bound evidence so the two cannot
          disagree. Headline evidence has two explicit classes: deterministic, machine-independent
          invariants and fixed artifact facts; or controlled, machine-dependent canonical timing
          whose host, source commit, protocol, run ID, and collection time remain attached to the
          rendered value. A build-time honesty gate blocks indicative work-box measurements from
          headline results and admits timing SVGs only through a digest-bound provenance helper.
        </p>
      </section>

      <section className="grid gap-4">
        {PAPERS.map((p) => (
          <Card key={p.slug} className="transition-colors hover:ring-foreground/20">
            <CardHeader>
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant={STATUS_VARIANT[p.status]}>
                  {STATUS_LABEL[p.status]}
                </Badge>
                <Badge variant="muted">{FAMILY_LABEL[p.family]}</Badge>
                <span className="text-xs text-muted-foreground">{p.venue}</span>
              </div>
              <CardTitle className="text-base leading-snug">{p.title}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <CardDescription className="leading-relaxed">{p.blurb}</CardDescription>
              <p className="text-xs text-muted-foreground">
                <strong className="text-foreground">Evidence:</strong> {p.evidence}
              </p>
              <div className="flex flex-wrap items-center gap-4 pt-1">
                <Link
                  href={`/papers/${p.slug}`}
                  className="inline-flex items-center gap-1 text-sm text-primary"
                >
                  <FileText className="size-3.5" aria-hidden />
                  Read paper
                  <ArrowRight className="size-3.5" aria-hidden />
                </Link>
                <a
                  href={pdfHref(p.slug)}
                  className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
                >
                  <Download className="size-3.5" aria-hidden />
                  PDF
                </a>
                {/* [OPUS-4.8] sq-1scgk — the single Typst source (the repro/artifact anchor). */}
                <a
                  href={sourceHref(p.source)}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
                >
                  <FileCode className="size-3.5" aria-hidden />
                  Source
                </a>
              </div>
            </CardContent>
          </Card>
        ))}
      </section>
    </div>
  );
}
