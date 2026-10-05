//! # sparq-zk — ZK derived-credential foundation for sparq
//!
//! Engine-side half of the ZK derived-credentials design
//! (`research/zkp-query-proofs-plan.md` v3): the **commitment pipeline** and
//! the **zk-trace seam**. Circuits and proof composition are later
//! deliverables; everything here is off-circuit Rust whose outputs (BN254
//! field elements, leaf orderings, witness input sets) are what those
//! circuits will consume.
//!
//! Modules:
//! - [`field`] — BN254 scalar-field helpers (Noir `Field` compatible).
//! - [`poseidon2`] — Poseidon2-BN254 permutation + sponge, bit-compatible
//!   with noir-lang/poseidon (cross-tested against `nargo`).
//! - [`canon`] — RDFC10 canonicalization of a named graph's content
//!   (W3C rdf-canon test-suite validated).
//! - [`encode`] — per-term / per-triple field encoding with graph-scoped
//!   bnode salting (plan §2.2, Q6).
//! - [`commit`] — per-named-graph flat Poseidon2 commitments (plan §2.2).
//! - [`ingest`] — the trusted-ingest Stage-2 path (sq-610 + sq-cn8): a salt
//!   mint that draws globally-unique per-graph RDFC10 salts from the OS CSPRNG
//!   and per-named-graph commitment wired through to the zk-trace consumer.
//! - [`registry`] — `<urn:sparq:zk>` registry-graph plumbing (plan Q13),
//!   mirroring `sparq-solid`'s `<urn:sparq:auth>` conventions.
//! - [`sig`] — issuer signatures over per-graph commitments (audit #3):
//!   Schnorr over Baby-JubJub with a Poseidon2 challenge, the verifier-side
//!   sound interim that binds `C(G)` to an issuer key in a disclosed key-set.
//! - [`trace`] — the zk-trace seam (plan §4.E): per-obligation input sets
//!   from an executed query, leaf-index resolution against per-graph
//!   commitment orderings, and the Q6 cross-graph bnode-join guard
//!   (prover side, plan §2.4 layer 2).
//! - [`verify`] — the verifier-side static re-check (plan §2.4 layer 3):
//!   independent re-derivation of fragment patterns and cross-graph join
//!   obligations from the query text.
//! - `vc_bridge` — the OFF-circuit W3C VC ingest bridge (OPT-IN, behind the
//!   `vc-bridge` feature): verify a source VC's Data-Integrity proof
//!   (`eddsa-rdfc-2022` / `ecdsa-rdfc-2019`) at the host, then re-commit + record
//!   the `zk:sourceCryptosuite` provenance (sq-9c5e, design §5). Its
//!   selective-disclosure counterpart `vc_bridge_sd` (`bbs-2023` /
//!   `ecdsa-sd-2023`, sq-u5y1f) DELEGATES the derived-proof check to a
//!   host-supplied verifier and fails closed without one — sparq implements no
//!   selective-disclosure verifier and asserts no SD soundness.
//!
//! NOTHING in the sparq workspace depends on this crate; default builds and
//! the wasm artifact are byte-identical with or without it.
//!
//! ## FIPS posture (CR-G4) [OPUS-4.8]
//!
//! The primitives here — BN254, Poseidon2, and Schnorr over Baby-JubJub
//! ([`sig`]) — are **not** FIPS-approved and are **not** inside any FIPS 140-3 /
//! CMVP-validated module. sparq makes **no FIPS claim and no CMVP claim**; these
//! are ZK-friendly primitives chosen for in-circuit efficiency, not regulatory
//! approval. This crate is `publish = false` and not in the default dependency
//! graph, so a FIPS-constrained operator keeps it out of FIPS scope by not
//! opting in. See `compliance/cryptoreview/fips-posture.md`.
#![forbid(unsafe_code)] // [OPUS-4.8] sq-emay: crate has zero `unsafe`

pub mod canon;
pub mod commit;
// [OPUS-5] sq-j3b9 (follow-up to sq-8jv7, gap CR-G5): the scalar-independent
// Baby-JubJub ladder used by the SECRET-scalar paths of `sig` (key derivation +
// Schnorr signing). Crate-internal — no public API, no new dependency, and the
// public-data `verify` path deliberately keeps the faster arkworks
// multiplication. Removes the square-and-multiply control-flow shape; does NOT
// claim signing is constant-time (arkworks field ops remain unasserted — see the
// module docs and `compliance/cryptoreview/side-channel-analysis.md`).
mod ct;
// [OPUS-4.8] sq-xojl: DUAL-LEAF value-lane host encoding (VALUE_HOOK + lexical
// leaf). OPT-IN behind the `dual-leaf` cargo feature (OFF by default): the
// default string-canonical pipeline is byte-unchanged. Carries the documented
// INV-VL downgrade (#769 accepted, CR-G8 / sq-qhy4); NOT externally audited.
#[cfg(feature = "dual-leaf")]
pub mod dual_leaf;
// [FABLE-5] sq-hh7a4: dual-leaf `xsd:boolean` host encoder (same feature gate,
// same INV-VL downgrade + CR-G8 / sq-qhy4 audit obligation as `dual_leaf`).
#[cfg(feature = "dual-leaf")]
pub mod dual_leaf_boolean;
pub mod encode;
pub mod field;
pub mod ingest;
// [FABLE-5] sq-we9vs: dual-leaf `xsd:dateTime`/`xsd:date` host encoders — the
// §13 signed scaled-epoch, Z-only, fail-closed lanes (same feature gate, same
// INV-VL downgrade + CR-G8 / sq-qhy4 audit obligation as `dual_leaf`).
#[cfg(feature = "dual-leaf")]
pub mod dual_leaf_datetime;
pub mod poseidon2;
mod poseidon2_constants;
pub mod registry;
// [OPUS-4.8] sq-bevd3 (Phase 3, epic sq-0dksu): the PER-METHOD security-properties
// annotation graph + the over-claim guards (no `Proven` on a positive property while
// sq-qhy4 is open; completeness; source-layer non-transfer). OPT-IN behind the
// `secprop-annotations` cargo feature (OFF by default): the default build is
// byte-unchanged and the module is compiled out. RECORDS claims + their epistemic
// basis; asserts NO soundness/privacy property (estate NOT externally audited,
// sq-qhy4).
#[cfg(feature = "secprop-annotations")]
pub mod secprop;
pub mod sig;
pub mod trace;
// [OPUS-4.8] sq-9c5e: OFF-circuit W3C Verifiable-Credential ingest bridge —
// verify a source VC's Data-Integrity proof (`eddsa-rdfc-2022` / `ecdsa-rdfc-2019`)
// at the HOST, then re-commit + record `zk:sourceCryptosuite` provenance. OPT-IN
// behind the `vc-bridge` cargo feature (OFF by default): the default build pulls
// no Ed25519/ECDSA/SHA-256 dependency and this module is compiled out. The
// `zk:sourceCryptosuite` registry slot itself is always present. Provenance-only;
// the query proof does NOT re-verify the VC proof in-circuit (design §5.3). NOT
// externally audited (sq-qhy4).
#[cfg(feature = "vc-bridge")]
pub mod vc_bridge;
// [OPUS-5] sq-txg1y (issue #3234): the ADDITIVE JSON-LD VC *envelope* entry point
// layered on top of `vc_bridge` — split a DI-secured VC JSON document into its
// unsecured document + proof configuration, multibase-decode `proofValue`, expand
// both halves to RDF (`oxjsonld`, caller-supplied `@context` allowlist, NO
// network), then run the SAME off-circuit `verify_source_proof` check. Same
// OFF-by-default `vc-bridge` gate; the RDF-native API is unchanged.
#[cfg(feature = "vc-bridge")]
pub mod vc_bridge_json;
// [OPUS-5] sq-u5y1f (issue #3235): the SELECTIVE-DISCLOSURE ingest SEAM for
// `bbs-2023` / `ecdsa-sd-2023` (design §5.3). A real BBS / `ecdsa-sd-2023` verifier
// is NOT in this repo, so the module is a DELEGATION boundary: the host plugs its
// own audited verifier into `SelectiveDisclosureVerifier`, sparq does the RDFC10
// canonicalization + re-commitment + provenance, and with no verifier supplied the
// ingest FAILS CLOSED. Asserts NO selective-disclosure soundness or unlinkability
// of its own; research-grade, NOT externally audited (sq-qhy4). Same OFF-by-default
// `vc-bridge` gate; adds NO dependency.
#[cfg(feature = "vc-bridge")]
pub mod vc_bridge_sd;
pub mod verify;

pub use field::Fr;
