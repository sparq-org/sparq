// [OPUS-4.8] sq-13rg — REAL in-browser zero-knowledge proving for the car-hire
// age-gate, via Noir + Barretenberg UltraHonk compiled to WASM.
//
// This is NOT a simulation. The age-gate sub-proof of the car-hire flagship runs
// the genuine sparq ZK circuit `zk/compose/filter_int_d2` (compiled with
// `nargo 1.0.0-beta.21` to the ACIR JSON shipped at `/zk/filter_int_d2.json`) and
// proves it with `@aztec/bb.js` (UltraHonk, the same prover the native crate drives
// over its `bb` subprocess) entirely in your tab:
//
//   private witness: the holder's exact age (the decimal digits)
//   public inputs:   challenge, operand_enc (the committed term anchor), op (≥),
//                    bound (25), expected (true)  — the age itself is NEVER public.
//
// The circuit (`sparq_zk_compose_core::filter_int`) rebuilds the canonical
// `"<digits>"^^xsd:integer` N-Triples token in-circuit, hashes it (blake3 blackbox),
// and asserts `h2(LITERAL, hs) == operand_enc` — binding the hidden value to the
// committed credential — then asserts the comparison verdict. A false claim
// (e.g. age 24 proving "≥ 25 = true") is UNSATISFIABLE: the witness solve fails, so
// no proof can be produced. That is the soundness the demo demonstrates live.
//
// HONESTY: the *cryptographic mechanics* here are real and run in your browser. The
// broader sparq ZK estate is research-grade, internally reviewed only, and NOT
// externally audited — see the page's honesty panel and SECURITY.md. Do not treat a
// proof produced here as production-trustworthy.
//
// bb.js + noir_js are lazy-loaded via dynamic import (client-only) so the ~MB WASM
// never enters the main page bundle, mirroring the lean-wasm REPL loader.

/** Field encodings of `"<age>"^^xsd:integer`, precomputed natively via
 *  `sparq_zk_compose::build::encode_int_literal` (the SAME encoder the scan proof
 *  uses for the operand column). Each is the `operand_enc` public input that binds
 *  the hidden age to the committed credential.
 *
 *  DRIFT GUARD (sq-1s2.4): these values are pinned against the native encoder by
 *  `crates/sparq-zk-compose/tests/site_age_enc_drift.rs`. If a circuit / encoder bump
 *  changes the term encoding, that test fails RED and prints the regenerated hex to
 *  paste here — do NOT let these go stale, or the live in-browser age-gate becomes
 *  unsatisfiable (the witness solve fails). */
const AGE_OPERAND_ENC: Record<number, string> = {
  24: "0x1c8a81ea95b253e105b99209deff1a4908be9568e588fbd89afea9f49f5f20cf",
  25: "0x2b5caeb2bbd290ab32434a9109030784c7faebadee7a9908d24dccb847910d1d",
  30: "0x132fa587351bf3f12fd3cbed64d5526f28791099d1d40870f94595873c78fa72",
  42: "0x1a4aa7fd962d0004ac2294cc98471ea1ebfdad74a8f702e89fedf83f92d0f97b",
};

/** The ages we can prove against (those for which we shipped a committed
 *  `operand_enc`). All are 2-digit, matching the `filter_int_d2` circuit member. */
export const PROVABLE_AGES = Object.keys(AGE_OPERAND_ENC)
  .map(Number)
  .sort((a, b) => a - b);

export const AGE_THRESHOLD = 25;

/**
 * [OPUS-4.8] sq-8dx2 — the fallback "captured ProofManifest + live in-tab verify".
 *
 * `verifyCapturedManifest` fetches the pre-captured car-hire manifest
 * (`/zk/car-hire-manifest.json`, produced by `scripts/capture-zk-manifest.mjs` or the
 * native `sparq_zk_compose::capture` seam) and runs a GENUINE bb.js `verifyProof` over
 * its bundled age-gate sub-proof — in your tab. This is the honest fallback for
 * devices/browsers where live PROVING (the expensive part) is too heavy: verify is
 * cheap, and the bundled proof bytes are a real UltraHonk transcript (a tamper of any
 * byte makes verify reject). It is NOT a fresh proof — it replays one captured proof
 * and checks it live. This demo path is not designed to keep the compared age confidential.
 *
 * The manifest carries a `subProofs` ARRAY (the fuller car-hire manifest may bundle
 * several native per-circuit sub-proofs). The in-tab fallback verifies the age-gate
 * member (`filter_int_d2`) — the only circuit whose ACIR is shipped to the browser;
 * verifying the other bundled members in-tab needs their circuits shipped (a follow-up).
 * This interface MUST stay in lock-step with the crate's serialized shape
 * (`sparq_zk_compose::capture::{CapturedCarHireManifest, CapturedSubProof}`).
 */
export interface CapturedSubProof {
  member: string;
  relation: string;
  proof: number[];
  publicInputs: string[];
}
export interface CapturedManifest {
  type: string;
  note: string;
  capturedAt: string;
  subProofs: CapturedSubProof[];
}

/** The age-gate circuit member — the only one whose ACIR the browser ships, hence the
 *  only bundled sub-proof the in-tab fallback can `verifyProof`. */
const AGE_GATE_MEMBER = "filter_int_d2";
export interface VerifyResult {
  /** True iff bb.js verified the captured sub-proof in-tab. */
  verified: boolean;
  member: string;
  relation: string;
  proofByteLength: number;
  publicInputs: string[];
  capturedAt: string;
  verifyMs: number;
  threads: number;
}

export interface ProofResult {
  /** The disclosed eligibility verdict (the only age-derived bit revealed). */
  eligible: boolean;
  /** The proof bytes. This demo path does not keep the compared age confidential. */
  proofByteLength: number;
  /** The public inputs the verifier sees (NOT the age). */
  publicInputs: string[];
  /** Independent in-tab verification of the proof against the same circuit. */
  verified: boolean;
  proveMs: number;
  verifyMs: number;
  /** bb.js thread count actually used (1 on a non-cross-origin-isolated host). */
  threads: number;
}

interface NoirModule {
  Noir: new (circuit: unknown) => {
    execute(inputs: Record<string, unknown>): Promise<{ witness: Uint8Array }>;
  };
}
/** Subset of `@aztec/bb.js`'s `UltraHonkBackendOptions` we use. We pass
 *  `verifierTarget: 'evm'` (the keccak-oracle flavour) to BOTH prove and verify —
 *  it leaves `disableZk` at its default `false`, so the proof stays FULLY
 *  zero-knowledge (age-hiding), while shrinking the wire proof ~43% (14656 → 8384 B).
 *  We NEVER pass any `*-no-zk` flavour: those set `disableZk:true` and strip the
 *  ZK masking, which would make the private age recoverable in principle. */
type VerifierTarget = "evm" | "noir-recursive" | "starknet";
interface BbBackendOptions {
  verifierTarget?: VerifierTarget;
}
interface BbModule {
  Barretenberg: { new: (opts: { threads: number }) => Promise<unknown> };
  UltraHonkBackend: new (
    bytecode: string,
    api: unknown,
  ) => {
    generateProof(
      witness: Uint8Array,
      options?: BbBackendOptions,
    ): Promise<{ proof: Uint8Array; publicInputs: string[] }>;
    verifyProof(
      proof: {
        proof: Uint8Array;
        publicInputs: string[];
      },
      options?: BbBackendOptions,
    ): Promise<boolean>;
  };
}

/**
 * [OPUS-4.8] The UltraHonk proving/verifying flavour. `verifierTarget: 'evm'` selects
 * the keccak transcript oracle and yields a ~43% smaller proof (8384 B vs the default
 * poseidon2 flavour's 14656 B) at no measured time cost, while keeping `disableZk`
 * false — i.e. the proof is STILL zero-knowledge and the private age is still hidden
 * (verified by a Node re-measurement: two proofs of the same witness differ, and the
 * age digits never appear in the 5 public inputs).
 *
 * Prove and verify MUST be passed the SAME options or the in-tab self-verify fails.
 *
 * ⚠️ `@aztec/bb.js` is pinned to a NIGHTLY (see package.json). The `verifierTarget`
 * option surface can shift between nightlies — RE-TEST this flavour (size still
 * ~8384 B, `verified === true`, proofs still randomised, age still not public) on ANY
 * bb.js version bump before trusting it. Never substitute a `*-no-zk` flavour.
 */
const PROOF_OPTIONS: BbBackendOptions = { verifierTarget: "evm" };

function basePath(): string {
  return process.env.NEXT_PUBLIC_BASE_PATH ?? "/sparq";
}

/**
 * [OPUS-4.8] The cold-start cost of the in-browser prover, paid once and shared.
 *
 * Three expensive things must happen before the first proof can be produced, and
 * none of them depend on the chosen age:
 *   1. dynamic-import the `@noir-lang/noir_js` + `@aztec/bb.js` chunks (the ~MB WASM
 *      glue, deliberately kept out of the main page bundle), and
 *   2. fetch + parse the committed ACIR artifact (`/zk/filter_int_d2.json`), and
 *   3. instantiate the Barretenberg WASM backend (`Barretenberg.new` — the dominant
 *      cold cost) and build the `UltraHonkBackend` over the circuit bytecode.
 *
 * {@link prewarmProver} kicks this off in the background on ZK-route mount so the
 * first "Generate ZK proof" click pays none of it. {@link proveAgeEligibility}
 * awaits the SAME promise as a safety net, so a click that lands before pre-warm
 * finishes (or when pre-warm was never called) is still correct — it simply waits.
 */
interface ProverContext {
  Noir: NoirModule["Noir"];
  backend: InstanceType<BbModule["UltraHonkBackend"]>;
  circuit: { bytecode: string };
  threads: number;
}

let proverPromise: Promise<ProverContext> | null = null;

/**
 * [OPUS-4.8] sq-5q63 — observable cold-start counter for the browser smoke test.
 *
 * Incremented exactly ONCE per real cold start — i.e. each time the expensive
 * {@link loadProver} body (dynamic-import of noir_js + bb.js, circuit fetch, and the
 * dominant `Barretenberg.new` WASM instantiate) actually runs, NOT on the memoised
 * fast path. Because {@link loadProver} shares one `proverPromise`, a correctly
 * pre-warmed prover leaves this at 1 across the first {@link proveAgeEligibility}; a
 * regression that re-paid the cold start on the first proof would bump it to 2.
 *
 * Mirrored onto `window.__zkProverColdStarts` purely so a headless Playwright run can
 * read it without scraping the UI. This is pure test observability: it does not touch
 * what is proved, the witness, the public inputs, or any security property of the
 * proof. The counter only ever increases; it is never reset by product code.
 */
let coldStarts = 0;

const COLD_START_GLOBAL = "__zkProverColdStarts" as const;

declare global {
  interface Window {
    /** @see {@link coldStarts} — test-only cold-start observability hook (sq-5q63). */
    [COLD_START_GLOBAL]?: number;
  }
}

function recordColdStart(): void {
  coldStarts += 1;
  if (typeof window !== "undefined") {
    window[COLD_START_GLOBAL] = coldStarts;
  }
}

async function loadProver(): Promise<ProverContext> {
  if (!proverPromise) {
    proverPromise = (async () => {
      // First (and, when pre-warm works, only) cold start: count it before the heavy
      // dynamic-import + WASM instantiate so the smoke test can assert it stays at 1.
      recordColdStart();
      const [{ Noir }, bb, circuit] = await Promise.all([
        import(/* webpackChunkName: "noir-js" */ "@noir-lang/noir_js") as Promise<NoirModule>,
        import(/* webpackChunkName: "bb-js" */ "@aztec/bb.js") as Promise<BbModule>,
        fetch(`${basePath()}/zk/filter_int_d2.json`).then((r) => {
          if (!r.ok) throw new Error(`circuit fetch failed: ${r.status}`);
          return r.json() as Promise<{ bytecode: string }>;
        }),
      ]);

      const threads = maxThreads();
      const api = await bb.Barretenberg.new({ threads });
      const backend = new bb.UltraHonkBackend(circuit.bytecode, api);
      return { Noir, backend, circuit, threads };
    })();
    proverPromise.catch(() => {
      proverPromise = null; // allow retry on transient failure (fetch/instantiate)
    });
  }
  return proverPromise;
}

/**
 * [OPUS-4.8] Eagerly pre-warm the ZK prover (dynamic-import + circuit fetch + WASM
 * backend instantiate) without blocking render. Safe to call on route mount and to
 * call repeatedly — it shares the single {@link loadProver} promise, so the cold
 * start happens at most once. Returns a promise that resolves when the prover is
 * ready (or rejects on a load failure, which resets the cache so a later click can
 * retry). This is a UX/perf measure only — it does not change what is proved, nor
 * any security property of the proof.
 */
export function prewarmProver(): Promise<unknown> {
  return loadProver();
}

/** Cross-origin isolation gates bb.js multithreading (SharedArrayBuffer). GitHub
 *  Pages cannot set COOP/COEP, so this is normally false → single-threaded. */
function maxThreads(): number {
  if (typeof window === "undefined") return 1;
  if (!window.crossOriginIsolated) return 1;
  return Math.min(navigator.hardwareConcurrency || 1, 8);
}

/** Two ASCII decimal digit-bytes of a 2-digit age (the private witness). */
function ageDigits(age: number): string[] {
  const s = String(age).padStart(2, "0");
  return [String(s.charCodeAt(0)), String(s.charCodeAt(1))];
}

/**
 * Generate AND verify a real UltraHonk proof that the holder's hidden age
 * satisfies `age ≥ 25`, entirely in the browser. Throws if `age` is not one of
 * {@link PROVABLE_AGES}. For an under-age value the circuit is unsatisfiable, so
 * `execute` throws — which is the honest "you cannot forge eligibility" outcome.
 */
export async function proveAgeEligibility(age: number): Promise<ProofResult> {
  const operandEnc = AGE_OPERAND_ENC[age];
  if (!operandEnc) {
    throw new Error(`no committed operand_enc fixture for age ${age}`);
  }
  const eligible = age >= AGE_THRESHOLD;

  // Reuse the shared, pre-warmed prover context (dynamic imports + circuit fetch +
  // instantiated UltraHonk backend). If pre-warm has not finished, this awaits the
  // same in-flight promise — never a second cold start.
  const { Noir, backend, circuit, threads } = await loadProver();

  const inputs = {
    challenge: "0x1", // per-presentation verifier nonce (fixed here for the demo)
    operand_enc: operandEnc,
    op: "3", // OP_GE
    bound: String(AGE_THRESHOLD),
    expected: eligible,
    digits: ageDigits(age), // PRIVATE — the exact age, never disclosed
  };

  const noir = new Noir(circuit);
  // For an under-age claim the verdict assertion fails here: no proof possible.
  const { witness } = await noir.execute(inputs);

  const t0 = performance.now();
  const proof = await backend.generateProof(witness, PROOF_OPTIONS);
  const proveMs = performance.now() - t0;

  const t1 = performance.now();
  // Verify with the SAME options as prove (the keccak-oracle flavour) — a mismatch
  // would make the in-tab self-verify reject a perfectly valid proof.
  const verified = await backend.verifyProof(proof, PROOF_OPTIONS);
  const verifyMs = performance.now() - t1;

  return {
    eligible,
    proofByteLength: proof.proof.length,
    publicInputs: proof.publicInputs,
    verified,
    proveMs,
    verifyMs,
    threads,
  };
}

/**
 * [OPUS-4.8] sq-8dx2 — fetch the captured car-hire manifest and VERIFY its bundled
 * sub-proof live in-tab with bb.js. The expensive proving was done ahead of time
 * (`scripts/capture-zk-manifest.mjs`); this runs only the cheap verify, so it works on
 * browsers/devices where live proving is too heavy. The proof bytes are a real
 * UltraHonk transcript — a single flipped byte makes `verifyProof` return false, which
 * is the honest "this is a real cryptographic check, not a mock" outcome. Throws on a
 * fetch/parse failure (the caller surfaces it).
 */
export async function verifyCapturedManifest(): Promise<VerifyResult> {
  const res = await fetch(`${basePath()}/zk/car-hire-manifest.json`);
  if (!res.ok) throw new Error(`manifest fetch failed: ${res.status}`);
  const manifest = (await res.json()) as CapturedManifest;
  // The fuller manifest bundles a `subProofs` array; verify the age-gate member (the
  // only circuit whose ACIR the browser ships). Fall back to the first sub-proof so a
  // singular-member fixture still verifies.
  const sp =
    manifest.subProofs?.find((p) => p.member === AGE_GATE_MEMBER) ??
    manifest.subProofs?.[0];
  if (!sp) throw new Error("captured manifest has no sub-proofs");

  const { backend, threads } = await loadProver();
  const proofBytes = Uint8Array.from(sp.proof);

  const t0 = performance.now();
  // Same keccak-oracle flavour the proof was captured with — a mismatch would make a
  // valid captured proof spuriously reject.
  const verified = await backend.verifyProof(
    { proof: proofBytes, publicInputs: sp.publicInputs },
    PROOF_OPTIONS,
  );
  const verifyMs = performance.now() - t0;

  return {
    verified,
    member: sp.member,
    relation: sp.relation,
    proofByteLength: proofBytes.length,
    publicInputs: sp.publicInputs,
    capturedAt: manifest.capturedAt,
    verifyMs,
    threads,
  };
}
