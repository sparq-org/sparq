"use client";

// [OPUS-4.8] sq-13rg / sq-4r4b — the ZK car-hire flagship demo. Real in-browser
// UltraHonk proving of the age-gate sub-proof (see src/lib/zk-prover.ts); honest
// about what is live, what is composed-but-not-yet-wired, and what is research-grade
// and NOT externally audited.

import * as React from "react";
import {
  ShieldCheck,
  Loader2,
  Play,
  Eye,
  EyeOff,
  FileLock2,
  ArrowRight,
  CircleCheck,
  CircleAlert,
  Cpu,
  KeyRound,
  CircleDot,
  PackageCheck,
} from "lucide-react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { cn } from "@/lib/utils";
import {
  proveAgeEligibility,
  prewarmProver,
  verifyCapturedManifest,
  PROVABLE_AGES,
  AGE_THRESHOLD,
  type ProofResult,
  type VerifyResult,
} from "@/lib/zk-prover";
import {
  CREDENTIAL_TURTLE,
  ELIGIBILITY_QUERY,
  DISCLOSURE,
  CIRCUIT_FAMILY,
  CIRCUIT_FAMILY_EXPLAINER,
  COMPOSITION_MEMBERS,
  PUBLIC_INPUT_LABELS,
  type FamilyStatus,
} from "@/data/zk-car-hire";
import { ZK_MEMBERS } from "@/data/zk-circuit-costs";

type Phase =
  | { kind: "idle" }
  | { kind: "loading" } // fetching circuit + bb.js/noir wasm
  | { kind: "proving" }
  | { kind: "verifying" }
  | { kind: "done"; result: ProofResult }
  | { kind: "rejected"; message: string } // under-age → unsatisfiable
  | { kind: "error"; message: string };

// [OPUS-4.8] sq-8dx2 — the verify-only fallback phases (captured manifest + live
// in-tab verify), kept separate from the live-prove Phase so each path's status copy
// stays exact and honest.
type VerifyPhase =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "verifying" }
  | { kind: "done"; result: VerifyResult }
  | { kind: "error"; message: string };

// [OPUS-4.8] sq-8dx2 — which engine the page runs. `prove` = fresh live UltraHonk proof
// of the age-gate (the default — "proving in your tab right now"). `verify` = the
// captured-manifest fallback: replay a real captured proof and verify it live in-tab
// (for browsers/devices where proving is too heavy). Both are REAL cryptography; only
// the age-gate relation is ever checked in-browser either way.
type RunMode = "prove" | "verify";

// [OPUS-4.8] sq-8dx2 — per-circuit progress status for the composition list. `live`
// rows transition queued→active→verified during a run; `composed` rows are members
// proven NATIVELY and never run in-browser — they are NEVER shown as "verified",
// only as composed-not-bundled, so no reader mistakes the demo for a full live
// composition. `rejected` marks the age-gate when the claim is unsatisfiable.
type StepStatus = "queued" | "active" | "verified" | "rejected" | "composed";

// [OPUS-4.8] Prover warm-up lifecycle, surfaced as a subtle indicator — mirrors the
// SPARQL REPL's Engine pill. The dynamic-import + circuit fetch + Barretenberg WASM
// instantiate is kicked off on mount (prewarmProver) so the first "Generate ZK proof"
// pays no cold start. The proving path awaits the same shared promise as a safety net.
type ProverState = "cold" | "warming" | "ready" | "error";

export function ZkCarHire() {
  const [age, setAge] = React.useState(30);
  const [phase, setPhase] = React.useState<Phase>({ kind: "idle" });
  const [verifyPhase, setVerifyPhase] = React.useState<VerifyPhase>({ kind: "idle" });
  const [mode, setMode] = React.useState<RunMode>("prove");
  const [prover, setProver] = React.useState<ProverState>("cold");

  // Pre-warm the in-browser ZK prover in the background on mount, off the render path.
  // A failure resets the indicator; proveAgeEligibility still awaits the shared init,
  // so the button never silently no-ops or throws on a cold prover. Route-scoped (this
  // component only renders on the ZK pages) so the ~MB bb.js WASM is never fetched on
  // other routes — the prover assets stay off /try, /papers, /benchmarks, etc.
  React.useEffect(() => {
    let cancelled = false;
    setProver("warming");
    prewarmProver()
      .then(() => {
        if (!cancelled) setProver("ready");
      })
      .catch(() => {
        // Transient load failure (fetch/instantiate) — the cache self-resets, so the
        // next "Generate ZK proof" retries the cold start. Surface it on the pill only.
        if (!cancelled) setProver("error");
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const run = React.useCallback(async () => {
    try {
      setPhase({ kind: "loading" });
      // small yield so the "loading" status paints before the heavy wasm fetch.
      await new Promise((r) => setTimeout(r, 0));
      setPhase({ kind: "proving" });
      const result = await proveAgeEligibility(age);
      // The shared prover init resolved (proveAgeEligibility awaits it): reflect ready.
      setProver("ready");
      setPhase({ kind: "verifying" });
      // verifyProof already ran inside proveAgeEligibility; reflect it.
      setPhase({ kind: "done", result });
      if (!result.verified) {
        toast.error("Proof did not verify", {
          description: "The in-tab verifier rejected the generated proof.",
        });
      }
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      // The circuit is unsatisfiable for an under-age claim: witness solve fails.
      if (/verdict mismatch|execution failed|unsatisfiable|assert/i.test(message)) {
        setPhase({
          kind: "rejected",
          message:
            "The circuit is unsatisfiable for this age — you cannot produce a proof of eligibility you do not have. (This is the soundness of the age-gate, live.)",
        });
      } else {
        setPhase({ kind: "error", message });
        toast.error("Proving failed", { description: message });
      }
    }
  }, [age]);

  // [OPUS-4.8] sq-8dx2 — the fallback: fetch the captured ProofManifest and run a REAL
  // bb.js verifyProof over its bundled age-gate sub-proof, live in your tab. Proving
  // (the expensive part) was done ahead of time; only the cheap verify runs here, so
  // this path works where live proving is too heavy. The proof bytes are a genuine
  // UltraHonk transcript — a tampered byte makes verify return false.
  const runVerify = React.useCallback(async () => {
    try {
      setVerifyPhase({ kind: "loading" });
      await new Promise((r) => setTimeout(r, 0));
      setVerifyPhase({ kind: "verifying" });
      const result = await verifyCapturedManifest();
      setProver("ready");
      setVerifyPhase({ kind: "done", result });
      if (!result.verified) {
        toast.error("Captured proof did not verify", {
          description: "The in-tab verifier rejected the bundled proof.",
        });
      }
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setVerifyPhase({ kind: "error", message });
      toast.error("Verify failed", { description: message });
    }
  }, []);

  const busy =
    phase.kind === "loading" ||
    phase.kind === "proving" ||
    phase.kind === "verifying";

  const verifyBusy =
    verifyPhase.kind === "loading" || verifyPhase.kind === "verifying";

  const statusText =
    phase.kind === "loading"
      ? "Loading the Noir + Barretenberg prover (WASM)…"
      : phase.kind === "proving"
        ? "Proving age ≥ 25 in your tab (UltraHonk)…"
        : phase.kind === "verifying"
          ? "Verifying the proof…"
          : phase.kind === "done"
            ? phase.result.verified
              ? "Proof generated and verified — in your tab."
              : "Proof generated but did NOT verify."
            : phase.kind === "rejected"
              ? "No proof produced — claim is unsatisfiable."
              : phase.kind === "error"
                ? "Proving failed."
                : "Idle — choose an age and generate a proof.";

  const verifyStatusText =
    verifyPhase.kind === "loading"
      ? "Loading the Barretenberg verifier (WASM)…"
      : verifyPhase.kind === "verifying"
        ? "Verifying the captured age-gate proof in your tab…"
        : verifyPhase.kind === "done"
          ? verifyPhase.result.verified
            ? "Captured proof verified — in your tab."
            : "Captured proof did NOT verify."
          : verifyPhase.kind === "error"
            ? "Verify failed."
            : "Idle — verify the captured proof.";

  const underAge = age < AGE_THRESHOLD;

  // [OPUS-4.8] sq-8dx2 — derive each composition row's honest status from the live run.
  // The age-gate (filter) is the ONLY member touched by an in-browser proof/verify; it
  // tracks the active phase. Every other member is `composed` (proven natively, NOT run
  // in-browser) and NEVER shows as "verified". This keeps the progress list truthful: a
  // green tick on a row means a real in-tab check happened, which is only ever the
  // age-gate.
  const ageGateStatus: StepStatus =
    mode === "prove"
      ? phase.kind === "loading" || phase.kind === "proving"
        ? "active"
        : phase.kind === "verifying"
          ? "active"
          : phase.kind === "done"
            ? phase.result.verified
              ? "verified"
              : "queued"
            : phase.kind === "rejected"
              ? "rejected"
              : "queued"
      : verifyPhase.kind === "loading" || verifyPhase.kind === "verifying"
        ? "active"
        : verifyPhase.kind === "done"
          ? verifyPhase.result.verified
            ? "verified"
            : "queued"
          : "queued";

  const steps = COMPOSITION_MEMBERS.map((m) => ({
    member: m,
    status: m.live ? ageGateStatus : ("composed" as StepStatus),
    gates: ZK_MEMBERS.find((x) => x.member === m.snapshot)?.gate_count ?? null,
  }));

  // The composed eligibility verdict is reached ONLY when the one live relation (the
  // age-gate) was cryptographically checked in your tab. It is honestly scoped: the
  // other members are composed-not-bundled, so this is "ELIGIBLE — as far as the live
  // age-gate proves", not a full-composition guarantee. The honesty panel says so.
  const liveVerified =
    mode === "prove"
      ? phase.kind === "done" && phase.result.verified && phase.result.eligible
      : verifyPhase.kind === "done" && verifyPhase.result.verified;

  return (
    <div className="space-y-8">
      {/* ── The flow: private credentials → query → proof → verdict ── */}
      <FlowStrip />

      <div className="grid gap-6 lg:grid-cols-2">
        {/* Private inputs */}
        <Card>
          <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
            <CardTitle className="flex items-center gap-2 text-base">
              <FileLock2 className="size-4 text-primary" />
              Your credentials (private)
            </CardTitle>
            <Badge variant="muted">
              <EyeOff className="size-3" /> never leaves your device
            </Badge>
          </CardHeader>
          <CardContent className="space-y-3">
            {/* [FABLE-5] tabIndex=0: keyboard-focusable scrollable region (scrollable-region-focusable, sq-0rbfn) */}
            <pre tabIndex={0} className="max-h-64 overflow-auto rounded-lg border bg-muted/40 p-3 font-mono text-[11.5px] leading-relaxed">
              {CREDENTIAL_TURTLE}
            </pre>
            <p className="text-xs text-muted-foreground">
              Two W3C Verifiable Credentials — a gov-ID and a DVLA driving
              licence. The car-hire desk never sees this Turtle; only commitments
              to it and the eligibility verdict.
            </p>
          </CardContent>
        </Card>

        {/* The query + the prove control */}
        <Card>
          <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
            <CardTitle className="flex items-center gap-2 text-base">
              <ShieldCheck className="size-4 text-primary" />
              The desk’s eligibility query (public)
            </CardTitle>
            <div className="flex flex-wrap items-center justify-end gap-2">
              <ProverIndicator prover={prover} />
              <Badge variant="success">
                <Cpu className="size-3" /> Live via bb.js
              </Badge>
            </div>
          </CardHeader>
          <CardContent className="space-y-4">
            {/* [FABLE-5] tabIndex=0: keyboard-focusable scrollable region (scrollable-region-focusable, sq-0rbfn) */}
            <pre tabIndex={0} className="overflow-auto rounded-lg border bg-muted/40 p-3 font-mono text-[11.5px] leading-relaxed">
              {ELIGIBILITY_QUERY}
            </pre>

            <div className="space-y-2">
              <label
                htmlFor="zk-age"
                className="flex items-center justify-between text-sm font-medium"
              >
                <span>Your real age (the private witness)</span>
                <span className="tabular text-muted-foreground">{age}</span>
              </label>
              <div className="flex flex-wrap gap-1.5" role="group" aria-label="Choose age">
                {PROVABLE_AGES.map((a) => (
                  <Button
                    key={a}
                    id={a === age ? "zk-age" : undefined}
                    variant={a === age ? "default" : "outline"}
                    size="sm"
                    aria-pressed={a === age}
                    onClick={() => {
                      setAge(a);
                      setPhase({ kind: "idle" });
                    }}
                  >
                    {a}
                  </Button>
                ))}
              </div>
              <p className="text-xs text-muted-foreground">
                {underAge ? (
                  <>
                    {age} &lt; {AGE_THRESHOLD} — the circuit will be{" "}
                    <strong>unsatisfiable</strong>: you cannot prove eligibility
                    you do not have.
                  </>
                ) : (
                  <>
                    {age} ≥ {AGE_THRESHOLD} — provable. The age value itself is the
                    private witness and is never disclosed.
                  </>
                )}
              </p>
            </div>

            {/* [OPUS-4.8] sq-8dx2 — engine toggle: live-prove (default) vs the
                captured-manifest + live-verify fallback for heavier browsers. */}
            <ModeToggle mode={mode} setMode={setMode} disabled={busy || verifyBusy} />

            {mode === "prove" ? (
              <div className="flex items-center gap-3">
                <Button onClick={run} disabled={busy}>
                  {busy ? (
                    <Loader2 className="size-4 animate-spin" />
                  ) : (
                    <Play className="size-4" />
                  )}
                  Generate ZK proof
                </Button>
                <p
                  aria-live="polite"
                  role="status"
                  className="text-xs text-muted-foreground"
                >
                  {statusText}
                </p>
              </div>
            ) : (
              <div className="flex items-center gap-3">
                <Button onClick={runVerify} disabled={verifyBusy}>
                  {verifyBusy ? (
                    <Loader2 className="size-4 animate-spin" />
                  ) : (
                    <PackageCheck className="size-4" />
                  )}
                  Verify captured proof
                </Button>
                <p
                  aria-live="polite"
                  role="status"
                  className="text-xs text-muted-foreground"
                >
                  {verifyStatusText}
                </p>
              </div>
            )}
          </CardContent>
        </Card>
      </div>

      {/* ── Per-circuit composition progress + the aria-live status pill ── */}
      <CompositionPanel
        steps={steps}
        mode={mode}
        eligible={liveVerified}
        running={mode === "prove" ? busy : verifyBusy}
      />

      {/* ── Verdict ── */}
      {mode === "prove" ? (
        <VerdictPanel phase={phase} age={age} />
      ) : (
        <VerifyVerdictPanel phase={verifyPhase} />
      )}

      {/* ── Reveal vs hide ── */}
      <DisclosurePanel />

      {/* ── What each circuit-family member proves (public vs hidden + live-vs-designed) ── */}
      <WhatThisProvesPanel />

      {/* ── The composed circuit family ── */}
      <CircuitFamilyPanel />

      {/* ── Honesty ── */}
      <HonestyPanel />
    </div>
  );
}

// [OPUS-4.8] sq-8dx2 — the live-prove vs captured-verify engine toggle. Live-prove is
// the default ("a real proof in your tab right now"); the captured-manifest fallback
// runs only the cheap verify, for browsers/devices where proving is too heavy. Both
// paths are REAL cryptography — only the age-gate relation is checked in-browser either
// way (see the composition panel + honesty note).
function ModeToggle({
  mode,
  setMode,
  disabled,
}: {
  mode: RunMode;
  setMode: (m: RunMode) => void;
  disabled: boolean;
}) {
  const opts: { key: RunMode; label: string; hint: string }[] = [
    { key: "prove", label: "Live prove", hint: "fresh proof in your tab (default)" },
    { key: "verify", label: "Captured + verify", hint: "fallback — replays a captured proof" },
  ];
  return (
    <div
      className="flex flex-col gap-1.5"
      role="group"
      aria-label="Proving mode"
      data-zk-mode={mode}
    >
      <div className="inline-flex w-fit rounded-lg bg-muted p-0.5 ring-1 ring-foreground/10">
        {opts.map((o) => (
          <button
            key={o.key}
            type="button"
            aria-pressed={mode === o.key}
            disabled={disabled}
            onClick={() => setMode(o.key)}
            className={cn(
              "rounded-md px-3 py-1 text-xs font-medium transition-colors disabled:opacity-50",
              mode === o.key
                ? "bg-card text-foreground ring-1 ring-foreground/10"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {o.label}
          </button>
        ))}
      </div>
      <p className="text-[11px] text-muted-foreground">
        {opts.find((o) => o.key === mode)?.hint}
      </p>
    </div>
  );
}

// [OPUS-4.8] sq-8dx2 — the per-circuit composition progress list + the aria-live
// status pill, ending in the composed verdict. The age-gate row transitions live
// (queued→active→verified); every other member is rendered as "composed natively · not
// run in-browser" and NEVER shows a verified tick — so a green row always means a real
// in-tab check happened (which is only ever the age-gate). The composed ELIGIBLE
// verdict is honestly scoped to what was actually verified live.
function StepIcon({ status }: { status: StepStatus }) {
  if (status === "active") return <Loader2 className="size-4 animate-spin text-primary" />;
  if (status === "verified") return <CircleCheck className="size-4 text-[var(--success)]" />;
  if (status === "rejected") return <CircleAlert className="size-4 text-[var(--warning)]" />;
  if (status === "composed")
    return <CircleDot className="size-4 text-muted-foreground/60" aria-hidden />;
  return <CircleDot className="size-4 text-muted-foreground/40" aria-hidden />;
}

function stepStatusLabel(status: StepStatus): string {
  switch (status) {
    case "active":
      return "checking in your tab…";
    case "verified":
      return "verified in your tab";
    case "rejected":
      return "unsatisfiable — no proof";
    case "composed":
      return "composed natively · not run in-browser";
    default:
      return "queued";
  }
}

function CompositionPanel({
  steps,
  mode,
  eligible,
  running,
}: {
  steps: { member: (typeof COMPOSITION_MEMBERS)[number]; status: StepStatus; gates: number | null }[];
  mode: RunMode;
  eligible: boolean;
  running: boolean;
}) {
  const liveCount = steps.filter((s) => s.member.live).length;
  const total = steps.length;
  // The aria-live pill summarises the run for assistive tech without scraping rows.
  const pill = running
    ? mode === "prove"
      ? "Proving the age-gate in your tab…"
      : "Verifying the captured proof in your tab…"
    : eligible
      ? "ELIGIBLE — the live age-gate proof verified in your tab"
      : "Idle — run the composition to check eligibility";
  return (
    <Card data-zk-composition>
      <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
        <CardTitle className="flex items-center gap-2 text-base">
          <Cpu className="size-4 text-primary" />
          The composed presentation — per-circuit progress
        </CardTitle>
        <Badge
          variant={eligible ? "success" : running ? "muted" : "outline"}
          aria-live="polite"
          role="status"
          data-zk-pill
          className="max-w-[18rem] truncate"
        >
          {running && <Loader2 className="size-3 animate-spin" />}
          {!running && eligible && <CircleCheck className="size-3" />}
          {pill}
        </Badge>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="measure text-sm text-muted-foreground">
          A full car-hire presentation composes one sub-proof per relation:{" "}
          <strong>2× scan + filter (age ≥ 25) + 2× hidden-issuer + revocation + join
          (same holder) + holder proof-of-possession</strong>. Below, each member shows
          its honest status. Exactly <strong>one</strong> of the {total} members — the
          age-gate <code className="font-mono">filter_int</code> — is proven and verified{" "}
          <strong>live in your browser</strong>; the other {total - liveCount} are
          compiled, gate-counted and proven by the native crate over{" "}
          <code className="font-mono">bb</code>, but are <strong>not</strong> run
          in-browser here. A green tick means a real in-tab cryptographic check
          happened.
        </p>
        <ol className="space-y-1.5">
          {steps.map((s, i) => (
            <li
              key={s.member.key}
              data-zk-step={s.member.key}
              data-zk-step-status={s.status}
              className={cn(
                "flex items-start gap-3 rounded-lg border bg-card p-3",
                s.status === "active" && "ring-1 ring-primary/40",
                s.status === "verified" && "ring-1 ring-[var(--success)]/30",
              )}
            >
              <span className="flex size-6 shrink-0 items-center justify-center">
                <StepIcon status={s.status} />
              </span>
              <span className="min-w-0 flex-1">
                <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
                  <span className="text-sm font-medium">
                    {i + 1}. {s.member.label}
                  </span>
                  <code className="font-mono text-[11px] text-muted-foreground">
                    {s.member.snapshot}
                  </code>
                  {s.gates != null && (
                    <span className="tabular text-[11px] text-muted-foreground">
                      · {s.gates.toLocaleString()} gates
                    </span>
                  )}
                </span>
                <span className="mt-0.5 block text-xs text-muted-foreground">
                  {s.member.proves}
                </span>
              </span>
              <span className="shrink-0">
                {s.member.live ? (
                  <Badge variant={s.status === "verified" ? "success" : "muted"}>
                    {stepStatusLabel(s.status)}
                  </Badge>
                ) : (
                  <Badge variant="muted">composed (native)</Badge>
                )}
              </span>
            </li>
          ))}
        </ol>
        <div
          className={cn(
            "rounded-lg p-4 text-center",
            eligible
              ? "bg-[color-mix(in_oklch,var(--success)_14%,transparent)] text-[var(--success)]"
              : "bg-muted text-muted-foreground",
          )}
          data-zk-verdict={eligible ? "eligible" : "pending"}
        >
          {/* [FABLE-5] opacity-100 (was 80): the label on the muted verdict card was 4.26:1, below WCAG AA 4.5:1 (color-contrast, sq-0rbfn) */}
          <p className="text-xs uppercase tracking-wide">
            Composed verdict
          </p>
          <p className="text-2xl font-semibold">
            {eligible ? "ELIGIBLE" : "—"}
          </p>
          <p className="mt-1 text-xs opacity-90">
            {eligible
              ? "as far as the live age-gate proves — the other members are composed natively, not verified in-browser here (research-grade, not externally audited)"
              : "run the age-gate to reach a verdict"}
          </p>
        </div>
      </CardContent>
    </Card>
  );
}

// [OPUS-4.8] sq-8dx2 — the fallback verdict: a captured proof verified live in-tab.
function VerifyVerdictPanel({ phase }: { phase: VerifyPhase }) {
  if (phase.kind === "error") {
    return (
      <pre className="overflow-x-auto rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-xs text-destructive">
        {phase.message}
      </pre>
    );
  }
  if (phase.kind !== "done") return null;
  const r = phase.result;
  return (
    <Card data-verify-result data-verify-ok={r.verified ? "true" : "false"}>
      <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
        <CardTitle className="flex items-center gap-2 text-base">
          {r.verified ? (
            <CircleCheck className="size-5 text-[var(--success)]" />
          ) : (
            <CircleAlert className="size-5 text-destructive" />
          )}
          Captured proof — verified live in your tab
        </CardTitle>
        <Badge variant={r.verified ? "success" : "muted"} className="tabular">
          {r.threads === 1 ? "single-thread" : `${r.threads} threads`}
        </Badge>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">
          This is the <strong>fallback</strong> path: proving (the expensive part) was
          captured ahead of time into a <code className="font-mono">ProofManifest</code>{" "}
          (<code className="font-mono">{r.member}</code>) — the bundled bytes are a{" "}
          <strong>real</strong> UltraHonk transcript, and the verify you just ran is a
          genuine in-tab <code className="font-mono">bb.js verifyProof</code>. A single
          tampered byte would make it reject. It proves the same age-gate relation
          (<em>{r.relation}</em>); the hidden age is not among the captured public inputs.
        </p>
        <div className="grid gap-3 sm:grid-cols-2">
          <Stat label="Verified in your tab" value={r.verified ? "yes" : "no"} />
          <Stat
            label="Proof size"
            value={`${(r.proofByteLength / 1024).toFixed(1)} KB`}
            sub={`${Math.round(r.proofByteLength / 32)} fields`}
          />
          <Stat label="Verify time" value={`${r.verifyMs.toFixed(0)} ms`} sub="non-canonical" />
          <Stat label="Captured" value={r.capturedAt} />
        </div>
        <a
          className="inline-flex items-center gap-1.5 text-xs text-primary underline-offset-4 hover:underline"
          href={`${siteBasePath()}/zk/car-hire-manifest.json`}
          download
        >
          <PackageCheck className="size-3.5" />
          Download the captured ProofManifest (JSON)
        </a>
      </CardContent>
    </Card>
  );
}

// [OPUS-4.8] sq-8dx2 — basePath for in-site download links (Pages serves under /sparq;
// local `next dev` and Tauri use ''). Mirrors next.config.ts's NEXT_PUBLIC_BASE_PATH
// switch and src/lib/zk-prover.ts's fetch path.
function siteBasePath(): string {
  return process.env.NEXT_PUBLIC_BASE_PATH ?? "/sparq";
}

// [OPUS-4.8] Subtle prover-readiness pill. Reuses the badge tokens; never blocks the
// UI and never alters any security claim — it only reports cold-start progress.
function ProverIndicator({ prover }: { prover: ProverState }) {
  // [OPUS-4.8] sq-5q63 — `data-prover` mirrors the lifecycle state for the headless
  // smoke test to await deterministically (regardless of label text/locale/styling).
  if (prover === "ready") {
    return (
      <Badge variant="muted" aria-live="polite" data-prover="ready">
        <CircleCheck className="size-3 text-[var(--success)]" /> Prover ready
      </Badge>
    );
  }
  if (prover === "error") {
    return (
      <Badge variant="warning" aria-live="polite" data-prover="error">
        <CircleAlert className="size-3" /> Prover load failed — retries on proof
      </Badge>
    );
  }
  return (
    <Badge variant="muted" aria-live="polite" data-prover="warming">
      <Loader2 className="size-3 animate-spin" /> Initializing ZK prover…
    </Badge>
  );
}

function FlowStrip() {
  const steps = [
    { icon: FileLock2, label: "RDF credentials", note: "private, on-device" },
    { icon: ShieldCheck, label: "SPARQL eligibility query", note: "the public relation" },
    { icon: KeyRound, label: "ZK proof (UltraHonk)", note: "generated in your tab" },
    { icon: CircleCheck, label: "Desk sees “Eligible”", note: "+ the proof, nothing else" },
  ];
  return (
    <ol className="grid gap-3 rounded-xl bg-muted/30 p-4 ring-1 ring-foreground/10 sm:grid-cols-7 sm:items-center">
      {steps.map((s, i) => (
        <React.Fragment key={s.label}>
          <li className="flex items-center gap-3 sm:col-span-1 sm:flex-col sm:gap-1.5 sm:text-center">
            <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <s.icon className="size-4" />
            </span>
            <span className="min-w-0">
              <span className="block text-xs font-medium leading-tight">{s.label}</span>
              <span className="block text-[11px] text-muted-foreground">{s.note}</span>
            </span>
          </li>
          {i < steps.length - 1 && (
            <li aria-hidden className="hidden justify-center text-muted-foreground/50 sm:flex">
              <ArrowRight className="size-4" />
            </li>
          )}
        </React.Fragment>
      ))}
    </ol>
  );
}

function VerdictPanel({ phase, age }: { phase: Phase; age: number }) {
  if (phase.kind === "rejected") {
    return (
      <Card>
        <CardContent className="flex flex-col gap-2 pt-5 sm:flex-row sm:items-center">
          <span className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-[color-mix(in_oklch,var(--warning)_18%,transparent)] text-[var(--warning)]">
            <CircleAlert className="size-5" />
          </span>
          <div>
            <p className="font-semibold">Not eligible — and no proof exists</p>
            <p className="text-sm text-muted-foreground">{phase.message}</p>
          </div>
        </CardContent>
      </Card>
    );
  }
  if (phase.kind === "error") {
    return (
      <pre className="overflow-x-auto rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-xs text-destructive">
        {phase.message}
      </pre>
    );
  }
  if (phase.kind !== "done") return null;
  const r = phase.result;
  return (
    // [OPUS-4.8] sq-5q63 — `data-proof-verified` lets the smoke test await the first
    // completed proof and assert it verified, without scraping copy.
    <Card data-proof-result data-proof-verified={r.verified ? "true" : "false"}>
      <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
        <CardTitle className="flex items-center gap-2 text-base">
          {r.verified ? (
            <CircleCheck className="size-5 text-[var(--success)]" />
          ) : (
            <CircleAlert className="size-5 text-destructive" />
          )}
          What the car-hire desk receives
        </CardTitle>
        <Badge variant={r.verified ? "success" : "muted"} className="tabular">
          {r.threads === 1 ? "single-thread" : `${r.threads} threads`}
        </Badge>
      </CardHeader>
      <CardContent className="space-y-4">
        <div
          className={cn(
            "rounded-lg p-4 text-center",
            r.eligible
              ? "bg-[color-mix(in_oklch,var(--success)_14%,transparent)] text-[var(--success)]"
              : "bg-muted text-muted-foreground",
          )}
        >
          {/* [FABLE-5] opacity-100 (was 80): same low-contrast muted-verdict-card label as above (color-contrast, sq-0rbfn) */}
          <p className="text-xs uppercase tracking-wide">
            Eligibility verdict (the only age-derived bit revealed)
          </p>
          <p className="text-2xl font-semibold">
            ELIGIBLE: {r.eligible ? "true" : "false"}
          </p>
        </div>

        <div className="grid gap-3 sm:grid-cols-2">
          <Stat label="Proof verified in your tab" value={r.verified ? "yes" : "no"} />
          {/* [OPUS-4.8] proofByteLength is BYTES (= fields × 32). Surface KB honestly
              and derive the field count, instead of mislabelling raw bytes as "fields". */}
          <Stat
            label="Proof size"
            value={`${(r.proofByteLength / 1024).toFixed(1)} KB`}
            sub={`${Math.round(r.proofByteLength / 32)} fields`}
          />
          <Stat label="Prove time" value={`${r.proveMs.toFixed(0)} ms`} sub="non-canonical" />
          <Stat label="Verify time" value={`${r.verifyMs.toFixed(0)} ms`} sub="non-canonical" />
        </div>

        <div className="rounded-lg border bg-muted/30 p-3">
          <p className="mb-1.5 flex items-center gap-1.5 text-xs font-medium">
            <Eye className="size-3.5" /> Public inputs the desk verifies against
          </p>
          <ul className="space-y-0.5 text-xs text-muted-foreground">
            {PUBLIC_INPUT_LABELS.map((l) => (
              <li key={l} className="font-mono">
                · {l}
              </li>
            ))}
          </ul>
          <p className="mt-2 flex items-center gap-1.5 text-xs font-medium text-[var(--success)]">
            <EyeOff className="size-3.5" /> The age {age} is the private witness — it
            is <span className="font-mono">not</span> among the public inputs.
          </p>
        </div>
      </CardContent>
    </Card>
  );
}

function Stat({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="rounded-lg border bg-card p-3">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="tabular text-lg font-semibold">
        {value}
        {sub && <span className="ml-1 text-[10px] font-normal text-muted-foreground">{sub}</span>}
      </p>
    </div>
  );
}

function DisclosurePanel() {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">What is revealed vs hidden</CardTitle>
      </CardHeader>
      <CardContent>
        <div tabIndex={0} className="overflow-x-auto rounded-lg ring-1 ring-foreground/10">
          <table className="w-full text-left text-sm">
            <thead className="bg-muted/50">
              <tr>
                <th className="px-4 py-2.5 font-medium text-[var(--success)]">
                  <span className="inline-flex items-center gap-1.5">
                    <Eye className="size-3.5" /> The desk sees (public)
                  </span>
                </th>
                <th className="px-4 py-2.5 font-medium">
                  <span className="inline-flex items-center gap-1.5">
                    <EyeOff className="size-3.5" /> Stays private
                  </span>
                </th>
              </tr>
            </thead>
            <tbody>
              {DISCLOSURE.map((row) => (
                <tr key={row.reveals} className="border-t align-top">
                  <td className="px-4 py-2.5">{row.reveals}</td>
                  <td className="px-4 py-2.5 text-muted-foreground">{row.hides}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </CardContent>
    </Card>
  );
}

// [OPUS-4.8] sq-1s2 — the CRITICAL HONESTY rendering: what each circuit-family member
// verifies, in plain language, with an explicit PUBLIC-vs-HIDDEN split and a LIVE /
// WIRED / DESIGNED status. Gate counts are joined live from the deterministic snapshot
// (src/data/zk-circuit-costs.json) via the representative member id — never hard-coded
// here. This panel must NEVER imply the deployed demo verifies signatures, scans, joins,
// revocation or holder keys: only the age-gate FILTER (filter_int_d2) is proven live.
function statusLabel(s: FamilyStatus): string {
  return s === "live" ? "live in your tab" : s === "wired" ? "wired (native)" : "designed";
}

function StatusBadge({ status }: { status: FamilyStatus }) {
  if (status === "live") {
    return (
      <Badge variant="success">
        <CircleCheck className="size-3" /> {statusLabel(status)}
      </Badge>
    );
  }
  // wired + designed are BOTH "not proven live in your browser" — keep them visibly
  // distinct from "live" so no reader mistakes a composed member for a live one.
  return (
    <Badge variant={status === "wired" ? "muted" : "warning"}>
      {status === "designed" && <CircleAlert className="size-3" />}
      {statusLabel(status)}
    </Badge>
  );
}

function WhatThisProvesPanel() {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">
          What each circuit proves — public vs hidden
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="measure text-sm text-muted-foreground">
          A full car-hire presentation composes one sub-proof per relation. Each member
          binds its statement to a public commitment and proves one SPARQL-shaped
          primitive over <strong>committed</strong> credential graphs. Below: what each
          verifies, what the desk sees (public) versus what stays the holder’s secret
          (hidden), and — the honest part — whether it runs <strong>live in your tab</strong>{" "}
          today or is only compiled / composed.
        </p>

        {/* The load-bearing live-vs-designed distinction, stated up-front. */}
        <div className="rounded-lg bg-[color-mix(in_oklch,var(--warning)_10%,transparent)] p-3 text-sm ring-1 ring-[var(--warning)]/25">
          <p className="flex items-start gap-2 text-foreground">
            <CircleAlert className="mt-0.5 size-4 shrink-0 text-[var(--warning)]" />
            <span>
              <strong>Live vs designed.</strong> Exactly one statement is proven live in
              your browser: the age-gate <code className="font-mono">FILTER</code>{" "}
              (<code className="font-mono">filter_int_d2</code>). The other members are{" "}
              <strong>wired</strong> (compiled + gate-counted + exercised by native tests)
              or <strong>designed</strong> (gadget complete, with a documented open seam) —
              <strong> not</strong> run in-browser. The full six-member cross-credential
              composition has <strong>never been assembled and verified as one unit</strong>.
              The deployed demo does <strong>not</strong> verify any signature, scan, join,
              revocation or holder key.
            </span>
          </p>
        </div>

        <div className="space-y-3">
          {CIRCUIT_FAMILY_EXPLAINER.map((f) => {
            const m = ZK_MEMBERS.find((x) => x.member === f.representative);
            return (
              <div key={f.family} className="rounded-lg border bg-card p-4">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-semibold capitalize">{f.family}</span>
                    <code className="font-mono text-[11.5px] text-muted-foreground">
                      {f.representative}
                    </code>
                    {m && (
                      <span className="tabular text-[11px] text-muted-foreground">
                        · {m.gate_count.toLocaleString()} gates
                      </span>
                    )}
                  </div>
                  <StatusBadge status={f.status} />
                </div>
                <p className="mt-1.5 text-sm">{f.proves}</p>
                <div className="mt-2 grid gap-2 sm:grid-cols-2">
                  <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
                    <Eye className="mt-0.5 size-3.5 shrink-0 text-[var(--success)]" />
                    <span>
                      <span className="font-medium text-foreground">Public:</span>{" "}
                      {f.publicInputs}
                    </span>
                  </p>
                  <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
                    <EyeOff className="mt-0.5 size-3.5 shrink-0" />
                    <span>
                      <span className="font-medium text-foreground">Hidden:</span>{" "}
                      {f.hidden}
                    </span>
                  </p>
                </div>
                <p className="mt-2 text-[11.5px] text-muted-foreground">
                  <span className="font-medium">Primitive:</span> {f.primitive}
                </p>
                <p className="mt-1 text-[11.5px] text-muted-foreground">
                  <span className="font-medium">Status:</span> {f.statusNote}
                </p>
              </div>
            );
          })}
        </div>

        <p className="text-xs text-muted-foreground">
          “Proves” statements are <em>as designed / as landed</em>, not an audited
          guarantee. The composition verifier is{" "}
          <strong className="text-foreground">not yet sound</strong>{" "}
          (<code className="font-mono">sq-qhy4</code>) and the estate is research-grade and{" "}
          <strong className="text-foreground">not externally audited</strong> — see the
          honest-limits note below.
        </p>
      </CardContent>
    </Card>
  );
}

function CircuitFamilyPanel() {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">The composed circuit family</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="measure text-sm text-muted-foreground">
          A full car-hire presentation composes one sub-proof per relation. This
          page proves the <strong>age-gate</strong> member live in your tab; the
          others are real sparq circuits (compiled + gate-counted today) that the
          native crate proves over <code className="font-mono">bb</code> and that
          compose into one <code className="font-mono">ProofManifest</code>. Wiring
          the remaining members into the browser is tracked as a follow-up.
        </p>
        <div tabIndex={0} className="overflow-x-auto rounded-lg ring-1 ring-foreground/10">
          <table className="w-full text-left text-sm">
            <thead className="bg-muted/50">
              <tr>
                <th className="px-4 py-2.5 font-medium">Circuit</th>
                <th className="px-4 py-2.5 font-medium">Proves</th>
                <th className="px-4 py-2.5 font-medium">Gates</th>
                <th className="px-4 py-2.5 font-medium">In-tab</th>
              </tr>
            </thead>
            <tbody>
              {CIRCUIT_FAMILY.map((c) => {
                // Gate count is sourced live from the deterministic snapshot JSON, not
                // re-typed here — the row's own `gates` is a build-time consistency check.
                const snapshot = ZK_MEMBERS.find((m) => m.member === c.name);
                const gates = snapshot ? snapshot.gate_count : c.gates;
                return (
                  <tr key={c.name} className="border-t align-top">
                    <td className="px-4 py-2.5 font-mono text-[12.5px]">{c.name}</td>
                    <td className="px-4 py-2.5 text-muted-foreground">{c.role}</td>
                    <td className="tabular px-4 py-2.5">{gates.toLocaleString()}</td>
                    <td className="px-4 py-2.5">
                      {c.live ? (
                        <Badge variant="success">live</Badge>
                      ) : (
                        <Badge variant="muted">composed</Badge>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        <p className="text-xs text-muted-foreground">
          Gate counts are the measured <code className="font-mono">bb gates</code>{" "}
          snapshot (non-canonical, for scale). All members sit roughly 15× under the
          bb.js single-thread browser ceiling.
        </p>
      </CardContent>
    </Card>
  );
}

function HonestyPanel() {
  return (
    <Card className="ring-[var(--warning)]/30">
      <CardHeader className="flex-row items-center gap-2 space-y-0">
        <CircleAlert className="size-5 text-[var(--warning)]" />
        <CardTitle className="text-base">
          Honest limits — read before you trust this
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3 text-sm text-muted-foreground">
        <p>
          The age-gate proof on this page is <strong>real</strong>: a genuine
          UltraHonk proof of the sparq{" "}
          <code className="font-mono">filter_int</code> circuit, generated and
          verified by <code className="font-mono">@aztec/bb.js</code> in your
          browser. The exact age is the private witness and never appears in the
          public inputs.
        </p>
        <p>
          <strong className="text-foreground">On proof size and speed.</strong>{" "}
          The proof is <strong>inherently KB-scale</strong>: UltraHonk is a
          transparent, no-trusted-setup proof system whose proofs are kilobytes of
          field elements — not the ~200&nbsp;byte single proof of a Groth16 SNARK, and
          there is no compact mode (a sub-KB single proof would need recursive
          aggregation, which is out of scope here). This page uses the keccak-oracle
          flavour (<code className="font-mono">verifierTarget:&nbsp;&apos;evm&apos;</code>),
          which is ~43% smaller than the default while staying{" "}
          <strong>fully zero-knowledge</strong>. As a hard guardrail, the demo never
          uses any <code className="font-mono">*-no-zk</code> flavour
          (<code className="font-mono">disableZk:&nbsp;true</code>): those strip the
          ZK masking and would make the private age recoverable in principle. On plain
          GitHub Pages a service-worker shim is the only lever that unlocks
          multithreaded proving (timings are non-canonical and host-dependent).
        </p>
        <p>
          <strong className="text-foreground">
            But the sparq ZK estate is research-grade and has NOT been externally
            audited.
          </strong>{" "}
          No external accredited cryptographer has reviewed any part of sparq’s
          bespoke cryptography. The verifier-soundness claim rests entirely on
          sparq’s own internal, single-model self-audits. An external-cryptographer
          audit is <strong>required</strong> before any ZK security, privacy or
          attestation claim may be relied upon in production (tracked as bead{" "}
          <code className="font-mono">sq-qhy4</code>, and as gap{" "}
          <code className="font-mono">CR-G1</code> in{" "}
          <code className="font-mono">compliance/cryptoreview/</code>).
        </p>
        <p>
          The published security posture (<code className="font-mono">SECURITY.md</code>)
          treats the v1 verifier as <strong>NOT sound</strong> for a relying party.
          An internal re-audit finds it &ldquo;sound as landed&rdquo; under a stated
          threat model — but that is remediation progress, <em>not</em> a production
          guarantee, and it is single-model (pending re-review). Known privacy
          deferrals remain: holder possession is not yet bound to the specific
          credential, and the status-list IRI/version are disclosed (linkability).
        </p>
        <p className="rounded-lg bg-muted/50 p-3 text-foreground">
          <strong>Bottom line:</strong> this page demonstrates the{" "}
          <em>flow and the mechanics</em> of a sparq ZK query-proof, with a real
          proof in your tab. It does <strong>not</strong> assert that the proof is
          cryptographically trustworthy for production use. Treat any
          &ldquo;verified&rdquo; result here as a research demonstration.
        </p>
        <p className="flex flex-wrap gap-3 pt-1">
          <a
            className="text-primary underline-offset-4 hover:underline"
            href="https://github.com/sparq-org/sparq/blob/main/SECURITY.md"
            target="_blank"
            rel="noopener noreferrer"
          >
            SECURITY.md
          </a>
          <a
            className="text-primary underline-offset-4 hover:underline"
            href="https://github.com/sparq-org/sparq/tree/main/compliance/cryptoreview"
            target="_blank"
            rel="noopener noreferrer"
          >
            Cryptographic review readiness
          </a>
          <a
            className="text-primary underline-offset-4 hover:underline"
            href="https://github.com/sparq-org/sparq/tree/main/zk/compose"
            target="_blank"
            rel="noopener noreferrer"
          >
            The Noir circuits
          </a>
        </p>
      </CardContent>
    </Card>
  );
}
