// [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.
// [OPUS-4.8] MS-G2 (sq-8wbn): make `// SAFETY:` mandatory on every first-party unsafe block.
#![warn(clippy::undocumented_unsafe_blocks)]
//! # sparq-zk-compose — ZK proof composition for sparq (stage 2)
//!
//! Drives the per-property Noir circuit family at `zk/compose/` into a full
//! query-result proof, and verifies one (plan v3 §S4.E modules ii/iii).
//!
//! Pipeline:
//! 1. [`build`] turns sparq-zk `GraphCommitment`s + a BGP pattern into a
//!    scan [`ProofInputs`], deriving the (k, n, r) [`CircuitId`] from the data.
//! 2. [`driver::CircuitProver`] proves it via nargo/bb subprocesses.
//! 3. A [`ProofManifest`] carries the public inputs + bb proof to the verifier.
//! 4. [`verifier`] re-derives the circuit id, re-runs the sparq-zk bnode
//!    re-check (`sparq_zk::verify::recheck`), checks binding-consistency
//!    edges, and verifies each sub-proof via bb.
//!
//! Like `sparq-zk`, NOTHING in the workspace depends on this crate — default
//! builds and the wasm artifact are byte-identical with or without it.
//!
//! Model: Opus 4.8 (Fable 5 unavailable — flag for re-review/upgrade).

pub mod build;
// [OPUS-4.8] sq-1s2.3 (FL1 follow-up): package native per-circuit ProofArtifacts into
// the browser-shippable captured car-hire ProofManifest (the /showcase/zk-car-hire
// in-tab verify fallback). Pure serialization — no toolchain; research-grade, NOT
// externally audited (sq-qhy4).
pub mod capture;
// [OPUS-4.8] sq-314: derivation steps + entailment-regime enforcement.
pub mod derivation;
// [OPUS-4.8] sq-cfmv: fail-closed (commitment-method × circuit) dispatch matrix.
// OPT-IN behind the `dual-leaf` feature (OFF by default). Rejects an
// unsupported / mismatched (method, circuit) pair, never silently mis-dispatches.
// NOT externally audited (sq-qhy4); enforces structural legality only.
#[cfg(feature = "dual-leaf")]
pub mod dispatch;
pub mod driver;
// [OPUS-4.8] sq-xqfg (HolderPoP T5): in-circuit holder-PoK host-side wiring (B2).
pub mod holder;
pub mod issuer;
pub mod manifest;
// [OPUS-5] sq-rsd3v.3: witnessed-rule-shape N3 derivation — the host mirror of
// the in-circuit relation, the fail-closed PROVABLE-SUBSET admission gate, and
// the rule-graph commitment (the rule author as ISSUER, so a proof says WHOSE
// rules). SEPARATE from `derivation` because an N3 rule set is DATASET-SUPPLIED,
// not a fixed public table. Research-grade, NOT externally audited (sq-qhy4);
// no manifest/dispatch wiring and no compiled member yet.
pub mod n3;
pub mod revocation;
// [OPUS-5] sq-rsd3v.6: owl:sameAs EQUALITY reasoning — the host mirror of the
// in-circuit union-find canonicalisation gadget, plus its witness builder.
// SEPARATE from `derivation` on purpose: encoding-equality re-checks are the
// wrong proxy under equality reasoning. Research-grade, NOT externally audited
// (sq-qhy4); no manifest/dispatch wiring and no compiled member yet.
pub mod sameas;
pub mod toml;
pub mod verifier;

pub use manifest::{
    AttestedStatusRef, BindingEdge, BindingMode, CircuitId, EntailmentRegime, FieldHex, FilterOp,
    FullyHiddenRevocation, HiddenIndexRevocation, HolderPokProof, HolderSetProof, ProofInputs,
    ProofManifest, RevocationStatus, StatusListSnapshot, SubProof,
};
// [OPUS-4.8] sq-1s2.3 (FL1 follow-up): browser-shippable captured-manifest packaging.
pub use capture::{
    public_inputs_to_hex, CaptureError, CapturedCarHireManifest, CapturedSubProof,
    CAR_HIRE_CAPTURE_NOTE,
};
// [OPUS-4.8] sq-3kd2g.6: the wave-1 extended-fragment presentation wrapper +
// per-solution UNION branch attribution schema. Opt-in (`extended-fragment`).
#[cfg(feature = "extended-fragment")]
pub use manifest::{BranchWitness, FragmentManifest};
// [OPUS-4.8] sq-314: derivation-step capability + entailment regime end-to-end.
pub use derivation::{regime_admits, DerivationStep, EntailmentRule};
// [OPUS-5] sq-rsd3v.6: the owl:sameAs canonicalisation witness + host re-check.
pub use sameas::{CanonEntry, CanonError, CanonTable, OWL_SAME_AS};
// [OPUS-5] sq-rsd3v.3: the N3 witnessed-rule-shape types + the fail-closed
// provable-subset gate. `N3RuleSet::commit` admits BEFORE it folds, so an
// out-of-subset rule graph has no root and can never reach the circuit.
pub use n3::{N3Builtin, N3Premise, N3Rule, N3RuleSet, N3Slot, N3SubsetError, MATH_NS};
// [OPUS-4.8] sq-cfmv: the fail-closed (method × circuit) dispatch resolver.
#[cfg(feature = "dual-leaf")]
pub use dispatch::{resolve_circuit, resolve_circuit_for_scheme, DispatchError};
// [OPUS-4.8] sq-3kd2g.6: the fail-closed wave-1 fragment DISPATCH routing gate.
// [OPUS-4.8] sq-h732x: + `verify_fragment_manifest`, the end-to-end extended-
// fragment verification entry point (dispatch routing + the shared crypto stage,
// stage-1a routed through `fragment_query`). NOT-yet-sound (sq-qhy4); the
// disclosed-solution term binding is deferred to sq-1zf94.
#[cfg(feature = "extended-fragment")]
pub use verifier::{dispatch_fragment, verify_fragment_manifest, FragmentDispatchError};
pub use verifier::EntailmentPolicy;
// [OPUS-4.8] sq-3e5 + sq-h2v: hidden-index revocation host helpers.
pub use revocation::{merkle_root, merkle_witness, revoke_prover_toml, MerkleWitness};
// [OPUS-5] sq-6qe / sq-kndw: the ACCEPTED-SET commitment host helpers — the relying
// party's `(list, version, status_list_root)` trust anchor behind one Merkle root —
// plus the FULLY-HIDDEN revocation prover path (witness builder + Prover.toml
// renderer) for the compiled `revoke_hidden_ref_d10_a4` member. On that mode the
// status-list IRI and version are NOT disclosed; the committed-index path still
// discloses both. Opt-in via `RevocationPolicy::{with_hidden_index_depth,
// with_accepted_set_depth}`. Not externally audited (sq-qhy4).
pub use revocation::{
    accepted_set_leaf, accepted_set_root, accepted_set_witness, hidden_ref_witness,
    revoke_hidden_ref_prover_toml, AcceptedStatusEntry, HiddenRefWitness,
};
// [OPUS-4.8] sq-z9l: hidden-issuer-attestation host helpers (in-circuit
// Schnorr-over-BabyJubJub + hidden-key set membership).
// [OPUS-4.8] sq-8k3h: `*_sparse` are the `O(n·depth)` builders for a very large
// registry (e.g. a holder set) — BIT-IDENTICAL root + path to the dense builders,
// so the depth-generic member is unchanged; only the host build is bounded no more.
pub use issuer::{
    hidden_issuer_prover_toml, key_membership_witness, key_membership_witness_sparse,
    key_set_root, key_set_root_sparse, HiddenIssuerWitness,
};
// [OPUS-4.8] sq-xqfg (HolderPoP T5): in-circuit holder-PoK host helpers (the
// B2 hidden-key tier — `hpk = hsk·G` + holder-key-digest binding).
pub use holder::{holder_pok_prover_toml, holder_pok_witness, HolderPokWitness};
// [OPUS-4.8] sq-3c00 (HolderPoP hidden-holder-SET tier): set-membership host
// helpers (the hidden-holder analogue of the issuer key-set helpers). Opt-in,
// NOT-yet-sound (sq-qhy4).
// [OPUS-4.8] sq-8k3h: `*_sparse` are the `O(n·depth)` holder-set builders for a
// VERY LARGE holder registry — BIT-IDENTICAL root + path to the dense builders, so
// the depth-generic `holder_set_d{depth}` member is unchanged; only the host build
// is no longer bounded to `2^depth`.
pub use holder::{
    holder_set_membership_witness, holder_set_membership_witness_sparse, holder_set_prover_toml,
    holder_set_root, holder_set_root_sparse, HolderSetWitness,
};
// [OPUS-4.8] audit #4: verifier-issued nonce + single-use store.
// [OPUS-4.8] sq-aih: FileSeenNonces is the DURABLE (restart-surviving) store;
// InMemorySeenNonces is NON-DURABLE / test-only.
pub use verifier::{FileSeenNonces, InMemorySeenNonces, SeenNonces, VerifierNonce};
// [OPUS-4.8] audit #12: revocation / freshness policy.
pub use verifier::RevocationPolicy;
// [OPUS-4.8] sq-cwq: external holder trust anchor for the HolderPop binding's PoP.
pub use verifier::HolderRegistry;
