// (sq-0hv3o) [SONNET-4.6] — /assurance: the web-appropriate front door to sparq's
// proof + testing estate, follow-up to sq-wir9k (root ASSURANCE.md, PR #1562).
//
// CANONICAL SOURCE. The authoritative, maintainer-facing walkthrough is the root
//   ASSURANCE.md  (https://github.com/sparq-org/sparq/blob/main/ASSURANCE.md)
// This page is a concise, web-appropriate presentation of the SAME estate — the
// 5-minute version + the layer table + the "what green does not mean" hedges — with
// every layer linking straight to its ASSURANCE.md section for the deep path. It is
// deliberately NOT a verbatim render of the markdown (there is no generic markdown
// factory in the site — the papers/specs factories emit Typst/ReSpec HTML, not
// arbitrary markdown), so the concise-native + link-to-canonical shape is used to
// keep drift contained: the stable layer table lives here, the prose depth lives in
// the one canonical file. Update ASSURANCE.md first; reflect material changes here.
//
// HONESTY. No performance numbers (they live only in the benchmarks dashboard). The
// ZK/MPC copy carries the not-externally-audited caveat verbatim in intent — the
// sq-qhy4 external-audit hedge is kept intact; this file is inside the
// scripts/check-privacy-claims.sh scan surface, so any crypto claim here must stay
// hedged/negative.

import type { Metadata } from "next";
import Link from "next/link";
import {
  AlertTriangle,
  ArrowUpRight,
  Layers,
  ShieldCheck,
  Terminal,
  Timer,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

export const metadata: Metadata = {
  title: "Assurance",
  description:
    "How to check that sparq works, in 15 minutes — the 5-minute health check, the layer-by-layer proof and testing estate, and an honest account of what a green build does and does not mean.",
};

const REPO = "https://github.com/sparq-org/sparq";
const ASSURANCE_MD = `${REPO}/blob/main/ASSURANCE.md`;

/** An external link to a repo artifact, with the small up-right glyph. */
function Artifact({ href, label }: { href: string; label: string }) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      className="inline-flex items-center gap-1 text-sm text-primary hover:underline"
    >
      {label}
      <ArrowUpRight className="size-3.5 opacity-70" aria-hidden />
    </a>
  );
}

// The 11 assurance layers, mirrored from ASSURANCE.md's "layers at a glance" table.
// `anchor` is the ASSURANCE.md heading slug so each row links to its deep section.
// `gate` classifies the PR-blocking status for the scannable right-hand pill:
//   full    — gates every PR
//   partial — gates in part / a nightly or advisory arm exists
//   none    — informational (nightly, not a PR gate)
type GateTier = "full" | "partial" | "none";

const LAYERS: {
  n: number;
  layer: string;
  anchor: string;
  question: string;
  blocks: string;
  gate: GateTier;
}[] = [
  {
    n: 1,
    layer: "W3C conformance ratchets",
    anchor: "1-w3c-conformance-ratchets",
    question: "Does it implement the specs?",
    blocks: "yes",
    gate: "full",
  },
  {
    n: 2,
    layer: "Independent oracles + differential testing",
    anchor: "2-independent-oracles--differential-testing",
    question: "Would an independent implementation agree?",
    blocks: "partly (nightly lanes)",
    gate: "partial",
  },
  {
    n: 3,
    layer: "Metamorphic self-checks (TLP / NoREC)",
    anchor: "3-metamorphic-self-checks-tlp--norec",
    question: "Is the engine internally consistent?",
    blocks: "yes (self-tests)",
    gate: "full",
  },
  {
    n: 4,
    layer: "Fuzzing",
    anchor: "4-fuzzing",
    question: "Does hostile input crash it?",
    blocks: "yes (smoke tier)",
    gate: "full",
  },
  {
    n: 5,
    layer: "Memory safety (the unsafe register, Miri, ASan)",
    anchor: "5-memory-safety-the-unsafe-register-miri-asan",
    question: "Is the unsafe surface bounded + justified?",
    blocks: "yes (count ratchet)",
    gate: "full",
  },
  {
    n: 6,
    layer: "Bounded mechanized proofs (Kani)",
    anchor: "6-bounded-mechanized-proofs-kani",
    question: "Is any input in a bounded domain missed?",
    blocks: "no (nightly, informational)",
    gate: "none",
  },
  {
    n: 7,
    layer: "Test-quality ratchets (coverage floors + mutation ceilings)",
    anchor: "7-test-quality-ratchets-coverage-floors--mutation-ceilings",
    question: "Do the tests actually assert?",
    blocks: "coverage yes / mutation advisory",
    gate: "partial",
  },
  {
    n: 8,
    layer: "Both-feature-state builds",
    anchor: "8-both-feature-state-builds",
    question: "Does every opt-in feature build, on and off?",
    blocks: "yes",
    gate: "full",
  },
  {
    n: 9,
    layer: "End-to-end product journeys (Playwright personas)",
    anchor: "9-end-to-end-product-journeys-playwright-personas",
    question: "Does the product work for a user?",
    blocks: "yes",
    gate: "full",
  },
  {
    n: 10,
    layer: "Docs honesty gates",
    anchor: "10-docs-honesty-gates",
    question: "Do the docs overclaim?",
    blocks: "yes",
    gate: "full",
  },
  {
    n: 11,
    layer: "Supply chain + static analysis",
    anchor: "11-supply-chain--static-analysis",
    question: "Are the dependencies and the build attested?",
    blocks: "yes",
    gate: "full",
  },
];

const GATE_STYLES: Record<GateTier, string> = {
  full: "border-[var(--success)]/40 bg-[var(--success)]/10 text-[var(--success)]",
  partial: "border-amber-500/40 bg-amber-500/10 text-amber-600 dark:text-amber-400",
  none: "border-border bg-muted text-muted-foreground",
};

/** The small right-hand pill classifying a layer's PR-blocking status. */
function BlocksPill({ gate, label }: { gate: GateTier; label: string }) {
  return (
    <span
      className={`inline-flex items-center whitespace-nowrap rounded-full border px-2 py-0.5 text-xs font-medium ${GATE_STYLES[gate]}`}
    >
      {label}
    </span>
  );
}

/** A code token used inline in prose. */
function Code({ children }: { children: React.ReactNode }) {
  return (
    <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs text-foreground">
      {children}
    </code>
  );
}

export default function AssurancePage() {
  return (
    <div className="space-y-10">
      {/* ── Header ──────────────────────────────────────────────── */}
      <header className="space-y-3">
        <div className="flex items-center gap-3">
          <span className="flex size-11 items-center justify-center rounded-xl bg-primary/10 text-primary">
            <ShieldCheck className="size-5" aria-hidden />
          </span>
          <div>
            <h1 className="text-2xl font-semibold">
              Assurance — how to check that sparq works
            </h1>
            <p className="text-sm text-muted-foreground">
              Verify the claims yourself, in about 15 minutes — without reading the
              codebase.
            </p>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <Badge variant="outline">conformance</Badge>
          <Badge variant="outline">memory safety</Badge>
          <Badge variant="outline">honest docs</Badge>
        </div>
      </header>

      {/* ── Lead ────────────────────────────────────────────────── */}
      <section className="measure space-y-3 text-sm text-muted-foreground">
        <p>
          sparq makes strong claims — W3C conformance, memory safety, honest
          documentation. This page is the walkthrough for{" "}
          <strong className="text-foreground">verifying those claims yourself</strong>:
          the 5-minute health check first, then one short entry per assurance layer —
          what it checks, where to watch it run, and what green does (and does not) mean.
          Every layer links straight to the canonical{" "}
          <Artifact href={ASSURANCE_MD} label="ASSURANCE.md" /> section for the full
          detail and the exact artifacts that enforce it.
        </p>
      </section>

      {/* ── The 5-minute version ────────────────────────────────── */}
      <section className="space-y-4">
        <div className="flex items-center gap-2">
          <Timer className="size-4 text-primary" aria-hidden />
          <h2 className="text-lg font-semibold">The 5-minute version</h2>
        </div>
        <p className="text-sm text-muted-foreground">
          One check blocks a merge:{" "}
          <Code>ci-fast</Code>. It is the single required branch-protection status on{" "}
          <Code>main</Code> — <Code>clippy -D warnings</Code>, tests on the core crates and
          the W3C SPARQL conformance ratchet. The{" "}
          <strong className="text-foreground">heavy suites run nightly against main</strong>{" "}
          (full workspace build + tests, the conformance / coverage / unsafe-count ratchets,
          the opt-in feature matrix, fuzzing, formal verification and supply-chain).
        </p>
        <ol className="space-y-3 text-sm text-muted-foreground">
          <li className="flex gap-3">
            <span className="flex size-6 shrink-0 items-center justify-center rounded-full bg-primary/10 text-xs font-semibold text-primary">
              1
            </span>
            <span>
              Open any merged PR (or the latest commit on <Code>main</Code>) and look at
              its <strong className="text-foreground">checks list</strong> — a green{" "}
              <Code>ci-fast</Code> means the fast core gate passed; the nightly runs on{" "}
              <Code>main</Code> cover every other layer below.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="flex size-6 shrink-0 items-center justify-center rounded-full bg-primary/10 text-xs font-semibold text-primary">
              2
            </span>
            <span>
              For{" "}
              <em>&ldquo;does it implement the specs?&rdquo;</em>, open the{" "}
              <strong className="text-foreground">conformance scoreboard</strong> — the
              job summary renders every W3C suite with its ratchet floor. Locally:{" "}
              <Code>cargo run --release -p sparq-conformance</Code>.
            </span>
          </li>
          <li className="flex gap-3">
            <span className="flex size-6 shrink-0 items-center justify-center rounded-full bg-primary/10 text-xs font-semibold text-primary">
              3
            </span>
            <span>
              Locally, the core gate is just{" "}
              <Code>cargo build --workspace &amp;&amp; cargo test --workspace</Code> plus{" "}
              <Code>cargo clippy --workspace --exclude sparq-py --all-targets -- -D warnings</Code>
              .
            </span>
          </li>
        </ol>
        <p className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2.5 text-xs text-muted-foreground">
          <strong className="text-foreground">Two honest qualifiers.</strong> Some
          heavyweight lanes (Kani proofs, Miri, ASan, long fuzz, the ZK toolchain suite,
          mutation testing) run <strong>nightly, not per-PR</strong>, so a green gate
          shows they were green on their <em>last scheduled run</em>, not on this exact
          commit. And the <Code>sparq-zk*</Code> / <Code>sparq-mpc</Code> crates are
          research scaffolds with no security guarantee yet — see{" "}
          {/* [FABLE-5] persistent underline: distinguishable without color (link-in-text-block, sq-0rbfn) */}
          <a href="#what-green-does-not-mean" className="text-primary underline underline-offset-2">
            what green does not mean
          </a>
          .
        </p>
      </section>

      {/* ── The layers at a glance ──────────────────────────────── */}
      <section className="space-y-4">
        <div className="flex items-center gap-2">
          <Layers className="size-4 text-primary" aria-hidden />
          <h2 className="text-lg font-semibold">The layers at a glance</h2>
        </div>
        <p className="text-sm text-muted-foreground">
          Eleven layers, each answering one question. Follow a row to its{" "}
          <Artifact href={ASSURANCE_MD} label="ASSURANCE.md" /> section for what it
          checks, where to watch it run, what green means, and its limits.
        </p>
        <div className="overflow-x-auto rounded-xl border">
          <table className="w-full border-collapse text-left text-sm">
            <thead>
              <tr className="border-b bg-muted/40 text-xs uppercase tracking-wide text-muted-foreground">
                <th scope="col" className="px-3 py-2.5 font-medium">
                  Layer
                </th>
                <th scope="col" className="px-3 py-2.5 font-medium">
                  Question it answers
                </th>
                <th scope="col" className="px-3 py-2.5 font-medium">
                  Blocks a PR?
                </th>
              </tr>
            </thead>
            <tbody>
              {LAYERS.map((l) => (
                <tr
                  key={l.n}
                  className="border-b last:border-0 align-top hover:bg-muted/30"
                >
                  <td className="px-3 py-2.5">
                    <a
                      href={`${ASSURANCE_MD}#${l.anchor}`}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="font-medium text-foreground hover:text-primary hover:underline"
                    >
                      {l.layer}
                    </a>
                  </td>
                  <td className="px-3 py-2.5 text-muted-foreground">{l.question}</td>
                  <td className="px-3 py-2.5">
                    <BlocksPill gate={l.gate} label={l.blocks} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      {/* ── What green does NOT mean ────────────────────────────── */}
      <section id="what-green-does-not-mean" className="scroll-mt-24 space-y-4">
        <div className="flex items-center gap-2">
          <AlertTriangle className="size-4 text-amber-500" aria-hidden />
          <h2 className="text-lg font-semibold">What green does not mean</h2>
        </div>
        <ul className="space-y-3 text-sm text-muted-foreground">
          <li className="rounded-lg border bg-card px-3 py-2.5">
            <strong className="text-foreground">
              ZK / MPC are research scaffolds — no security guarantee yet.
            </strong>{" "}
            The <Code>sparq-zk*</Code> crates model proving query results over committed
            data, but the v1 verifier has{" "}
            <strong className="text-foreground">not</strong> been externally audited:
            independent cryptographer sign-off (bead <Code>sq-qhy4</Code>) is required
            before any production ZK claim, and until then a &ldquo;verified&rdquo; result
            must not be presented as a guarantee to a relying party.{" "}
            <Code>sparq-mpc</Code> is honest-majority / semi-honest only, and is not
            secure against a malicious majority. The authoritative wording is{" "}
            <Artifact href={`${REPO}/blob/main/SECURITY.md`} label="SECURITY.md" />; the
            docs-honesty gate keeps every doc consistent with it.
          </li>
          <li className="rounded-lg border bg-card px-3 py-2.5">
            <strong className="text-foreground">Kani proofs are bounded</strong> —
            exhaustive only up to a small input bound, on the mmap-free seam, in an
            informational nightly lane.
          </li>
          <li className="rounded-lg border bg-card px-3 py-2.5">
            <strong className="text-foreground">Nightly ≠ per-commit.</strong> Kani, Miri,
            ASan, long-budget fuzz, the ZK toolchain suite, SHACL differential fuzz and
            mutation testing run on schedule; a PR&rsquo;s green gate reflects their last
            scheduled run.
          </li>
          <li className="rounded-lg border bg-card px-3 py-2.5">
            <strong className="text-foreground">Conformance floors are floors.</strong>{" "}
            The documented-divergence and not-yet-run lists in the conformance reports are
            part of the claim — they are deliberately not hidden.
          </li>
          <li className="rounded-lg border bg-card px-3 py-2.5">
            <strong className="text-foreground">
              This is an experimental, pre-1.0 engine.
            </strong>{" "}
            The assurance estate above is how correctness is <em>kept</em>, not a maturity
            certification.
          </li>
        </ul>
      </section>

      {/* ── Reproduce it locally ────────────────────────────────── */}
      <section className="space-y-4">
        <div className="flex items-center gap-2">
          <Terminal className="size-4 text-primary" aria-hidden />
          <h2 className="text-lg font-semibold">Reproduce it locally</h2>
        </div>
        <p className="text-sm text-muted-foreground">
          Every claim on this page is reproducible from a clean checkout. The full
          command table — including the fuzz, Kani and Playwright entry points — lives in{" "}
          <Artifact href={ASSURANCE_MD} label="ASSURANCE.md" />. The core three:
        </p>
        <div className="overflow-x-auto rounded-xl border">
          <table className="w-full border-collapse text-left text-sm">
            <thead>
              <tr className="border-b bg-muted/40 text-xs uppercase tracking-wide text-muted-foreground">
                <th scope="col" className="px-3 py-2.5 font-medium">
                  Claim
                </th>
                <th scope="col" className="px-3 py-2.5 font-medium">
                  Command (repo root)
                </th>
              </tr>
            </thead>
            <tbody>
              {[
                {
                  claim: "Everything builds + tests pass",
                  cmd: "cargo build --workspace && cargo test --workspace",
                },
                {
                  claim: "Lint-clean at -D warnings",
                  cmd: "cargo clippy --workspace --exclude sparq-py --all-targets -- -D warnings",
                },
                {
                  claim: "No overclaiming docs",
                  cmd: "bash scripts/check-privacy-claims.sh",
                },
              ].map((r) => (
                <tr key={r.claim} className="border-b last:border-0 align-top">
                  <td className="px-3 py-2.5 text-muted-foreground">{r.claim}</td>
                  <td className="px-3 py-2.5">
                    <code className="font-mono text-xs text-foreground">{r.cmd}</code>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>

      {/* ── Front doors / CTAs ──────────────────────────────────── */}
      <section className="flex flex-wrap gap-3 border-t pt-6">
        <Button asChild variant="outline" size="sm">
          <a href={ASSURANCE_MD} target="_blank" rel="noopener noreferrer">
            Read ASSURANCE.md (canonical)
            <ArrowUpRight className="size-3.5 opacity-60" aria-hidden />
          </a>
        </Button>
        <Button asChild variant="outline" size="sm">
          <a
            href={`${REPO}/blob/main/SECURITY.md`}
            target="_blank"
            rel="noopener noreferrer"
          >
            SECURITY.md — threat model + disclosure
            <ArrowUpRight className="size-3.5 opacity-60" aria-hidden />
          </a>
        </Button>
        <Button asChild variant="outline" size="sm">
          <a
            href={`${REPO}/blob/main/CONTRIBUTING.md`}
            target="_blank"
            rel="noopener noreferrer"
          >
            CONTRIBUTING.md — the contributor gate
            <ArrowUpRight className="size-3.5 opacity-60" aria-hidden />
          </a>
        </Button>
        <Button asChild variant="outline" size="sm">
          <Link href="/assurance/served-conformance">Served-surface conformance</Link>
        </Button>
        <Button asChild variant="outline" size="sm">
          <Link href="/benchmarks">Benchmarks dashboard</Link>
        </Button>
      </section>
    </div>
  );
}
