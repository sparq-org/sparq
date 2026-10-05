// [OPUS-4.8] sq-gum8 — per-paper page. Static-exported via generateStaticParams over
// src/data/papers.ts. Renders: the in-site HTML (built from the same .typ + evidence as the
// PDF, read at build time from src/generated/papers/<slug>.html), a "Download PDF" button
// (basePath-prefixed static asset), and a provenance stamp. No client JS / WASM compiler.
import { readFileSync } from "node:fs";
import { join } from "node:path";

import type { Metadata } from "next";
import { notFound } from "next/navigation";
import Link from "next/link";
import { ArrowLeft, Download, FileCode } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { PaperHtml } from "@/components/papers/paper-html";
import {
  PaperProvenance,
  type Provenance,
} from "@/components/papers/paper-provenance";
import {
  PAPERS,
  paperBySlug,
  STATUS_LABEL,
  STATUS_VARIANT,
  FAMILY_LABEL,
} from "@/data/papers";
import { LATEST, GENERATED_AT } from "@/data/benchmarks";
import evidence from "@/data/paper-evidence.json";
import { withBasePath } from "@/lib/base-path";

export function generateStaticParams() {
  return PAPERS.map((p) => ({ slug: p.slug }));
}

export const dynamicParams = false;

export async function generateMetadata({
  params,
}: {
  params: Promise<{ slug: string }>;
}): Promise<Metadata> {
  const { slug } = await params;
  const paper = paperBySlug(slug);
  if (!paper) return {};
  return { title: paper.title, description: paper.blurb };
}

// Read the build-time-generated HTML fragment. If it is missing (e.g. a local dev run with
// no Typst installed produced only a placeholder), fall back to an honest notice.
function readPaperHtml(slug: string): string {
  try {
    return readFileSync(
      join(process.cwd(), "src", "generated", "papers", `${slug}.html`),
      "utf8",
    );
  } catch {
    return `<p>This paper has not been built yet. Run <code>npm run build-papers</code> with the Typst CLI installed.</p>`;
  }
}

function provenanceFor(): Provenance {
  const records = Object.values(
    (evidence as { records: Record<string, { environment: string }> }).records,
  );
  const canonical = records.filter((r) => r.environment === "canonical").length;
  return {
    commit: LATEST.commit ? LATEST.commit.slice(0, 8) : "unknown",
    generatedAt: GENERATED_AT ?? "",
    canonical,
    indicative: records.length - canonical,
  };
}

// basePath-aware PDF asset link. [OPUS-4.8] sq-9vw5 — env-switched (was hardcoded `/sparq`)
// so the PDF resolves under both the Pages `/sparq` prefix and the Tauri root-relative build.
function pdfHref(slug: string): string {
  return withBasePath(`/papers/${slug}.pdf`);
}

// [OPUS-4.8] sq-1scgk — the paper's single Typst source on GitHub: the authoring artifact the
// PDF AND the in-site HTML both compile from (build-papers.mjs), so it is the real repro anchor.
// Matches the site's existing source-link convention (sparq-org/sparq blob/main — see the /surface/*
// readmeHref/skillHref links). An absolute external link, so no basePath prefix.
function sourceHref(source: string): string {
  return `https://github.com/sparq-org/sparq/blob/main/site/papers/${source}`;
}

export default async function PaperPage({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = await params;
  const paper = paperBySlug(slug);
  if (!paper) notFound();

  const html = readPaperHtml(slug);
  const prov = provenanceFor();

  return (
    <div className="space-y-6">
      <Button variant="ghost" size="sm" asChild className="-ml-2">
        <Link href="/papers">
          <ArrowLeft className="size-4" aria-hidden />
          All papers
        </Link>
      </Button>

      <header className="space-y-3">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant={STATUS_VARIANT[paper.status]}>
            {STATUS_LABEL[paper.status]}
          </Badge>
          <Badge variant="muted">{FAMILY_LABEL[paper.family]}</Badge>
          <span className="text-xs text-muted-foreground">{paper.venue}</span>
        </div>
        <div className="flex flex-wrap items-start justify-between gap-3">
          <p className="measure text-sm text-muted-foreground">{paper.blurb}</p>
          <Button asChild size="sm" className="shrink-0">
            <a href={pdfHref(paper.slug)} download>
              <Download className="size-4" aria-hidden />
              Download PDF
            </a>
          </Button>
        </div>
      </header>

      {/* [OPUS-4.8] sq-1scgk — Artifacts & reproduction. Surfaces the paper's real artifacts:
          the downloadable PDF and the single Typst source the PDF + in-site render both compile
          from (the repro anchor). Only existing artifacts are linked — no invented metadata; the
          per-number evidence provenance is stamped by <PaperProvenance> below. */}
      <section
        aria-labelledby="artifacts-heading"
        className="rounded-lg border bg-muted/30 p-4"
      >
        <h2 id="artifacts-heading" className="text-sm font-semibold">
          Artifacts &amp; reproduction
        </h2>
        <div className="mt-3 flex flex-wrap items-center gap-x-5 gap-y-2 text-sm">
          <a
            href={pdfHref(paper.slug)}
            download
            className="inline-flex items-center gap-1.5 text-primary"
          >
            <Download className="size-3.5" aria-hidden />
            PDF
          </a>
          <a
            href={sourceHref(paper.source)}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1.5 text-muted-foreground hover:text-foreground"
          >
            <FileCode className="size-3.5" aria-hidden />
            Typst source
          </a>
        </div>
        <p className="mt-3 text-xs text-muted-foreground">
          The PDF and the in-site render below compile from the same single Typst source, fed the
          same paper-bound evidence, so the two cannot disagree. Every headline number traces to a
          named test or dataset, gated to deterministic, machine-independent evidence — see the
          provenance stamp at the foot of the page.
        </p>
      </section>

      <PaperHtml html={html} />

      <PaperProvenance prov={prov} />
    </div>
  );
}
