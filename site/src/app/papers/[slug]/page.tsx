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
  type Paper,
} from "@/data/papers";
import baseEvidence from "@/data/paper-evidence.json";
import timingEvidence from "@/data/paper-evidence.canonical-timing.generated.json";
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

interface EvidenceRecord {
  environment: string;
  kind?: string;
  papers?: string[];
  timing_provenance?: {
    study_id: string;
    run_id: string;
    collected_at_utc: string;
    source_git_commit: string;
    analysis_git_commit: string;
    host_class: string;
    host_label: string;
    tenancy: string;
    noise_limitation: string;
    workload: string;
    dataset: string;
    query_scope: string;
    protocol: string;
    publisher_path: string;
    publisher_sha256: string;
    input_file_count: number;
    raw_archive_kind: "committed" | "external";
    raw_archive_location: string;
    raw_archive_public_url: string;
    raw_archive_sha256: string;
    raw_archive_bytes: number;
    raw_archive_build_verification: "local-rehash" | "descriptor-only";
    raw_archive_member_verification_authority: "publisher-recorded";
    raw_archive_manifest_member: string;
    raw_archive_manifest_sha256: string;
    raw_archive_manifest_bytes: number;
    raw_archive_manifest_entry_count: number;
    raw_archive_regular_members: number;
    raw_archive_all_members_rehashed: true;
    raw_archive_exact_member_set: true;
    raw_archive_sanitization_scan_passed: true;
    raw_archive_deterministic_tar_headers: true;
    raw_archive_zstd_version: string;
    raw_archive_zstd_executable_sha256: string;
  };
}

function provenanceFor(paper: Paper): Provenance {
  const records = {
    ...(baseEvidence as { records: Record<string, EvidenceRecord> }).records,
    ...(timingEvidence as { records: Record<string, EvidenceRecord> }).records,
  };
  const prefixes = paper.evidencePrefixes;
  const scoped = Object.entries(records).filter(([key, record]) => {
    const prefixMatch = !prefixes?.length || prefixes.some((prefix) => key.startsWith(prefix));
    if (!prefixMatch) return false;
    return record.environment !== "canonical-timing" || record.papers?.includes(paper.slug) === true;
  });
  const timingRuns = new Map<string, Provenance["timingRuns"][number]>();
  for (const [, record] of scoped) {
    const run = record.timing_provenance;
    if (record.environment !== "canonical-timing" || !run) continue;
    const id = `${run.study_id}:${run.run_id}:${run.source_git_commit}:${run.analysis_git_commit}`;
    timingRuns.set(id, {
      studyId: run.study_id,
      runId: run.run_id,
      collectedAt: run.collected_at_utc,
      sourceCommit: run.source_git_commit,
      analysisCommit: run.analysis_git_commit,
      host: `${run.host_class}: ${run.host_label}; tenancy ${run.tenancy}`,
      noiseLimitation: run.noise_limitation,
      workload: run.workload,
      dataset: run.dataset,
      queryScope: run.query_scope,
      protocol: run.protocol,
      publisherPath: run.publisher_path,
      publisherSha256: run.publisher_sha256,
      inputFileCount: run.input_file_count,
      rawArchiveKind: run.raw_archive_kind,
      rawArchiveLocation: run.raw_archive_location,
      rawArchivePublicUrl: run.raw_archive_public_url,
      rawArchiveSha256: run.raw_archive_sha256,
      rawArchiveBytes: run.raw_archive_bytes,
      rawArchiveVerification: run.raw_archive_build_verification,
      rawArchiveMemberVerificationAuthority: run.raw_archive_member_verification_authority,
      rawArchiveManifestMember: run.raw_archive_manifest_member,
      rawArchiveManifestSha256: run.raw_archive_manifest_sha256,
      rawArchiveManifestBytes: run.raw_archive_manifest_bytes,
      rawArchiveManifestEntryCount: run.raw_archive_manifest_entry_count,
      rawArchiveRegularMembers: run.raw_archive_regular_members,
      rawArchiveAllMembersRehashed: run.raw_archive_all_members_rehashed,
      rawArchiveExactMemberSet: run.raw_archive_exact_member_set,
      rawArchiveSanitizationScanPassed: run.raw_archive_sanitization_scan_passed,
      rawArchiveDeterministicTarHeaders: run.raw_archive_deterministic_tar_headers,
      rawArchiveZstdVersion: run.raw_archive_zstd_version,
      rawArchiveZstdExecutableSha256: run.raw_archive_zstd_executable_sha256,
    });
  }
  return {
    scope: prefixes?.length ? "paper-namespace" : "factory",
    deterministicCanonical: scoped.filter(([, r]) => r.environment === "canonical").length,
    canonicalTiming: scoped.filter(([, r]) => r.environment === "canonical-timing").length,
    timingFigures: scoped.filter(([, r]) => r.kind === "canonical-timing-figure").length,
    indicative: scoped.filter(([, r]) => r.environment === "indicative").length,
    timingRuns: [...timingRuns.values()].sort((a, b) => a.runId.localeCompare(b.runId)),
  };
}

// basePath-aware PDF asset link. [OPUS-4.8] sq-9vw5 — env-switched (was hardcoded `/sparq`)
// so the PDF resolves under both the Pages `/sparq` prefix and the Tauri root-relative build.
function pdfHref(slug: string): string {
  return withBasePath(`/papers/${slug}.pdf`);
}

// [OPUS-4.8] sq-1scgk — the paper's single Typst source on GitHub: the authoring artifact the
// PDF AND the in-site HTML both compile from (build-papers.mjs), so it is the real repro anchor.
// Matches the site's existing source-link convention (jeswr/sparq blob/main — see the /surface/*
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
  const prov = provenanceFor(paper);

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
          same merged paper-bound evidence, so the two cannot disagree. A headline is either a
          deterministic canonical fact or a controlled canonical timing measurement with its run
          provenance attached. Indicative work-box values cannot use a headline accessor. See the
          paper-scoped provenance stamp at the foot of the page.
        </p>
      </section>

      <PaperHtml html={html} />

      <PaperProvenance prov={prov} />
    </div>
  );
}
