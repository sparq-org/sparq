//! [FABLE-5] sq-pbz04.4.5 (epic sq-pbz04.4) — the OWL 2 **Direct-Semantics arm** ratchet:
//! the `inference::dl_suite` runner over the DIRECT-sanctioned arm of the OWL WG
//! test-repository export (`tests/w3c/owl2/all.rdf`), with PINNED sparq-extension floors.
//! 🤖 SPARQ agent.
//!
//! ## What is pinned (and why EXACT equality, not a `>=` ratchet)
//!
//! Two floors, both MEASUREMENTS of what the layered `sparq-reason-dl` checker genuinely
//! computes at the pinned export snapshot — HONESTLY sparq-EXTENSION rows over the
//! **scoped fragment — NOT full OWL 2 DL**, never folded into standards-conformance
//! totals (design record `research/owl2-direct-semantics-scoping.md` §4):
//!
//! - [`gated::DL_PROFILE_FLOOR`] — profile-identification lane passes: the L2 syntactic
//!   checker vs the export's POSITIVE `test:profile` tags ONLY (the design record's
//!   positive-only fallback — see the runner module docs for the measurement that
//!   forced it: L2's `In` is fragment-grammar membership and cannot refute full-profile
//!   membership, so the explicit-negative direction is not checked).
//! - [`gated::DL_DIRECT_FLOOR`] — Direct consistency / inconsistency / positive- /
//!   negative-entailment passes through the L4 `DirectChecker` dispatch under the PINNED
//!   deterministic count budget (`dl_suite::pinned_budget`; wall-clock budgets banned).
//!
//! Both are asserted with EXACT equality (the `ql_entailment_floor` precedent, not the
//! `>=` el-suite shape) — deliberately: the bead's INVARIANT is *an abstention is NEVER
//! counted as a pass*, and a `>=` floor cannot catch the inflation direction (a mutation
//! that counts abstentions as passes only RAISES the count). With `==`, a regression AND
//! an unexplained rise both go RED; a genuine improvement re-pins the consts in a
//! reviewed PR. The abstained totals are pinned the same way, so every selected row
//! stays accounted for in exactly one bucket.
//!
//! ## Divergences: pinned per-case, audited per-mechanism, NEVER passes
//!
//! A `Fail` row means the checker produced a DEFINITIVE verdict that contradicts the
//! export's expectation. Every such row is pinned BY NAME in
//! [`gated::DOCUMENTED_DIVERGENCES`] with an audited mechanism (exact SET equality: an
//! unpinned new fail AND a stale entry that stops failing both go RED). The mechanisms
//! are NOT laundered as acceptable: M4 was a genuine fidelity gap in the merged L1
//! extractor that this conformance arm DISCOVERED; it was FIXED by sq-pbz04.4.12 (see
//! M4 below) and the 4 entries removed from the divergence pin as promised. M1 was
//! FIXED by sq-pbz04.4.11, and M2 (conclusion-bnode existential reading) by sq-pbz04.4.13
//! — its 2 entries removed (they now PASS). The mechanisms:
//!
//! - **M1 — FIXED (sq-pbz04.4.11).** Named-composite inlining previously lost the
//!   name↔expression binding; `extract()` now emits `EquivalentClasses(A, expr)` for
//!   every NAMED class carrying an inline backbone definition, and the 12 corpus cases
//!   that required this binding now PASS.
//! - **M2 — FIXED (sq-pbz04.4.13).** A blank-node individual in the CONCLUSION is now read
//!   EXISTENTIALLY: a tree-shaped anonymous assertion set rolls up into an existential class
//!   assertion the ALCH tableau decides SOUNDLY, so `somevaluesfrom2bnode` and
//!   `WebOnt-someValuesFrom-003` now PASS (removed from the divergence pin). Any non-rollable
//!   conclusion bnode (shared / cyclic / nominal / free-existential-root) abstains fail-closed
//!   (`UnknownReason::ConclusionAnonymousIndividual`), NEVER the old wrong skolem-constant
//!   `NotEntailed` — e.g. `WebOnt-AnnotationProperty-002` (whose `ap` edge is an annotation, so
//!   its conclusion `_:x owl:Thing` is an unanchored free-existential root) shifts from a
//!   coincidental skolem pass to an honest abstention. [OPUS-4.8]
//! - **M3 — reserved vocabulary treated as plain names.** OWL-Full-shaped axioms over
//!   reserved IRIs (`rdfs:Class ≡ owl:Class`, `owl:imports` domain/range, `rdf:type`
//!   domain) extract as ordinary names; the legacy dual-tagged expectation assumes the
//!   reserved semantics.
//! - **M4 — FIXED (sq-pbz04.4.12).** Orphan/cyclic `rdf:first`/`rdf:rest` cells (including
//!   `rdf:nil rdf:rest _:b`, cyclic lists) and unconsumed anonymous class-expression
//!   backbones now correctly refuse extraction (`MalformedList` /
//!   `MalformedClassExpression`), yielding an `OutOfFragment` abstention instead of a wrong
//!   `Consistent` verdict (I5.5-003/004) or a trivially-entailed empty non-conclusion
//!   (I5.5-006/007). The load-bearing part of the fix (the escalated-review REFUTATION of
//!   the first attempt): the orphan-reachability seed must NOT count an ignorable declaration
//!   typing (`_:list a rdf:List`, `_:x a owl:Class`) as a consumer — that bypass wrongly
//!   rescued the very cyclic/orphan cells the check exists to catch. Beyond I5.5-006/-007,
//!   closing it also correctly refuses three graphs that carry an anonymous composite
//!   appearing in NO axiom and had been passing only by that accident (I5.26-001, I5.26-006
//!   consistency; I5.5-005 positive-entailment) — all honest fail-closed abstentions, never
//!   wrong verdicts. [OPUS-4.8]
//! - **M5 — ontology-header stripping vs a header-sensitive legacy expectation.** The
//!   harness strips `owl:Ontology` typings on both sides (the RL/EL-lane convention);
//!   WebOnt-Ontology-003's OWL-1-era non-entailment hinges on the stripped header.
//! - **M6 — the premise/input literal does not parse with oxrdfxml** (a nested
//!   `rdf:RDF` property element the strict parser rejects).
//! - **M7 — FIXED (sq-pbz04.4.16).** L2 requires `ObjectIntersectionOf` arity ≥ 2 (the
//!   structural-spec arity); the official tagging accepts the OWL-1-era RDF
//!   singleton-intersection encoding `_:B owl:intersectionOf ( :A )` by NORMALIZING it to
//!   its sole member `:A`. The L1 extractor now performs that same model-preserving
//!   normalization (`⊓{A} ≡ A` under Direct Semantics — a 1-ary intersection has exactly
//!   its single conjunct's interpretation), so the 27 positively-tagged singleton cases
//!   (`WebOnt-I5.26-002`/`-005`, `WebOnt-disjointWith-003`…`-009`, each EL/QL/RL) now
//!   extract to the atomic member and PASS — the whole `PROFILE_DIVERGENCES` pin cleared.
//!   The arity rule is UNCHANGED for genuine ≥ 2-member intersections. (WebOnt-I5.26-001
//!   was already removed under sq-pbz04.4.12: its anonymous singleton intersection appears
//!   in no axiom, so the M4 fix ABSTAINS on it rather than reaching this rule.) [FABLE-5]
//!
//! ## Explicit-negative profile lane (sq-pbz04.4.16)
//!
//! A SECOND profile direction is now wired: the export's manifest-level
//! `owl:NegativePropertyAssertion(case, test:profile, {EL|QL|RL})` triples assert a case is
//! NOT in a profile. Each checkable such row is driven with `expect_in = false` into the
//! SEPARATE [`gated::DL_PROFILE_NEGATIVE_REFUTED`] lane. This is HONESTLY a partial check:
//! L2's `In` is axiom-grammar membership over the extracted ALCH shadow and cannot always
//! refute FULL-profile membership (the profile specs' non-grammar restrictions — anonymous
//! individuals, datatype maps, global/species constraints — are unimplemented at L2, design
//! record §4/§5, deferred). So an `In` verdict on an explicit negation is an honest MEASURED
//! GAP, not a soundness bug: it is pinned as a gap floor with RISE-ONLY semantics (the gap
//! may only shrink), NEVER asserted to zero, and kept OUT of the positive lane's fail set so
//! the positive M7-cleared floor never moves when the negative lane grows.
//!
//! ## Feature gating (both states)
//!
//! Behind the opt-in `dl-direct` feature (forwards to `sparq-reason-dl/dispatch` plus the
//! default-off `sparq-reason-dl/dl_transitive` ALCHS extension). OFF:
//! this file compiles to a single self-SKIP `#[test]` — no Direct-Semantics code links.
//! The export is fetched by `scripts/fetch-inference-suites.sh` into the gitignored
//! `tests/w3c/owl2/`; when absent the runner SKIPS so a fresh offline checkout stays
//! green. The floor consts are read TEXTUALLY by `tests/scoreboard_floors.rs`, so the
//! central scoreboard's mirrored values can never silently drift.

// [FABLE-5] cfg gate, not a runtime branch — zero Direct-Semantics code compiles in the
// default state (the lean opt-in posture).
#[cfg(not(feature = "dl-direct"))]
#[test]
fn dl_suite_skipped_without_feature() {
    eprintln!(
        "SKIP: the OWL 2 Direct-Semantics arm is OFF — build with `--features dl-direct` \
         (and run scripts/fetch-inference-suites.sh) to run it."
    );
}

#[cfg(feature = "dl-direct")]
mod gated {
    use sparq_conformance::inference::dl_suite::{self, TriState};
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    /// PROFILE-IDENTIFICATION lane pass count (positive `test:profile` tags only) — the
    /// MEASURED value at the pinned export snapshot, EXACT-pinned (module docs).
    /// Mirrored in `scoreboard::SUITES` and read TEXTUALLY by
    /// `tests/scoreboard_floors.rs`. A sparq EXTENSION measurement over the L1/L2
    /// fragment, NOT a W3C ProfileIdentificationTest conformance claim.
    /// [FABLE-5] sq-pbz04.4.5
    /// Re-pinned by sq-pbz04.4.16 (M7 singleton-intersection normalization): +27 (68 → 95).
    /// The 27 pinned M7 rows (`WebOnt-I5.26-002`/`-005`, `WebOnt-disjointWith-003`…`-009`,
    /// each EL/QL/RL) whose singleton `owl:intersectionOf ( :A )` L1 now normalizes to its
    /// member all PASS; `PROFILE_DIVERGENCES` is now empty. [FABLE-5]
    /// Re-pinned by sq-pbz04.4.9 (datatype-map IRI refusal at L1): −1 (95 → 94). One
    /// positively-tagged profile row (`WebOnt-I5.3-015`, tagged EL — the same case whose
    /// premise carries `xsd:integer`/`xsd:string` ranges) previously EXTRACTED under the
    /// opaque-datatype reading and passed the L2 grammar walk; L1 now refuses the datatype
    /// IRI as a `DataConstruct`, so its profile set is `Unknown` → the row moves to the
    /// abstained bucket (`DL_PROFILE_ABSTAINED` 117 → 118). Honest fail-closed shift — a
    /// pass under a wrong opaque-datatype reading becomes an honest abstention (the M4
    /// precedent). [FABLE-5]
    /// Re-pinned by sq-pbz04.4.8 (L1 built-in fixed-extension property refusal): −1 (94 → 93).
    /// `New-Feature-BottomObjectProperty-001` is tagged `test:profile EL` and previously
    /// extracted with `owl:bottomObjectProperty` read as an ordinary named role; L1 now
    /// refuses the built-in IRI, so its profile set is `Unknown` → the row moves to the
    /// abstained bucket (`DL_PROFILE_ABSTAINED` 118 → 119). The same honest fail-closed shift
    /// as the sq-pbz04.4.9 datatype-map precedent: a pass under a wrong opaque-role reading
    /// becomes an honest abstention. [SONNET-4.6]
    pub const DL_PROFILE_FLOOR: usize = 93;

    /// EXPLICIT-NEGATIVE profile lane REFUTED (pass) count — checkable
    /// `owl:NegativePropertyAssertion(case, test:profile, {EL|QL|RL})` rows the L2 checker
    /// DEFINITIVELY refuted (`NotIn`), driven `expect_in = false`. RISE-only in the ABSENCE
    /// of an extraction-boundary change: a genuine new full-profile restriction only ADDS
    /// refutations. [FABLE-5] sq-pbz04.4.16
    /// Re-pinned by sq-pbz04.4.9 (datatype-map IRI refusal at L1): −2 (139 → 137). Two
    /// explicit-negation rows whose input carries a bare datatype IRI in a class position now
    /// REFUSE extraction (`DataConstruct`), so L2 can no longer answer `NotIn` on them — they
    /// move from refuted to abstained (`DL_PROFILE_NEGATIVE_ABSTAINED` 615 → 617; checkable
    /// denominator 319 → 317). Not a refutation regression: the rows became honestly
    /// out-of-fragment, exactly the fail-closed L1 boundary. [FABLE-5]
    /// Re-pinned by sq-zfwzq ([GPT-5.6]): +1 (137 → 138). Enabling fail-closed extraction
    /// of `owl:TransitiveProperty` makes one formerly-abstained negative-profile row
    /// definitively refutable; the two other newly checkable rows enter the measured In-gap.
        /// Re-pinned by sq-pbz04.4.8 (L1 built-in fixed-extension property refusal): −4
    /// (138 → 134). Four explicit-negation rows whose input carries `owl:topObjectProperty` /
    /// `owl:bottomObjectProperty` in a property position now REFUSE extraction, so L2 can no
    /// longer answer `NotIn` on them — they move from refuted to abstained
    /// (`DL_PROFILE_NEGATIVE_ABSTAINED` 614 → 619; checkable denominator 320 → 315). Not a
    /// refutation regression: the rows became honestly out-of-fragment, exactly the
    /// fail-closed L1 boundary (the sq-pbz04.4.9 datatype precedent). [SONNET-4.6]
    pub const DL_PROFILE_NEGATIVE_REFUTED: usize = 134;

    /// EXPLICIT-NEGATIVE profile lane In-GAP — checkable explicit-negation rows where L2
    /// answered `In` (could not refute full-profile membership from axiom-grammar membership
    /// over the ALCH shadow). An HONEST measured limitation, NOT a soundness bug; FALL-only
    /// (the gap may only shrink as L2 grows the deferred full-profile restrictions,
    /// sq-1ivw7-style follow-ups). The In-vs-negative gap is `IN_GAP` of
    /// `REFUTED + IN_GAP` = 182 of 320 (the sq-zfwzq transitivity extraction broadening;
    /// improved from the pre-lane measurement 211/322 — the
    /// M7 normalization moved the 27 singleton cases into the checkable set on BOTH sides).
    /// Unchanged by sq-pbz04.4.9: the two datatype rows that left the checkable set were
    /// REFUTED (not In-gap), so IN_GAP stays 180 while the denominator drops 319 → 317.
    /// [FABLE-5] sq-pbz04.4.16
    /// Re-pinned by sq-zfwzq ([GPT-5.6]): +2 (180 → 182) as two transitive-property inputs
    /// become structurally checkable but L2 axiom-grammar membership cannot refute their
    /// full-profile negations. This is the existing honest In-gap, never a fabricated pass.
        /// Re-pinned by sq-pbz04.4.8 (L1 built-in fixed-extension property refusal): −1
    /// (182 → 181), denominator 320 → 315. One In-gap row left the checkable set with the
    /// four refuted ones; the gap is now 181 of 315. [SONNET-4.6]
    pub const DL_PROFILE_NEGATIVE_IN_GAP: usize = 181;

    /// EXPLICIT-NEGATIVE profile lane ABSTAINED — explicit-negation rows where L1 extraction
    /// refused (out-of-fragment input), so L2 abstained (`Unknown`) rather than answering
    /// either direction. Pinned so the tri-state accounting closes. [FABLE-5] sq-pbz04.4.16
    /// Re-pinned by sq-pbz04.4.9 (datatype-map IRI refusal at L1): +2 (615 → 617) — the two
    /// datatype rows that left [`DL_PROFILE_NEGATIVE_REFUTED`] land here. [FABLE-5]
    /// Re-pinned by sq-zfwzq ([GPT-5.6]): −3 (617 → 614); all three rows moved out only
    /// because `dl_transitive` now extracts their transitivity axiom, closing accounting
    /// with +1 refuted pass and +2 measured In-gap rows above.
        /// Re-pinned by sq-pbz04.4.8 (L1 built-in fixed-extension property refusal): +5
    /// (614 → 619) — the four rows that left [`DL_PROFILE_NEGATIVE_REFUTED`] plus the one
    /// that left [`DL_PROFILE_NEGATIVE_IN_GAP`] land here, closing the accounting.
    /// [SONNET-4.6]
    pub const DL_PROFILE_NEGATIVE_ABSTAINED: usize = 619;

    /// EXPLICIT-NEGATIVE profile lane OUT-OF-SCOPE — explicit-negation rows excluded at
    /// selection (owl:imports / functional-syntax-only input). [FABLE-5] sq-pbz04.4.16
    pub const DL_PROFILE_NEGATIVE_OUT_OF_SCOPE: usize = 74;

    /// DIRECT consistency/entailment lane pass count through the L4 `DirectChecker` —
    /// the MEASURED value at the pinned export snapshot, EXACT-pinned (module docs).
    /// Mirrored in `scoreboard::SUITES` and read TEXTUALLY by
    /// `tests/scoreboard_floors.rs`. A sparq EXTENSION measurement over the scoped
    /// fragment — NOT full OWL 2 DL.
    ///
    /// COMPOSITION: 98 consistency + 14 inconsistency + 72 positive-entailment +
    /// 2 negative-entailment = 186. [FABLE-5] sq-pbz04.4.5 / [OPUS-4.8]
    /// sq-pbz04.4.12/.13 / [GPT-5.6] sq-zfwzq (was 96+14+70+2 before sq-zfwzq;
    /// see the re-pin notes below).
    /// Re-pinned by sq-pbz04.4.11 (M1 named-composite fix): +8 net.
    /// Re-pinned by sq-pbz04.4.12 (M4 orphan/cyclic fix): −3 (192 → 189) — three graphs with
    /// an unconsumed anonymous composite (`WebOnt-I5.26-001`/`-006` consistency; `WebOnt-I5.5-005`
    /// positive-entailment) now honestly abstain, plus the two M4 negative-entailment rows.
    /// Re-pinned by sq-pbz04.4.13 (M2 conclusion-bnode existential-reading fix): +1 (189 → 190).
    /// Both M2 positive-entailment cases `somevaluesfrom2bnode` and `WebOnt-someValuesFrom-003`
    /// now roll their tree-shaped conclusion bnodes into existential class assertions the
    /// tableau certifies (68 → 70 positive-entailment passes); one previously-passing case,
    /// `WebOnt-AnnotationProperty-002`, correctly shifts Pass → abstain because its conclusion
    /// bnode is a FREE-EXISTENTIAL ROOT (`_:x owl:Thing` with the anchoring edge being an
    /// annotation, so it is unanchored / non-rollable — its old skolem pass was coincidental
    /// and unsound for a non-trivial class). Net positive-entailment +1 (70 − 1 = 69), all
    /// four consistency/inconsistency lanes unchanged. [OPUS-4.8] sq-pbz04.4.13
    /// Re-pinned by sq-pbz04.4.16 (M7 singleton-intersection normalization): −8 (190 → 182),
    /// ALL in the CONSISTENCY lane (105 → 97) and ALL honest fail-closed abstentions (verified:
    /// the fail set is UNCHANGED at 5, M3/M5/M6 — never a wrong verdict). MECHANISM: `WebOnt-
    /// disjointWith-003`…`-009` (7) and `WebOnt-I5.26-005` (1) previously carried a singleton
    /// `owl:intersectionOf ( :A )` whose arity-1 shape put them OUT of every profile, routing
    /// consistency to the ALCH tableau (a definitive `Consistent`). The M7 normalization
    /// collapses those singletons to their member, so the graphs are now correctly IN-RL and
    /// dispatch routes them to the RL branch — which fail-closed ABSTAINS via the documented
    /// `RlDivergenceGuard("owl:disjointWith")` (RL rule-set incompleteness on disjointness, the
    /// DisjointClasses-001/-003 divergences), because "no clash" does not certify consistency
    /// there. Honest routing to a guarded branch, not a lost capability — the tableau would
    /// still decide these if reached; graduating them back is a deliberate future re-pin once
    /// the RL divergence guard is narrowed or a tableau fallback is added. [FABLE-5]
    /// UNCHANGED at 182 by sq-pbz04.4.9, but COMPOSITION shifted (97+69 → 96+70 across the
    /// consistency/positive-entailment split). TWO independent effects net to zero on the pass
    /// total AND leave the audited fail set (M3/M5/M6, 5 rows) exactly unchanged:
    ///   (a) the new `SubObjectPropertyOf` conclusion refutation encoding (the sq-pbz04.4.9
    ///       feature) graduates one property-axiom-conclusion positive-entailment row from a
    ///       (previously `UnencodedConclusion`) abstention to a definitive PASS: +1 pos-ent;
    ///   (b) the datatype-map IRI refusal at L1 (`extract.rs` `datatype_iri_name`) makes the
    ///       `WebOnt-I5.3-015` consistency-lane row (its premise carries `xsd:integer`/
    ///       `xsd:string` ranges) refuse extraction, so it moves from a definitive `Consistent`
    ///       PASS to an honest `OutOfFragment` abstention: −1 consistency. Crucially, that same
    ///       L1 refusal is what REMOVES the would-be NEW positive-entailment divergence
    ///       `WebOnt-I5.3-015` (whose conclusion `p ⊑ q` the RBox encoding would otherwise have
    ///       decided `NotEntailed` — sound-as-scoped under opaque datatypes, but a wrong-looking
    ///       definitive verdict on an input the fragment cannot actually reason about; the
    ///       escalated soundness verdict directed refusing at L1 rather than pinning it). Net:
    ///       DIRECT pass total 182 → 182, fail set unchanged. [FABLE-5] sq-pbz04.4.9
    /// Re-pinned by sq-zfwzq ([GPT-5.6]): +4 (182 → 186): +2 consistency
    /// (`rdfbased-sem-char-transitive-inst`, `WebOnt-TransitiveProperty-001`) and +2
    /// positive entailment for those same composed-edge cases (96 → 98 consistency,
    /// 70 → 72 positive-entailment). They previously failed closed at L1; the premise
    /// now extracts into ALCHS and the declaration-free conclusion roles reuse only the
    /// premise-confirmed role kind. The audited five-row divergence set is unchanged.
    /// UNCHANGED at 186 by sq-fj8lj ([FABLE-5] — `dl-direct` now enables
    /// `sparq-reason-dl/dispatch_ql`, graduating the QL dispatch branch to the
    /// sparq-reason-ql DL-Lite_R checker): MEASURED at the pinned export, ZERO corpus rows
    /// dispatch to the QL branch — reaching it needs in-QL ∧ not-RL ∧ not-EL, which forces
    /// a complement shape absent from the extractable corpus (the pre-change abstention
    /// histogram carried no "QL consistency pending" entry), so no row's verdict moves.
    /// Re-pinned by sq-pbz04.4.8 ([SONNET-4.6] — the guard-abstention tableau fall-through,
    /// design record §4 as amended): **+41 (186 → 227)**, composition 136 consistency +
    /// 17 inconsistency + 72 positive-entailment + 2 negative-entailment. This is the bead,
    /// and it is the largest single move this floor has made. MECHANISM: when the owning
    /// RL/EL/QL branch abstains, the dispatch now re-asks the ALCH tableau — complete for the
    /// whole L1 fragment, hence for every in-profile ontology too — instead of returning
    /// `Unknown`. The pre-change abstention histogram's three guard buckets (16 RL-divergence,
    /// 26 EL-unapplied, 1 EL-⊤ = 43 rows) are GONE from the post-change histogram entirely;
    /// 41 of those rows graduated to a definitive expected verdict (+38 consistency, +3
    /// inconsistency) and the other 2 are accounted for by the L1 built-in-property refusal
    /// below, NOT by a new budget abstention (`deterministic count budget exhausted` is
    /// unchanged at 16). The 8 `WebOnt-disjointWith-003`…`-009` / `WebOnt-I5.26-005`
    /// consistency rows that sq-pbz04.4.16 re-routed into `RlDivergenceGuard` are among those
    /// recovered — verified per row, not inferred from the totals — exactly as that re-pin
    /// note anticipated ("graduating them back is a deliberate future re-pin once … a tableau
    /// fallback is added").
    ///
    /// The audited five-row divergence set is UNCHANGED — verified, and the verification
    /// mattered: the FIRST measurement of the fall-through alone put two NEW rows in the fail
    /// bucket (`New-Feature-BottomObjectProperty-001` / `New-Feature-TopObjectProperty-001`,
    /// both `InconsistencyTest`s). Those were not fall-through bugs but a pre-existing L1 gap
    /// the profile guards had been accidentally MASKING: L1 read `owl:bottomObjectProperty` /
    /// `owl:topObjectProperty` as ordinary named roles, dropping the fixed extensions (∅ and
    /// ΔI × ΔI) that are the sole reason those two ontologies are inconsistent. Fixed at L1 by
    /// refusing the built-in property IRIs fail-closed at every property position — the
    /// sq-pbz04.4.9 precedent, where the escalated soundness verdict likewise directed
    /// refusing at L1 rather than pinning a wrong definitive verdict. With that refusal the
    /// inconsistency lane's fail count returns to 0 and both rows abstain honestly.
    /// [SONNET-4.6]
    pub const DL_DIRECT_FLOOR: usize = 227;

    /// Abstained (fail-closed OutOfFragment / guard / deferred / budget) row totals,
    /// EXACT-pinned so the tri-state accounting is closed: profile lane, then the four
    /// reasoning lanes summed. [FABLE-5] sq-pbz04.4.5
    /// Re-pinned by sq-pbz04.4.12 (M4 orphan/cyclic fix): +3 (114 → 117). Closing the
    /// declaration-typing reachability bypass makes the profile lane's extraction refuse the
    /// three `WebOnt-I5.26-001` profile rows (EL/QL/RL): the input carries an anonymous
    /// `owl:intersectionOf` that appears in no axiom, so extraction now fails → the profile
    /// set is `Unknown` → abstain (previously it extracted and answered a wrong `NotIn`, the
    /// M7 singleton-intersection divergence — now superseded by the honest abstention).
    /// [OPUS-4.8] sq-pbz04.4.12
    /// Re-pinned by sq-pbz04.4.9 (datatype-map IRI refusal at L1): +1 (117 → 118). The
    /// `WebOnt-I5.3-015` positive profile row (EL-tagged; its premise carries `xsd:integer`/
    /// `xsd:string` ranges) now REFUSES extraction, moving from a pass to an abstention (see
    /// [`DL_PROFILE_FLOOR`]). [FABLE-5]
        /// Re-pinned by sq-pbz04.4.8 (L1 built-in fixed-extension property refusal): +1
    /// (118 → 119). The `New-Feature-BottomObjectProperty-001` positive profile row (EL-tagged)
    /// now REFUSES extraction — see [`DL_PROFILE_FLOOR`]. [SONNET-4.6]
    pub const DL_PROFILE_ABSTAINED: usize = 119;
    /// See [`DL_PROFILE_ABSTAINED`].
    /// Re-pinned by sq-pbz04.4.11: +4 from the 4 consistency cases that shifted from
    /// Pass to OutOfFragment (budget exhaustion on the now-more-complete model).
    /// Re-pinned by sq-pbz04.4.12 (M4 orphan/cyclic fix): MEASURED 463 against the pinned
    /// export. Closing the declaration-typing reachability bypass moves the 2 M4
    /// negative-entailment rows (I5.5-006/-007) from Fail (wrong definitive verdict) to
    /// OutOfFragment (fail-closed refusal), and additionally REFUSES 3 previously-passing
    /// rows that carried an unconsumed anonymous composite the declaration typing had been
    /// rescuing (I5.26-001, I5.26-006 consistency; I5.5-005 positive-entailment) — all
    /// honest abstentions, never wrong verdicts. [OPUS-4.8] sq-pbz04.4.12
    /// Re-pinned by sq-pbz04.4.13 (M2 conclusion-bnode fix): +1 (463 → 464). The
    /// free-existential-root conclusion bnode of `WebOnt-AnnotationProperty-002` shifts from a
    /// (coincidental) Pass to a fail-closed `ConclusionAnonymousIndividual` abstention; the two
    /// graduated M2 cases move Fail → Pass (they never counted as abstentions). [OPUS-4.8]
    /// Re-pinned by sq-pbz04.4.16 (M7 singleton-intersection normalization): +8 (464 → 472).
    /// The 8 consistency cases (`WebOnt-disjointWith-003`…`-009`, `WebOnt-I5.26-005`) that the
    /// M7 fix re-routes from the ALCH tableau (Pass) to the RL branch move to a fail-closed
    /// `RlDivergenceGuard("owl:disjointWith")` abstention — see [`DL_DIRECT_FLOOR`] for the full
    /// mechanism (honest routing to a guarded branch, never a wrong verdict). [FABLE-5]
    /// UNCHANGED at 472 by sq-pbz04.4.9 (measured), composition-shifted: the new
    /// `SubObjectPropertyOf` conclusion encoding graduates property-axiom-conclusion rows OUT
    /// of abstention (−N), while the L1 datatype-map refusal moves the `WebOnt-I5.3-015`
    /// consistency row plus its would-be positive-entailment divergence INTO OutOfFragment
    /// abstention (+N) — the two effects net to zero on the total; see [`DL_DIRECT_FLOOR`].
    /// [FABLE-5] sq-pbz04.4.9
    /// Re-pinned by sq-zfwzq ([GPT-5.6]): −4 (472 → 468), exactly the two consistency
    /// and two positive-entailment rows graduated into [`DL_DIRECT_FLOOR`].
    /// UNCHANGED at 468 by sq-fj8lj (the graduated QL dispatch branch): zero corpus rows
    /// reach the QL branch — see the [`DL_DIRECT_FLOOR`] note.
    /// Re-pinned by sq-pbz04.4.8 ([SONNET-4.6]): **−41 (468 → 427)**, composition 194
    /// consistency + 96 inconsistency + 123 positive-entailment + 14 negative-entailment.
    /// The mirror image of the [`DL_DIRECT_FLOOR`] +41: the guard-abstention tableau
    /// fall-through is strictly abstention-REDUCING, so every row it graduated leaves this
    /// bucket and nothing enters it from that change. The L1 built-in-property refusal moves
    /// its 2 rows WITHIN the bucket (guard → out-of-fragment), not into or out of it, which
    /// is why the two deltas are exactly equal and opposite. [SONNET-4.6]
    pub const DL_DIRECT_ABSTAINED: usize = 427;

    /// Audited, PINNED divergence rows (module docs — mechanisms M3/M5/M6): every row
    /// where a checker verdict contradicts the export expectation, keyed by the
    /// runner's row key. EXACT set equality is asserted: an UNPINNED new fail and a
    /// STALE entry that stops failing both turn the lane RED (the el-suite /
    /// ql_entailment_floor discipline). M4 was FIXED by sq-pbz04.4.12 and removed from
    /// this list — those 4 cases now ABSTAIN (OutOfFragment refusal) rather than producing
    /// wrong definitive verdicts. M1 (named-composite name-binding) was FIXED by
    /// sq-pbz04.4.11; M2 (conclusion-bnode existential reading) was FIXED by sq-pbz04.4.13 and
    /// its two cases removed — they now PASS. [FABLE-5] sq-pbz04.4.5 / [OPUS-4.8] sq-pbz04.4.13
    const DOCUMENTED_DIVERGENCES: &[(&str, &str)] = &[
        // M2 (anonymous individuals in the conclusion read as constants) FIXED by
        // sq-pbz04.4.13 — `somevaluesfrom2bnode` / `WebOnt-someValuesFrom-003` now roll their
        // tree-shaped conclusion bnodes into existential class assertions and PASS; a
        // non-rollable conclusion bnode (e.g. WebOnt-AnnotationProperty-002's
        // free-existential root) abstains fail-closed, never a wrong verdict. [OPUS-4.8]
        //
        // --- M3: reserved vocabulary treated as plain names ---
        (
            "owl2-dl/positive-entailment: WebOnt-Class-001",
            "M3 — conclusion `rdfs:Class owl:equivalentClass owl:Class` is an \
             OWL-Full-shaped reserved-vocabulary axiom",
        ),
        (
            "owl2-dl/positive-entailment: WebOnt-imports-010",
            "M3 — conclusion gives owl:imports a domain/range (reserved vocabulary)",
        ),
        (
            "owl2-dl/positive-entailment: WebOnt-I5.3-014",
            "M3 — premise gives rdf:type an rdfs:domain (reserved vocabulary); the \
             expected subclass conclusion needs that OWL-Full reading",
        ),
        // M4 (orphan/cyclic list and unconsumed backbone) FIXED by sq-pbz04.4.12 —
        // WebOnt-I5.5-003/-004/-006/-007 now correctly ABSTAIN (OutOfFragment refusal)
        // instead of producing wrong Consistent / Entailed verdicts. [OPUS-4.8]
        //
        // --- M5: header-stripping vs a header-sensitive legacy expectation ---
        (
            "owl2-dl/negative-entailment: WebOnt-Ontology-003",
            "M5 — with the owl:Ontology headers stripped (the harness convention shared \
             with the RL/EL lanes), Car ≡ Automobile makes the swapped typings genuinely \
             entailed; the OWL-1-era non-entailment hinges on the header triple",
        ),
        // --- M6: the inline ontology literal does not parse with oxrdfxml ---
        (
            "owl2-dl/consistency: FS2RDF-literals-ar",
            "M6 — the premise literal embeds a nested rdf:RDF property element oxrdfxml \
             rejects (strict RDF/XML)",
        ),
    ];

    /// PINNED positive-profile divergence rows (positive tags where L2 answers `NotIn`) —
    /// same exact-set discipline as [`DOCUMENTED_DIVERGENCES`]. NOW EMPTY: the single
    /// audited mechanism (M7 — the OWL-1-era singleton `owl:intersectionOf ( :A )` encoding
    /// L2's ≥ 2-member arity rule rejected) was FIXED by sq-pbz04.4.16 (L1 now normalizes a
    /// 1-ary intersection to its sole member, model-preservingly), so all 27 rows
    /// (`WebOnt-I5.26-002`/`-005`, `WebOnt-disjointWith-003`…`-009`, each EL/QL/RL) now PASS
    /// and `DL_PROFILE_FLOOR` rose 68 → 95. `WebOnt-I5.26-001` had already been removed by
    /// sq-pbz04.4.12 (its anonymous singleton appears in no axiom → the M4 orphan fix
    /// ABSTAINS, never reaching the arity rule). The exact-set assertion keeps the lane
    /// RED if any future regression re-introduces a positive-tag `NotIn`. [FABLE-5]
    const PROFILE_DIVERGENCES: &[(&str, &str)] = &[];

    /// RENDER ROUND-TRIP arm (sq-pbz04.4.17) — the number of DIRECT-arm corpus documents
    /// (premise / input / conclusion / non-conclusion literals) where L1 extraction
    /// succeeded AND the forward renderer's output re-extracted to an EQUAL structural
    /// model. EXACT-pinned like the other lanes so the arm can never silently go vacuous
    /// (a selection bug that skips every document would show as 0 ≠ the pin, not as a
    /// green no-op). NOT a scoreboard row: this is an internal renderer-fidelity
    /// invariant over the scoped ALCH fragment, not a conformance-shaped measurement.
    ///
    /// MEASURED at the pinned export snapshot AFTER the sq-pbz04.4.17 renderer fix: the
    /// arm's first run caught 15 REAL mis-renders (14 named-composite-equivalence
    /// reference divergences, e.g. `WebOnt-description-logic-201`…`-209`, plus the
    /// self-referential `WebOnt-someValuesFrom-003` rendering to a cyclic expression the
    /// extractor refuses) — fixed by always emitting the explicit
    /// `A owl:equivalentClass _:b` shape (see `render.rs`), which moved those 15 from
    /// violations to passes (278 → 293). [FABLE-5] sq-pbz04.4.17
    /// Re-pinned by sq-pbz04.4.9 (datatype-map IRI refusal at L1): −2 (293 → 291). Two
    /// DIRECT-arm documents carried a bare `xsd:*` datatype IRI in a class-expression
    /// position (an `rdfs:range`/`rdfs:domain` object) and had been round-tripping through
    /// the OLD opaque-object-class reading; L1 now refuses that as a `DataConstruct`
    /// (`extract.rs` `datatype_iri_name`), so they move from round-tripped to
    /// extraction-refused (`DL_RENDER_ROUNDTRIP_REFUSED` 378 → 380). Honest shift — a
    /// coincidental round-trip under a wrong reading becomes an honest refusal; `violations`
    /// stays 0 (no fidelity bug). [FABLE-5]
    /// Re-pinned by sq-zfwzq ([GPT-5.6]): +5 (291 → 296). Five corpus documents whose
    /// only formerly-unmodelled role characteristic was `owl:TransitiveProperty` now
    /// extract and round-trip through the feature-gated structural axiom; violations stay 0.
        /// Re-pinned by sq-pbz04.4.8 (L1 built-in fixed-extension property refusal): −2
    /// (296 → 294), with `DL_RENDER_ROUNDTRIP_REFUSED` +2 (375 → 377) — the accounting stays
    /// closed at 672 documents. The two `New-Feature-{Top,Bottom}ObjectProperty-001` premise
    /// documents no longer extract, so they leave the round-trip pass set for the
    /// extraction-refused bucket. `violations` remains EMPTY, which is the load-bearing
    /// assertion here: no renderer/extractor fidelity bug is involved. [SONNET-4.6]
    pub const DL_RENDER_ROUNDTRIP_FLOOR: usize = 294;
    /// See [`DL_RENDER_ROUNDTRIP_FLOOR`] — documents the L1 extractor refused
    /// (out-of-ALCH-fragment; the renderer contract is scoped to successful extractions).
    /// Re-pinned by sq-pbz04.4.9: +2 (378 → 380) — see [`DL_RENDER_ROUNDTRIP_FLOOR`].
    /// Re-pinned by sq-zfwzq ([GPT-5.6]): −5 (380 → 375), exactly the five documents
    /// graduated to [`DL_RENDER_ROUNDTRIP_FLOOR`].
        /// Re-pinned by sq-pbz04.4.8: +2 (375 → 377). See [`DL_RENDER_ROUNDTRIP_FLOOR`].
    /// [SONNET-4.6]
    pub const DL_RENDER_ROUNDTRIP_REFUSED: usize = 377;
    /// See [`DL_RENDER_ROUNDTRIP_FLOOR`] — documents whose RDF/XML literal oxrdfxml
    /// rejects (the M6 mechanism, `FS2RDF-literals-ar`); they reach no lane's extraction.
    pub const DL_RENDER_ROUNDTRIP_PARSE_FAILED: usize = 1;
    /// See [`DL_RENDER_ROUNDTRIP_FLOOR`] — total documents examined (closed accounting:
    /// `DOCUMENTS = FLOOR + REFUSED + PARSE_FAILED + violations(0)`).
    pub const DL_RENDER_ROUNDTRIP_DOCUMENTS: usize = 672;

    /// RENDER ROUND-TRIP arm, **RDF-BASED-only slice** (sq-pbz04.4.18) — the same
    /// renderer-fidelity invariant over the corpus complement the DIRECT arm never sees:
    /// cases carrying `test:semantics test:RDF-BASED` and NOT `test:DIRECT`. EXACT-pinned
    /// on the same discipline as [`DL_RENDER_ROUNDTRIP_FLOOR`], and likewise NOT a
    /// scoreboard row — no reasoning verdict is attributed to an RDF-BASED test anywhere
    /// (the round trip is purely syntactic, so it makes no semantics claim at all).
    ///
    /// ## MEASURED: the slice is 7 cases / 13 documents, NOT "several hundred"
    ///
    /// The bead estimated "several hundred MORE ontology documents". That is WRONG at the
    /// pinned snapshot, and the measurement is the correction: of the export's 493
    /// `test:TestCase` nodes, **479 are DUAL-tagged** (DIRECT ∧ RDF-BASED) and so were
    /// already fully covered by the DIRECT arm's 672 documents; 6 are DIRECT-only, 1 is
    /// untagged, and only **7 are RDF-BASED-only** — `WebOnt-Class-005`,
    /// `WebOnt-I4.6-003`, `WebOnt-I4.6-005`, `WebOnt-Restriction-005`, `WebOnt-Thing-005`,
    /// `WebOnt-equivalentClass-008`, `WebOnt-miscellaneous-302` (all `Proposed`-status
    /// OWL-1-era WebOnt cases), carrying 13 RDF/XML ontology documents between them.
    /// So the bead's own "marginal value is honestly LOW" call was right, and understated
    /// it: the extension adds 13 documents (+1.9% corpus), not hundreds. It is pinned
    /// anyway because it is closed-form and cheap, and because pinning it CLOSES the
    /// question — the ELIGIBLE slice (non-`Rejected`, recognised check kind, DIRECT and/or
    /// RDF-BASED) is now exhaustively covered, and this const records the true size so the
    /// "hundreds of unexamined documents" premise cannot be re-raised. The 1 case tagged
    /// with NEITHER semantics, and any case with no recognised check kind, is in neither
    /// arm: the two arms are disjoint, but their union is not the whole export.
    ///
    /// 10 of the 13 round-trip; violations are 0 — the renderer is faithful on every
    /// RDF-BASED-only document L1 accepts, so this slice found NO new fidelity bug (unlike
    /// the DIRECT arm's first run, which caught 15).
    pub const DL_RDF_BASED_ROUNDTRIP_FLOOR: usize = 10;
    /// See [`DL_RDF_BASED_ROUNDTRIP_FLOOR`] — RDF-BASED-only documents the L1 extractor
    /// refused (out-of-ALCH-fragment). The bead expected refusals to DOMINATE this slice
    /// ("most refuse extraction at L1 — OWL-Full-heavy"); measured, they do not (3 of 13),
    /// because these 7 cases are OWL-1-era WebOnt and syntactically tame.
    pub const DL_RDF_BASED_ROUNDTRIP_REFUSED: usize = 3;
    /// See [`DL_RDF_BASED_ROUNDTRIP_FLOOR`] — RDF-BASED-only documents oxrdfxml rejects.
    /// ZERO: the M6 mechanism (`FS2RDF-literals-ar`) is a DIRECT-arm case.
    pub const DL_RDF_BASED_ROUNDTRIP_PARSE_FAILED: usize = 0;
    /// See [`DL_RDF_BASED_ROUNDTRIP_FLOOR`] — total RDF-BASED-only documents examined
    /// (closed accounting: `DOCUMENTS = FLOOR + REFUSED + PARSE_FAILED + violations(0)`).
    pub const DL_RDF_BASED_ROUNDTRIP_DOCUMENTS: usize = 13;

    /// Locate the OWL WG export the same way the inference binary + el-suite lane do.
    fn owl_export() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/owl2/all.rdf")
    }

    /// The RENDER ROUND-TRIP corpus arm (sq-pbz04.4.17): extends the renderer's 13
    /// hand-written round-trip tests (`sparq-reason-dl::render::tests`) to EVERY ontology
    /// document of the DIRECT-arm corpus. The load-bearing assertion is that
    /// `report.violations` is EMPTY: whenever L1 extraction succeeds, `render_to_triples`'
    /// output MUST re-extract to an equal structural model — a violation is a REAL
    /// renderer/extractor fidelity bug and is NOT pinnable as an acceptable divergence
    /// (unlike the M3/M5/M6 semantic-expectation divergences of the reasoning lanes).
    /// The counts are EXACT-pinned so the arm cannot silently go vacuous.
    #[test]
    fn dl_render_roundtrip_corpus() {
        let export = owl_export();
        if !export.exists() {
            eprintln!(
                "SKIP: OWL WG export not present at {} — run scripts/fetch-inference-suites.sh",
                export.display()
            );
            return;
        }
        let text = std::fs::read_to_string(&export)
            .unwrap_or_else(|e| panic!("read {}: {e}", export.display()));
        let report = dl_suite::run_render_roundtrip_arm(&text).expect("all.rdf parses");
        // [FABLE-5] Grep-able ratchet line — pass count in field $8, positional args
        // (CodeQL rust/unused-variable guard).
        println!("{}", report.render());
        println!(
            "OWL 2 DL render round-trip ratchet pass {} of {} (floor {})",
            report.round_tripped,
            report.documents,
            DL_RENDER_ROUNDTRIP_FLOOR
        );
        // The invariant: NO violations, ever — not a pinnable divergence set.
        assert!(
            report.violations.is_empty(),
            "render round-trip violations (REAL renderer/extractor fidelity bugs): {:#?}",
            report.violations
        );
        // Exact pins (the module-doc discipline: `==`, so an unexplained rise and a
        // regression both go red; re-pin deliberately when the fragment or corpus moves).
        assert_eq!(
            report.round_tripped, DL_RENDER_ROUNDTRIP_FLOOR,
            "render round-trip pass count moved (got {}, pinned {}) — re-pin \
             DL_RENDER_ROUNDTRIP_FLOOR deliberately with evidence",
            report.round_tripped, DL_RENDER_ROUNDTRIP_FLOOR
        );
        assert_eq!(
            report.extraction_refused, DL_RENDER_ROUNDTRIP_REFUSED,
            "render round-trip refused count moved — the accounting is pinned"
        );
        assert_eq!(
            report.parse_failed, DL_RENDER_ROUNDTRIP_PARSE_FAILED,
            "render round-trip parse-failed count moved — the accounting is pinned"
        );
        assert_eq!(
            report.documents, DL_RENDER_ROUNDTRIP_DOCUMENTS,
            "render round-trip document count moved — the accounting is pinned"
        );
        assert!(report.accounting_closed(), "every document in exactly one bucket");
    }

    /// The RENDER ROUND-TRIP arm over the **RDF-BASED-only** slice (sq-pbz04.4.18) — the
    /// corpus complement the DIRECT arm never sees. Same load-bearing assertion: EMPTY
    /// `violations`; a mis-render here is a REAL renderer/extractor fidelity bug and is NOT
    /// pinnable. Counts EXACT-pinned so the arm cannot silently go vacuous — which matters
    /// MORE here than in the DIRECT arm, because this slice is small (13 documents): a
    /// selection bug that skipped it entirely would otherwise look like a green no-op.
    #[test]
    fn dl_rdf_based_roundtrip_corpus() {
        let export = owl_export();
        if !export.exists() {
            eprintln!(
                "SKIP: OWL WG export not present at {} — run scripts/fetch-inference-suites.sh",
                export.display()
            );
            return;
        }
        let text = std::fs::read_to_string(&export)
            .unwrap_or_else(|e| panic!("read {}: {e}", export.display()));
        let report =
            dl_suite::run_render_roundtrip_rdf_based_arm(&text).expect("all.rdf parses");
        println!("{}", report.render());
        println!(
            "OWL 2 RDF-Based-only render round-trip ratchet pass {} of {} (floor {})",
            report.round_tripped,
            report.documents,
            DL_RDF_BASED_ROUNDTRIP_FLOOR
        );
        assert!(
            report.violations.is_empty(),
            "RDF-BASED-only render round-trip violations (REAL renderer/extractor fidelity \
             bugs): {:#?}",
            report.violations
        );
        assert_eq!(
            report.round_tripped, DL_RDF_BASED_ROUNDTRIP_FLOOR,
            "RDF-BASED-only round-trip pass count moved (got {}, pinned {}) — re-pin \
             DL_RDF_BASED_ROUNDTRIP_FLOOR deliberately with evidence",
            report.round_tripped, DL_RDF_BASED_ROUNDTRIP_FLOOR
        );
        assert_eq!(
            report.extraction_refused, DL_RDF_BASED_ROUNDTRIP_REFUSED,
            "RDF-BASED-only refused count moved — the accounting is pinned"
        );
        assert_eq!(
            report.parse_failed, DL_RDF_BASED_ROUNDTRIP_PARSE_FAILED,
            "RDF-BASED-only parse-failed count moved — the accounting is pinned"
        );
        assert_eq!(
            report.documents, DL_RDF_BASED_ROUNDTRIP_DOCUMENTS,
            "RDF-BASED-only document count moved — the accounting is pinned"
        );
        assert!(report.accounting_closed(), "every document in exactly one bucket");
    }

    /// The two round-trip arms must be DISJOINT (sq-pbz04.4.18): the RDF-BASED-only slice
    /// is the COMPLEMENT of the DIRECT selection, not "all RDF-BASED cases", so no
    /// dual-tagged case's documents are counted in both floors. Asserted on the violation
    /// KEYS' case identifiers would be vacuous (both violation sets are empty), so this
    /// pins the property that actually encodes disjointness: the two document counts are
    /// pinned separately AND their sum is the total the two selections cover — a bug that
    /// made `RdfBasedOnly` mean "all RDF-BASED" would blow the 13 pin up past 600.
    #[test]
    fn dl_roundtrip_arms_are_disjoint() {
        let export = owl_export();
        if !export.exists() {
            eprintln!("SKIP: OWL WG export not present at {}", export.display());
            return;
        }
        let text = std::fs::read_to_string(&export)
            .unwrap_or_else(|e| panic!("read {}: {e}", export.display()));
        let direct = dl_suite::run_render_roundtrip_arm(&text).expect("all.rdf parses");
        let rdf_based =
            dl_suite::run_render_roundtrip_rdf_based_arm(&text).expect("all.rdf parses");
        // The arms self-identify, so a report can never be mistaken for the other slice.
        assert_eq!(direct.arm, dl_suite::RoundTripArm::Direct);
        assert_eq!(rdf_based.arm, dl_suite::RoundTripArm::RdfBasedOnly);
        // Disjointness: the RDF-BASED-only slice is a small complement, NOT a superset of
        // (or overlapping with) the DIRECT corpus. If the semantics filter regressed to
        // "any RDF-BASED", this count would jump to ~the whole corpus.
        assert!(
            rdf_based.documents < direct.documents / 10,
            "RDF-BASED-only slice ({} docs) should be a small complement of the DIRECT arm \
             ({} docs) — a much larger count means the DIRECT-exclusion was dropped and the \
             two floors now double-count dual-tagged cases",
            rdf_based.documents,
            direct.documents
        );
    }

    /// Render round-trip derivation canary — NOT a W3C case, NOT counted in any pin.
    /// Proves the arm drives the REAL extract → render → re-extract pipeline end-to-end
    /// on a mini manifest with KNOWN counts: two in-fragment documents round-trip, one
    /// out-of-fragment document (owl:sameAs) is refused, and nothing is a violation. A
    /// neutered arm (one that skips every document, or one that counts refusals as
    /// round-trips) turns these exact counts RED — and the canary runs WITHOUT the
    /// corpus file, so the arm logic is exercised even on an offline checkout.
    #[test]
    fn dl_render_roundtrip_derivation_canary() {
        let mini = r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
         xmlns:owl="http://www.w3.org/2002/07/owl#"
         xmlns:test="http://www.w3.org/2007/OWL/testOntology#">
  <test:TestCase rdf:about="http://ex/canary-roundtrip">
    <rdf:type rdf:resource="http://www.w3.org/2007/OWL/testOntology#PositiveEntailmentTest"/>
    <test:identifier rdf:datatype="http://www.w3.org/2001/XMLSchema#string">canary-roundtrip</test:identifier>
    <test:status rdf:resource="http://www.w3.org/2007/OWL/testOntology#Approved"/>
    <test:semantics rdf:resource="http://www.w3.org/2007/OWL/testOntology#DIRECT"/>
    <test:rdfXmlPremiseOntology rdf:datatype="http://www.w3.org/2001/XMLSchema#string">&lt;rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:owl="http://www.w3.org/2002/07/owl#" xmlns:rdfs="http://www.w3.org/2000/01/rdf-schema#" xml:base="http://example.org/"&gt;&lt;owl:Ontology/&gt;&lt;owl:Class rdf:about="A"&gt;&lt;rdfs:subClassOf&gt;&lt;owl:Restriction&gt;&lt;owl:onProperty&gt;&lt;owl:ObjectProperty rdf:about="r"/&gt;&lt;/owl:onProperty&gt;&lt;owl:someValuesFrom rdf:resource="B"/&gt;&lt;/owl:Restriction&gt;&lt;/rdfs:subClassOf&gt;&lt;/owl:Class&gt;&lt;/rdf:RDF&gt;</test:rdfXmlPremiseOntology>
    <test:rdfXmlConclusionOntology rdf:datatype="http://www.w3.org/2001/XMLSchema#string">&lt;rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:owl="http://www.w3.org/2002/07/owl#" xmlns:rdfs="http://www.w3.org/2000/01/rdf-schema#" xml:base="http://example.org/"&gt;&lt;owl:Ontology/&gt;&lt;owl:Class rdf:about="A"&gt;&lt;rdfs:subClassOf&gt;&lt;owl:Class rdf:about="C"/&gt;&lt;/rdfs:subClassOf&gt;&lt;/owl:Class&gt;&lt;/rdf:RDF&gt;</test:rdfXmlConclusionOntology>
  </test:TestCase>
  <test:TestCase rdf:about="http://ex/canary-out-of-fragment">
    <rdf:type rdf:resource="http://www.w3.org/2007/OWL/testOntology#ConsistencyTest"/>
    <test:identifier rdf:datatype="http://www.w3.org/2001/XMLSchema#string">canary-out-of-fragment</test:identifier>
    <test:status rdf:resource="http://www.w3.org/2007/OWL/testOntology#Approved"/>
    <test:semantics rdf:resource="http://www.w3.org/2007/OWL/testOntology#DIRECT"/>
    <test:rdfXmlPremiseOntology rdf:datatype="http://www.w3.org/2001/XMLSchema#string">&lt;rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:owl="http://www.w3.org/2002/07/owl#" xml:base="http://example.org/"&gt;&lt;owl:Ontology/&gt;&lt;rdf:Description rdf:about="a"&gt;&lt;owl:sameAs rdf:resource="b"/&gt;&lt;/rdf:Description&gt;&lt;/rdf:RDF&gt;</test:rdfXmlPremiseOntology>
  </test:TestCase>
</rdf:RDF>"#;
        let report = dl_suite::run_render_roundtrip_arm(mini).expect("canary export parses");
        // 3 documents: the ∃r.B premise + the A⊑C conclusion round-trip through the REAL
        // renderer; the owl:sameAs premise is refused by L1 (out-of-fragment) — and a
        // refusal is NEVER counted as a round-trip.
        assert_eq!(report.documents, 3);
        assert_eq!(report.round_tripped, 2);
        assert_eq!(report.extraction_refused, 1);
        assert_eq!(report.parse_failed, 0);
        assert!(report.violations.is_empty(), "{:?}", report.violations);
        assert!(report.accounting_closed());
    }

    /// RDF-BASED-only SELECTION canary (sq-pbz04.4.18) — NOT a W3C case, NOT counted in
    /// any pin, and runs WITHOUT the corpus file so the selection logic is exercised on an
    /// offline checkout. A mini manifest of exactly two cases, one DUAL-tagged
    /// (DIRECT ∧ RDF-BASED) and one RDF-BASED-only, pins the discrimination in BOTH
    /// directions: the dual-tagged case belongs to the DIRECT arm ONLY, the RDF-BASED-only
    /// case to the RDF-BASED arm ONLY. Each arm therefore sees exactly ONE document.
    ///
    /// This is the test that fails if the filter regresses. Dropping the `!DIRECT`
    /// exclusion makes the RDF-BASED arm see 2 documents (double-counting the dual case);
    /// matching the wrong local name (`RDFBASED` — the export hyphenates it, `RDF-BASED`)
    /// makes it see 0.
    #[test]
    fn dl_rdf_based_selection_canary() {
        // Two in-fragment documents (A ⊑ ∃r.B and A ⊑ C), one per case, so any
        // mis-selection shows up as a document COUNT change, not as a refusal.
        let mini = r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
         xmlns:owl="http://www.w3.org/2002/07/owl#"
         xmlns:test="http://www.w3.org/2007/OWL/testOntology#">
  <test:TestCase rdf:about="http://ex/canary-dual-tagged">
    <rdf:type rdf:resource="http://www.w3.org/2007/OWL/testOntology#ConsistencyTest"/>
    <test:identifier rdf:datatype="http://www.w3.org/2001/XMLSchema#string">canary-dual-tagged</test:identifier>
    <test:status rdf:resource="http://www.w3.org/2007/OWL/testOntology#Approved"/>
    <test:semantics rdf:resource="http://www.w3.org/2007/OWL/testOntology#DIRECT"/>
    <test:semantics rdf:resource="http://www.w3.org/2007/OWL/testOntology#RDF-BASED"/>
    <test:rdfXmlPremiseOntology rdf:datatype="http://www.w3.org/2001/XMLSchema#string">&lt;rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:owl="http://www.w3.org/2002/07/owl#" xmlns:rdfs="http://www.w3.org/2000/01/rdf-schema#" xml:base="http://example.org/"&gt;&lt;owl:Ontology/&gt;&lt;owl:Class rdf:about="A"&gt;&lt;rdfs:subClassOf&gt;&lt;owl:Restriction&gt;&lt;owl:onProperty&gt;&lt;owl:ObjectProperty rdf:about="r"/&gt;&lt;/owl:onProperty&gt;&lt;owl:someValuesFrom rdf:resource="B"/&gt;&lt;/owl:Restriction&gt;&lt;/rdfs:subClassOf&gt;&lt;/owl:Class&gt;&lt;/rdf:RDF&gt;</test:rdfXmlPremiseOntology>
  </test:TestCase>
  <test:TestCase rdf:about="http://ex/canary-rdf-based-only">
    <rdf:type rdf:resource="http://www.w3.org/2007/OWL/testOntology#ConsistencyTest"/>
    <test:identifier rdf:datatype="http://www.w3.org/2001/XMLSchema#string">canary-rdf-based-only</test:identifier>
    <test:status rdf:resource="http://www.w3.org/2007/OWL/testOntology#Approved"/>
    <test:semantics rdf:resource="http://www.w3.org/2007/OWL/testOntology#RDF-BASED"/>
    <test:rdfXmlPremiseOntology rdf:datatype="http://www.w3.org/2001/XMLSchema#string">&lt;rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:owl="http://www.w3.org/2002/07/owl#" xmlns:rdfs="http://www.w3.org/2000/01/rdf-schema#" xml:base="http://example.org/"&gt;&lt;owl:Ontology/&gt;&lt;owl:Class rdf:about="A"&gt;&lt;rdfs:subClassOf&gt;&lt;owl:Class rdf:about="C"/&gt;&lt;/rdfs:subClassOf&gt;&lt;/owl:Class&gt;&lt;/rdf:RDF&gt;</test:rdfXmlPremiseOntology>
  </test:TestCase>
</rdf:RDF>"#;
        let direct = dl_suite::run_render_roundtrip_arm(mini).expect("canary export parses");
        let rdf_based =
            dl_suite::run_render_roundtrip_rdf_based_arm(mini).expect("canary export parses");
        // DIRECT arm: the dual-tagged case ONLY (the RDF-BASED-only case is not sanctioned
        // under Direct semantics).
        assert_eq!(direct.documents, 1, "DIRECT arm must see only the dual-tagged case");
        assert_eq!(direct.round_tripped, 1);
        // RDF-BASED-only arm: the RDF-BASED-only case ONLY. `2` here means the `!DIRECT`
        // exclusion was dropped and the arms now overlap.
        assert_eq!(
            rdf_based.documents, 1,
            "RDF-BASED-only arm must EXCLUDE the dual-tagged case (2 = overlapping arms, \
             0 = the RDF-BASED local name stopped matching)"
        );
        assert_eq!(rdf_based.round_tripped, 1);
        assert_eq!(rdf_based.extraction_refused, 0);
        assert_eq!(rdf_based.parse_failed, 0);
        assert!(rdf_based.violations.is_empty(), "{:?}", rdf_based.violations);
        assert!(rdf_based.accounting_closed());
        // Violation keys are segmented per arm, so concatenating two reports keeps the
        // rows distinguishable even for the same case identifier.
        assert_eq!(rdf_based.arm.label(), "RDF-Based-only");
        assert_eq!(direct.arm.label(), "Direct-Semantics");
    }

    #[test]
    fn dl_direct_arm_ratchet() {
        let export = owl_export();
        if !export.exists() {
            eprintln!(
                "SKIP: OWL WG export not present at {} — run scripts/fetch-inference-suites.sh",
                export.display()
            );
            return;
        }
        let text = std::fs::read_to_string(&export)
            .unwrap_or_else(|e| panic!("read {}: {e}", export.display()));
        let report = dl_suite::run_direct_arm(&text).expect("all.rdf parses");

        // [FABLE-5] The grep-able ratchet lines — pass count in field $7. Positional
        // `println!` args (CodeQL rust/unused-variable guard).
        println!("{}", report.render());
        println!(
            "OWL 2 DL profile-identification ratchet pass {} of {} (floor {})",
            report.profile.pass,
            report.profile.total(),
            DL_PROFILE_FLOOR
        );
        let reasoning_total = report.consistency.total()
            + report.inconsistency.total()
            + report.positive_entailment.total()
            + report.negative_entailment.total();
        println!(
            "OWL 2 DL direct ratchet pass {} of {} (floor {})",
            report.reasoning_pass(),
            reasoning_total,
            DL_DIRECT_FLOOR
        );

        // Divergence discipline (module docs): every Fail row must be a PINNED, audited
        // entry, and every pinned entry must still fail — exact set equality, both
        // lanes. An unpinned fail is an unaudited regression; a stale pin means a fix
        // landed and the floors must be deliberately re-pinned.
        let reasoning_fails: BTreeSet<String> = [
            &report.consistency,
            &report.inconsistency,
            &report.positive_entailment,
            &report.negative_entailment,
        ]
        .iter()
        .flat_map(|lane| lane.fails.iter().map(|(k, _)| k.clone()))
        .collect();
        let pinned_reasoning: BTreeSet<String> = DOCUMENTED_DIVERGENCES
            .iter()
            .map(|(k, _)| (*k).to_string())
            .collect();
        assert_eq!(
            reasoning_fails, pinned_reasoning,
            "reasoning-lane divergence set moved — unpinned new fails: {:?}; stale pins \
             (now passing/abstaining — re-audit + re-pin): {:?}",
            reasoning_fails.difference(&pinned_reasoning).collect::<Vec<_>>(),
            pinned_reasoning.difference(&reasoning_fails).collect::<Vec<_>>()
        );
        let profile_fails: BTreeSet<String> =
            report.profile.fails.iter().map(|(k, _)| k.clone()).collect();
        let pinned_profile: BTreeSet<String> =
            PROFILE_DIVERGENCES.iter().map(|(k, _)| (*k).to_string()).collect();
        assert_eq!(
            profile_fails, pinned_profile,
            "profile-lane divergence set moved — unpinned new fails: {:?}; stale pins: {:?}",
            profile_fails.difference(&pinned_profile).collect::<Vec<_>>(),
            pinned_profile.difference(&profile_fails).collect::<Vec<_>>()
        );

        // The EXACT pins (module docs: `==`, not `>=`, so abstention-inflation and
        // regression BOTH go red; re-pin deliberately when the fragment grows).
        assert_eq!(
            report.profile.pass, DL_PROFILE_FLOOR,
            "profile-identification pass count moved (got {}, pinned {}) — re-pin \
             DL_PROFILE_FLOOR deliberately with evidence",
            report.profile.pass, DL_PROFILE_FLOOR
        );
        assert_eq!(
            report.reasoning_pass(),
            DL_DIRECT_FLOOR,
            "Direct-lane pass count moved (got {}, pinned {}) — re-pin DL_DIRECT_FLOOR \
             deliberately with evidence",
            report.reasoning_pass(),
            DL_DIRECT_FLOOR
        );
        assert_eq!(
            report.profile.out_of_fragment_total(),
            DL_PROFILE_ABSTAINED,
            "profile-lane abstention total moved — the tri-state accounting is pinned"
        );
        let reasoning_abstained = report.consistency.out_of_fragment_total()
            + report.inconsistency.out_of_fragment_total()
            + report.positive_entailment.out_of_fragment_total()
            + report.negative_entailment.out_of_fragment_total();
        assert_eq!(
            reasoning_abstained, DL_DIRECT_ABSTAINED,
            "Direct-lane abstention total moved — the tri-state accounting is pinned"
        );

        // Closed accounting: every selected row is in exactly one bucket.
        assert_eq!(
            report.profile.total(),
            report.profile.pass
                + report.profile.fails.len()
                + report.profile.out_of_fragment_total()
                + report.profile.out_of_scope_total()
        );

        // ---- Explicit-negative profile lane (sq-pbz04.4.16) ----------------------------
        // These are EXACT-pinned MEASUREMENTS of the explicit-negation direction (module
        // docs). REFUTED (pass) is rise-only; the In-GAP is the honest L2 limitation and is
        // fall-only (it shrinks as L2 grows the deferred full-profile restrictions — never
        // rises without an unaudited regression). Both go RED on ANY move so a change is a
        // deliberate re-pin, exactly like the positive floors.
        println!(
            "OWL 2 DL profile explicit-negative lane: refuted {} of {} checkable (In-gap {}), abstained {}",
            report.profile_negative.pass,
            report.profile_negative.pass + report.profile_negative.fails.len(),
            report.profile_negative.fails.len(),
            report.profile_negative.out_of_fragment_total()
        );
        assert_eq!(
            report.profile_negative.pass, DL_PROFILE_NEGATIVE_REFUTED,
            "negative-lane REFUTED count moved (got {}, pinned {}) — re-pin \
             DL_PROFILE_NEGATIVE_REFUTED deliberately with evidence (rise-only)",
            report.profile_negative.pass, DL_PROFILE_NEGATIVE_REFUTED
        );
        assert_eq!(
            report.profile_negative.fails.len(),
            DL_PROFILE_NEGATIVE_IN_GAP,
            "negative-lane In-GAP moved (got {}, pinned {}) — the gap is fall-only; a RISE is \
             an unaudited regression, a FALL is a deliberate re-pin when L2 grows a sound \
             full-profile restriction",
            report.profile_negative.fails.len(),
            DL_PROFILE_NEGATIVE_IN_GAP
        );
        assert_eq!(
            report.profile_negative.out_of_fragment_total(),
            DL_PROFILE_NEGATIVE_ABSTAINED,
            "negative-lane abstention total moved — the tri-state accounting is pinned"
        );
        assert_eq!(
            report.profile_negative.out_of_scope_total(),
            DL_PROFILE_NEGATIVE_OUT_OF_SCOPE,
            "negative-lane out-of-scope total moved — the tri-state accounting is pinned"
        );
        // Closed accounting for the negative lane too.
        assert_eq!(
            report.profile_negative.total(),
            report.profile_negative.pass
                + report.profile_negative.fails.len()
                + report.profile_negative.out_of_fragment_total()
                + report.profile_negative.out_of_scope_total()
        );
    }

    /// Lane-local derivation canary — NOT a W3C case, NOT counted in any floor. Proves
    /// the arm drives REAL Direct-Semantics reasoning end-to-end (mini manifest → RDF/XML
    /// premise/conclusion → L1 extraction → L4 dispatch → ALCH refutation): a neutered
    /// checker (one that abstains everywhere, or a runner that miscounts abstentions as
    /// passes) turns the exact expected counts RED.
    #[test]
    fn dl_entailment_derivation_canary() {
        let mini = r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
         xmlns:owl="http://www.w3.org/2002/07/owl#"
         xmlns:test="http://www.w3.org/2007/OWL/testOntology#">
  <test:TestCase rdf:about="http://ex/canary-entails">
    <rdf:type rdf:resource="http://www.w3.org/2007/OWL/testOntology#PositiveEntailmentTest"/>
    <rdf:type rdf:resource="http://www.w3.org/2007/OWL/testOntology#NegativeEntailmentTest"/>
    <test:identifier rdf:datatype="http://www.w3.org/2001/XMLSchema#string">canary-entails</test:identifier>
    <test:status rdf:resource="http://www.w3.org/2007/OWL/testOntology#Approved"/>
    <test:semantics rdf:resource="http://www.w3.org/2007/OWL/testOntology#DIRECT"/>
    <test:rdfXmlPremiseOntology rdf:datatype="http://www.w3.org/2001/XMLSchema#string">&lt;rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:owl="http://www.w3.org/2002/07/owl#" xmlns:rdfs="http://www.w3.org/2000/01/rdf-schema#" xml:base="http://example.org/"&gt;&lt;owl:Ontology/&gt;&lt;owl:Class rdf:about="A"&gt;&lt;rdfs:subClassOf&gt;&lt;owl:Class rdf:about="B"/&gt;&lt;/rdfs:subClassOf&gt;&lt;/owl:Class&gt;&lt;owl:Class rdf:about="B"&gt;&lt;rdfs:subClassOf&gt;&lt;owl:Class rdf:about="C"/&gt;&lt;/rdfs:subClassOf&gt;&lt;/owl:Class&gt;&lt;/rdf:RDF&gt;</test:rdfXmlPremiseOntology>
    <test:rdfXmlConclusionOntology rdf:datatype="http://www.w3.org/2001/XMLSchema#string">&lt;rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:owl="http://www.w3.org/2002/07/owl#" xmlns:rdfs="http://www.w3.org/2000/01/rdf-schema#" xml:base="http://example.org/"&gt;&lt;owl:Ontology/&gt;&lt;owl:Class rdf:about="A"&gt;&lt;rdfs:subClassOf&gt;&lt;owl:Class rdf:about="C"/&gt;&lt;/rdfs:subClassOf&gt;&lt;/owl:Class&gt;&lt;/rdf:RDF&gt;</test:rdfXmlConclusionOntology>
    <test:rdfXmlNonConclusionOntology rdf:datatype="http://www.w3.org/2001/XMLSchema#string">&lt;rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:owl="http://www.w3.org/2002/07/owl#" xmlns:rdfs="http://www.w3.org/2000/01/rdf-schema#" xml:base="http://example.org/"&gt;&lt;owl:Ontology/&gt;&lt;owl:Class rdf:about="C"&gt;&lt;rdfs:subClassOf&gt;&lt;owl:Class rdf:about="A"/&gt;&lt;/rdfs:subClassOf&gt;&lt;/owl:Class&gt;&lt;/rdf:RDF&gt;</test:rdfXmlNonConclusionOntology>
  </test:TestCase>
</rdf:RDF>"#;
        let report = dl_suite::run_direct_arm(mini).expect("canary export parses");
        // A ⊑ B ⊑ C: the conclusion A ⊑ C is ENTAILED (refutation A⊓¬C unsatisfiable on
        // the real ALCH tableau) and the non-conclusion C ⊑ A is definitively NOT
        // entailed — both DEFINITIVE verdicts, so both rows are passes and nothing
        // abstained. Exact counts: an abstain-everywhere checker OR an
        // abstention-counted-as-pass mutation breaks these.
        assert_eq!(report.positive_entailment.pass, 1);
        assert_eq!(report.negative_entailment.pass, 1);
        assert_eq!(
            report.positive_entailment.out_of_fragment_total()
                + report.negative_entailment.out_of_fragment_total(),
            0
        );
        assert!(report.all_fails().is_empty(), "{:?}", report.all_fails());
        // And the tri-state mapping itself: an abstention maps to OutOfFragment.
        assert!(matches!(
            dl_suite::positive_entailment_tri(
                &sparq_reason_dl::check::EntailmentVerdict::Unknown(
                    sparq_reason_dl::check::UnknownReason::QlConsistencyPending
                )
            ),
            TriState::OutOfFragment(_)
        ));
    }
}
