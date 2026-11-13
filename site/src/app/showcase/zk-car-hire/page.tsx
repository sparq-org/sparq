import type { Metadata } from "next";
import Link from "next/link";
import { ArrowLeft, ExternalLink } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ZkCarHire } from "@/components/zk-car-hire";

// [OPUS-4.8] sq-13rg / sq-4r4b — the ZK cross-credential car-hire flagship.
// [OPUS-4.8] sq-vw3ax.6 — rebuilt DEMO-FIRST (research/website-redesign.md §4): minimal header
// → the live demo immediately → one-line caption → the full security caveat behind a closed
// <details>. The research-grade / NOT-externally-audited qualifier stays VISIBLE in the header
// badge (privacy-claims gate); the soundness/linkability detail relocates into the disclosure.
export const metadata: Metadata = {
  title: "ZK car-hire — a live zero-knowledge proof demo",
  description:
    "Research-grade (not externally audited): a real zero-knowledge proof — age ≥ 25 over hidden credential data — generated in your browser via Noir + Barretenberg UltraHonk. This demo path is not designed to keep the compared age confidential; the v1 verifier carries no production soundness guarantee.",
};

const REPO_URL = "https://github.com/sparq-org/sparq";

export default function ZkCarHirePage() {
  return (
    <div className="space-y-6">
      <Button variant="ghost" size="sm" asChild className="-ml-2">
        <Link href="/examples">
          <ArrowLeft className="size-4" aria-hidden="true" />
          Back to examples
        </Link>
      </Button>

      {/* Minimal header: title + the tier + research-grade badges (the privacy qualifier stays
          visible up top; the full hedge moves into the caveat disclosure below). */}
      <header className="space-y-2">
        <h1 className="flex items-center gap-2 text-2xl font-semibold tracking-tight">
          <span className="text-[var(--warning)]" aria-hidden>
            ★
          </span>
          Prove you may hire a car with a zero-knowledge proof
        </h1>
        <div className="flex flex-wrap gap-2">
          <Badge variant="success">Live via bb.js — real proof in your tab</Badge>
          <Badge variant="warning">Research-grade · not externally audited</Badge>
        </div>
      </header>

      {/* The demo, immediately. */}
      <ZkCarHire />

      {/* One-line caption. */}
      <p className="measure text-sm text-muted-foreground">
        The age-gate proves <em>age ≥ 25</em> over a hidden date of birth with a real
        UltraHonk zero-knowledge proof, generated and verified entirely in your browser —
        the exact age, the licence number, and the holder&rsquo;s identity are never revealed.
      </p>

      {/* The full security caveat — behind a closed disclosure. */}
      <details className="rounded-xl border bg-card px-4 py-3 text-sm">
        <summary className="cursor-pointer select-none font-medium">
          Details &amp; full security caveat
        </summary>
        <div className="mt-3 space-y-3 text-muted-foreground">
          <p>
            A renter holds two W3C Verifiable Credentials — a gov-ID and a DVLA driving
            licence. The car-hire desk must learn only that the renter is{" "}
            <strong className="text-foreground">≥ 25</strong>, holds a{" "}
            <strong className="text-foreground">valid, non-revoked licence</strong>, and that
            both credentials belong to the{" "}
            <strong className="text-foreground">same person</strong> — and nothing else: not
            the date of birth, not the licence number, not the identity.
          </p>
          <p>
            The proof runs in-tab via the third-party{" "}
            <code className="font-mono">bb.js</code> UltraHonk prover over a Noir circuit; the
            ~MB prover chunk loads only when you run the demo. The v1 verifier is{" "}
            <strong className="text-foreground">research-grade</strong>: sound as landed under
            its stated threat model, but{" "}
            <strong className="text-foreground">not externally audited</strong> and pending
            re-review — treat the timings and gate counts as indicative engineering numbers,{" "}
            <strong className="text-foreground">not</strong> an audited cryptographic
            guarantee. Reproduction commands, the full circuit/public-input contract, and the
            soundness/linkability discussion live in the crate README and{" "}
            <code className="font-mono">SKILL.md</code>.
          </p>
          <p className="flex flex-wrap gap-4">
            <a
              className="inline-flex items-center gap-1 text-primary underline-offset-4 hover:underline"
              href={`${REPO_URL}/tree/main/crates/sparq-zk`}
              target="_blank"
              rel="noopener noreferrer"
            >
              sparq-zk on GitHub <ExternalLink className="size-3.5" />
            </a>
            <a
              className="inline-flex items-center gap-1 text-primary underline-offset-4 hover:underline"
              href={`${REPO_URL}/blob/main/skills/zk/SKILL.md`}
              target="_blank"
              rel="noopener noreferrer"
            >
              ZK SKILL.md <ExternalLink className="size-3.5" />
            </a>
          </p>
        </div>
      </details>
    </div>
  );
}
