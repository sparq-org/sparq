//! sparq-zk-compose circuit core (plan S4.E module (ii), v1).
//!
//! Generic implementations of the per-property proof circuits; the tiny bin
//! packages (`scan_k{K}_n{N}_r{R}`, `filter_int_d{D}`, `filter_f64`) are the
//! compiled circuit-family members -- each `main` is one monomorphisation of a
//! function in this library (Q5: a circuit FAMILY over a (k, n) lattice, not
//! one dynamic circuit).
//!
//! Bit-compatibility contract: every hash here is
//! `poseidon::poseidon2::Poseidon2::hash` (noir-lang/poseidon v0.3.0), the
//! exact function `crates/sparq-zk/src/poseidon2.rs` ports and cross-tests
//! against `nargo` (`tests/poseidon2_noir_cross.rs`). `tests.nr` re-asserts
//! the same fixture vectors from the Noir side.

pub mod hashes;
pub mod scan;
pub mod filter_int;
pub mod filter_float;
// [OPUS-4.8] sq-1q9h: signed xsd:integer + xsd:decimal FILTER coverage.
pub mod filter_signed;
// [OPUS-4.8] sq-xojl: DUAL-LEAF value-lane FILTER (VALUE_HOOK + lexical leaf;
// no in-circuit blake3). Carries the documented INV-VL downgrade (#769 accepted,
// CR-G8 / sq-qhy4). Opt-in, NOT externally audited.
pub mod filter_value;
pub mod revoke;
pub mod issuer;
// [OPUS-4.8] sq-xqfg (HolderPoP T5): in-circuit holder Proof-of-Possession (B2).
pub mod holder;
// [OPUS-4.8] sq-bwwl (hidden-join sq-8mwz): in-circuit hidden cross-credential JOIN.
pub mod join;
// [FABLE-5] sq-3kd2g.2 (#1591): bounded-depth path reachability -- property
// paths `p+`/`p*` under the bounded existence-only statement of
// research/zksparql-fragment-extension.md sec 4 (depth D a compile-time constant
// per member, disclosed by the VK identity; mechanism (a) on #1591).
pub mod path;
// [FABLE-5] sq-g91d: in-circuit RDFS single-step entailment -- proves a derived
// triple's UNDISCLOSED antecedents are members of the committed graph (the
// privacy upgrade for the Rdfs/Owl regimes over the host disclosed-base re-check
// `bind_entailment`). Single-step / disclosed-base scope; NOT-yet-sound
// (sq-qhy4). Compiled bin member + verifier wiring are follow-ups.
pub mod entail;
// [OPUS-4.8] sq-rsd3v.2 (#2614): in-circuit RDFS derivation relation (ZK over
// HIDDEN antecedents -- design row A1). Research-grade, NOT externally audited
// (sq-qhy4); the in-circuit relation only -- the off-circuit ProofTree->slot
// witness mapper + a compiled bin-package monomorphisation are follow-ups.
pub mod derivation;
// [OPUS-5] sq-rsd3v.6 (#3265): owl:sameAs EQUALITY reasoning -- in-circuit
// union-find canonicalisation + representative-membership. SEPARATELY gated
// from `derivation` on purpose: encoding-equality re-checks are UNSOUND under
// equality reasoning, so `owl:sameAs` must NOT ride the fixed-shape RDFS /
// OWL-RL-minus-sameAs path. Research-grade, NOT externally audited (sq-qhy4);
// no cost claim (`bb gates` not run) and no compiled bin package yet.
pub mod sameas;
// [OPUS-5] sq-rsd3v.3 (#2946): witnessed-rule-shape N3 derivation -- `derivation`
// generalised from a FIXED public rule table to a DATASET-SUPPLIED rule graph
// committed into `rules_root` (the rule author as issuer), plus the whitelisted
// `math:` builtin gadgets. Enforces the declared PROVABLE SUBSET in-circuit --
// notably SAFETY (every conclusion variable bound in the premise), without which
// the private substitution would let a prover derive anything. Research-grade,
// NOT externally audited (sq-qhy4); the relation only -- no compiled bin package,
// no `bb gates` measurement, no `verify_manifest` dispatch.
pub mod n3;

mod tests;
