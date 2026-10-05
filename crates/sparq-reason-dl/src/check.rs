//! L4 — fragment-dispatch Direct-Semantics checker + entailment-by-refutation.
//!
//! 🤖 SPARQ agent [FABLE-5]. Bead sq-pbz04.4.4 (epic sq-pbz04.4, design record
//! `research/owl2-direct-semantics-scoping.md` §4 — the SPEC; its soundness arguments are
//! binding on this module). Feature-gated behind `dispatch`, which pulls the existing
//! profile reasoners (`sparq-reason` RL, `sparq-reason-el` EL) as optional dependencies.
//!
//! # Dispatch (design record §4)
//!
//! [`DirectChecker::consistency`] extracts the graph through the fail-closed L1 mapping,
//! asks the L2 syntactic profile checker where the ontology lives, and dispatches IN ORDER:
//!
//! | # | Branch | Engine | `Consistent` | `Inconsistent` | Abstention |
//! |---|--------|--------|--------------|----------------|------------|
//! | 1 | RL (L2 says in-RL) | `sparq_reason::materialize(Profile::OwlRl)` + `inconsistencies()` | only past the **divergence guard** | sound via **Theorem PR1** (preconditions CHECKED, not assumed) | [`UnknownReason::RlPr1Preconditions`], [`UnknownReason::RlDivergenceGuard`] |
//! | 2 | EL (in-EL) | `sparq_reason_el::Classifier::classify` | only for a pure ⊤-free EL+⊥ TBox (argument below) | never (see the ⊤ guard) | [`UnknownReason::ElSkippedAxioms`], [`UnknownReason::ElUnappliedAxioms`], [`UnknownReason::ElTopGuard`] |
//! | 3 | QL (in-QL) | `sparq_reason_ql::check_consistency` (opt-in feature `dispatch_ql`, [FABLE-5] sq-fj8lj); without it, none — deferred | only past the QL crate's own capture accounting (see §"QL branch soundness") | sound at any capture level (monotonicity) | [`UnknownReason::QlCaptureGap`]; without `dispatch_ql`, [`UnknownReason::QlConsistencyPending`] (always) |
//! | 4 | ALCH (everything else L1 extracted) | the L3 tableau | complete for the fragment | complete for the fragment | [`UnknownReason::ResourceBudget`] |
//! | 4′ | any of 1–3 that ABSTAINED (except PR1 punning) | the same L3 tableau | complete for the fragment | complete for the fragment | the ORIGINAL branch's abstention, kept verbatim |
//! | — | extraction failed | none | never | never | [`UnknownReason::OutOfFragment`] |
//!
//! Row 4′ is the guard-abstention fall-through ([SONNET-4.6] sq-pbz04.4.8) — see below.
//!
//! **Opt-in transitive roles ([GPT-5.6] sq-zfwzq, feature `dl_transitive`):** an ontology
//! declaring `owl:TransitiveProperty` (the feature-gated `TransitiveObjectProperty` axiom
//! kind) routes STRAIGHT to the ALCH+S tableau before the profile dispatch — the only
//! branch whose soundness/completeness argument covers transitivity (tableau module docs
//! §5a). The RL/EL guards below also recognise the axiom kind fail-closed (defence in
//! depth). With the feature OFF, extraction refuses transitivity before dispatch, exactly
//! as before.
//!
//! The first branch whose profile test matches OWNS the verdict. Every verdict carries the
//! [`Branch`] that produced it (the traceability invariant), and every guard fails CLOSED:
//! uncertainty is [`UnknownReason`], never a guessed `Consistent`/`Inconsistent`.
//!
//! ## Guard-abstention tableau fall-through ([SONNET-4.6] sq-pbz04.4.8)
//!
//! When the OWNING profile branch ABSTAINS, the dispatch re-asks the SAME ontology of the
//! L3 ALCH tableau (`tableau_fall_through`) and prefers its definitive verdict, attributed
//! to [`Branch::AlchTableau`]. This needs no new soundness argument: every ontology the L1
//! mapping accepts is inside the §3 fragment by construction (that is exactly what licenses
//! branch 4 as the catch-all), and the tableau is sound AND complete for it — so an
//! in-profile ontology is no harder for the tableau than a profile-free one. It is the
//! mirror image of the refutation budget fallback below ([OPUS-5] sq-pbz04.4.10), which
//! re-asks the profile branches when the TABLEAU abstains.
//!
//! Rules, all of them the dispatch's own:
//!
//! 1. The profile branch keeps FIRST refusal — the fall-through never pre-empts a branch
//!    that decided, so no existing verdict or attribution moves.
//! 2. The fall-through is strictly ABSTENTION-REDUCING: if the tableau also abstains (its
//!    count budget), the owner's original outcome is returned UNCHANGED — same verdict, same
//!    reason, same [`Branch`]. The tableau never overwrites a guard's diagnosis with a
//!    `ResourceBudget` one it earned second.
//! 3. [`UnknownReason::RlPr1Preconditions`] does NOT fall through. It is the one abstention
//!    that is not an incompleteness guard: usage-level punning means the input is ill-posed
//!    as an OWL 2 DL ontology, so the L1 shadow is one arbitrary reading of it rather than
//!    a faithful one, and a definitive verdict on that shadow would be a verdict on a
//!    different ontology. Fail-closed, unchanged. (An input that puns but is in NO profile
//!    already reaches the tableau through branch 4; making that boundary uniform is
//!    deliberately out of this bead's scope.)
//! 4. With `dl_transitive` a transitivity-bearing ontology never gets here — it short-
//!    circuits to the ALCH+S tableau BEFORE the profile dispatch, so the fall-through has
//!    nothing to add for it.
//!
//! **The QL do-not-duplicate rule (sq-pbz04.3.4), re-examined explicitly.** The record's QL
//! step says DL-Lite_R consistency is deferred to the QL workstream and must not be
//! DUPLICATED here. The fall-through does not duplicate it: it re-uses the ALCH tableau that
//! already exists in this crate and is already argued complete for the fragment — no
//! DL-Lite_R reasoning is written, and no QL-specific claim is made. Ownership is also
//! preserved: with `dispatch_ql` the QL crate is still asked FIRST and its verdicts still
//! win, so the fall-through only ever fires on [`UnknownReason::QlCaptureGap`] — the case
//! the QL crate itself declines. Without `dispatch_ql` it fires on the blanket
//! [`UnknownReason::QlConsistencyPending`], which is a deferral of the QL PROCEDURE, not a
//! claim that the ontology is undecidable here.
//!
//! ## RL branch soundness
//!
//! - `Inconsistent`: the implemented RL/RDF rules are sound, and for RL ontologies Theorem
//!   PR1 (W3C OWL 2 Profiles §4.3) ties rule-derived inconsistency to Direct-Semantics
//!   inconsistency. The PR1 preconditions are **checked**: (a) *no punning* — no id is used
//!   as more than one of {class, object property, individual} anywhere in the L1 model
//!   (usage-level punning is exactly what reaches the rule closure; declaration-only typing
//!   contributes no logical triples in the L1 fragment, and unknown typings fail extraction);
//!   (b) *no annotation axioms* — guaranteed structurally: the fail-closed L1 mapping has no
//!   annotation-axiom shape (built-in annotations are ignored, unknown predicates refuse the
//!   graph). A punning violation abstains BOTH verdicts (under punning the input is not an
//!   OWL 2 DL ontology, so a Direct-Semantics verdict is ill-posed).
//! - `Consistent` ("no clash") additionally needs rule-set *completeness*, which sparq's RL
//!   does not globally have — 13 documented divergences (`DOCUMENTED_DIVERGENCES` in
//!   sparq-conformance's `owl_suite.rs`). The **divergence guard** abstains when the model
//!   touches any implicated construct reachable through the L1∩RL funnel:
//!   `owl:complementOf` (DisjointClasses-001/-003, New-Feature-ObjectQCR-002),
//!   `owl:unionOf` (WebOnt-I5.5-005), and `owl:disjointWith` (DisjointClasses-001/-003) —
//!   a deliberate over-approximation (fail-closed). The other ten entries implicate
//!   constructs (property chains/transitivity, maxQC, `differentFrom`/`AllDifferent`,
//!   functional/inverse-functional/reflexive/disjoint properties, datatype ranges) that L1
//!   extraction already refuses, so they cannot reach this branch.
//!
//! ## EL branch soundness
//!
//! `Classifier::classify` is a TBox classifier: it applies `subClassOf` /
//! `equivalentClass` / `disjointWith` axioms only, counting out-of-fragment class
//! expressions in `Report::skipped_axioms`; ABox assertions, `rdfs:subPropertyOf`,
//! `rdfs:domain`, and `rdfs:range` are neither applied nor counted, and it does NOT track
//! the satisfiability of ⊤ (verified empirically: `owl:Thing rdfs:subClassOf owl:Nothing`
//! yields an empty unsatisfiable-class list). Three guards therefore protect the verdict:
//! skipped axioms abstain ([`UnknownReason::ElSkippedAxioms`], per the record); axiom kinds
//! the classifier does not apply abstain ([`UnknownReason::ElUnappliedAxioms`]); any
//! occurrence of `owl:Thing` abstains ([`UnknownReason::ElTopGuard`]). Past all three the
//! input is a pure EL+⊥ TBox not mentioning ⊤, and `Consistent` is sound by a direct model
//! construction: interpret every named class and role as empty over a one-element domain —
//! by induction every ⊤-free EL class expression (named class, ⊥, ⊓, ∃) denotes ∅, so every
//! GCI/equivalence/disjointness holds. This branch never returns `Inconsistent` (a pure
//! ⊤-free EL+⊥ TBox is always consistent; anything else is guarded away).
//!
//! ## QL branch soundness (opt-in `dispatch_ql`, [FABLE-5] sq-fj8lj)
//!
//! With the `dispatch_ql` feature the QL branch reconstructs the ORIGINAL input triples from
//! the dict (a faithful id-to-term rendering of exactly the graph L1 extracted — nothing is
//! translated through the L1 model) and hands them to `sparq_reason_ql::check_consistency`,
//! the sq-p6yb7 DL-Lite_R checker (violation-query composition over PerfectRef). The
//! cross-crate soundness argument deliberately does NOT rely on the L2 profile test agreeing
//! with the QL crate's fragment: L2's `In(QL)` is a ROUTING decision only. The QL crate
//! re-extracts its own TBox from the raw triples with its OWN capture accounting, and:
//! - `Inconsistent` is sound at ANY capture level (a violation witnessed on a captured
//!   SUBSET of the axioms is an inconsistency of the whole KB, by monotonicity) — and for
//!   an ontology in OWL 2 QL, DL-Lite_R unsatisfiability of the encoded KB coincides with
//!   Direct-Semantics inconsistency (the profile's defining correspondence, W3C QL §3 /
//!   Calvanese et al. JAR 2007);
//! - `Consistent` is returned by the QL crate ONLY when its `TBox::fully_captured()` holds
//!   AND `consistency_uncaptured == 0` — i.e. its own accounting certifies that every schema
//!   triple was captured, so the cln(T)-closure completeness argument applies to the WHOLE
//!   KB. Any L2-grammar-vs-QL-capture semantic drift (e.g. the QL super-∃ with a named
//!   filler, which the W3C grammar admits but the QL crate conservatively skips) therefore
//!   lands in `Unknown`, never in a manufactured verdict;
//! - anything else maps to [`UnknownReason::QlCaptureGap`] (fail-closed, carrying the QL
//!   crate's own gap accounting).
//!
//! Note the structural consequence of the in-order dispatch: a graph reaches this branch
//! only when it is in-QL but NOT in-RL and NOT in-EL, which (over the L1 axiom kinds)
//! forces an `ObjectComplementOf` somewhere. The QL crate now captures the QL-legal
//! superclass form of that shape — `A rdfs:subClassOf [ owl:complementOf B ]`, i.e. the
//! DL-Lite_R negative inclusion `A ⊑ ¬B` (sparq issue #2513) — so a graph routed here whose
//! complements are all in that position can graduate `Consistent` as well as `Inconsistent`
//! (disjointness violations were already decided at any capture level). The remaining
//! complement positions the QL crate still counts uncaptured — a NAMED-subject
//! `A owl:complementOf B` (the biconditional `A ≡ ¬B`, strictly stronger than a negative
//! inclusion) and a complement nested inside another class expression — keep landing in
//! `Unknown(QlCaptureGap)`; that is the honest fail-closed reading, not a defect. Without the
//! feature the branch abstains with [`UnknownReason::QlConsistencyPending`] exactly as before.
//!
//! # Entailment by refutation (design record §4)
//!
//! [`DirectChecker::entailment`] checks `O ⊨ α` per conclusion axiom by encoding the
//! refutation into a consistency question and deciding it with the **L3 ALCH tableau**
//! (sound and complete for the entire L1 fragment, which contains every premise the L1
//! mapping accepts and every encoding below — so routing refutations to the tableau, the
//! strongest complete branch, preserves the record's "definitive verdicts only from a
//! complete branch" rule; when, and only when, the tableau runs OUT OF BUDGET on a
//! refutation, the profile branches get a second look — see §"Refutation budget fallback"):
//!
//! - `SubClassOf(C, D)` — `O ∪ {(C ⊓ ¬D)(x)}` with `x` fresh: inconsistent iff `O ⊨ C ⊑ D`.
//! - `ClassAssertion(C, a)` — `O ∪ {(¬C)(a)}`: inconsistent iff `O ⊨ C(a)`.
//! - `ObjectPropertyAssertion(R, a, b)` — `O ∪ {B(b), (∀R.¬B)(a)}` with `B` a FRESH class
//!   name. Sound AND complete (record §4; re-verified for this implementation): if every
//!   model has `(a,b) ∈ R` then `b ∈ B` and `a ∈ ∀R.¬B` clash, so the union is
//!   inconsistent; conversely a model `I` of `O` with `(aᴵ,bᴵ) ∉ Rᴵ` extends to a model of
//!   the union by interpreting `Bᴵ = {bᴵ}` (`B` occurs nowhere in `O`, and no R-successor
//!   of `aᴵ` is `bᴵ`), so non-entailment yields consistency.
//! - `EquivalentClasses(C, D)` — the two GCI refutations (`C ⊑ D` and `D ⊑ C`); exact by
//!   the Direct-Semantics desugaring the record §3 lists.
//! - `DisjointClasses(C, D)` — `O ∪ {(C ⊓ D)(x)}`, `x` fresh: the `C ⊓ D ⊑ ⊥` desugaring.
//! - `ObjectPropertyDomain(R, C)` / `ObjectPropertyRange(R, C)` — the `∃R.⊤ ⊑ C` /
//!   `⊤ ⊑ ∀R.C` desugarings (record §3), then the GCI refutation.
//! - `SubObjectPropertyOf(R, S)` — `O ∪ {R(a,b), B(b), (∀S.¬B)(a)}` with `a`, `b` FRESH
//!   individuals and `B` a FRESH class name ([FABLE-5] sq-pbz04.4.9; record §4 as amended —
//!   the role-subsumption lift of the fresh-class trick). Sound AND complete: if every
//!   model of `O` has `Rᴵ ⊆ Sᴵ`, then in any model of the union `(a,b) ∈ R ⊆ S` makes `b`
//!   an `S`-successor of `a`, so `a ∈ ∀S.¬B` forces `b ∈ ¬B`, clashing with `B(b)` — the
//!   union is inconsistent. Conversely a model `I` of `O` with some `(u,v) ∈ Rᴵ ∖ Sᴵ`
//!   extends to a model of the union via `aᴵ = u`, `bᴵ = v`, `Bᴵ = {v}` (`a`/`b`/`B` occur
//!   nowhere in `O`; `(u,v) ∉ Sᴵ` means no `S`-successor of `u` is `v`, the sole member of
//!   `B`, so `(∀S.¬B)(a)` holds) — non-entailment yields consistency. The tableau side
//!   holds because its `∀`-rule fires modulo the role hierarchy (`System::is_subrole`,
//!   reflexive-transitive), so the `R`-edge `a → b` is seen by `∀S` exactly when
//!   `O ⊢ R ⊑* S`. All three additions stay inside the L1 fragment.
//! - `TransitiveObjectProperty(R)` (opt-in `dl_transitive`, [GPT-5.6] sq-zfwzq) —
//!   `O ∪ {R(a,b), R(b,c), B(c), (∀R.¬B)(a)}` with `a`, `b`, `c` FRESH individuals and
//!   `B` a FRESH class name: the two-step-chain lift of the fresh-class trick. Sound AND
//!   complete: if `O ⊨ Trans(R)`, then in any model of the union `Rᴵ` is transitive, so
//!   `(a,b), (b,c) ∈ R` give `(a,c) ∈ R` — `a ∈ ∀R.¬B` forces `c ∈ ¬B`, clashing with
//!   `B(c)`: the union is inconsistent. Conversely a model `I` of `O` in which `Rᴵ` is
//!   NOT transitive has a witness chain `(u,v), (v,w) ∈ Rᴵ` with `(u,w) ∉ Rᴵ`; extend it
//!   via `aᴵ = u`, `bᴵ = v`, `cᴵ = w`, `Bᴵ = {w}` (`a`/`b`/`c`/`B` occur nowhere in `O`;
//!   `(u,w) ∉ Rᴵ` means no `R`-successor of `u` is `w`, the sole member of `B`, so
//!   `(∀R.¬B)(a)` holds) — non-entailment yields consistency. All four additions stay
//!   inside the feature-extended L1 fragment, and the tableau deciding them is the
//!   §5a-argued ALCH+S calculus (sound and complete for that fragment), so the reduction
//!   is exact however `O` happens to entail the transitivity — e.g. a declared `Trans(R)`
//!   (the ∀₊-rule carries `∀R.¬B` down the `a → b → c` chain to the clash at `c`), a
//!   declared-transitive equivalent role, or an `R` forced empty by `⊤ ⊑ ∀R.⊥`.
//!
//! `Entailed` needs every conclusion axiom's refutation(s) unsatisfiable; a single
//! definitively-satisfiable refutation is `NotEntailed` (sound because the tableau is
//! complete — exactly the record's rule that negative-entailment verdicts come only from a
//! complete branch); anything else abstains. An empty conclusion ontology is trivially
//! `Entailed`. Fresh names are minted as `urn:` IRIs checked against every id occurring in
//! the premise AND conclusion models.
//!
//! ## Refutation budget fallback ([OPUS-5] sq-pbz04.4.10)
//!
//! The tableau OWNS every refutation: it is the only branch complete for the whole L1
//! fragment, so routing there FIRST loses no verdict. What it can lose is a verdict to its
//! deterministic COUNT budget — and a large in-RL premise is exactly the shape where the RL
//! materializer decides in one pass what the tableau cannot finish. So when, and ONLY when, a
//! refutation returns [`UnknownReason::ResourceBudget`], `refutation_profile_fallback` re-asks
//! the SAME question of the profile branches, under the SAME guard discipline as
//! [`DirectChecker::consistency`]:
//!
//! 1. an augmented ontology carrying a transitivity axiom is not re-asked at all — only the
//!    ALCH+S tableau is argued for transitivity, the same ownership rule `consistency` applies
//!    (opt-in `dl_transitive` only);
//! 2. `profiles(O ∪ additions)` routes in the same RL → EL order; a refutation in neither
//!    profile has no fallback and keeps its `ResourceBudget` abstention;
//! 3. both branches consume TRIPLES (RL materializes; the EL classifier re-extracts its own
//!    TBox), so the augmented MODEL is serialised by the L1 forward renderer
//!    ([`crate::render::render_to_triples`], sq-pbz04.4.7). Its round-trip invariant is
//!    VERIFIED PER CALL — re-extract the rendering and compare axiom multisets — rather than
//!    trusted: a rendering gap costs a fallback, it can never manufacture a verdict;
//! 4. `rl_branch` / `el_branch` then decide with their PR1 / divergence / skipped /
//!    unapplied / ⊤ guards completely unchanged. Any abstention on the fallback keeps the
//!    ORIGINAL `ResourceBudget` reason — the honest one, since the tableau is what ran out.
//!
//! Soundness needs no new argument. `Inconsistent` from the RL branch is sound by the same
//! checked-PR1 bridge, so the refutation is unsatisfiable and that component IS entailed;
//! `Consistent` past the divergence guard is sound by the same argument, so the refutation is
//! satisfiable and the axiom is NOT entailed. The verdict is therefore attributable to a
//! branch whose story is already written above, and [`EntailmentOutcome::branch`] names THAT
//! branch instead of [`Branch::AlchTableau`] whenever the fallback broke the tie. The fallback
//! is strictly abstention-reducing: it runs only after the tableau has already abstained, and
//! it can only replace `Unknown(ResourceBudget)` with a definitive verdict.
//!
//! **What it actually recovers, stated honestly.** Every refutation encoding above adds at
//! least one ABox assertion, and the EL classifier applies no ABox axiom — so on today's
//! encodings the EL arm ALWAYS abstains ([`UnknownReason::ElUnappliedAxioms`]). It is wired
//! for the routing discipline and for future encodings, not because it decides anything now.
//! And every encoding except the `DisjointClasses` one introduces an `owl:complementOf`, which
//! the RL divergence guard implicates — so the RL arm recovers `Inconsistent` (⇒ `Entailed`)
//! freely, because that verdict is returned BEFORE the guard, while `Consistent` (⇒
//! `NotEntailed`) survives the guard only for the complement-free `DisjointClasses`
//! refutation. Recovering an abstention is the whole claim; nothing here widens a verdict the
//! guards would have refused.
//!
//! **Measured against the pinned W3C corpus — where it recovers NOTHING.** The L5 DIRECT arm
//! (`sparq-conformance`, `dl-direct`, pinned export + pinned budget) does hit the trigger: 7
//! entailment rows abstain on `ResourceBudget`, 65 refutation components in total. The fallback
//! declines EVERY one of them at step 2 — the augmented ontology is in neither RL nor EL, so no
//! branch may own it (measured: 0 round-trip mismatches, 0 guard abstentions, 65 no-profile).
//! The structural reason is the encodings themselves: `(C ⊓ ¬D)(x)` is RL-legal only when `C` is
//! ALSO an RL superClassExpression, and the corpus rows whose refutations are hard enough to
//! exhaust the tableau are exactly the ones carrying an `∃`/`⊔` shape that is not. So the arm's
//! floors (`DL_DIRECT_FLOOR` / `DL_DIRECT_ABSTAINED`) and its pinned divergence set are
//! UNCHANGED by this fallback — verified by running the lane, not assumed. What it buys is
//! robustness for the shape the bead named (a large in-RL premise with an RL-legal refutation,
//! pinned by `entailment_budget_fallback_rl_recovers_*` in `tests/check.rs`), NOT a corpus
//! improvement. This note exists so nobody re-derives the measurement or over-claims the
//! mechanism; narrowing the RL divergence guard, not more routing, is what would move those 7.
//!
//! ## Conclusion anonymous individuals ([OPUS-4.8] sq-pbz04.4.13)
//!
//! Official OWL 2 Direct-Semantics entailment reads a blank-node individual in the CONCLUSION
//! EXISTENTIALLY (`a p _:x` means "`a` has *some* `p`-successor", not "`a p c` for a specific
//! constant `c`"). L1 maps a blank node to an ordinary (skolem) constant: entailment-preserving
//! on the PREMISE side (kept), but UNSOUND on the CONCLUSION side — the skolem refutation of
//! `a p _:x` stays satisfiable even when the premise's `∃p.⊤` typing grants the existential, so
//! it would certify a WRONG `NotEntailed`. Before the refutation loop, `roll_conclusion_anonymous`
//! resolves conclusion blank nodes under the existential reading: a TREE-shaped anonymous
//! assertion set (each blank node the object of exactly one property edge, reaching only blank
//! successors, acyclic, anchored to a named root) ROLLS UP into an existential class assertion
//! `root : ∃p.(⊓ types ⊓ ⊓ ∃q.⟨child⟩)` the tableau decides SOUNDLY in both directions (so
//! somevaluesfrom2bnode / WebOnt-someValuesFrom-003 become genuine passes). Any non-rollable
//! shape — shared between two assertions, cyclic, a named/nominal successor, or an unanchored
//! free-existential root — abstains fail-closed ([`UnknownReason::ConclusionAnonymousIndividual`]),
//! NEVER a skolem-constant `NotEntailed`.
//!
//! **Declaration-free conclusion roles ([GPT-5.6] sq-zfwzq).** OWL declarations have no
//! Direct-Semantics consequences, and W3C conclusion documents commonly omit an
//! `owl:ObjectProperty` declaration for a role already typed by the premise. In the
//! `dl_transitive` extension, L1 deliberately refuses an otherwise-ambiguous RDF predicate,
//! so entailment augments the conclusion's extraction input with declarations for **only**
//! named object properties already proved to be roles by a fully-extracted premise that
//! declares transitivity. This is semantics-preserving (the added declarations carry no
//! logical content), never guesses an unknown predicate's kind, is discarded after extraction,
//! and leaves the pre-feature ALCH entailment boundary unchanged. It is load-bearing for the
//! DIRECT transitive-chain entailment cases, whose conclusion is just the composed role
//! assertion.
//!
//! # Honest boundary
//!
//! This module never claims completeness beyond the argued fragment: the only complete
//! branch is the ALCH tableau (within budget); RL `Consistent` is guard-narrowed, EL
//! `Consistent` is ⊤-free-TBox-narrowed, QL consistency is capture-accounting-narrowed
//! under the opt-in `dispatch_ql` feature (and wholly deferred without it), and every
//! uncertain case is a typed [`UnknownReason`].

use crate::extract::extract;
use crate::model::{Axiom, ClassExpression, ObjectPropertyExpression, Ontology};
use crate::profile::profiles;
// [OPUS-5] sq-pbz04.4.10: the L1 forward renderer is the Ontology→triples encoder the
// refutation budget fallback needs to hand an augmented model to the triple-consuming RL/EL
// branches (module docs §"Refutation budget fallback").
use crate::render::render_to_triples;
use crate::tableau::{self, Budget, ExhaustedBudget};
#[cfg(feature = "dl_transitive")]
use oxrdf::NamedNode;
#[cfg(any(feature = "dl_transitive", feature = "dispatch_ql"))]
use oxrdf::Term as OTerm;
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{is_inline, Dict, Id, TermParts};
use sparq_reason::{inconsistencies, materialize, Profile};
use sparq_reason_el::Classifier;

// -------------------------------------------------------------------------------------------
// Public verdict types
// -------------------------------------------------------------------------------------------

/// The dispatch branch that produced a verdict — the traceability half of the module
/// invariant: every verdict is attributable to a branch whose soundness/completeness story
/// is written in the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Branch {
    /// The fail-closed L1 extraction itself (the graph never reached a reasoner).
    Extraction,
    /// OWL 2 RL materialization + clash scan (`sparq-reason`), PR1-checked.
    RlMaterialization,
    /// OWL 2 EL consequence-based classification (`sparq-reason-el`), triple-guarded.
    ElClassification,
    /// OWL 2 QL without the `dispatch_ql` feature — consistency deferred (the arm that
    /// sq-fj8lj graduates; produced only when `dispatch_ql` is OFF).
    QlDeferred,
    /// OWL 2 QL DL-Lite_R consistency by violation-query composition
    /// (`sparq_reason_ql::check_consistency`, sq-p6yb7) — produced only under the opt-in
    /// `dispatch_ql` feature (module docs §"QL branch soundness"). [FABLE-5] sq-fj8lj
    QlConsistency,
    /// The L3 ALCH completion-forest tableau (complete for the L1 fragment). Produced both
    /// for an ontology in NO profile and — since [SONNET-4.6] sq-pbz04.4.8 — for one whose
    /// owning profile branch abstained and whose verdict the tableau then supplied (module
    /// docs §"Guard-abstention tableau fall-through").
    AlchTableau,
}

/// Why the checker abstained. Fail-closed: no reason ever degrades into a verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnknownReason {
    /// The L1 mapping refused the graph (rendered `ExtractError`): the input uses a
    /// construct outside the ALCH structural fragment, or is malformed.
    OutOfFragment(String),
    /// The RL branch matched but a Theorem PR1 precondition failed (usage-level punning);
    /// the Direct-Semantics bridge does not apply, so BOTH verdicts are withheld.
    RlPr1Preconditions(String),
    /// The RL branch found no clash, but the model touches a construct implicated in the
    /// documented RL rule-set divergences, so "no clash" does not certify consistency.
    RlDivergenceGuard(String),
    /// The EL classifier skipped that many axioms (out-of-fragment class expressions); a
    /// skipped axiom could be the inconsistency.
    ElSkippedAxioms(usize),
    /// The model contains an axiom kind the EL classifier does not apply (ABox assertions,
    /// `rdfs:subPropertyOf`, `rdfs:domain`, `rdfs:range`) — and does not count as skipped.
    ElUnappliedAxioms(String),
    /// The model mentions `owl:Thing`: the EL classifier does not track ⊤'s
    /// satisfiability (empirically verified), so a ⊤-involving verdict is withheld.
    ElTopGuard,
    /// The ontology dispatched to the QL branch with the `dispatch_ql` feature OFF: the
    /// Direct-Semantics consistency procedure lives in sparq-reason-ql (sq-p6yb7) and is
    /// engaged only opt-in — never duplicated here. Produced only without `dispatch_ql`
    /// ([FABLE-5] sq-fj8lj graduated the feature-on path).
    QlConsistencyPending,
    /// The QL branch ran the sparq-reason-ql DL-Lite_R checker (feature `dispatch_ql`) and
    /// it abstained: its own extraction accounting could not certify full capture of the KB
    /// (`fully_captured()` false, or a structurally-uncaptured negative axiom such as
    /// `owl:complementOf`), so a `Consistent` claim would be unsound and no violation query
    /// matched. The payload carries the QL crate's gap accounting. Fail-closed.
    /// [FABLE-5] sq-fj8lj
    QlCaptureGap(String),
    /// The ALCH tableau exhausted its deterministic count budget mid-search.
    ResourceBudget(ExhaustedBudget),
    /// Entailment only: the conclusion-axiom kind has no argued refutation encoding.
    /// Every axiom kind the L1 model can currently express HAS an argued encoding
    /// (`SubObjectPropertyOf` gained one under sq-pbz04.4.9; the opt-in
    /// `TransitiveObjectProperty` kind gained the two-step-chain lift under sq-zfwzq), so
    /// this reason is not produced today — it is RETAINED fail-closed for future model
    /// growth (a new axiom kind abstains here until its encoding is argued in the design
    /// record). [GPT-5.6]
    UnencodedConclusion(String),
    /// Entailment only: the CONCLUSION ontology mentions a blank-node individual in a shape
    /// that does NOT roll up into a tree-shaped existential class assertion (it is shared
    /// between two assertions, cyclic, has a named or a nominal successor, or is a free
    /// existential root). Official OWL 2 Direct-Semantics entailment reads a conclusion
    /// blank node EXISTENTIALLY, so the L1 skolem-constant reading would be UNSOUND for a
    /// `NotEntailed` verdict — the checker abstains fail-closed instead (sq-pbz04.4.13).
    /// The payload names the first offending anonymous individual. [OPUS-4.8]
    ConclusionAnonymousIndividual(String),
}

/// Tri-state Direct-Semantics consistency verdict (design record §4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsistencyVerdict {
    /// The ontology has a model — asserted only by a branch complete for its guard.
    Consistent,
    /// The ontology has no model — asserted only where the producing branch is sound.
    Inconsistent,
    /// No verdict; the payload says exactly why (fail-closed).
    Unknown(UnknownReason),
}

impl ConsistencyVerdict {
    /// `true` iff the verdict is [`ConsistencyVerdict::Consistent`].
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        matches!(self, ConsistencyVerdict::Consistent)
    }

    /// `true` iff the verdict is [`ConsistencyVerdict::Inconsistent`].
    #[must_use]
    pub fn is_inconsistent(&self) -> bool {
        matches!(self, ConsistencyVerdict::Inconsistent)
    }

    /// `true` iff the checker abstained.
    #[must_use]
    pub fn is_unknown(&self) -> bool {
        matches!(self, ConsistencyVerdict::Unknown(_))
    }
}

/// Tri-state Direct-Semantics entailment verdict (refutation-based, design record §4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntailmentVerdict {
    /// Every conclusion axiom is entailed (each refutation is unsatisfiable).
    Entailed,
    /// At least one conclusion axiom is definitively NOT entailed (its refutation is
    /// satisfiable, certified by the complete ALCH tableau).
    NotEntailed,
    /// No verdict; the payload says exactly why (fail-closed).
    Unknown(UnknownReason),
}

impl EntailmentVerdict {
    /// `true` iff the verdict is [`EntailmentVerdict::Entailed`].
    #[must_use]
    pub fn is_entailed(&self) -> bool {
        matches!(self, EntailmentVerdict::Entailed)
    }

    /// `true` iff the verdict is [`EntailmentVerdict::NotEntailed`].
    #[must_use]
    pub fn is_not_entailed(&self) -> bool {
        matches!(self, EntailmentVerdict::NotEntailed)
    }

    /// `true` iff the checker abstained.
    #[must_use]
    pub fn is_unknown(&self) -> bool {
        matches!(self, EntailmentVerdict::Unknown(_))
    }
}

/// A consistency verdict plus the branch that produced it (traceability invariant).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsistencyOutcome {
    /// The tri-state verdict.
    pub verdict: ConsistencyVerdict,
    /// The dispatch branch that owned the decision.
    pub branch: Branch,
}

/// An entailment verdict plus the branch that produced it (traceability invariant).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntailmentOutcome {
    /// The tri-state verdict.
    pub verdict: EntailmentVerdict,
    /// The branch that owned the decision — an ATTRIBUTION, not a claim that a tableau
    /// ran: [`Branch::AlchTableau`] covers both executed refutations and pre-tableau
    /// abstentions (e.g. `UnencodedConclusion`, where the verdict is `Unknown` and no
    /// refutation was attempted); [`Branch::Extraction`] when an input graph was refused.
    /// [`Branch::RlMaterialization`] / [`Branch::ElClassification`] appear when the
    /// refutation budget fallback (module docs; [OPUS-5] sq-pbz04.4.10) decided a component
    /// the tableau had abandoned to its budget — a mixed-provenance verdict is attributed to
    /// the first profile branch that broke the tie, i.e. the branch WITHOUT which the outcome
    /// would have been an `Unknown(ResourceBudget)` abstention.
    pub branch: Branch,
}

// -------------------------------------------------------------------------------------------
// The checker
// -------------------------------------------------------------------------------------------

/// The L4 fragment-dispatch Direct-Semantics checker (module docs; design record §4).
///
/// Stateless apart from the deterministic tableau [`Budget`] used by the ALCH branch and
/// every refutation check. Construct with [`DirectChecker::new`] (default budget) or
/// [`DirectChecker::with_budget`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectChecker {
    budget: Budget,
}

impl Default for DirectChecker {
    fn default() -> DirectChecker {
        DirectChecker::new()
    }
}

impl DirectChecker {
    /// A checker with the default deterministic tableau budget.
    #[must_use]
    pub fn new() -> DirectChecker {
        DirectChecker {
            budget: Budget::default(),
        }
    }

    /// A checker with a caller-pinned tableau budget (conformance lanes pin their own for
    /// reproducibility).
    #[must_use]
    pub fn with_budget(budget: Budget) -> DirectChecker {
        DirectChecker { budget }
    }

    /// Decide Direct-Semantics consistency of the ontology encoded in `triples` by
    /// fragment dispatch (module docs table).
    ///
    /// `dict` is mutable because the RL branch materializes (interning vocabulary /
    /// derived terms); existing ids are never changed. The dispatch order is RL → EL →
    /// QL → ALCH; extraction failure short-circuits to
    /// [`UnknownReason::OutOfFragment`]. Every guard fails closed. An abstaining profile
    /// branch falls through to the ALCH tableau and keeps its own abstention if that adds
    /// nothing (module docs §"Guard-abstention tableau fall-through").
    #[must_use]
    pub fn consistency(&self, dict: &mut Dict, triples: &[[Id; 3]]) -> ConsistencyOutcome {
        let onto = match extract(dict, triples) {
            Ok(onto) => onto,
            Err(e) => {
                return ConsistencyOutcome {
                    verdict: ConsistencyVerdict::Unknown(UnknownReason::OutOfFragment(format!(
                        "{}",
                        e
                    ))),
                    branch: Branch::Extraction,
                }
            }
        };
        // [GPT-5.6] sq-zfwzq (opt-in `dl_transitive`): an ontology declaring a TRANSITIVE
        // role routes STRAIGHT to the ALCH+S tableau — the only branch whose soundness /
        // completeness argument covers transitivity (tableau module docs §5a). Although a
        // transitive ontology can be syntactically in-RL/in-EL, the RL rule set's documented
        // divergences implicate transitivity ("no clash" would not certify consistency) and
        // the EL classifier does not apply the axiom kind — both branches would abstain
        // (their guards below keep that fail-closed as defence in depth), so per the
        // record's "definitive verdicts only from a complete branch" rule the tableau owns
        // the verdict. With the feature OFF this arm does not exist (extraction refuses
        // `owl:TransitiveProperty` before dispatch).
        #[cfg(feature = "dl_transitive")]
        if onto
            .axioms()
            .iter()
            .any(|a| matches!(a, Axiom::TransitiveObjectProperty { .. }))
        {
            return ConsistencyOutcome {
                verdict: tableau_consistency(&onto, self.budget),
                branch: Branch::AlchTableau,
            };
        }
        let ps = profiles(&onto);
        let owner = if ps.rl.is_in() {
            Some(rl_branch(dict, triples, &onto))
        } else if ps.el.is_in() {
            Some(el_branch(dict, triples, &onto))
        } else if ps.ql.is_in() {
            // [FABLE-5] sq-fj8lj: with `dispatch_ql` the QL branch delegates to the
            // sparq-reason-ql DL-Lite_R checker over the RAW input triples (module docs
            // §"QL branch soundness" — the QL crate's OWN capture accounting owns the
            // verdict; L2's `In` only routes). Without the feature, deferred as before.
            #[cfg(feature = "dispatch_ql")]
            {
                Some(ql_branch(dict, triples))
            }
            #[cfg(not(feature = "dispatch_ql"))]
            {
                Some(ConsistencyOutcome {
                    verdict: ConsistencyVerdict::Unknown(UnknownReason::QlConsistencyPending),
                    branch: Branch::QlDeferred,
                })
            }
        } else {
            None
        };
        match owner {
            // The profile branch DECIDED: dispatch-in-order, branch-owned verdict (record §4).
            Some(outcome) if !outcome.verdict.is_unknown() => outcome,
            // The profile branch ABSTAINED: fall through to the tableau (record §4 amendment,
            // sq-pbz04.4.8 — module docs §"Guard-abstention tableau fall-through").
            Some(outcome) => tableau_fall_through(outcome, &onto, self.budget),
            // Everything the L1 mapping accepts is inside the ALCH fragment by construction.
            None => ConsistencyOutcome {
                verdict: tableau_consistency(&onto, self.budget),
                branch: Branch::AlchTableau,
            },
        }
    }

    /// Decide whether the premise ontology entails the conclusion ontology (both given as
    /// triples over the SAME `dict`), by per-axiom refutation on the ALCH tableau
    /// (module docs).
    ///
    /// `dict` is mutable only to mint the fresh `urn:` individual/class names the
    /// encodings need; existing ids are never changed. `Entailed` requires every
    /// conclusion axiom entailed; the first definitively-not-entailed axiom returns
    /// `NotEntailed`; otherwise any abstention returns `Unknown` (fail-closed). An empty
    /// conclusion is trivially `Entailed`.
    #[must_use]
    pub fn entailment(
        &self,
        dict: &mut Dict,
        premise: &[[Id; 3]],
        conclusion: &[[Id; 3]],
    ) -> EntailmentOutcome {
        let unknown_extraction = |e: String| EntailmentOutcome {
            verdict: EntailmentVerdict::Unknown(UnknownReason::OutOfFragment(e)),
            branch: Branch::Extraction,
        };
        let prem = match extract(dict, premise) {
            Ok(onto) => onto,
            Err(e) => return unknown_extraction(format!("premise: {}", e)),
        };
        // [GPT-5.6] sq-zfwzq: declarations have no Direct-Semantics import. Seed the
        // conclusion extraction with declarations for premise-confirmed roles so a bare
        // conclusion role assertion is classifiable without weakening L1 globally.
        // [SONNET-4.6] Role declarations are needed only by the transitive-role extension.
        let conclusion_for_extraction = conclusion.to_vec();
        #[cfg(feature = "dl_transitive")]
        let conclusion_for_extraction = {
            let mut conclusion_for_extraction = conclusion_for_extraction;
            let mut premise_roles = FxHashSet::default();
            if prem
                .axioms()
                .iter()
                .any(|axiom| matches!(axiom, Axiom::TransitiveObjectProperty { .. }))
            {
                collect_role_ids(&prem, &mut premise_roles);
            }
            let conclusion_predicates: FxHashSet<Id> =
                conclusion.iter().map(|triple| triple[1]).collect();
            if !premise_roles.is_empty() {
                let rdf_type = dict.intern(&OTerm::NamedNode(NamedNode::new_unchecked(
                    "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
                )));
                let object_property = dict.intern(&OTerm::NamedNode(NamedNode::new_unchecked(
                    "http://www.w3.org/2002/07/owl#ObjectProperty",
                )));
                for role in premise_roles {
                    if conclusion_predicates.contains(&role) {
                        conclusion_for_extraction.push([role, rdf_type, object_property]);
                    }
                }
            }
            conclusion_for_extraction
        };
        let concl = match extract(dict, &conclusion_for_extraction) {
            Ok(onto) => onto,
            Err(e) => return unknown_extraction(format!("conclusion: {}", e)),
        };
        // [OPUS-4.8] sq-pbz04.4.13 — conclusion anonymous individuals are EXISTENTIAL under
        // the official OWL 2 Direct-Semantics entailment reading. L1 maps a conclusion blank
        // node to an ordinary (skolem) constant; that reading is entailment-preserving on the
        // PREMISE side but UNSOUND on the conclusion side (it would certify a `NotEntailed`
        // that is wrong existentially — the pinned M2 divergences somevaluesfrom2bnode /
        // WebOnt-someValuesFrom-003). `roll_conclusion_anonymous` rolls a tree-shaped bnode
        // assertion set into an existential class assertion the tableau decides SOUNDLY (both
        // directions); a non-rollable shape (shared / cyclic / nominal / free-root bnode)
        // abstains fail-closed, never a skolem `NotEntailed`.
        let concl_axioms = match roll_conclusion_anonymous(dict, &concl) {
            Ok(axioms) => axioms,
            Err(reason) => {
                return EntailmentOutcome {
                    verdict: EntailmentVerdict::Unknown(reason),
                    branch: Branch::AlchTableau,
                }
            }
        };
        let mut fresh = FreshNames::new(dict, [&prem, &concl]);
        let mut first_unknown: Option<UnknownReason> = None;
        // [OPUS-5] sq-pbz04.4.10: the first PROFILE branch that decided a budget-exhausted
        // refutation, if any — the traceability attribution (see `EntailmentOutcome::branch`).
        let mut fallback_branch: Option<Branch> = None;
        for axiom in &concl_axioms {
            match self.axiom_entailed(&prem, axiom, &mut fresh) {
                AxiomVerdict::Entailed(via) => {
                    if let Some(branch) = via {
                        fallback_branch.get_or_insert(branch);
                    }
                }
                AxiomVerdict::NotEntailed(via) => {
                    // Sound early exit: the conjunction of conclusion axioms fails as soon
                    // as one conjunct definitively fails, regardless of abstentions.
                    return EntailmentOutcome {
                        verdict: EntailmentVerdict::NotEntailed,
                        branch: via.or(fallback_branch).unwrap_or(Branch::AlchTableau),
                    };
                }
                AxiomVerdict::Unknown(reason) => {
                    first_unknown.get_or_insert(reason);
                }
            }
        }
        let verdict = match first_unknown {
            Some(reason) => EntailmentVerdict::Unknown(reason),
            None => EntailmentVerdict::Entailed,
        };
        // An abstaining outcome is still the tableau's — the fallback only ever ATTACHES to a
        // component it definitively decided, and a decided component cannot be the abstention.
        let branch = match verdict {
            EntailmentVerdict::Entailed => fallback_branch.unwrap_or(Branch::AlchTableau),
            _ => Branch::AlchTableau,
        };
        EntailmentOutcome { verdict, branch }
    }

    /// One conclusion axiom: build its refutation check(s), decide each on the tableau,
    /// and aggregate (all-unsat = entailed; any-sat = not entailed; else unknown).
    fn axiom_entailed(
        &self,
        premise: &Ontology,
        conclusion: &Axiom,
        fresh: &mut FreshNames,
    ) -> AxiomVerdict {
        let Some(checks) = refutation_checks(conclusion, fresh) else {
            return AxiomVerdict::Unknown(UnknownReason::UnencodedConclusion(format!(
                "no argued refutation encoding for the conclusion-axiom kind of {:?}",
                conclusion
            )));
        };
        let mut first_unknown: Option<UnknownReason> = None;
        let mut fallback_branch: Option<Branch> = None;
        for additions in checks {
            let mut augmented = premise.clone();
            augmented.axioms.extend(additions);
            match tableau::consistency(&augmented, self.budget) {
                tableau::Verdict::Unsatisfiable => {} // this component is entailed
                tableau::Verdict::Satisfiable => return AxiomVerdict::NotEntailed(fallback_branch),
                tableau::Verdict::Unknown(tableau::UnknownReason::ResourceBudget(b)) => {
                    // [OPUS-5] sq-pbz04.4.10 — budget robustness: the tableau ran out on THIS
                    // refutation, so re-ask the profile branches under their own guards
                    // (module docs §"Refutation budget fallback"). Any abstention there keeps
                    // the ORIGINAL `ResourceBudget` reason: the tableau is what ran out.
                    match refutation_profile_fallback(fresh.dict_mut(), &augmented) {
                        Some(outcome) if outcome.verdict.is_inconsistent() => {
                            // Refutation unsatisfiable ⇒ this component IS entailed.
                            fallback_branch.get_or_insert(outcome.branch);
                        }
                        Some(outcome) if outcome.verdict.is_consistent() => {
                            // Refutation satisfiable ⇒ the axiom is NOT entailed.
                            return AxiomVerdict::NotEntailed(Some(outcome.branch));
                        }
                        // `None` (no branch owned the question) and — structurally unreachable,
                        // since the fallback filters them out — any non-definitive verdict both
                        // land here and keep the tableau's honest `ResourceBudget` reason.
                        // Written as a catch-all so the fail-closed default cannot be lost to a
                        // future refactor of the fallback's return contract.
                        _ => {
                            first_unknown.get_or_insert(UnknownReason::ResourceBudget(b));
                        }
                    }
                }
                tableau::Verdict::Unknown(tableau::UnknownReason::OutOfFragment(m)) => {
                    // Unreachable in practice (both inputs already extracted, and every
                    // encoding stays inside ALCH) — kept fail-closed regardless.
                    first_unknown.get_or_insert(UnknownReason::OutOfFragment(m));
                }
            }
        }
        match first_unknown {
            Some(reason) => AxiomVerdict::Unknown(reason),
            None => AxiomVerdict::Entailed(fallback_branch),
        }
    }
}

/// Per-conclusion-axiom tri-state (internal aggregation of one axiom's refutation checks).
///
/// The definitive arms carry the PROFILE branch that decided a budget-exhausted refutation
/// component, if any ([OPUS-5] sq-pbz04.4.10); `None` means every component of this axiom was
/// decided by the tableau itself.
enum AxiomVerdict {
    Entailed(Option<Branch>),
    NotEntailed(Option<Branch>),
    Unknown(UnknownReason),
}

/// Re-ask a BUDGET-EXHAUSTED refutation of the profile branches (module docs
/// §"Refutation budget fallback"; [OPUS-5] sq-pbz04.4.10).
///
/// Returns `Some` only for a DEFINITIVE verdict produced past the branch's own guards — the
/// identical guard discipline [`DirectChecker::consistency`] applies, because these are the
/// identical branch functions. `None` means "no fallback verdict"; the caller then keeps the
/// tableau's honest [`UnknownReason::ResourceBudget`] abstention. Every `None` arm below is a
/// fail-closed refusal, never a guess.
fn refutation_profile_fallback(
    dict: &mut Dict,
    augmented: &Ontology,
) -> Option<ConsistencyOutcome> {
    // Transitivity: only the ALCH+S tableau's argument covers it, so no profile branch may own
    // the question — the same ownership rule `consistency` applies before its profile dispatch.
    // (The RL divergence guard and the EL unapplied-kind guard would also abstain; this arm is
    // the explicit ownership statement, not a reliance on them.)
    #[cfg(feature = "dl_transitive")]
    if augmented
        .axioms()
        .iter()
        .any(|axiom| matches!(axiom, Axiom::TransitiveObjectProperty { .. }))
    {
        return None;
    }
    let ps = profiles(augmented);
    // Same RL → EL order as the consistency dispatch. QL is deliberately NOT routed: its
    // branch's soundness rests on the sparq-reason-ql crate's own capture accounting over the
    // ORIGINAL input graph, and a refutation graph is a synthesised augmentation, not the
    // user's input — so extending the QL arm needs its own argument, not this one.
    let rl = ps.rl.is_in();
    let el = !rl && ps.el.is_in();
    if !rl && !el {
        return None;
    }
    // Both branches consume TRIPLES, so serialise the augmented model with the L1 forward
    // renderer — and VERIFY its round-trip invariant here rather than trusting it: re-extract
    // the rendering and require the same axiom multiset. A rendering gap then costs a fallback
    // (we return `None` and keep the budget abstention); it can never feed a branch a graph
    // that says something other than the refutation we meant to ask about.
    let rendered = render_to_triples(augmented, dict);
    let round_trip = extract(dict, &rendered).ok()?;
    if !same_axiom_multiset(&round_trip, augmented) {
        return None;
    }
    let outcome = if rl {
        rl_branch(dict, &rendered, augmented)
    } else {
        el_branch(dict, &rendered, augmented)
    };
    if outcome.verdict.is_unknown() {
        None
    } else {
        Some(outcome)
    }
}

/// Order-insensitive axiom-multiset equality — the render round-trip preserves the structural
/// model but not the axiom ORDER (`render_to_triples` emits the property declarations first),
/// and the fallback's faithfulness obligation is exactly "the same axioms", nothing stronger.
/// Compares `Debug` renderings because [`Axiom`] is `PartialEq` but neither `Ord` nor `Hash`;
/// the rendering is injective over the model (every field is printed).
fn same_axiom_multiset(left: &Ontology, right: &Ontology) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut l: Vec<String> = left.axioms().iter().map(|a| format!("{:?}", a)).collect();
    let mut r: Vec<String> = right.axioms().iter().map(|a| format!("{:?}", a)).collect();
    l.sort_unstable();
    r.sort_unstable();
    l == r
}

/// Collect every named object property whose role kind is established by a fully-extracted
/// premise model. Used only to add semantically inert declarations to conclusion extraction.
#[cfg(feature = "dl_transitive")]
fn collect_role_ids(onto: &Ontology, into: &mut FxHashSet<Id>) {
    for axiom in onto.axioms() {
        match axiom {
            Axiom::SubClassOf { sub, sup } => {
                collect_role_ids_in_ce(sub, into);
                collect_role_ids_in_ce(sup, into);
            }
            Axiom::EquivalentClasses(left, right) | Axiom::DisjointClasses(left, right) => {
                collect_role_ids_in_ce(left, into);
                collect_role_ids_in_ce(right, into);
            }
            Axiom::SubObjectPropertyOf { sub, sup } => {
                into.extend([sub.named(), sup.named()]);
            }
            Axiom::ObjectPropertyDomain { property, domain } => {
                into.insert(property.named());
                collect_role_ids_in_ce(domain, into);
            }
            Axiom::ObjectPropertyRange { property, range } => {
                into.insert(property.named());
                collect_role_ids_in_ce(range, into);
            }
            Axiom::ClassAssertion { class, .. } => collect_role_ids_in_ce(class, into),
            Axiom::ObjectPropertyAssertion { property, .. } => {
                into.insert(property.named());
            }
            #[cfg(feature = "dl_transitive")]
            Axiom::TransitiveObjectProperty { property } => {
                into.insert(property.named());
            }
        }
    }
}

#[cfg(feature = "dl_transitive")]
fn collect_role_ids_in_ce(class: &ClassExpression, into: &mut FxHashSet<Id>) {
    match class {
        ClassExpression::ObjectIntersectionOf(members)
        | ClassExpression::ObjectUnionOf(members) => {
            for member in members {
                collect_role_ids_in_ce(member, into);
            }
        }
        ClassExpression::ObjectComplementOf(inner) => collect_role_ids_in_ce(inner, into),
        ClassExpression::ObjectSomeValuesFrom(property, filler)
        | ClassExpression::ObjectAllValuesFrom(property, filler) => {
            into.insert(property.named());
            collect_role_ids_in_ce(filler, into);
        }
        ClassExpression::Class(_) | ClassExpression::Thing | ClassExpression::Nothing => {}
    }
}

// -------------------------------------------------------------------------------------------
// RL branch
// -------------------------------------------------------------------------------------------

/// The RL branch (module docs §"RL branch soundness").
fn rl_branch(dict: &mut Dict, triples: &[[Id; 3]], onto: &Ontology) -> ConsistencyOutcome {
    let out = |verdict| ConsistencyOutcome {
        verdict,
        branch: Branch::RlMaterialization,
    };
    if let Some(msg) = pr1_punning_violation(onto) {
        return out(ConsistencyVerdict::Unknown(
            UnknownReason::RlPr1Preconditions(msg),
        ));
    }
    let mut work = triples.to_vec();
    materialize(Profile::OwlRl, dict, &mut work);
    let clashes = inconsistencies(dict, &work);
    if !clashes.is_empty() {
        return out(ConsistencyVerdict::Inconsistent);
    }
    if let Some(construct) = rl_divergence_guard(onto) {
        return out(ConsistencyVerdict::Unknown(
            UnknownReason::RlDivergenceGuard(construct),
        ));
    }
    out(ConsistencyVerdict::Consistent)
}

/// Usage-level punning scan for the PR1 precondition: no id may be used as more than one
/// of {class, object property, individual} across the model. Returns a diagnostic naming
/// the first overlap.
fn pr1_punning_violation(onto: &Ontology) -> Option<String> {
    let mut classes: FxHashSet<Id> = FxHashSet::default();
    let mut properties: FxHashSet<Id> = FxHashSet::default();
    let mut individuals: FxHashSet<Id> = FxHashSet::default();
    let mut ces: Vec<&ClassExpression> = Vec::new();
    for axiom in onto.axioms() {
        ces.extend(axiom_ces(axiom));
        match axiom {
            Axiom::SubObjectPropertyOf { sub, sup } => {
                properties.extend([sub.named(), sup.named()]);
            }
            Axiom::ObjectPropertyDomain { property, .. }
            | Axiom::ObjectPropertyRange { property, .. } => {
                properties.insert(property.named());
            }
            Axiom::ClassAssertion { individual, .. } => {
                individuals.insert(*individual);
            }
            Axiom::ObjectPropertyAssertion {
                property,
                source,
                target,
            } => {
                properties.insert(property.named());
                individuals.extend([*source, *target]);
            }
            // [GPT-5.6] sq-zfwzq: a transitivity declaration uses its subject as a property.
            #[cfg(feature = "dl_transitive")]
            Axiom::TransitiveObjectProperty { property } => {
                properties.insert(property.named());
            }
            Axiom::SubClassOf { .. }
            | Axiom::EquivalentClasses(_, _)
            | Axiom::DisjointClasses(_, _) => {}
        }
    }
    while let Some(ce) = ces.pop() {
        match ce {
            ClassExpression::Class(id) => {
                classes.insert(*id);
            }
            ClassExpression::Thing | ClassExpression::Nothing => {}
            ClassExpression::ObjectIntersectionOf(members)
            | ClassExpression::ObjectUnionOf(members) => ces.extend(members),
            ClassExpression::ObjectComplementOf(inner) => ces.push(inner),
            ClassExpression::ObjectSomeValuesFrom(p, filler)
            | ClassExpression::ObjectAllValuesFrom(p, filler) => {
                properties.insert(p.named());
                ces.push(filler);
            }
        }
    }
    let overlap = |a: &FxHashSet<Id>, b: &FxHashSet<Id>| a.intersection(b).next().copied();
    if let Some(id) = overlap(&classes, &individuals) {
        return Some(format!("id {} is used as both class and individual", id));
    }
    if let Some(id) = overlap(&classes, &properties) {
        return Some(format!("id {} is used as both class and property", id));
    }
    if let Some(id) = overlap(&properties, &individuals) {
        return Some(format!("id {} is used as both property and individual", id));
    }
    None
}

/// The RL divergence guard (module docs §"RL branch soundness"): the first implicated
/// construct the model touches, or `None` when "no clash" may soundly read as consistent.
fn rl_divergence_guard(onto: &Ontology) -> Option<String> {
    for axiom in onto.axioms() {
        // [GPT-5.6] sq-zfwzq: the documented RL divergences implicate transitivity, so "no
        // clash" must not certify consistency here. Defence in depth only — dispatch routes
        // transitive ontologies to the ALCH+S tableau BEFORE this branch can match.
        #[cfg(feature = "dl_transitive")]
        if matches!(axiom, Axiom::TransitiveObjectProperty { .. }) {
            return Some(
                "owl:TransitiveProperty (implicated by the documented RL rule-set \
                 divergences on property chains/transitivity)"
                    .to_string(),
            );
        }
        if matches!(axiom, Axiom::DisjointClasses(_, _)) {
            return Some(
                "owl:disjointWith (implicated by DOCUMENTED_DIVERGENCES \
                 DisjointClasses-001/-003)"
                    .to_string(),
            );
        }
        if let Some(construct) = axiom_ces(axiom).into_iter().find_map(implicated_construct) {
            return Some(construct);
        }
    }
    None
}

/// The class expressions carried by an axiom (none for `SubObjectPropertyOf` /
/// `ObjectPropertyAssertion`).
fn axiom_ces(axiom: &Axiom) -> Vec<&ClassExpression> {
    match axiom {
        Axiom::SubClassOf { sub, sup } => vec![sub, sup],
        Axiom::EquivalentClasses(l, r) | Axiom::DisjointClasses(l, r) => vec![l, r],
        Axiom::ObjectPropertyDomain { domain, .. } => vec![domain],
        Axiom::ObjectPropertyRange { range, .. } => vec![range],
        Axiom::ClassAssertion { class, .. } => vec![class],
        Axiom::SubObjectPropertyOf { .. } | Axiom::ObjectPropertyAssertion { .. } => Vec::new(),
        #[cfg(feature = "dl_transitive")]
        Axiom::TransitiveObjectProperty { .. } => Vec::new(),
    }
}

/// First divergence-implicated construct inside a class expression, if any.
fn implicated_construct(ce: &ClassExpression) -> Option<String> {
    match ce {
        ClassExpression::Class(_) | ClassExpression::Thing | ClassExpression::Nothing => None,
        ClassExpression::ObjectComplementOf(_) => Some(
            "owl:complementOf (implicated by DOCUMENTED_DIVERGENCES DisjointClasses-001/-003, \
             New-Feature-ObjectQCR-002)"
                .to_string(),
        ),
        ClassExpression::ObjectUnionOf(_) => Some(
            "owl:unionOf (implicated by DOCUMENTED_DIVERGENCES WebOnt-I5.5-005)".to_string(),
        ),
        ClassExpression::ObjectIntersectionOf(members) => {
            members.iter().find_map(implicated_construct)
        }
        ClassExpression::ObjectSomeValuesFrom(_, filler)
        | ClassExpression::ObjectAllValuesFrom(_, filler) => implicated_construct(filler),
    }
}

// -------------------------------------------------------------------------------------------
// EL branch
// -------------------------------------------------------------------------------------------

/// The EL branch (module docs §"EL branch soundness").
fn el_branch(dict: &Dict, triples: &[[Id; 3]], onto: &Ontology) -> ConsistencyOutcome {
    let out = |verdict| ConsistencyOutcome {
        verdict,
        branch: Branch::ElClassification,
    };
    let report = Classifier::classify(dict, triples).report();
    if report.skipped_axioms > 0 {
        return out(ConsistencyVerdict::Unknown(UnknownReason::ElSkippedAxioms(
            report.skipped_axioms,
        )));
    }
    if let Some(kind) = el_unapplied_kind(onto) {
        return out(ConsistencyVerdict::Unknown(
            UnknownReason::ElUnappliedAxioms(kind),
        ));
    }
    if onto
        .axioms()
        .iter()
        .any(|axiom| axiom_ces(axiom).into_iter().any(ce_mentions_thing))
    {
        return out(ConsistencyVerdict::Unknown(UnknownReason::ElTopGuard));
    }
    // Pure ⊤-free EL+⊥ TBox: consistent by the empty-interpretation model construction
    // (module docs). The classifier run above contributed the skipped-axioms honesty gate.
    out(ConsistencyVerdict::Consistent)
}

/// The first axiom kind the EL classifier neither applies nor counts, if any.
fn el_unapplied_kind(onto: &Ontology) -> Option<String> {
    onto.axioms().iter().find_map(|axiom| match axiom {
        Axiom::SubClassOf { .. }
        | Axiom::EquivalentClasses(_, _)
        | Axiom::DisjointClasses(_, _) => None,
        Axiom::SubObjectPropertyOf { .. } => Some("SubObjectPropertyOf".to_string()),
        Axiom::ObjectPropertyDomain { .. } => Some("ObjectPropertyDomain".to_string()),
        Axiom::ObjectPropertyRange { .. } => Some("ObjectPropertyRange".to_string()),
        Axiom::ClassAssertion { .. } => Some("ClassAssertion".to_string()),
        Axiom::ObjectPropertyAssertion { .. } => Some("ObjectPropertyAssertion".to_string()),
        // [GPT-5.6] sq-zfwzq: the EL classifier does not apply transitivity — abstain
        // (defence in depth; dispatch routes transitive ontologies to the tableau first).
        #[cfg(feature = "dl_transitive")]
        Axiom::TransitiveObjectProperty { .. } => Some("TransitiveObjectProperty".to_string()),
    })
}

/// Whether a class expression mentions `owl:Thing` anywhere.
fn ce_mentions_thing(ce: &ClassExpression) -> bool {
    match ce {
        ClassExpression::Thing => true,
        ClassExpression::Class(_) | ClassExpression::Nothing => false,
        ClassExpression::ObjectIntersectionOf(members)
        | ClassExpression::ObjectUnionOf(members) => members.iter().any(ce_mentions_thing),
        ClassExpression::ObjectComplementOf(inner) => ce_mentions_thing(inner),
        ClassExpression::ObjectSomeValuesFrom(_, filler)
        | ClassExpression::ObjectAllValuesFrom(_, filler) => ce_mentions_thing(filler),
    }
}

// -------------------------------------------------------------------------------------------
// QL branch ([FABLE-5] sq-fj8lj, opt-in `dispatch_ql`)
// -------------------------------------------------------------------------------------------

/// The QL branch (module docs §"QL branch soundness"): reconstruct the raw input triples and
/// delegate to the sparq-reason-ql DL-Lite_R checker, whose OWN extraction accounting owns
/// the verdict (`check_consistency` = `TBox::extract` over the same kb + violation sweep —
/// exactly the pairing its `check_consistency_with` contract requires).
#[cfg(feature = "dispatch_ql")]
fn ql_branch(dict: &Dict, triples: &[[Id; 3]]) -> ConsistencyOutcome {
    use sparq_reason_ql::QlConsistency;
    let out = |verdict| ConsistencyOutcome {
        verdict,
        branch: Branch::QlConsistency,
    };
    let kb = match ql_kb(dict, triples) {
        Ok(kb) => kb,
        // Unreachable in practice (these triples already extracted through L1, so subjects/
        // predicates are IRIs or blank nodes) — kept fail-closed regardless.
        Err(msg) => return out(ConsistencyVerdict::Unknown(UnknownReason::QlCaptureGap(msg))),
    };
    match sparq_reason_ql::check_consistency(&kb) {
        QlConsistency::Consistent => out(ConsistencyVerdict::Consistent),
        QlConsistency::Inconsistent(_) => out(ConsistencyVerdict::Inconsistent),
        QlConsistency::Unknown(gap) => out(ConsistencyVerdict::Unknown(
            UnknownReason::QlCaptureGap(format!("{:?}", gap)),
        )),
    }
}

/// Faithfully render the id triples back into oxrdf triples — the EXACT graph L1 extracted,
/// not a translation of the L1 model — so the QL crate's accounting sees the same input.
#[cfg(feature = "dispatch_ql")]
fn ql_kb(dict: &Dict, triples: &[[Id; 3]]) -> Result<Vec<oxrdf::Triple>, String> {
    use oxrdf::NamedOrBlankNode;
    let mut kb = Vec::with_capacity(triples.len());
    for t in triples {
        let subject = match dict.term(t[0]) {
            OTerm::NamedNode(n) => NamedOrBlankNode::NamedNode(n),
            OTerm::BlankNode(b) => NamedOrBlankNode::BlankNode(b),
            other => return Err(format!("subject {} is not an IRI or blank node", other)),
        };
        let predicate = match dict.term(t[1]) {
            OTerm::NamedNode(n) => n,
            other => return Err(format!("predicate {} is not an IRI", other)),
        };
        kb.push(oxrdf::Triple::new(subject, predicate, dict.term(t[2])));
    }
    Ok(kb)
}

// -------------------------------------------------------------------------------------------
// ALCH branch
// -------------------------------------------------------------------------------------------

/// Guard-abstention fall-through ([SONNET-4.6] sq-pbz04.4.8; module docs
/// §"Guard-abstention tableau fall-through", design record §4 as amended).
///
/// `owner` is the abstaining outcome of the profile branch the in-order dispatch selected.
/// Re-ask the SAME ontology of the ALCH tableau — the one branch complete for the whole L1
/// fragment — and prefer its definitive verdict; keep the owner's abstention otherwise.
/// Strictly abstention-reducing: it runs only after a branch has already abstained, and it
/// can only replace an `Unknown` with a verdict the tableau's own completeness argument
/// already covers.
fn tableau_fall_through(
    owner: ConsistencyOutcome,
    onto: &Ontology,
    budget: Budget,
) -> ConsistencyOutcome {
    // The PR1 punning abstention is NOT an incompleteness guard: it says the input is
    // ill-posed as an OWL 2 DL ontology (an id used as class AND individual AND/OR
    // property), so the L1 shadow the tableau would decide is one arbitrary reading of it.
    // Fail closed — this abstention keeps its ownership (module docs).
    if matches!(
        owner.verdict,
        ConsistencyVerdict::Unknown(UnknownReason::RlPr1Preconditions(_))
    ) {
        return owner;
    }
    let verdict = tableau_consistency(onto, budget);
    if verdict.is_unknown() {
        // The tableau added nothing (budget exhaustion): the honest attribution is the
        // branch that OWNED the dispatch and the guard it tripped, not the tableau's.
        owner
    } else {
        ConsistencyOutcome {
            verdict,
            branch: Branch::AlchTableau,
        }
    }
}

/// Map a tableau verdict into the dispatch verdict (the ALCH branch is complete for the
/// whole L1 fragment, so both definitive verdicts pass through).
fn tableau_consistency(onto: &Ontology, budget: Budget) -> ConsistencyVerdict {
    match tableau::consistency(onto, budget) {
        tableau::Verdict::Satisfiable => ConsistencyVerdict::Consistent,
        tableau::Verdict::Unsatisfiable => ConsistencyVerdict::Inconsistent,
        tableau::Verdict::Unknown(tableau::UnknownReason::ResourceBudget(b)) => {
            ConsistencyVerdict::Unknown(UnknownReason::ResourceBudget(b))
        }
        tableau::Verdict::Unknown(tableau::UnknownReason::OutOfFragment(m)) => {
            // Unreachable in practice: the ontology was already extracted. Fail-closed.
            ConsistencyVerdict::Unknown(UnknownReason::OutOfFragment(m))
        }
    }
}

// -------------------------------------------------------------------------------------------
// Refutation encodings
// -------------------------------------------------------------------------------------------

/// Fresh-name minting for the refutation encodings: `urn:` IRIs interned into the dict and
/// checked against every id occurring in the premise/conclusion models, so the freshness
/// premise of the module-doc arguments holds by construction.
struct FreshNames<'d> {
    dict: &'d mut Dict,
    used: FxHashSet<Id>,
    counter: usize,
}

impl<'d> FreshNames<'d> {
    fn new(dict: &'d mut Dict, ontologies: [&Ontology; 2]) -> FreshNames<'d> {
        let mut used = FxHashSet::default();
        for onto in ontologies {
            collect_ids(onto, &mut used);
        }
        FreshNames {
            dict,
            used,
            counter: 0,
        }
    }

    /// The dict the fresh names are interned into, reborrowed for the refutation budget
    /// fallback (which must render the augmented model back to triples). [OPUS-5] sq-pbz04.4.10
    fn dict_mut(&mut self) -> &mut Dict {
        self.dict
    }

    /// Mint an id occurring in neither input model.
    fn mint(&mut self) -> Id {
        loop {
            let iri = format!("urn:sparq-reason-dl:check:fresh:{}", self.counter);
            self.counter += 1;
            let id = self.dict.intern_iri(&iri);
            if self.used.insert(id) {
                return id;
            }
        }
    }
}

/// Every id (class, property, individual) occurring in the model.
fn collect_ids(onto: &Ontology, into: &mut FxHashSet<Id>) {
    let mut ces: Vec<&ClassExpression> = Vec::new();
    for axiom in onto.axioms() {
        ces.extend(axiom_ces(axiom));
        match axiom {
            Axiom::SubObjectPropertyOf { sub, sup } => {
                into.extend([sub.named(), sup.named()]);
            }
            Axiom::ObjectPropertyDomain { property, .. }
            | Axiom::ObjectPropertyRange { property, .. } => {
                into.insert(property.named());
            }
            Axiom::ClassAssertion { individual, .. } => {
                into.insert(*individual);
            }
            Axiom::ObjectPropertyAssertion {
                property,
                source,
                target,
            } => {
                into.extend([property.named(), *source, *target]);
            }
            #[cfg(feature = "dl_transitive")]
            Axiom::TransitiveObjectProperty { property } => {
                into.insert(property.named());
            }
            Axiom::SubClassOf { .. }
            | Axiom::EquivalentClasses(_, _)
            | Axiom::DisjointClasses(_, _) => {}
        }
    }
    while let Some(ce) = ces.pop() {
        match ce {
            ClassExpression::Class(id) => {
                into.insert(*id);
            }
            ClassExpression::Thing | ClassExpression::Nothing => {}
            ClassExpression::ObjectIntersectionOf(members)
            | ClassExpression::ObjectUnionOf(members) => ces.extend(members),
            ClassExpression::ObjectComplementOf(inner) => ces.push(inner),
            ClassExpression::ObjectSomeValuesFrom(p, filler)
            | ClassExpression::ObjectAllValuesFrom(p, filler) => {
                into.insert(p.named());
                ces.push(filler);
            }
        }
    }
}

/// The refutation check(s) for one conclusion axiom (module docs): each inner `Vec<Axiom>`
/// is one set of additions whose UNSATISFIABILITY (together with the premise) certifies
/// one component of the axiom; the axiom is entailed iff EVERY component's union is
/// unsatisfiable. `None` = no argued encoding for this axiom kind.
fn refutation_checks(conclusion: &Axiom, fresh: &mut FreshNames) -> Option<Vec<Vec<Axiom>>> {
    let gci = |sub: &ClassExpression, sup: &ClassExpression, fresh: &mut FreshNames| {
        vec![Axiom::ClassAssertion {
            class: ClassExpression::ObjectIntersectionOf(vec![
                sub.clone(),
                ClassExpression::ObjectComplementOf(Box::new(sup.clone())),
            ]),
            individual: fresh.mint(),
        }]
    };
    match conclusion {
        Axiom::SubClassOf { sub, sup } => Some(vec![gci(sub, sup, fresh)]),
        Axiom::EquivalentClasses(l, r) => Some(vec![gci(l, r, fresh), gci(r, l, fresh)]),
        // DisjointClasses(C, D) ≡ C ⊓ D ⊑ ⊥ (record §3): refute with a fresh instance of
        // the intersection.
        Axiom::DisjointClasses(l, r) => Some(vec![vec![Axiom::ClassAssertion {
            class: ClassExpression::ObjectIntersectionOf(vec![l.clone(), r.clone()]),
            individual: fresh.mint(),
        }]]),
        // Domain/Range desugar to GCIs (record §3): ∃R.⊤ ⊑ C and ⊤ ⊑ ∀R.C.
        Axiom::ObjectPropertyDomain { property, domain } => {
            let some_top = ClassExpression::some(property.named(), ClassExpression::Thing);
            Some(vec![gci(&some_top, domain, fresh)])
        }
        Axiom::ObjectPropertyRange { property, range } => {
            let all_range = ClassExpression::only(property.named(), range.clone());
            Some(vec![gci(&ClassExpression::Thing, &all_range, fresh)])
        }
        Axiom::ClassAssertion { class, individual } => Some(vec![vec![Axiom::ClassAssertion {
            class: ClassExpression::ObjectComplementOf(Box::new(class.clone())),
            individual: *individual,
        }]]),
        // The fresh-class trick (module docs; sound AND complete).
        Axiom::ObjectPropertyAssertion {
            property,
            source,
            target,
        } => {
            let b = fresh.mint();
            Some(vec![vec![
                Axiom::ClassAssertion {
                    class: ClassExpression::Class(b),
                    individual: *target,
                },
                Axiom::ClassAssertion {
                    class: ClassExpression::only(
                        property.named(),
                        ClassExpression::ObjectComplementOf(Box::new(ClassExpression::Class(b))),
                    ),
                    individual: *source,
                },
            ]])
        }
        // The fresh-individual-pair role-subsumption trick (module docs; record §4 as
        // amended by sq-pbz04.4.9 — sound AND complete). [FABLE-5]
        Axiom::SubObjectPropertyOf { sub, sup } => {
            let a = fresh.mint();
            let b = fresh.mint();
            let witness = fresh.mint();
            Some(vec![vec![
                Axiom::ObjectPropertyAssertion {
                    property: sub.clone(),
                    source: a,
                    target: b,
                },
                Axiom::ClassAssertion {
                    class: ClassExpression::Class(witness),
                    individual: b,
                },
                Axiom::ClassAssertion {
                    class: ClassExpression::only(
                        sup.named(),
                        ClassExpression::ObjectComplementOf(Box::new(ClassExpression::Class(
                            witness,
                        ))),
                    ),
                    individual: a,
                },
            ]])
        }
        // The two-step-chain lift of the fresh-class trick for a TRANSITIVITY conclusion
        // (module docs; [GPT-5.6] sq-zfwzq — sound AND complete): `O ⊨ Trans(R)` iff
        // `O ∪ {R(a,b), R(b,c), B(c), (∀R.¬B)(a)}` is unsatisfiable, with `a`/`b`/`c`/`B`
        // fresh. A non-transitive witness chain `(u,v), (v,w) ∈ Rᴵ, (u,w) ∉ Rᴵ` in some
        // model of `O` is exactly what keeps the union satisfiable (`B = {w}`).
        #[cfg(feature = "dl_transitive")]
        Axiom::TransitiveObjectProperty { property } => {
            let a = fresh.mint();
            let b = fresh.mint();
            let c = fresh.mint();
            let witness = fresh.mint();
            Some(vec![vec![
                Axiom::ObjectPropertyAssertion {
                    property: property.clone(),
                    source: a,
                    target: b,
                },
                Axiom::ObjectPropertyAssertion {
                    property: property.clone(),
                    source: b,
                    target: c,
                },
                Axiom::ClassAssertion {
                    class: ClassExpression::Class(witness),
                    individual: c,
                },
                Axiom::ClassAssertion {
                    class: ClassExpression::only(
                        property.named(),
                        ClassExpression::ObjectComplementOf(Box::new(ClassExpression::Class(
                            witness,
                        ))),
                    ),
                    individual: a,
                },
            ]])
        }
    }
}

// -------------------------------------------------------------------------------------------
// Conclusion anonymous-individual rolling ([OPUS-4.8] sq-pbz04.4.13)
// -------------------------------------------------------------------------------------------
//
// Official OWL 2 Direct-Semantics entailment reads a blank-node individual in the CONCLUSION
// ontology EXISTENTIALLY (an "existential variable" — the somevaluesfrom2bnode test's own
// description). L1 maps every blank node to an ordinary dict id, i.e. a (skolem) CONSTANT. On
// the PREMISE side skolemisation is entailment-preserving and stays; on the CONCLUSION side it
// is NOT — a `NotEntailed` certificate from the skolem reading is UNSOUND, because the premise
// may entail the conclusion existentially (a p _:x) without entailing it for any specific
// constant. So a conclusion blank-node assertion set must be resolved BEFORE the refutation
// loop: rolled up into an existential class assertion when it is tree-shaped, or refused
// fail-closed otherwise (never a skolem `NotEntailed`).
//
// ROLLING-UP is the standard tree-shaped-query technique: an anonymous individual reached by a
// single property edge `s p _:x`, whose asserted types are `C₁ … Cₙ` and whose own outgoing
// edges are `_:x qⱼ _:yⱼ`, denotes exactly `s : ∃p.(C₁ ⊓ … ⊓ Cₙ ⊓ ∃q₁.⟨roll _:y₁⟩ ⊓ …)`. This
// is SOUND and COMPLETE for the tree shape (no inverse roles, no shared variables, no nominals
// — all guaranteed by the rollability checks + the ALCH fragment), so a `NotEntailed` from a
// ROLLED existential class assertion is decided by the complete tableau and is genuinely
// sound, and the graduated cases (somevaluesfrom2bnode / WebOnt-someValuesFrom-003) become
// real passes rather than abstentions.

/// Whether `id` is an anonymous (blank-node) individual in `dict`.
fn is_anonymous_individual(dict: &Dict, id: Id) -> bool {
    !is_inline(id) && matches!(dict.term_parts(id), TermParts::Blank(_))
}

/// Render an id for a fail-closed diagnostic (positional-arg friendly — CodeQL note).
fn term_of(dict: &Dict, id: Id) -> String {
    if is_inline(id) {
        return format!("(inline literal id {})", id);
    }
    match dict.term_parts(id) {
        TermParts::Iri { prefix, suffix } => format!("<{}{}>", prefix, suffix),
        TermParts::Lit { value, .. } => format!("\"{}\"", value),
        TermParts::Blank(label) => format!("_:{}", label),
        TermParts::Triple(_) => "<<triple term>>".to_string(),
    }
}

/// Resolve the CONCLUSION ontology's anonymous (blank-node) individuals under the official
/// EXISTENTIAL reading (module note above; sq-pbz04.4.13).
///
/// Returns the blank-node-FREE conclusion axiom list the caller decides by refutation:
/// - if the conclusion mentions no blank-node individual, its axioms unchanged (clone);
/// - otherwise, the blank-node ABox assertions ROLLED UP into existential class assertions on
///   their named roots, with every blank-node-free conclusion axiom kept verbatim.
///
/// # Errors
/// [`UnknownReason::ConclusionAnonymousIndividual`] when a blank-node individual is NOT in a
/// tree-shaped roll-up: it must be the object of EXACTLY ONE property assertion (its unique
/// parent edge), reach only blank-node successors (a named/literal successor would need a
/// nominal, out of the ALCH fragment), and be acyclic / anchored to a named root. Fail-closed
/// — the caller abstains, never a skolem-constant `NotEntailed`.
fn roll_conclusion_anonymous(dict: &Dict, concl: &Ontology) -> Result<Vec<Axiom>, UnknownReason> {
    // Fast path: no anonymous individual anywhere → decide the conclusion verbatim, so the
    // blank-node-free path (every existing entailment case) is byte-identical to before.
    let mentions_bnode = concl.axioms().iter().any(|axiom| match axiom {
        Axiom::ClassAssertion { individual, .. } => is_anonymous_individual(dict, *individual),
        Axiom::ObjectPropertyAssertion { source, target, .. } => {
            is_anonymous_individual(dict, *source) || is_anonymous_individual(dict, *target)
        }
        _ => false,
    });
    if !mentions_bnode {
        return Ok(concl.axioms().to_vec());
    }

    // Build the roll-up structure over the ABox. `in_degree` counts parent property edges per
    // blank individual; `types` collects each blank individual's asserted class types;
    // `outgoing` its child property edges; `top_edges` the named→blank edges that ANCHOR a
    // tree; `clean` keeps every blank-node-free axiom verbatim.
    let mut in_degree: FxHashMap<Id, usize> = FxHashMap::default();
    let mut types: FxHashMap<Id, Vec<ClassExpression>> = FxHashMap::default();
    let mut outgoing: FxHashMap<Id, Vec<(Id, Id)>> = FxHashMap::default();
    let mut top_edges: Vec<(Id, Id, Id)> = Vec::new();
    let mut clean: Vec<Axiom> = Vec::new();

    for axiom in concl.axioms() {
        match axiom {
            Axiom::ClassAssertion { class, individual } => {
                if is_anonymous_individual(dict, *individual) {
                    in_degree.entry(*individual).or_insert(0);
                    types.entry(*individual).or_default().push(class.clone());
                } else {
                    clean.push(axiom.clone());
                }
            }
            Axiom::ObjectPropertyAssertion {
                property,
                source,
                target,
            } => {
                let s_anon = is_anonymous_individual(dict, *source);
                let t_anon = is_anonymous_individual(dict, *target);
                match (s_anon, t_anon) {
                    (false, false) => clean.push(axiom.clone()),
                    (false, true) => {
                        // named --p--> blank : a top edge anchoring a tree.
                        *in_degree.entry(*target).or_insert(0) += 1;
                        top_edges.push((property.named(), *source, *target));
                    }
                    (true, true) => {
                        // blank --p--> blank : an internal tree edge.
                        in_degree.entry(*source).or_insert(0);
                        *in_degree.entry(*target).or_insert(0) += 1;
                        outgoing
                            .entry(*source)
                            .or_default()
                            .push((property.named(), *target));
                    }
                    (true, false) => {
                        // blank --p--> named : the anonymous individual has a NAMED successor,
                        // rollable only through a nominal {named} — out of ALCH. Recorded so
                        // `roll_bnode` refuses at the exact offending node.
                        in_degree.entry(*source).or_insert(0);
                        outgoing
                            .entry(*source)
                            .or_default()
                            .push((property.named(), *target));
                    }
                }
            }
            other => clean.push(other.clone()),
        }
    }

    // Every blank-node individual must have EXACTLY ONE parent edge: in-degree 0 is an
    // unanchored free existential root, in-degree ≥ 2 is a node SHARED between two assertions.
    // Both are outside the tree-shaped roll-up (and exactly the negative-probe shapes).
    for (&b, &deg) in &in_degree {
        if deg != 1 {
            return Err(UnknownReason::ConclusionAnonymousIndividual(format!(
                "anonymous individual {} has {} parent property-assertion(s); a tree-shaped \
                 roll-up needs exactly one (shared / unanchored anonymous individual)",
                term_of(dict, b),
                deg
            )));
        }
    }

    // Fold each named-anchored subtree into an existential class assertion on its named root.
    // `consumed` records every blank node folded into a tree; a blank node reached from no
    // named root (a pure cycle / an anchorless component) stays unconsumed and is refused.
    let mut rolled: Vec<Axiom> = clean;
    let mut consumed: FxHashSet<Id> = FxHashSet::default();
    for &(property, source, target) in &top_edges {
        let mut visiting: FxHashSet<Id> = FxHashSet::default();
        let filler = roll_bnode(dict, target, &types, &outgoing, &mut visiting, &mut consumed)?;
        rolled.push(Axiom::ClassAssertion {
            class: ClassExpression::ObjectSomeValuesFrom(
                ObjectPropertyExpression::ObjectProperty(property),
                Box::new(filler),
            ),
            individual: source,
        });
    }

    // Any blank-node individual not folded into a named-rooted tree is a cyclic / anchorless
    // shape (each has in-degree 1 from the check above, so a component with no named anchor is
    // necessarily cyclic) — refuse fail-closed.
    for &b in in_degree.keys() {
        if !consumed.contains(&b) {
            return Err(UnknownReason::ConclusionAnonymousIndividual(format!(
                "anonymous individual {} is not reachable from a named root (cyclic or \
                 anchorless anonymous shape)",
                term_of(dict, b)
            )));
        }
    }
    Ok(rolled)
}

/// Roll one blank-node subtree (rooted at `b`) into the class expression whose existential the
/// parent edge asserts: the conjunction of `b`'s asserted class types and `∃p.⟨child subtree⟩`
/// per outgoing edge. An empty conjunction is `owl:Thing`. Cycle- and nominal-guarded.
fn roll_bnode(
    dict: &Dict,
    b: Id,
    types: &FxHashMap<Id, Vec<ClassExpression>>,
    outgoing: &FxHashMap<Id, Vec<(Id, Id)>>,
    visiting: &mut FxHashSet<Id>,
    consumed: &mut FxHashSet<Id>,
) -> Result<ClassExpression, UnknownReason> {
    if !visiting.insert(b) {
        return Err(UnknownReason::ConclusionAnonymousIndividual(format!(
            "anonymous individual {} lies on a cycle (no tree-shaped roll-up)",
            term_of(dict, b)
        )));
    }
    // A node reachable through two named-anchored paths would already have in-degree ≥ 2 and be
    // refused before we get here; `consumed` insertion is the fold record.
    consumed.insert(b);
    let mut members: Vec<ClassExpression> = types.get(&b).cloned().unwrap_or_default();
    if let Some(edges) = outgoing.get(&b) {
        for &(property, target) in edges {
            if !is_anonymous_individual(dict, target) {
                // A named/literal successor of an anonymous individual needs a nominal {t}.
                return Err(UnknownReason::ConclusionAnonymousIndividual(format!(
                    "anonymous individual {} has a non-anonymous successor {} (a nominal \
                     roll-up is out of the ALCH fragment)",
                    term_of(dict, b),
                    term_of(dict, target)
                )));
            }
            let filler = roll_bnode(dict, target, types, outgoing, visiting, consumed)?;
            members.push(ClassExpression::ObjectSomeValuesFrom(
                ObjectPropertyExpression::ObjectProperty(property),
                Box::new(filler),
            ));
        }
    }
    visiting.remove(&b);
    Ok(match members.len() {
        0 => ClassExpression::Thing,
        1 => members.pop().expect("len checked == 1"),
        _ => ClassExpression::ObjectIntersectionOf(members),
    })
}

// -------------------------------------------------------------------------------------------
// Direct unit tests for the transitive defence-in-depth guards ([GPT-5.6] sq-zfwzq).
// The dispatch routes transitive ontologies to the tableau BEFORE the RL/EL branches, so
// these private guard arms are unreachable end-to-end BY DESIGN — they exist so a future
// dispatch change cannot silently hand a transitive ontology to an incomplete branch. The
// tests pin that contract directly (the integration suite covers the routed path).
// -------------------------------------------------------------------------------------------

#[cfg(all(test, feature = "dl_transitive"))]
mod transitive_guard_tests {
    use super::*;
    use crate::model::ObjectPropertyExpression as OPE;

    fn transitive_onto() -> Ontology {
        Ontology {
            axioms: vec![Axiom::TransitiveObjectProperty {
                property: OPE::ObjectProperty(10),
            }],
        }
    }

    #[test]
    fn rl_divergence_guard_abstains_on_transitivity() {
        let msg = rl_divergence_guard(&transitive_onto())
            .expect("the RL guard must implicate transitivity");
        assert!(msg.contains("TransitiveProperty"), "got {}", msg);
    }

    #[test]
    fn el_unapplied_kind_flags_transitivity() {
        assert_eq!(
            el_unapplied_kind(&transitive_onto()).as_deref(),
            Some("TransitiveObjectProperty")
        );
    }

    #[test]
    fn refutation_budget_fallback_never_owns_a_transitive_refutation() {
        // [OPUS-5] sq-pbz04.4.10: only the ALCH+S tableau's argument covers transitivity, so
        // the budget fallback must decline a transitivity-bearing augmented ontology OUTRIGHT —
        // the same ownership rule `consistency` applies before its own profile dispatch.
        let mut dict = Dict::new();
        assert!(refutation_profile_fallback(&mut dict, &transitive_onto()).is_none());
    }

    #[test]
    fn pr1_scan_counts_transitive_property_as_property_use() {
        // The property id 10 is ALSO used as a class: the PR1 punning scan must see the
        // overlap through the TransitiveObjectProperty arm.
        let onto = Ontology {
            axioms: vec![
                Axiom::TransitiveObjectProperty {
                    property: OPE::ObjectProperty(10),
                },
                Axiom::SubClassOf {
                    sub: ClassExpression::Class(10),
                    sup: ClassExpression::Thing,
                },
            ],
        };
        assert!(
            pr1_punning_violation(&onto).is_some(),
            "class/property punning through the transitivity axiom must be seen"
        );
    }
}

// -------------------------------------------------------------------------------------------
// Direct unit tests for the refutation budget fallback ([OPUS-5] sq-pbz04.4.10).
//
// The end-to-end RL recovery / guard-refusal contract is pinned by the integration suite
// (`tests/check.rs`, `entailment_budget_fallback_*`). These cover the arms that suite cannot
// reach through `entailment`: the EL routing arm (structurally always an abstention on today's
// ABox-bearing encodings — module docs), and the render-round-trip faithfulness comparison the
// fallback verifies before it trusts a rendering.
// -------------------------------------------------------------------------------------------

#[cfg(test)]
mod fallback_tests {
    use super::*;

    /// `{ A ⊑ B, (∃p.C ⊓ ∃q.D)(x) }` over a real dict: OUT of RL (`ObjectSomeValuesFrom` is not
    /// an RL superClassExpression, so the class assertion is not RL-legal) but IN EL.
    fn el_not_rl(dict: &mut Dict) -> Ontology {
        let iri = |dict: &mut Dict, local: &str| dict.intern_iri(&format!("http://ex/{}", local));
        let (a, b) = (iri(dict, "A"), iri(dict, "B"));
        let (c, d) = (iri(dict, "C"), iri(dict, "D"));
        let (p, q) = (iri(dict, "p"), iri(dict, "q"));
        let x = iri(dict, "x");
        Ontology {
            axioms: vec![
                Axiom::SubClassOf {
                    sub: ClassExpression::Class(a),
                    sup: ClassExpression::Class(b),
                },
                Axiom::ClassAssertion {
                    class: ClassExpression::ObjectIntersectionOf(vec![
                        ClassExpression::some(p, ClassExpression::Class(c)),
                        ClassExpression::some(q, ClassExpression::Class(d)),
                    ]),
                    individual: x,
                },
            ],
        }
    }

    #[test]
    fn el_arm_is_routed_but_abstains_on_the_abox_assertion() {
        let mut dict = Dict::new();
        let onto = el_not_rl(&mut dict);
        // Precondition: this really is the EL-not-RL routing case, so the arm below is the EL
        // one. (If a future profile change moved it, this assert — not the fallback — reds.)
        let ps = profiles(&onto);
        assert!(!ps.rl.is_in(), "expected NOT in RL");
        assert!(ps.el.is_in(), "expected in EL");
        // The EL classifier applies no ABox axiom, so `ElUnappliedAxioms` abstains and the
        // fallback declines — the caller keeps its `ResourceBudget` reason.
        assert!(
            el_unapplied_kind(&onto).is_some(),
            "the EL guard must flag the ClassAssertion"
        );
        assert!(
            refutation_profile_fallback(&mut dict, &onto).is_none(),
            "the EL arm must decline, never manufacture a verdict"
        );
    }

    #[test]
    fn fallback_declines_when_no_profile_owns_the_question() {
        // `¬B ⊑ C` — a complement in SUBCLASS position is in no profile, so there is no branch
        // to re-ask and the fallback declines without rendering anything.
        let mut dict = Dict::new();
        let b = dict.intern_iri("http://ex/B");
        let c = dict.intern_iri("http://ex/C");
        let onto = Ontology {
            axioms: vec![Axiom::SubClassOf {
                sub: ClassExpression::ObjectComplementOf(Box::new(ClassExpression::Class(b))),
                sup: ClassExpression::Class(c),
            }],
        };
        let ps = profiles(&onto);
        assert!(!ps.rl.is_in() && !ps.el.is_in());
        assert!(refutation_profile_fallback(&mut dict, &onto).is_none());
    }

    #[test]
    fn rl_arm_recovers_an_inconsistent_refutation() {
        // The `a : B` refutation shape: `{ A ⊑ B, a : A, a : ¬B }` is in RL, and the
        // materializer + clash scan decide it INCONSISTENT (cax-sco then cls-com) — the verdict
        // the starved tableau could not produce. Attributed to the RL branch.
        let mut dict = Dict::new();
        let a = dict.intern_iri("http://ex/A");
        let b = dict.intern_iri("http://ex/B");
        let ind = dict.intern_iri("http://ex/a");
        let onto = Ontology {
            axioms: vec![
                Axiom::SubClassOf {
                    sub: ClassExpression::Class(a),
                    sup: ClassExpression::Class(b),
                },
                Axiom::ClassAssertion {
                    class: ClassExpression::Class(a),
                    individual: ind,
                },
                Axiom::ClassAssertion {
                    class: ClassExpression::ObjectComplementOf(Box::new(ClassExpression::Class(b))),
                    individual: ind,
                },
            ],
        };
        let outcome =
            refutation_profile_fallback(&mut dict, &onto).expect("the RL branch must decide");
        assert_eq!(outcome.verdict, ConsistencyVerdict::Inconsistent);
        assert_eq!(outcome.branch, Branch::RlMaterialization);
    }

    #[test]
    fn round_trip_faithfulness_is_order_insensitive_but_content_strict() {
        let sub = |x, y| Axiom::SubClassOf {
            sub: ClassExpression::Class(x),
            sup: ClassExpression::Class(y),
        };
        let one = Ontology {
            axioms: vec![sub(1, 2), sub(3, 4)],
        };
        let reordered = Ontology {
            axioms: vec![sub(3, 4), sub(1, 2)],
        };
        let different = Ontology {
            axioms: vec![sub(1, 2), sub(3, 5)],
        };
        let shorter = Ontology {
            axioms: vec![sub(1, 2)],
        };
        assert!(same_axiom_multiset(&one, &reordered), "order must not matter");
        assert!(!same_axiom_multiset(&one, &different), "content must matter");
        assert!(!same_axiom_multiset(&one, &shorter), "arity must matter");
    }
}
