// [OPUS-4.8] sq-1s2 — the per-member ZK circuit GATE / TIME cost breakdown on the
// in-site ZK benchmark page. Consumes the deterministic snapshot
// (src/data/zk-circuit-costs.json) via the typed accessor — no number is hard-coded
// here. Matches the benchmark-page UX (rounded ring tables, muted headers, honest
// "soon" / placeholder framing). Gate counts are the only deterministic figures; every
// timing is labelled INDICATIVE / NON-CANONICAL; null timings render as "—", never a
// guess.
//
// PRIVACY-CLAIMS GATE (sq-qhy4 / sq-9hrn / sq-1s2): the v1 composition verifier is NOT
// externally audited and NOT-yet-sound. These are indicative engineering micro-numbers,
// not an audited cryptographic guarantee — surfaced verbatim from the JSON caveats.
import { CircleAlert } from "lucide-react";

import {
  ZK_MEMBERS_BY_GATES,
  ZK_CONSTANT_COSTS,
  ZK_GATE_TOOL,
  ZK_BB_VERSION,
  ZK_NARGO_VERSION,
  ZK_TIMING_CAVEAT,
  ZK_SOUNDNESS_CAVEAT,
  ZK_HEAVIEST,
  ZK_CHEAPEST,
  ZK_ISSUER_MEMBER,
  ZK_LIVE_MEMBER,
  zkFamilyOf,
  type ZkCircuitMember,
} from "@/data/zk-circuit-costs";

const FAMILY_LABEL: Record<ReturnType<typeof zkFamilyOf>, string> = {
  filter: "filter",
  scan: "scan",
  join: "join",
  issuer: "issuer",
  holder: "holder",
  revoke: "revoke",
};

/** A measured single-thread integer-ms, or an em-dash when unmeasured (never a guess). */
function ms(value: number | null): string {
  return value == null ? "—" : `${Math.round(value)} ms`;
}

/** The flat filter gate count (every filter_*_d* member is identical) — derived, not typed. */
function filterFlatGates(): number | null {
  const flat = ZK_MEMBERS_BY_GATES.filter(
    (m) => m.member.startsWith("filter_int") || m.member.startsWith("filter_f64_d"),
  );
  if (flat.length === 0) return null;
  const first = flat[0].gate_count;
  return flat.every((m) => m.gate_count === first) ? first : null;
}

function CostRow({ m }: { m: ZkCircuitMember }) {
  const isHeaviest = m.member === ZK_HEAVIEST.member;
  const isCheapest = m.member === ZK_CHEAPEST.member;
  return (
    <tr className="border-b align-top last:border-0 hover:bg-muted/30">
      <td className="px-3 py-2">
        <span className="block font-mono text-[12.5px]">{m.member}</span>
        <span className="text-[11px] text-muted-foreground">
          {FAMILY_LABEL[zkFamilyOf(m.member)]}
          {m.exercised_live && " · live in-browser"}
          {isHeaviest && " · heaviest"}
          {isCheapest && " · cheapest"}
        </span>
      </td>
      <td className="px-3 py-2 text-right font-mono tabular-nums">
        {m.gate_count.toLocaleString()}
      </td>
      <td className="px-3 py-2 text-right font-mono tabular-nums text-muted-foreground">
        {ms(m.prove_ms_t1)}
      </td>
      <td className="px-3 py-2 text-right font-mono tabular-nums text-muted-foreground">
        {ms(m.prove_ms_tN)}
      </td>
    </tr>
  );
}

export function ZkCircuitCostBreakdown() {
  const flatFilter = filterFlatGates();
  return (
    <section className="space-y-4">
      <header className="space-y-1.5">
        <h2 className="text-lg font-semibold">Circuit gate &amp; proving-time cost</h2>
        <p className="measure text-sm text-muted-foreground">
          Per-member cost for the sparq ZK circuit family, sorted by gate count. Gate
          counts are the <strong>deterministic</strong>{" "}
          <code className="font-mono">{ZK_GATE_TOOL}</code> snapshot (bb{" "}
          <code className="font-mono">{ZK_BB_VERSION}</code>, nargo{" "}
          <code className="font-mono">{ZK_NARGO_VERSION}</code>) — the only figures of
          record here. Proving times are <strong>indicative / non-canonical</strong> and
          are shown only where a measurement exists (&ldquo;—&rdquo; = unmeasured, not
          estimated).
        </p>
      </header>

      {/* Honesty banner — the privacy-claims gate. */}
      <div className="rounded-lg bg-[color-mix(in_oklch,var(--warning)_10%,transparent)] p-3 text-sm ring-1 ring-[var(--warning)]/25">
        <p className="flex items-start gap-2 text-muted-foreground">
          <CircleAlert className="mt-0.5 size-4 shrink-0 text-[var(--warning)]" />
          <span>
            <strong className="text-foreground">Research-grade, not externally audited.</strong>{" "}
            {ZK_SOUNDNESS_CAVEAT} A fast or passing proof is <strong>not</strong> a
            soundness or privacy guarantee.
          </span>
        </p>
      </div>

      <div tabIndex={0} className="overflow-x-auto rounded-lg ring-1 ring-foreground/10">
        <table className="w-full border-collapse text-sm">
          <thead>
            <tr className="border-b bg-muted/40 text-left">
              <th className="px-3 py-2 font-medium">Member</th>
              <th className="px-3 py-2 text-right font-medium">
                Gates <span className="font-normal text-muted-foreground">(circuit_size)</span>
              </th>
              <th className="px-3 py-2 text-right font-medium">
                Prove t1 <span className="font-normal text-muted-foreground">(indic.)</span>
              </th>
              <th className="px-3 py-2 text-right font-medium">
                Prove t8 <span className="font-normal text-muted-foreground">(indic.)</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {ZK_MEMBERS_BY_GATES.map((m) => (
              <CostRow key={m.member} m={m} />
            ))}
          </tbody>
        </table>
      </div>

      {/* Hotspots — every number derived from the snapshot, not typed. */}
      <div className="rounded-lg border bg-muted/20 p-4 text-sm">
        <p className="mb-1.5 font-medium">Hotspots</p>
        <ul className="space-y-1 text-muted-foreground">
          <li>
            <strong className="text-foreground">Scan is the heavy end</strong> —{" "}
            <code className="font-mono">{ZK_HEAVIEST.member}</code> is heaviest at{" "}
            <span className="tabular">{ZK_HEAVIEST.gate_count.toLocaleString()}</span> gates
            (per-graph Poseidon2 commitment recompute + the completeness double-loop).
          </li>
          {ZK_ISSUER_MEMBER && (
            <li>
              <strong className="text-foreground">In-circuit signature verification is expensive</strong>{" "}
              — <code className="font-mono">{ZK_ISSUER_MEMBER.member}</code> at{" "}
              <span className="tabular">{ZK_ISSUER_MEMBER.gate_count.toLocaleString()}</span>{" "}
              gates (four ~251-bit twisted-Edwards scalar muls dominate — <code className="font-mono">s*G</code>{" "}
              and <code className="font-mono">e*pk</code> for the Schnorr equation, plus the two{" "}
              <code className="font-mono">[L]*P</code> prime-order-subgroup checks the sq-l15mi
              torsion-key soundness fix added; the heaviest non-scan operator).
            </li>
          )}
          {flatFilter != null && (
            <li>
              <strong className="text-foreground">Filter is flat in digit-count</strong> — every{" "}
              <code className="font-mono">filter_*</code> token-binding member is{" "}
              <span className="tabular">{flatFilter.toLocaleString()}</span> gates (the
              blake3 token fits one 64-byte block; <code className="font-mono">D</code>{" "}
              only changes what leaks).
            </li>
          )}
          <li>
            <strong className="text-foreground">Cheapest</strong> —{" "}
            <code className="font-mono">{ZK_CHEAPEST.member}</code> at{" "}
            <span className="tabular">{ZK_CHEAPEST.gate_count.toLocaleString()}</span> gates
            (a depth-10 Merkle bit-unset).
          </li>
        </ul>
      </div>

      {/* Constant-across-the-family costs + browser overhead. */}
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="rounded-lg border bg-card p-4 text-sm">
          <p className="mb-1.5 font-medium">Proof size &amp; verify — constant across the family</p>
          <p className="text-muted-foreground">
            UltraHonk succinctness: proof, vk size and verify time do <strong>not</strong>{" "}
            vary by member.
          </p>
          <ul className="mt-2 space-y-0.5 text-muted-foreground">
            <li>
              Proof:{" "}
              <span className="tabular text-foreground">
                {(ZK_CONSTANT_COSTS.proof_bytes_noir_recursive / 1024).toFixed(1)} KB
              </span>{" "}
              (noir-recursive) /{" "}
              <span className="tabular text-foreground">
                {(ZK_CONSTANT_COSTS.proof_bytes_evm_keccak / 1024).toFixed(1)} KB
              </span>{" "}
              (evm/keccak, still zero-knowledge)
            </li>
            <li>
              Verifying key:{" "}
              <span className="tabular text-foreground">
                {(ZK_CONSTANT_COSTS.vk_bytes / 1024).toFixed(1)} KB
              </span>
            </li>
            <li>
              Verify:{" "}
              <span className="tabular text-foreground">
                ~{ZK_CONSTANT_COSTS.verify_ms_aarch64} ms
              </span>{" "}
              <span className="text-[11px]">(aarch64, indicative)</span>
            </li>
          </ul>
        </div>

        <div className="rounded-lg border bg-card p-4 text-sm">
          <p className="mb-1.5 font-medium">Browser single-thread overhead (~4.2×)</p>
          <p className="text-muted-foreground">
            The deployed GitHub Pages demo is forced single-threaded: bb.js worker fan-out
            needs <code className="font-mono">SharedArrayBuffer</code>, which requires
            cross-origin isolation that Pages cannot set. A COI service-worker shim is the
            lever that unlocks multithreaded proving.
          </p>
          {ZK_LIVE_MEMBER && (
            <p className="mt-2 text-muted-foreground">
              On the one live member (
              <code className="font-mono">{ZK_LIVE_MEMBER.member}</code>): prove{" "}
              <span className="tabular text-foreground">{ms(ZK_LIVE_MEMBER.prove_ms_t1)}</span>{" "}
              single-thread →{" "}
              <span className="tabular text-foreground">{ms(ZK_LIVE_MEMBER.prove_ms_tN)}</span>{" "}
              at 8 threads (indicative). Threads change only speed, never the proof or its
              zero-knowledge property.
            </p>
          )}
        </div>
      </div>

      <p className="text-xs text-muted-foreground">{ZK_TIMING_CAVEAT}</p>
    </section>
  );
}
