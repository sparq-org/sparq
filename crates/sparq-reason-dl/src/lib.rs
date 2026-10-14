#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)] // sq-pbz04.4.1: this crate has zero `unsafe`.

//! 🤖 SPARQ agent. See the crate README (rendered above) for the capability overview and the
//! honest boundary. This is layer **L1** of the OWL 2 Direct-Semantics workstream (epic
//! sq-pbz04.4, design record `research/owl2-direct-semantics-scoping.md`): a purely
//! **structural** OWL model for the ALCH fragment plus a **fail-closed reverse RDF mapping**.
//! No reasoning happens here — L1 turns an RDF graph into a typed [`Ontology`] it understood
//! IN FULL, or refuses it with a typed [`ExtractError`].
//!
//! The load-bearing entry point is [`extract()`] (`(Dict, &[[Id; 3]])` → [`Ontology`] |
//! [`ExtractError`]); the typed model lives in [`model`]. The later layers build on it:
//! the L2 syntactic profile checker ([`profile`]), the L3 ALCH tableau ([`nnf`] /
//! [`tableau`]), and — behind the OPT-IN `dispatch` cargo feature, which pulls the existing
//! RL/EL profile reasoners as optional dependencies — the L4 fragment-dispatch
//! Direct-Semantics checker + entailment-by-refutation (the `check` module's
//! `DirectChecker`; code spans, not links, because the module is feature-gated). The
//! additive OPT-IN `dispatch_ql` feature (sq-fj8lj) graduates the dispatch's QL
//! branch by pulling sparq-reason-ql's `ql-consistency` DL-Lite_R checker; without it the
//! QL branch abstains (`QlConsistencyPending`).

pub mod datatypes;
pub mod extract;
pub mod model;

pub mod nnf;
pub mod profile;
pub mod render;
pub mod tableau;

// sq-pbz04.4.4 (L4): the fragment-dispatch checker, compiled ONLY under the
// `dispatch` feature so the L1–L3 layers stay dependency-light (design record §6).
#[cfg(feature = "dispatch")]
pub mod check;

#[doc(inline)]
pub use extract::{extract, ExtractError};
#[doc(inline)]
pub use model::{Axiom, ClassExpression, ObjectPropertyExpression, Ontology};
// sq-pbz04.4.7: forward RDF renderer (diagnostic / round-trip aid).
#[doc(inline)]
pub use render::{render_to_triples, render_to_turtle, triples_to_turtle};
#[cfg(feature = "dispatch")]
#[doc(inline)]
pub use check::DirectChecker;
