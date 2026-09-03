// Per-paper provenance stamp. Deterministic invariants and controlled machine timings are
// distinct evidence classes: the latter retain exact run, host, source-commit and protocol
// identity instead of borrowing the unrelated benchmark dashboard's LATEST metadata.
import { GitCommitHorizontal, ShieldCheck } from "lucide-react";

import { Badge } from "@/components/ui/badge";

export interface TimingRunProvenance {
  studyId: string;
  runId: string;
  collectedAt: string;
  sourceCommit: string;
  analysisCommit: string;
  host: string;
  noiseLimitation: string;
  workload: string;
  dataset: string;
  queryScope: string;
  protocol: string;
  publisherPath: string;
  publisherSha256: string;
  inputFileCount: number;
  rawArchiveKind: "committed" | "external";
  rawArchiveLocation: string;
  rawArchivePublicUrl: string;
  rawArchiveSha256: string;
  rawArchiveBytes: number;
  rawArchiveVerification: "local-rehash" | "descriptor-only";
  rawArchiveMemberVerificationAuthority: "publisher-recorded";
  rawArchiveManifestMember: string;
  rawArchiveManifestSha256: string;
  rawArchiveManifestBytes: number;
  rawArchiveManifestEntryCount: number;
  rawArchiveRegularMembers: number;
  rawArchiveAllMembersRehashed: true;
  rawArchiveExactMemberSet: true;
  rawArchiveSanitizationScanPassed: true;
  rawArchiveDeterministicTarHeaders: true;
  rawArchiveZstdVersion: string;
  rawArchiveZstdExecutableSha256: string;
}

export interface Provenance {
  /** Whether counts are narrowed by a registry evidence-prefix declaration. */
  scope: "paper-namespace" | "factory";
  /** Deterministic, machine-independent records scoped to this paper. */
  deterministicCanonical: number;
  /** Controlled but machine-dependent records scoped to this paper, including figures. */
  canonicalTiming: number;
  /** Digest-bound SVG records included in canonicalTiming. */
  timingFigures: number;
  /** Indicative/work-box records, structurally barred from headline accessors. */
  indicative: number;
  /** Unique acquisition runs carried by canonical-timing records. */
  timingRuns: TimingRunProvenance[];
}

function shortCommit(commit: string): string {
  return commit.length >= 8 ? commit.slice(0, 8) : commit;
}

export function PaperProvenance({ prov }: { prov: Provenance }) {
  const total = prov.deterministicCanonical + prov.canonicalTiming + prov.indicative;
  return (
    <footer className="mt-10 space-y-3 rounded-lg bg-muted/40 p-4 text-xs text-muted-foreground">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="muted" className="gap-1">
          <ShieldCheck aria-hidden />
          honesty gate enforced
        </Badge>
        {prov.timingRuns.map((run) => (
          <Badge
            key={`${run.studyId}:${run.runId}:${run.sourceCommit}:${run.analysisCommit}`}
            variant="muted"
            className="gap-1"
          >
            <GitCommitHorizontal aria-hidden />
            source {shortCommit(run.sourceCommit)} · analysis {shortCommit(run.analysisCommit)}
          </Badge>
        ))}
      </div>

      <p>
        {prov.scope === "paper-namespace"
          ? "This paper's configured evidence namespace contains "
          : "No per-paper evidence namespace is configured for this legacy entry; the factory ledger contains "}
        {total} evidence record{total === 1 ? "" : "s"}: {" "}
        <strong className="text-foreground">{prov.deterministicCanonical}</strong> deterministic
        canonical, <strong className="text-foreground">{prov.canonicalTiming}</strong> canonical
        timing ({prov.timingFigures} digest-bound figure{prov.timingFigures === 1 ? "" : "s"}),
        and <strong className="text-foreground">{prov.indicative}</strong> indicative. Deterministic
        canonical records are machine-independent invariants or fixed artifact facts. Canonical
        timing records are machine-dependent measurements from a declared controlled run; every
        value is JSON-pointer-bound to its committed run envelope, and every timing SVG is
        admitted only after its SHA-256 and byte length match both the committed bytes and the
        analyzer output manifest. Indicative records cannot back a headline.
      </p>

      {prov.timingRuns.length > 0 ? (
        <div className="space-y-1.5">
          <p className="font-medium text-foreground">Canonical timing acquisition runs</p>
          <ul className="space-y-1">
            {prov.timingRuns.map((run) => (
              <li
                key={`${run.studyId}:${run.runId}:${run.sourceCommit}:${run.analysisCommit}`}
                className="break-words"
              >
                <code>{run.runId}</code> · {run.host} · {run.workload} · dataset: {run.dataset} ·
                query scope: {run.queryScope} · collected {" "}
                <time dateTime={run.collectedAt}>{run.collectedAt}</time>{" "}
                · source {" "}
                <a
                  href={`https://github.com/sparq-org/sparq/commit/${run.sourceCommit}`}
                  className="text-primary"
                >
                  <code>{shortCommit(run.sourceCommit)}</code>
                </a>{" "}
                · analysis {" "}
                <a
                  href={`https://github.com/sparq-org/sparq/commit/${run.analysisCommit}`}
                  className="text-primary"
                >
                  <code>{shortCommit(run.analysisCommit)}</code>
                </a>{" "}
                · protocol <code>{run.protocol}</code>
                {" "}· raw inputs:{" "}
                {run.rawArchivePublicUrl ? (
                  <>
                    <a href={run.rawArchivePublicUrl} className="text-primary">
                      {run.rawArchiveKind === "committed" ? "committed archive mirror" : "external archive"}
                    </a>
                    {run.rawArchiveKind === "committed" ? <> (<code>{run.rawArchiveLocation}</code>)</> : null}
                  </>
                ) : (
                  <code>{run.rawArchiveLocation}</code>
                )}{" "}
                (<code>sha256:{run.rawArchiveSha256}</code>, {run.rawArchiveBytes} bytes)
                <br />
                Archive verification: {run.rawArchiveVerification === "local-rehash" ? (
                  <>the paper sync/build re-hashed the committed archive bytes locally</>
                ) : (
                  <>descriptor only; the build did not fetch the external archive</>
                )}
                <br />
                Member verification ({run.rawArchiveMemberVerificationAuthority}): the locally
                hash-verified publisher <code>{run.publisherPath}</code> (<code>sha256:{run.publisherSha256}</code>)
                records that all {run.rawArchiveRegularMembers} regular members were re-hashed,
                the member set exactly matched <code>{run.rawArchiveManifestMember}</code> (<code>
                sha256:{run.rawArchiveManifestSha256}</code>, {run.rawArchiveManifestBytes} bytes,
                {run.rawArchiveManifestEntryCount} entries), the sanitization scan passed, and tar
                headers were deterministic. {run.inputFileCount} analysis input descriptor
                {run.inputFileCount === 1 ? "" : "s"} name exact archive members. Compression:
                zstd {run.rawArchiveZstdVersion} (<code>executable sha256:
                {run.rawArchiveZstdExecutableSha256}</code>).
                <br />
                Host limitation: {run.noiseLimitation}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {prov.timingRuns.length > 0 ? (
        <p>
          For a committed raw archive, the sync/build independently re-hashes the archive container
          and checks its byte length. Member-level re-hashing, exact-set comparison, sanitization, and
          deterministic-header checks are publisher-recorded attestations: the factory validates their
          shape and the publisher&apos;s committed bytes, but does not independently decompress and repeat
          those checks. An external archive, when used as a size fallback, is explicitly labelled
          descriptor-only and must be digest-checked after download. No public URL is claimed unless
          one exists in the envelope.
        </p>
      ) : null}
    </footer>
  );
}
