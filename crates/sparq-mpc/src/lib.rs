// [OPUS-4.8] Milestone-0 scaffold for MPC over federated SPARQL (RQ2).
//! # sparq-mpc — MPC over federated SPARQL (RQ2)
//!
//! Distributed half of the verifiable-data-sublanguage vision: a set of
//! mutually-distrusting **holders** jointly evaluate ONE SPARQL query over the
//! *union* of their privately-held, issuer-signed RDF named graphs and produce
//! one verifiable response carrying a zero-knowledge proof that the result is
//! (a) the correct PAG-semantics evaluation of the query AND (b) derived only
//! from issuer-attested sources — while disclosing the minimum inter-source
//! information needed to compute it.
//!
//! Blueprint: `research/mpc-zkp-research-and-architecture.md`. This crate is
//! the engine-side sibling of the single-holder ZK estate (`sparq-zk`,
//! `sparq-zk-compose`) and wraps it rather than replacing it (architecture
//! §4.4).
//!
//! ## FIPS posture (CR-G4) [OPUS-4.8]
//!
//! This crate's cryptography (Shamir secret sharing + a CSPRNG masking seam over
//! the BN254 scalar field, reusing the `sparq-zk` primitives) is **not**
//! FIPS-approved and is **not** inside any FIPS 140-3 / CMVP-validated module.
//! sparq makes **no FIPS claim and no CMVP claim**. The crate is `publish =
//! false`, native-only, and not in the default dependency graph, so a
//! FIPS-constrained operator keeps it out of FIPS scope by not opting in. (The
//! malicious-security / collaborative-proof core is in any case DEFERRED behind
//! honest `NotYetImplemented` stubs — see below.) See
//! `compliance/cryptoreview/fips-posture.md`.
//!
//! ## What Milestone 0 actually delivers (and what it deliberately does not)
//!
//! This is a CONSERVATIVE scaffold. It builds ONLY the parts that are
//! **invariant** to the two open design forks (architecture §5.2):
//!
//! - **Q1 (research risk):** verifying a holder's BBS+/EdDSA signature over a
//!   *secret-shared* witness inside a collaborative proof is unsolved in the
//!   literature — "the join nobody has built". See [`proof`].
//! - **Q2 (trust model):** honest-majority vs dishonest-majority reshapes the
//!   whole MPC primitive choice. See [`backend`].
//!
//! Everything fork-dependent is an explicit, compiling stub that returns
//! [`MpcError::NotYetImplemented`] with the gating milestone/issue named — NO
//! fake crypto. The scaffold's value is the invariant structure + a REAL
//! working per-holder local sub-evaluation ([`holder`]) + the build plan
//! (`PLAN.md`).
//!
//! ## Module map (each cites its architecture section)
//!
//! - [`holder`] — **(§4.1 Parties; §4.3 step 1; §4.2 "minimise data
//!   sharing")** Per-holder local SPARQL sub-evaluation. The one piece that is
//!   real and tested at M0: each holder evaluates a query fragment over its
//!   OWN named graphs locally via `sparq-engine`, returning local partial
//!   results. Holders never ship raw graphs — this IS the invariant
//!   minimise-data-sharing core.
//! - [`backend`] — **(§3.1; §4.2 trust model; §5.2 Q2)** The [`MpcBackend`]
//!   trait abstracts the secret-sharing / MPC primitive so honest- vs
//!   dishonest-majority become swappable impls. Q2 is RESOLVED FOR v1:
//!   honest-majority. The configurability seam is documented here.
//! - [`field`] / [`shamir`] — **(§3.1; §4.2; §4.3 step 4) — M3, REAL crypto.**
//!   [`shamir::ShamirBackend`] is the first concrete [`MpcBackend`]: honest-
//!   majority Shamir `t`-of-`n` secret sharing over the prime field [`field`].
//!   It secret-shares a holder's private input, runs the secure cumulative-sum
//!   aggregate (zero-round local addition), reconstructs only the disclosed
//!   output, and supplies the secret-shared equality primitive the hidden-value
//!   join uses. Semi-honest. **Masking randomness is a CSPRNG** ([`rng`],
//!   sq-1vt): the dealer's coefficients and the equality mask come from
//!   OS-seeded ChaCha20; a deterministic PRNG is available only behind a
//!   test-only feature gate.
//! - [`join`] — **(§2 convention #6; §4.3 step 4; §5.2 Q3)** The [`GlobalJoin`]
//!   trait + [`DisclosedKeyJoin`] (M2, crypto-free disclosed-key equi-join over
//!   GLOBAL IRIs) AND [`HiddenValueJoin`] (M3): joining on a PRIVATE key via the
//!   Shamir-backed secret-shared equality test, disclosing only the result
//!   payload — the capability M2 could not provide. (The crypto-free path
//!   remains the default where the key is a public global IRI, convention #4.)
//! - [`oblivious`] — **(§3 the substrate gap; §4.1 L2; §8 step 1) — sq-18lk, the
//!   keystone hidden-regime primitive.** Oblivious **shuffle** (Waksman/Beneš
//!   permutation network over secret-shared columns — sound today; the in-process
//!   simulation routes via cleartext control bits held by the dealer, so each
//!   switch is a local swap costing **0 multiplications**, while the *deployed*
//!   protocol uses **secret** control bits and pays **1 multiplication per
//!   switch** for the arithmetic conditional swap — surfaced via
//!   [`oblivious::ShuffleCost`]) + oblivious **sort** (Batcher odd-even mergesort
//!   network
//!   whose compare-exchange access pattern is data-independent — the obliviousness
//!   substrate). DISTINCT / ORDER BY / GROUP BY-over-hidden / MIN-MAX /
//!   OPTIONAL-MINUS / the set-returning oblivious-join output path / ~linear joins
//!   all reduce to it. The disclosed-key sort is sound; the secret-key comparator
//!   is honestly gated on degree reduction (no fake secure comparison). See the
//!   module docs for the per-primitive security/leakage statement.
//! - [`proof`] — **(§4.3 step 5; §4.4; §5.1 hard dependency; §5.2 Q1)** The
//!   [`CollaborativeProof`] / [`Attestation`] boundary that will emit the ZKP
//!   that the result is correct AND issuer-attested. Interface + doc; impl
//!   gated on the ZK-foundation remediation (#3 issuer-sig / #4 replay / #5/#6
//!   FILTER-binding / #8/#9 attribution / #12 revocation) and on Q1.
//! - [`partial`] — shared value types crossing module boundaries
//!   ([`PartialResult`], [`HolderId`], [`MpcError`]). No crypto.
//!
//! ## Why native-only / not in the wasm build
//!
//! MPC is inter-process / inter-host and the eventual crypto (secret sharing,
//! collaborative proving) has no browser story at this milestone. Keeping
//! `sparq-mpc` out of `sparq-wasm`'s dependency graph guarantees the browser
//! bundle carries zero MPC surface (mirrors how `sparq-zk` is isolated).
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

// [OPUS-4.8] sq-km34.1: IT-MAC authenticated secret sharing — the FOUNDATION for
// honest-majority malicious-with-abort security. `AuthenticatedShare = ([x],[α·x])`
// layered over the degree-t Shamir sharing, plus the session-global secret-shared
// MAC key `[α]` (no party knows α; never reconstructed) and the FREE linear ops
// (add/scale/sub/add-constant applied to both [x] and [α·x]). Foundation only —
// MAC-carrying multiplication (sq-km34.2), the batched MAC-check (sq-km34.4), and
// the registry/per-operator wiring (sq-km34.7) are SEPARATE beads. See the module
// docs and research/mpc-malicious-security-design.md §2.1–2.2.
pub mod authenticated;
pub mod backend;
// [OPUS-4.8] sq-dwb5: batched / vector secret sharing — generalise the
// single-scalar `share_private_input` to a holder sharing a `Vec<Fp>` (per-row
// hidden values / a salary vector / per-graph commitments) under a DOCUMENTED
// row-binding (`RowBinding::Positional` / `Keyed`), so the secure aggregate +
// hidden-value join can range over more than one value per source. Carries the
// `BatchedShares` / `BatchedAuthShares` types, element-wise reconstruct, and the
// multi-row `per_row_sum` secure aggregate (the demonstrated end-to-end path).
// See the module docs for the row-binding contract.
pub mod batched;
// [OPUS-4.8] sq-rrz4: secure secret-shared greater-than / threshold over Fp that
// opens ONLY the boolean verdict bit, never the operands. Bit-decomposition
// MSB-first comparison chaining multiplications via mul_shares_raw + the landed
// BGW degree-reduction (sq-dvuc). Realises the §4.3 four-flatmates "cumulative
// salary > £100k" disclosing only the verdict. Honest-majority, semi-honest (NOT
// malicious) — reported as OperatorClass::Comparison. See the module docs.
pub mod compare;
// [OPUS-4.8] sq-ka8m (sq-mnv5 residual; design Hole 4): the MALICIOUS-SECURE
// (honest-majority, with-abort) twin of `compare`. Carries an IT-MAC through the
// WHOLE decompose+compare chain (`MacSession::auth_mul`, §2.4 route (a)) and
// MAC-checks the boolean verdict BEFORE opening it (`MacSession::mac_check`, §2.5),
// so a tamper in ANY gate (forged share / wrong degree_reduce re-sharing) aborts
// fail-closed — at the minimal n=2t+1 where RS redundancy is zero (soundness from
// the secret α). Reported as OperatorClass::Comparison @ Malicious+Abort. See the
// module docs and `research/mpc-malicious-security-design.md`.
pub mod auth_compare;
// [OPUS-4.8] sq-6fv7 (sq-ka8m residual): the IT-MAC-HARDENED twin of
// `compare::disclose_threshold_verdict` — the federation £100k path that operates
// on an EXISTING secret-shared sum. sq-ka8m's `auth_compare` made the cleartext
// fresh-operand comparison malicious-secure, but the disclose path's THREE
// decomposition opens (the square-protocol `a²`, the masked open `c = sum + r`, and
// the boolean `verdict`) were still semi-honest (no integrity check). This module
// routes all three through the §2.5 batched IT-MAC check: the existing sum is
// authenticated (`MacSession::authenticate_existing`), the mask bits come from an
// AUTHENTICATED square protocol, the masked open and the verdict are MAC-checked
// BEFORE they are acted on, so a tamper on any of those three OPENS aborts
// fail-closed at the minimal n=2t+1. Honest-majority; see the tier correction below
// before reading that as malicious-with-abort — it is not.
// [OPUS-4.8] sq-km34.6: the production path is now the AUTHENTICATED RABBIT chain —
// every product/reduce in the solved-bits / LTBits / ripple-add / ripple-sub wrap
// recovery is a `MacSession::auth_mul`, lifting the malicious magnitude from the
// masked-open 2^20 to the full 2^60 (parity with the semi-honest `compare` path).
// Read the module docs' "What the MAC-check does NOT cover" before making any
// tamper-detection claim: `auth_mul` ADOPTS a value tamper on its second operand and
// `mac_check` only covers values that are opened, so "any tamper aborts" is NOT the
// delivered property. [SONNET-4.6] Stronger than a caveat — it is EXPLOITABLE: the
// range proof's zero-test mask sits in that adopted slot, so zeroing it switches the
// range proof off and the path returns a WRONG VERDICT instead of aborting (witness
// `a_zeroed_zero_test_mask_defeats_the_range_proof_and_flips_the_verdict`). The
// delivered tier is therefore honest-majority TAMPER-EVIDENT-with-abort on the opened
// values, NOT AXIS-1 `Malicious`; do not deploy it as the integrity tier until
// `MacSession` gains multiplication-gate verification binding BOTH operands. Review
// round 2 acted on that: the public entry point is now named
// `experimental_tamper_evident_disclose_threshold_verdict`, so the API surface no
// longer claims the tier the code misses. See the module docs.
pub mod auth_disclose;
// [OPUS-4.8] sq-sxm: the (security model × N × query class) benchmark MATRIX
// harness + its deterministic communication/round/multiplication counter — the
// IN-PROCESS counting tier (Tier 1). New modules, isolated from the protocol
// primitives so they do not conflict with sibling beads editing backend.rs
// (sq-a6p1) / oblivious.rs (sq-jnkm): `metrics` is the counter, `bench` drives the
// matrix and reads the per-cell ACTUAL security off `operator_descriptor`. The
// real network transport + tc/netem (Tier 2/3) is bead sq-tg6b; EC2 scale-out is
// sq-hoaj. See the module docs for the critical-honesty constraint (no
// single-process wall-clock masquerading as an MPC latency).
pub mod bench;
// [OPUS-4.8] sq-py8h.1: bounded property-path over DISCLOSED global-IRI keys —
// the crypto-free "headline federation" regime. Unrolls a bounded path
// `?a (p){m,k} ?b` (sequence / exact {k} / range {m,k} / reflexive {0,k} /
// alternation) into a finite set of fixed BGP chains, evaluates each as a fold
// of DisclosedKeyJoin OUTSIDE the cryptographic core, then UNION + DEDUP of the
// endpoint pairs. NO MPC round runs. The HIDDEN-intermediate regime (secret
// ?z_i) is the separate sq-py8h.2 and is NOT here. See the module docs for the
// regime + semantic boundary statement.
pub mod bounded_path;
// [OPUS-4.8] sq-py8h.2: the HIDDEN-intermediate fixed exactly-`k` property-path
// chain — the cryptographic core of the bounded property-path operator. Each hop's
// match is a secret-shared `secure_equal_to_bit` (never opened); the `k` hops are
// chained by AND (`mul_shares_raw` + `degree_reduce`, the sq-dvuc keystone) and
// OR-folded per distinct endpoint pair; the final connected-bit drives
// `oblivious_set_output` (padded+shuffled to B). Intermediate node-sets are never
// reconstructed. Honest-majority / semi-honest only. See the module docs.
pub mod field;
// [OPUS-4.8] sq-py8h.4: hidden-key endpoint DISTINCT — collapse duplicate SECRET
// endpoint pairs (both endpoints private keys) via the secret-control oblivious sort
// (SortingNetwork compare-exchange decided by a never-opened secure comparator +
// arithmetic conditional swap) + adjacent-equality scan (secret keep-bit) +
// oblivious compaction (oblivious_set_output). The gated sub-piece of the bounded
// property-path operator; needs the now-landed secure comparator (sq-rrz4) + degree
// reduction (sq-dvuc). Honest-majority / semi-honest only. See the module docs.
pub mod hidden_distinct;
pub mod hidden_path;
pub mod holder;
pub mod join;
pub mod metrics;
pub mod partial;
// [OPUS-4.8] sq-dl81: the domain-separated, collision-resistant Term->Fp join-key
// encoder + the injectivity (no-false-match) contract the hidden-value join rests
// on. Turns "the holder's untested encoding responsibility" into a documented hash
// with a stated birthday bound plus a fail-closed collision-detection path.
pub mod term_encode;
// [OPUS-4.8] sq-6y92: the end-to-end federated MPC pipeline DRIVER — the glue
// that composes holder → share → join → secure-threshold → reconstruct →
// ProofStatement into one worked four-flatmates federated response (architecture
// §4.3 steps 1–6), with the per-operator disclosed-vs-hidden routing (RQ2a) made
// explicit. Composes EXISTING primitives only; proof.prove stays the honest stub.
pub mod pipeline;
pub mod proof;
// [OPUS-5] sq-34ml: the two NON-RESEARCH M4-v1 prerequisites split out of the
// sq-bjl Q1 SPIKE (feasibility record §2/§4) — the out-of-circuit
// freshness/replay binding (ZK audit #4 is NOT automatic when the issuer
// signature is checked beside the proof rather than inside it) and the
// federated/multi-source `reconstruct_public_inputs` layout spanning every
// holder's commitments/rows/attribution, byte-compared unchanged (#1 generalised).
// SCOPING + LAYOUT ONLY: still hard-gated on the ZK foundation (§5.1), and
// `proof.rs` stays an honest `NotYetImplemented`.
pub mod federated_binding;
// [OPUS-4.8] sq-it50: the owned ChaCha20 CSPRNG backing SecureRng — private
// implementation detail (not a public API), so its key schedule can be
// ZeroizeOnDrop-scrubbed (which ecosystem rand_chacha cannot do from our side).
mod chacha;
// [OPUS-4.8] sq-1vt: the CSPRNG masking seam (production SecureRng + test-only
// InsecureTestRng). The real protocol's secret-sharing randomness lives here.
pub mod rng;
// [OPUS-4.8] sq-yyro: the DEALER-LESS correlated-randomness seam. `rng` fixes the
// randomness QUALITY (CSPRNG); this fixes WHO draws it — the contract a PRSS /
// honest-majority coin-toss / dealer-less VSS source must satisfy to replace the
// single-trusted-dealer simulation. `ShamirDealer` reports
// `RandomnessModel::TrustedDealerSim`; `crate::prss` is the first dealer-less
// GENERATOR behind it. NO variant is `deployable()` — acceptance stays fail-closed.
// See research/mpc-distributed-randomness-design.md.
pub mod randomness;
// [OPUS-5] sq-yyro follow-on (#3531): PRSS — replicated-PRF pseudo-random secret
// sharing behind the `randomness` seam. Non-interactive (0 online rounds) degree-t
// masks from a one-time replicated-seed setup; SMALL-n honest-majority only (the
// C(n,t) seed count fails closed past `prss::MAX_PRSS_SEEDS`). The generator is
// real; the SETUP is a simulated one-time trusted setup, dealer-less VSS is still
// refused, and the source is still NOT `deployable()`. See the module docs.
pub mod prss;
// [OPUS-4.8] sq-m34i (MPC WI-1): Reed-Solomon consistency-checked + robust
// (Berlekamp-Welch) reconstruction over Fp — detect-and-abort / correct tampered
// shares when redundancy is present. Closes malicious-security gap (D) at the
// Shamir layer (parent bead sq-uu0u). See the module docs for the threat model.
pub mod robust;
pub mod shamir;
// [OPUS-4.8] sq-18lk: oblivious shuffle (Waksman/Benes net) + sort (Batcher
// odd-even mergesort) substrate over Shamir Fp — the keystone hidden-regime primitive
// (ORQ SOSP'25). DISTINCT / ORDER BY / GROUP BY-over-hidden / MIN-MAX /
// OPTIONAL-MINUS / the set-returning oblivious-join output path / ~linear joins
// all reduce to it. The shuffle is sound today; the sort NETWORK + its
// data-independent access pattern (the substrate) are sound, with the secret-key
// comparator honestly gated on degree reduction. See the module docs.
pub mod oblivious;
// [OPUS-4.8] sq-jnkm: oblivious result-size protection + match-bit aggregation
// output path for SET-returning hidden joins, built on the sq-18lk shuffle
// substrate. Closes leaks L1 (true result cardinality, padded to a public bound)
// and L2 (the per-pair match graph / key fan-out, destroyed by the oblivious
// shuffle) that HiddenValueJoin's per-pair open exposes. The output TRANSFORM +
// the disclosed-key path are sound today; deriving the secret match bit from
// secret keys WITHOUT opening it is honestly gated on secure-compare
// (sq-rrz4/sq-dvuc). See the module docs for the residual-leakage statement.
pub mod oblivious_join;
// [OPUS-4.8] sq-tg6b: the NETWORK tier of the MPC benchmark matrix. `transport`
// is the REAL multi-process loopback transport (Tier 2) — each party is its own
// PROCESS exchanging the actual protocol messages over a socket, so wall-clock
// latency becomes a meaningful MPC cost (vs the in-process counting tier's
// MODELLED communication in `bench`/`metrics`). `netprofiles` is the tc/netem
// LAN/WAN shaping (Tier 3), gated to a privileged host or the EC2 bench bead
// (sq-hoaj) because `tc qdisc` needs CAP_NET_ADMIN — NOT available unprivileged.
// Native-only (sockets + child processes have no browser story); the
// wasm-exclusion invariant is preserved (only std::net / std::io / std::process).
pub mod netprofiles;
pub mod transport;

// [OPUS-4.8] sq-nuok: adversarial-share negative suite + 'no fake crypto' stub
// gate. Test-only; compiled only under `cfg(test)` so it can drive the seedable
// simulation RNG (`ShamirBackend::new_seeded`) and the deferred-stub trait
// surface. Asserts the honest-but-robust properties that ARE claimed and PINS
// that the deferred parts fail closed. See the module docs.
#[cfg(test)]
mod adversarial_tests;

// [OPUS-4.8] sq-2fms: federation-level OWA / omission negative suite. Test-only.
// The adversarial-share suite (`adversarial_tests`) pins SHARE-level tampering;
// this is its FEDERATION-level complement: a dropped holder / truncated partial /
// omitted row/contributor must not FORGE a valid result (a guarantee that holds
// today — omission only loses completeness), while honestly PINNING that such an
// omission is NOT yet cryptographically detectable pre-M4 (no §4.3-step-1
// signed-count binding). See the module docs.
#[cfg(test)]
mod owa_omission_tests;

// [OPUS-4.8] sq-7leq: encode the witness-validation-before-proving TEST
// OBLIGATION for the collaborative-proof (coZK) path (re-audit §3, bead sq-9hrn).
// Test-only. A PASSING meta-test pins the current honest fail-closed posture (the
// deferred `prove` never proves over any witness), and an `#[ignore]`d R-WV /
// T1–T4 suite encodes the OPEN soundness obligation against the future prover
// (sq-f7bu/sq-bjl) so the gap is measurable, not silent. Does NOT claim the
// collaborative path is sound (sq-qhy4 single-prover audit; multi-prover audit
// still required). See the module docs.
#[cfg(test)]
mod witness_validation_tests;

// [OPUS-4.8] sq-km34.1: the IT-MAC authenticated-sharing foundation surface.
pub use authenticated::{
    auth_add, auth_add_constant, auth_scale, auth_sub, auth_sum, AuthenticatedShare, MacKey,
};
pub use backend::{
    AbortKind, AdversaryModel, BackendInfo, BackendRegistry, CorruptionThreshold,
    MaliciousSecurity, MpcBackend, OperatorClass, OutputGuarantee, PublicVerifiability,
    SecurityDescriptor, SecurityRequirement, TrustModel,
};
// [OPUS-4.8] sq-dwb5: the batched / vector secret-sharing surface + row-binding.
pub use batched::{per_row_sum, BatchedAuthShares, BatchedShares, RowBinding};
// [OPUS-4.8] sq-sxm: the benchmark-matrix harness surface (in-process counting tier).
pub use bench::{
    cell, run_matrix, CellSecurity, MatrixCell, MatrixResults, QueryClass, DEFAULT_PARTIES,
};
// [OPUS-4.8] sq-py8h.1: the disclosed-key bounded property-path surface.
pub use bounded_path::{
    eval_bounded_path_disclosed, BoundedRepetition, DisclosedEdges, PathForm, PathStep,
};
// [OPUS-4.8] sq-py8h.2: the HIDDEN-intermediate exactly-`k` chain surface (the
// cryptographic regime — intermediate node-sets never opened).
pub use hidden_path::{
    eval_bounded_path_hidden, eval_bounded_path_hidden_slots, eval_exact_k_chain_hidden,
    eval_exact_k_chain_hidden_slots, HiddenBoundedPath, HiddenEdge, HiddenEdges, HiddenNode,
    PredicatedEdge, PredicatedEdges, MAX_CHAIN_TUPLES,
};
// [OPUS-4.8] sq-py8h.4: the hidden-key endpoint DISTINCT surface — collapse
// duplicate SECRET endpoint pairs (oblivious sort by a never-opened secure
// comparator + adjacent-equality keep-bit + oblivious compaction).
pub use hidden_distinct::{
    distinct_hidden_pairs, distinct_hidden_pairs_oblivious, distinct_hidden_pairs_slots,
    DistinctCost, SecretEndpointPair, MAX_DISTINCT_ROWS,
};
// [OPUS-4.8] sq-py8h.5: the planner guard + cost-model surface for the hidden
// bounded property-path operator (reject statically-large unrolls, refuse a hidden
// UNBOUNDED path fail-closed, emit modelled CommCounter cost).
pub use hidden_path::planner::{
    plan_hidden_bounded_path, BoundedPathPlan, HiddenPathRequest, PathUpperBound,
};
// [OPUS-4.8] sq-rrz4 + sq-g7t5 + sq-bgsn: the secure-comparison surface (verdict-only
// disclosure). The in-MPC bit-decomposition magnitude-bound constants for
// `disclose_threshold_verdict` (the sum is bit-decomposed in-MPC, never
// reconstructed). sq-bgsn lifts the production path to the Rabbit-style full-field
// decomposition, so the supported magnitude is now `< 2^RABBIT_VALUE_BITS = 2^60`
// (the `RABBIT_*` constants), up from the masked-open path's `< 2^DECOMP_VALUE_BITS
// = 2^20` (the `DECOMP_*` constants, retained for the malicious twin / tests).
pub use compare::{
    disclose_threshold_verdict, open_verdict, secure_equal_to_bit, secure_greater_than,
    secure_threshold, COMPARE_BITS, COMPARE_MAX_EXCLUSIVE, DECOMP_MASK_BITS,
    DECOMP_STAT_SECURITY_BITS, DECOMP_VALUE_BITS, DECOMP_VALUE_MAX_EXCLUSIVE, RABBIT_MASK_BITS,
    RABBIT_VALUE_BITS, RABBIT_VALUE_MAX_EXCLUSIVE,
};
// [OPUS-4.8] sq-ka8m: the malicious-secure (honest-majority, with-abort) comparison
// surface — IT-MAC-carried decompose+compare chain, verdict MAC-checked before open.
pub use auth_compare::{malicious_greater_than, malicious_threshold, open_auth_verdict};
// [OPUS-4.8] sq-6fv7: the IT-MAC-hardened disclose path over an EXISTING sum — the
// three decomposition opens (a², c=sum+r, verdict) routed through the MAC-check.
// [SONNET-4.6] Review round 2 — RENAMED from `malicious_disclose_threshold_verdict`.
// The old name asserted a tier the code does not deliver (a demonstrated wrong-verdict
// path under its own named adversarial setting), and a doc caveat on a `malicious_*`
// drop-in is not containment. The break predates the sq-km34.6 Rabbit lift — the
// pre-lift masked-open path fed `auth_secret_is_zero` the same adopted second-operand
// mask twice — so reverting would hide it, not fix it; the fix is a `MacSession`
// change (see the module docs). No alias is kept under the old name: an alias would
// preserve exactly the misleading surface. Nothing in the workspace depends on
// `sparq-mpc` (`publish = false`), so no caller is silently retargeted.
pub use auth_disclose::experimental_tamper_evident_disclose_threshold_verdict;
pub use field::Fp;
pub use holder::{Holder, HolderResult};
pub use join::{
    BatchedHiddenInput, BatchedJoinOutput, DisclosedKeyJoin, GlobalJoin, HiddenKeyedRows,
    HiddenValueJoin, JoinPlan,
};
pub use metrics::{CommCounter, FIELD_BYTES};
pub use oblivious::{
    shuffle, sort_by, sort_with_keys, AccessPattern, Comparator, SecretColumn, ShuffleCost,
    SortByResult, SortCost, SortWithKeysResult, SortingNetwork, Switch, WaksmanNetwork,
};
pub use partial::{HolderId, MpcError, PartialResult};
// [OPUS-4.8] sq-dl81: the collision-resistant Term->Fp join-key encoder surface.
pub use term_encode::{encode_term, Collision, EncodeError, KeyEncoder, DOMAIN_TAG};
// [OPUS-4.8] sq-6y92: the federated-pipeline driver surface (architecture §4.3).
pub use pipeline::{
    federation_holder_id, run_federated, FederatedQuery, FederatedResponse, Flatmate,
    OperatorRouting, Routing,
};
pub use proof::{Attestation, CollaborativeProof, ProofStatement};
// [OPUS-5] sq-34ml: the M4-v1 out-of-circuit freshness/replay binding + the
// federated public-input layout surface. A statement shape + a byte layout, NOT
// a proof — see the module docs for the hard gate it does not close.
pub use federated_binding::{
    check_freshness, key_set_digest, BindingError, FederatedStatement, FieldWord, FreshnessBinding,
    HolderSegment, InMemorySeenChallenges, SeenChallenges, ValidityWindow, VerifierChallenge,
    COMMITMENT_DOMAIN_TAG, FRESHNESS_DOMAIN_TAG, HOLDER_DOMAIN_TAG, KEY_SET_DOMAIN_TAG, WORD_BYTES,
};
// [OPUS-4.8] sq-jnkm: the oblivious set-returning output path surface.
pub use oblivious_join::{
    oblivious_join_output, oblivious_set_output, oblivious_set_output_hidden_keys, Candidate,
    MatchBit, ObliviousOutput, ObliviousOutputCost, OutputSlot,
};
pub use rng::{MpcRng, SecureRng};
// [OPUS-4.8] sq-yyro: the dealer-less correlated-randomness seam surface.
pub use randomness::{DistributedRandomness, RandomnessModel};
pub use robust::{reconstruct_robust, reconstruct_robust_attributed, RobustReconstruction};
// [OPUS-4.8] sq-km34.1: `MacSession` mints the session-global `[α]` and produces
// authenticated sharings; `ShamirDealer::new_mac_session` is the entry point.
pub use shamir::{MacSession, ShamirBackend, Share};
// [OPUS-4.8] sq-tg6b: the network-tier surface — the real loopback transport
// (Tier 2) and the tc/netem LAN/WAN profiles (Tier 3, privilege-gated).
pub use netprofiles::NetemProfile;
pub use transport::{run_cell_networked, Channel, Coordinator, Message, NetworkCell, StepCode};
