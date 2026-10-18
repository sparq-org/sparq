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
//!
//! ## L4 fragment dispatch guards
//!
//! **Fragment dispatch (L4, bead sq-pbz04.4.4, feature `dispatch`):**
//! `check::DirectChecker::consistency` dispatches an extracted ontology to the first matching
//! branch — **RL** (`sparq-reason` materialization + clash scan; `Inconsistent` sound via *checked*
//! Theorem PR1 preconditions, `Consistent` only past a divergence guard over the constructs
//! implicated in the documented RL rule-set divergences), **EL** (`sparq-reason-el` classification;
//! verdicts only for a pure ⊤-free EL+⊥ TBox with zero skipped axioms), **QL** (opt-in
//! `dispatch_ql`, sq-fj8lj: `sparq-reason-ql`'s DL-Lite_R violation-query checker, whose OWN
//! capture accounting owns the verdict; without it, always `Unknown(QlConsistencyPending)`), or the
//! **ALCH tableau** (complete for the fragment).
//!
//! A profile branch that ABSTAINS falls through to that tableau (sq-pbz04.4.8) — sound with no new
//! argument, since every L1-extracted ontology is inside the ALCH fragment by construction; a
//! branch that DECIDED is never preempted, an abstaining tableau returns the branch's own reason
//! unchanged, and the PR1 punning abstention (ill-posed input, not an incompleteness guard) does
//! not fall through.
//!
//! The **empty built-in roles** `owl:bottomObjectProperty` / `owl:bottomDataProperty` are decided
//! by the checker (#4762): its extraction rewrites `∃⊥.C` to `⊥`, `∀⊥.C` to `⊤`, an assertion over
//! them to `⊥(a)`, and drops `⊥ ⊑ r`, then hands that ontology straight to the tableau (never to a
//! profile branch, which would re-read the raw triples); the public `extract()` still refuses them,
//! and ⊥ as a super-property, `rdfs:domain`/`rdfs:range` on it, cardinality over it, and every use
//! of `owl:topObjectProperty` / `owl:topDataProperty` stay refused.
//!
//! `check::DirectChecker::entailment` decides premise ⊨ conclusion per conclusion-axiom by sound
//! refutation encodings on the tableau (GCI / class-assertion / the fresh-class trick, its
//! sq-pbz04.4.9 role-subsumption lift for `SubObjectPropertyOf`, the sq-zfwzq transitivity lift
//! when enabled, + the record's desugarings); a future unencoded kind abstains.
//!
//! A **blank-node individual in the conclusion is read EXISTENTIALLY** (sq-pbz04.4.13): a
//! tree-shaped anonymous assertion set rolls up into an `∃`-class assertion decided soundly, and a
//! non-rollable shape (shared / cyclic / nominal / free-root) abstains
//! `ConclusionAnonymousIndividual` — never a skolem-constant `NotEntailed`.
//!
//! A refutation that exhausts the tableau's deterministic count budget — and ONLY then — is
//! re-asked of the RL/EL branches under their own unchanged guards (sq-pbz04.4.10): strictly
//! abstention-reducing, it can replace `Unknown(ResourceBudget)` with a definitive verdict but
//! never widen one a guard refused.
//!
//! Every verdict carries its producing `Branch`; every guard fails closed (`Unknown(reason)`, never
//! a guess). The `dispatch` feature pulls `sparq-reason` + `sparq-reason-el` as optional deps —
//! **off by default**, so L1–L3 stay dependency-light.

pub mod extract;
pub mod model;

pub mod nnf;
// #7031: the full OWL 2 structural model, its RDF mapping and the OWL 2 DL profile check.
pub mod owl2dl;
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
