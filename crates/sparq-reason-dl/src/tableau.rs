//! L3 (part 2) — the terminating ALCH tableau: consistency + class satisfiability.
//!
//! 🤖 SPARQ agent. Bead sq-pbz04.4.3 (epic sq-pbz04.4, design record
//! `research/owl2-direct-semantics-scoping.md` §3). A completion-forest tableau with GCI
//! internalisation, the `⊓`/`⊔`/`∃`/`∀`/GCI completion rules matched modulo the role
//! hierarchy, ancestor **subset blocking**, and chronological backtracking over
//! `⊔`-branches. The termination / soundness / completeness argument is reproduced in
//! full below (§3–§5), following Baader & Sattler, *An Overview of Tableau Algorithms
//! for Description Logics*, Studia Logica 69(1):5–40, 2001.
//!
//! # 1. Fragment (exactly ALCH — nothing more)
//!
//! - **Concepts:** named classes, `owl:Thing` (⊤), `owl:Nothing` (⊥), `⊓`, `⊔`, `¬`,
//!   `∃R.C`, `∀R.C` over **named** object properties only.
//! - **TBox:** arbitrary GCIs `C ⊑ D`; `owl:equivalentClass` as two GCIs;
//!   `owl:disjointWith` as `C ⊓ D ⊑ ⊥`; `rdfs:domain` as `∃R.⊤ ⊑ C`; `rdfs:range` as
//!   `⊤ ⊑ ∀R.C`.
//! - **RBox:** `rdfs:subPropertyOf` hierarchies over named object properties; `⊑*` is
//!   the reflexive-transitive closure of the asserted inclusions.
//! - **ABox:** ground class assertions `C(a)` and role assertions `R(a, b)`.
//! - **NOT in the fragment** (and structurally unrepresentable in [`crate::model`]):
//!   inverse roles, transitivity (opt-in-recoverable: §5a, the `dl_transitive` feature)
//!   and all other role characteristics, cardinality /
//!   functionality, nominals (`oneOf`/`hasValue`), datatypes and data properties,
//!   `owl:sameAs` / `owl:differentFrom`, property chains, keys. The L1 extractor
//!   ([`crate::extract()`]) fail-closes on all of them, so
//!   [`consistency_from_extraction`] returns [`UnknownReason::OutOfFragment`] **before
//!   the tableau starts** — an out-of-fragment input never receives a verdict.
//!
//! # 2. The procedure
//!
//! **Preprocessing.** Every TBox axiom is desugared to GCIs as above and each GCI
//! `Cᵢ ⊑ Dᵢ` is **internalised** into the universal constraint
//! `Uᵢ = nnf(¬Cᵢ) ⊔ nnf(Dᵢ)` (negation normal form via [`crate::nnf`]; NNF is an
//! equivalence-preserving rewrite). ABox class assertions are NNF'd and seeded into the
//! labels of their individuals' **root nodes**; ABox role assertions become edges
//! between roots. When there is no ABox individual, a single fresh unconstrained root is
//! created: the OWL 2 Direct Semantics interprets over **non-empty** domains, so
//! TBox-only inconsistencies such as `⊤ ⊑ ⊥` are still caught. For class
//! satisfiability of `C` an extra fresh root labelled `{nnf(C)}` is added (`C` is
//! satisfiable w.r.t. `O` iff `O ∪ {C(a_fresh)}` is consistent, `a_fresh` occurring
//! nowhere else). Every concept that can ever appear in a label is interned into a
//! fixed table up front; expansion manipulates only table indices and **cannot mint a
//! new concept** (debug-asserted against `nnf::subexpression_closure`).
//!
//! **Completion rules** (on the current label `L(x)`; `S ⊑* R` written for the role
//! closure):
//!
//! - **GCI-rule:** if `Uᵢ ∉ L(x)` for some internalised constraint: add `Uᵢ` to `L(x)`.
//! - **⊓-rule:** if `(C₁ ⊓ … ⊓ Cₙ) ∈ L(x)` and some `Cᵢ ∉ L(x)`: add every `Cᵢ`.
//! - **∀-rule:** if `∀R.C ∈ L(x)`, there is an edge `x –S→ y` with `S ⊑* R`, and
//!   `C ∉ L(y)`: add `C` to `L(y)`.
//! - **⊔-rule:** if `(C₁ ⊔ … ⊔ Cₙ) ∈ L(x)` and no `Cᵢ ∈ L(x)`: **choose** some `Cᵢ`
//!   and add it (the only nondeterministic rule; explored by chronological
//!   backtracking, disjuncts left to right).
//! - **∃-rule:** if `∃R.C ∈ L(x)`, `x` is **not blocked** (below), and there is no edge
//!   `x –S→ y` with `S ⊑* R` and `C ∈ L(y)`: create a fresh tree successor `y` of `x`
//!   with edge label `R` and `L(y) = {C}`.
//!
//! A **clash** is `⊥ ∈ L(x)` or `{A, ¬A} ⊆ L(x)` for a named class `A` (in NNF these
//! are the only inconsistent local patterns); a clash anywhere abandons the branch. A
//! branch with no applicable rule is **quiescent** and yields `Satisfiable`; when every
//! branch clashes the input is `Unsatisfiable`.
//!
//! **Blocking (ancestor subset blocking).** A node `y` is *blocked* iff some non-root
//! node `v` on the tree path from `y`'s root down to `y` (including `y` itself) has an
//! ancestor `x` in that path with `L(v) ⊆ L(x)`. This explicit form is equivalent to
//! the standard recursive definition — "`v` is *directly blocked* iff no strict
//! ancestor of `v` is directly blocked and `L(v) ⊆ L(x)` for some ancestor `x`; `y` is
//! blocked iff `y` or a strict ancestor of `y` is directly blocked" — because the
//! *topmost* `v` with the subset property is exactly the directly blocked one. Roots
//! (ABox individuals, the fresh roots) are never blocked, but a root **may serve as a
//! blocker**: soundness of that is argued in §5 (it needs no inverse roles). Blocking
//! gates **only** the ∃-rule and is re-evaluated on the *current* labels at every
//! prospective application — never cached. All other rules keep firing on and into
//! blocked nodes; in particular the ∀-rule keeps propagating into a blocked node, so
//! the subset test is never made against a stale label (labels grow monotonically: a
//! blocked node can become unblocked when its label outgrows its blocker's, and the
//! implementation must — and does — notice).
//!
//! # 3. Termination (following Baader & Sattler 2001, the ALC-with-GCIs configuration;
//! role hierarchies change nothing because blocking is label-based only)
//!
//! 1. **Finite closure.** Let `sub(O)` be the subexpression closure
//!    ([`crate::nnf::subexpression_closure`]) of the preprocessed input: all `Uᵢ`, the
//!    NNF of every ABox-asserted concept, and (for class satisfiability) `nnf(C)`.
//!    `sub(O)` is finite and linear in the input size (NNF does not duplicate
//!    subterms). Every label is always a subset of `sub(O)`: the seeds are members, and
//!    each rule adds only ⊓/⊔-members or ∃/∀-fillers of concepts already in some label
//!    (all subexpressions), or a `Uᵢ` (a seed). The implementation enforces this
//!    structurally — the closure is interned before expansion and the rules move only
//!    indices. Hence at most `2^|sub(O)|` distinct labels exist.
//! 2. **Monotone rules.** Within one branch, rules only add concepts to labels or add
//!    nodes and edges; nothing is ever removed (backtracking abandons a branch and
//!    resumes from a snapshot of an earlier choice point — it never shrinks a label
//!    within a branch). Each rule instance's applicability is extinguished by the very
//!    addition it makes, so on a fixed node set only finitely many applications occur.
//! 3. **Depth bound — where blocking is load-bearing.** Only the ∃-rule lengthens a
//!    tree path, and it is gated on the focus node being unblocked. Suppose it is about
//!    to fire on a node `y` whose tree path from its root contains more than
//!    `2^|sub(O)|` non-root nodes. By (1) and pigeonhole, two nodes on that path carry
//!    *equal* labels at this instant; the lower one, `v`, then satisfies `L(v) ⊆ L(x)`
//!    for its ancestor `x` — so `y` is blocked by the §2 definition (`v` lies on `y`'s
//!    path), and the rule does not fire: contradiction. Subset blocking can only fire
//!    *earlier* than this equality bound. Every tree path therefore stays at most
//!    `2^|sub(O)| + 1` nodes deep.
//! 4. **Node bound.** A node gains tree successors only via the ∃-rule, at most once
//!    per `∃R.C ∈ sub(O)`: after firing, the fresh `R`-successor containing `C`
//!    falsifies the rule's precondition forever (labels and edges only grow). So
//!    out-degree is bounded by `|sub(O)|` plus, at roots, the asserted ABox edges.
//!    Finitely many roots + bounded depth (3) + bounded out-degree ⇒ each branch's
//!    forest is finite; with (2), each branch performs finitely many rule applications.
//!    The ⊔-search tree is finitely branching (disjunction width) with depth bounded by
//!    the per-branch application count, so chronological backtracking visits finitely
//!    many branches. The procedure terminates on **every** input — independently of the
//!    budgets, which bound *work*, not termination. ∎
//!
//! # 4. Soundness (an `Unsatisfiable` verdict is true)
//!
//! Call a forest *satisfiable* when there is a model `I` of the ontology and a map `π`
//! from **all** forest nodes (blocked or not) into `Δ^I` with `π(n) ∈ C^I` for every
//! `C ∈ L(n)` and `(π(n), π(m)) ∈ S^I` for every edge `n –S→ m`. The initial forest is
//! satisfiable iff the input is: ABox roots carry exactly the (NNF'd,
//! equivalence-preserved) asserted facts; the fresh unconstrained root is always
//! satisfiable because Direct-Semantics domains are non-empty; the class-satisfiability
//! root encodes `O ∪ {C(a_fresh)}` as in §2. Every deterministic rule preserves forest
//! satisfiability: GCI- and ⊓-additions are entailed at `π(x)` (`I ⊨ Cᵢ ⊑ Dᵢ` makes
//! `Uᵢ` hold everywhere); a ∀-addition holds at `π(y)` because
//! `(π(x), π(y)) ∈ S^I ⊆ R^I` (`I` satisfies the role inclusions, hence their
//! composition `⊑*`); the ∃-rule extends `π` by mapping the fresh node to a witness of
//! `π(x) ∈ (∃R.C)^I`. For the ⊔-rule, some disjunct holds at `π(x)`, so **at least one
//! branch** preserves satisfiability. Because `π` covers blocked nodes too (§2: rules
//! keep firing there, and each addition is entailed at the node's `π`-image), a
//! satisfiable forest can never exhibit a clash *anywhere* — so abandoning a branch on
//! any clash is safe. `Unsatisfiable` is returned only when the exhaustive,
//! terminating (§3) search has clashed on every branch; by preservation the initial
//! forest — hence the input — has no model. ∎
//!
//! # 5. Completeness (a `Satisfiable` verdict is true)
//!
//! `Satisfiable` is returned only from a quiescent, clash-free forest `F`. Build a
//! finite interpretation `I`: for a directly blocked node `m`, let `σ(m)` be its
//! blocker (nearest ancestor `x` with `L(m) ⊆ L(x)` on the final labels); for unblocked
//! `m`, `σ(m) = m`. Take `Δ` = the unblocked nodes of `F` (non-empty: roots are never
//! blocked and at least one root always exists); `A^I = {n ∈ Δ : A ∈ L(n)}`; each ABox
//! individual denotes its root; for every edge `n –S→ m` with `n ∈ Δ`, put
//! `(n, σ(m))` into `T^I` for every `T` with `S ⊑* T`. (This is total: if `n` is
//! unblocked, every strict ancestor of `m` is unblocked, so a blocked `m` is *directly*
//! blocked and its blocker lies on `n`'s path — unblocked, hence in `Δ`.) Then for
//! every `n ∈ Δ` and `D ∈ L(n)`: `n ∈ D^I`, by induction on `D`:
//!
//! - `⊤` trivially; `⊥ ∉ L(n)` and, for literals, `{A, ¬A} ⊄ L(n)` (clash-freeness)
//!   give `A`/`¬A` via the definition of `A^I`.
//! - `⊓`/`⊔`: `n` is unblocked and quiescent, so the conjuncts / a disjunct are in
//!   `L(n)`; induction.
//! - `∃R.C ∈ L(n)`: `n` is unblocked, so quiescence means some edge `n –S→ m` with
//!   `S ⊑* R` and `C ∈ L(m)` exists. Then `(n, σ(m)) ∈ R^I` and
//!   `C ∈ L(m) ⊆ L(σ(m))` — equality when `m` is unblocked, the *subset-blocking
//!   condition on the final labels* otherwise (this is why blocking is re-evaluated
//!   dynamically and never cached). Induction gives `σ(m) ∈ C^I`.
//! - `∀R.C ∈ L(n)`: every `R^I`-edge out of `n` is `(n, σ(m))` for an `F`-edge
//!   `n –S→ m` with `S ⊑* R`; quiescence of the ∀-rule (which fires into `m` whether or
//!   not `m` is blocked) put `C ∈ L(m) ⊆ L(σ(m))`; induction. **This is the step that
//!   needs the absence of inverse roles — see below.**
//!
//! The GCI-rule keeps every `Uᵢ` in every label, so by the above every element of `Δ`
//! satisfies every internalised GCI; role inclusions hold by the `⊑*`-closure in the
//! edge construction; ABox assertions hold at the (never redirected) roots. So
//! `I ⊨ O`, and for class satisfiability the fresh root witnesses `C^I ≠ ∅`. ∎
//!
//! ## Why SUBSET blocking suffices here — and would NOT with inverse roles
//!
//! (The load-bearing note; design record §3/§5. Cf. Baader & Sattler 2001 §5;
//! Horrocks & Sattler, *A Description Logic with Transitive and Inverse Roles and Role
//! Hierarchies*, J. Logic & Computation 9(3), 1999.)
//!
//! The construction above replaces a directly blocked node `m` by its blocker
//! `x = σ(m)`: it redirects `m`'s single incoming tree edge `n –S→ m` to `n –S→ x`.
//! Every obligation this redirect must honour flows **downward along the edge**: `n`'s
//! `∀`-constraints demand certain concepts at the edge target, and `L(m) ⊆ L(x)`
//! delivers exactly them, because the ∀-rule had already pushed each such concept into
//! `L(m)`. In ALCH **no construct propagates a constraint upward against edge
//! direction**: with only named roles, giving `x` a new incoming edge imposes nothing
//! on `x`, and `x`'s label imposes nothing on its new predecessor `n`. So the only
//! guarantee a blocker must give is "at least the blocked node's concepts" — a subset
//! test. This also covers a root acting as blocker: the redirect burdens it with
//! nothing.
//!
//! With inverse roles this breaks: `∀R⁻.E ∈ L(x)` imposes `E` on every
//! `R`-**predecessor** of `x`. In `F` that obligation was discharged against `x`'s own
//! predecessor, but the redirect gives `x` a *new* predecessor `n` that never saw it —
//! and `L(m) ⊆ L(x)` is the wrong direction to transfer `x`'s inverse obligations to
//! `m`'s context (nothing forces `∀R⁻.E ∈ L(m)`, let alone `E ∈ L(n)`). A `Satisfiable`
//! verdict would then no longer be true: subset blocking must be strengthened to at
//! least **equality** blocking for ALCI, and to pairwise/dynamic blocking once number
//! restrictions are added (Horrocks & Sattler 1999). That is exactly why inverse roles
//! are a deferred, separately-argued extension in the design record's deferral ledger
//! (§5), and why [`crate::model::ObjectPropertyExpression`] has no inverse variant for
//! this module to silently mis-handle. (Transitive roles, by contrast, are the
//! first-in-line extension: absent inverses they need a `∀₊`-propagation rule but no
//! blocking change — Horrocks & Sattler 1999; implemented behind the OPT-IN
//! `dl_transitive` feature and argued in full in §5a below, sq-zfwzq.)
//!
//! # 5a. Transitive roles — the opt-in `dl_transitive` extension (sq-zfwzq)
//!
//! Under the OPT-IN `dl_transitive` cargo feature (OFF by default; with it off this module
//! compiles to exactly the ALCH tableau above) the fragment grows by TRANSITIVE role
//! declarations `Trans(T)` (`owl:TransitiveProperty`, recorded by L1 as the feature-gated
//! `Axiom::TransitiveObjectProperty` variant — a code span, not a link, because the variant
//! is compiled out by default): ALCH + role hierarchies + transitivity — Horrocks &
//! Sattler's *S with role hierarchies* (SH without inverses), still **no** inverses, number
//! restrictions, or nominals (those stay fail-closed §5-ledger deferrals in BOTH feature
//! states). ONE completion rule is added (Horrocks & Sattler 1999; Baader & Sattler 2001
//! §5):
//!
//! - **∀₊-rule:** if `∀R.C ∈ L(x)`, there is an edge `x –S→ y`, and some **transitive**
//!   role `T` has `S ⊑* T` and `T ⊑* R`, and `∀T.C ∉ L(y)`: add `∀T.C` to `L(y)`.
//!
//! Like the ∀-rule it keeps firing into blocked nodes (the subset test must never see a
//! stale label). **Blocking is UNCHANGED** — ancestor subset blocking, gating only the
//! ∃-rule. That is the load-bearing Horrocks–Sattler claim, and it is *argued* below (the
//! soundness/completeness/termination deltas), not asserted.
//!
//! ## Termination (§3 extended)
//!
//! Extend the interned universe to the ∀₊-closure
//! `sub⁺(O) = sub(O) ∪ { ∀T.C : ∀R.C ∈ sub⁺(O), Trans(T), T ⊑* R }`. The fixpoint is
//! finite: every generated concept is `∀T.C` for a transitive input role `T` and the filler
//! `C` of a `∀`-member of `sub(O)`, so `|sub⁺(O)| ≤ |sub(O)| · (1 + |{T : Trans(T)}|)`;
//! chained generation closes after one round because `T' ⊑* T` and `T ⊑* R` give
//! `T' ⊑* R` (`⊑*` is transitive), so the `∀T'.C` a generated `∀T.C` would demand was
//! already generated from the original `∀R.C`. `preprocess` pre-interns `sub⁺(O)` (the
//! §3-step-1 debug assertion is extended by the same seeds) and expansion still moves only
//! table indices — no rule can mint a concept outside `sub⁺(O)`. The ∀₊-rule is monotone
//! and self-extinguishing exactly like the ∀-rule, so §3 steps 2–4 go through verbatim with
//! `sub(O)` replaced by the finite `sub⁺(O)`: every tree path stays at most
//! `2^|sub⁺(O)| + 1` nodes deep and the search terminates on **every** input. ∎
//!
//! ## Soundness (§4 extended)
//!
//! With `I`, `π` as in §4, every model of a transitive input interprets `T^I` transitively.
//! Suppose the ∀₊-rule fires: `∀R.C ∈ L(x)`, edge `x –S→ y`, `Trans(T)`, `S ⊑* T ⊑* R`.
//! For any `T`-successor `z` of `π(y)`: `(π(x), π(y)) ∈ S^I ⊆ T^I` and `(π(y), z) ∈ T^I`,
//! so transitivity gives `(π(x), z) ∈ T^I ⊆ R^I`, hence `z ∈ C^I` from
//! `π(x) ∈ (∀R.C)^I`. So `π(y) ∈ (∀T.C)^I`: the addition is entailed at `π(y)` and forest
//! satisfiability is preserved; the rest of §4 is unchanged. ∎
//!
//! ## Completeness (§5 extended)
//!
//! The §5 construction must now interpret transitive roles transitively. From a quiescent,
//! clash-free forest define per role `R` the edge set
//! `E(R) = { (n, σ(m)) : n ∈ Δ, F-edge n –S→ m, S ⊑* R }` (exactly the §5 edges) and
//! interpret `R^I = E(R) ∪ ⋃ { E(T)⁺ : Trans(T), T ⊑* R }` (`⁺` = transitive closure).
//! Then (a) **role inclusions hold**: `R ⊑* Q` gives `E(R) ⊆ E(Q)` and every transitive
//! `T ⊑* R` also has `T ⊑* Q`, so `R^I ⊆ Q^I`; (b) **every transitive `T` is transitive
//! in `I`**: each `T' ⊑* T` gives `E(T') ⊆ E(T)`, hence `E(T')⁺ ⊆ E(T)⁺`, so
//! `T^I = E(T)⁺` — a transitive closure. The §5 induction changes in exactly one case,
//! `∀R.C ∈ L(n)` with `(n, z) ∈ R^I`:
//!
//! - `(n, z) ∈ E(R)`: as in §5 — quiescence of the ∀-rule put `C ∈ L(m) ⊆ L(σ(m))`.
//! - `(n, z) ∈ E(T)⁺`, `Trans(T)`, `T ⊑* R`: there is an `E(T)`-path
//!   `n = x₀ → x₁ → … → x_k = z` (`k ≥ 1`), each step `(xᵢ, xᵢ₊₁) = (xᵢ, σ(mᵢ))` for an
//!   `F`-edge `xᵢ –Sᵢ→ mᵢ` with `Sᵢ ⊑* T` and `xᵢ ∈ Δ`. If `k = 1`, the plain ∀-rule at
//!   `x₀ = n` (with `S₀ ⊑* T ⊑* R`) put `C ∈ L(m₀) ⊆ L(σ(m₀)) = L(z)`. Otherwise the
//!   ∀₊-rule at `n` (`S₀ ⊑* T`, `Trans(T)`, `T ⊑* R`) put `∀T.C ∈ L(m₀) ⊆ L(x₁)`, and
//!   inductively along the path: from `∀T.C ∈ L(xᵢ)` (`1 ≤ i < k`), the ∀₊-rule with
//!   `Sᵢ ⊑* T`, `Trans(T)`, `T ⊑* T` re-propagates `∀T.C ∈ L(mᵢ) ⊆ L(xᵢ₊₁)`, and at the
//!   final step the plain ∀-rule on `∀T.C ∈ L(x_{k−1})` puts
//!   `C ∈ L(m_{k−1}) ⊆ L(σ(m_{k−1})) = L(z)`. Induction on `C` finishes as in §5.
//!
//! Every obligation above transfers **downward** along an edge via `L(m) ⊆ L(σ(m))` — the
//! subset-blocking condition on the final labels — including the NEW `∀T.C` obligations the
//! ∀₊-rule pushed into every blocked node (it fires into blocked nodes, and subset blocking
//! is re-evaluated on current labels). Nothing propagates upward against edge direction, so
//! subset blocking remains sufficient — exactly the "absent inverses" proviso of Horrocks &
//! Sattler 1999; with inverses this section's argument would fail at the same point §5's
//! does. ∎
//!
//! # 6. Budgets and determinism
//!
//! Budgets are **deterministic counts only** ([`Budget`]): `max_nodes` bounds the total
//! number of forest nodes ever created over the whole search (roots included; never
//! decremented on backtracking), `max_rule_applications` bounds completion-rule
//! applications (including each ⊔-disjunct trial and re-applications after
//! backtracking). Exhaustion returns [`UnknownReason::ResourceBudget`] — **never a
//! verdict**. Wall-clock budgets are banned by design (design record §3: conformance
//! floors must be reproducible). All scan orders are fixed (nodes and edges in creation
//! order, label concepts in interning order, universal constraints in axiom order,
//! disjuncts left to right), so for a given [`crate::model::Ontology`] value and
//! [`Budget`] the outcome — including *which* budget trips — is identical on every run.
//!
//! # 7. Honest scope
//!
//! This module decides consistency / class satisfiability **only for the exact ALCH
//! fragment of §1** — extended by transitive roles per §5a when (and only when) the
//! opt-in `dl_transitive` feature is enabled — as delivered by the L1 structural model.
//! It is **not** an OWL 2 DL
//! reasoner, and no claim of soundness or completeness is made beyond the argued
//! fragment. Anything the L1 extractor could not map — genuinely out-of-fragment
//! constructs *or* malformed encodings — yields `Unknown(OutOfFragment)`, fail-closed,
//! before any reasoning. Within the fragment, the unique-name assumption is neither
//! made nor needed: no ALCH construct can force two individuals to be equal or
//! distinct, and the §5 construction simply keeps roots distinct. Concept
//! satisfiability w.r.t. general TBoxes in ALC is EXPTIME-complete (Schild's PDL
//! correspondence; *The Description Logic Handbook*, 2nd ed., ch. 2–3), and role
//! hierarchies do not change this; this implementation is a straightforward blocking
//! tableau and is **not** claimed worst-case optimal — the budgets exist to bound its
//! work deterministically.
//!
//! # 8. References
//!
//! - F. Baader, U. Sattler: *An Overview of Tableau Algorithms for Description
//!   Logics*. Studia Logica 69(1):5–40, 2001. (Completion rules, subset blocking for
//!   ALC with general TBoxes, termination/soundness/completeness scheme.)
//! - I. Horrocks, U. Sattler: *A Description Logic with Transitive and Inverse Roles
//!   and Role Hierarchies*. J. Logic & Computation 9(3):385–410, 1999. (Why inverses
//!   force stronger blocking; role hierarchies in the rules.)
//! - Baader, Calvanese, McGuinness, Nardi, Patel-Schneider (eds.): *The Description
//!   Logic Handbook*, 2nd ed., ch. 2–3. (ALC complexity, blocking taxonomy.)
//! - W3C OWL 2 Direct Semantics (non-empty domains):
//!   `<https://www.w3.org/TR/owl2-direct-semantics/>`.
//! - In-repo: `research/owl2-direct-semantics-scoping.md` §3 (the spec this module
//!   implements) and §5 (the deferral ledger).

use crate::extract::ExtractError;
use crate::model::{Axiom, ClassExpression, Ontology};
use crate::nnf::{is_nnf, nnf, nnf_complement, subexpression_closure};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::Id;
use std::collections::BTreeSet;

// -------------------------------------------------------------------------------------------
// Public API
// -------------------------------------------------------------------------------------------

/// Deterministic, count-based work budget for one tableau run (module docs §6).
///
/// Both limits are counts of work performed across the WHOLE backtracking search and are
/// never decremented; exhaustion yields [`Verdict::Unknown`] with
/// [`UnknownReason::ResourceBudget`], never a verdict. Wall-clock budgets are banned by
/// design (they would make conformance floors non-reproducible).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Maximum total number of forest nodes ever created (roots included). If the ABox
    /// alone needs more roots than this, the run returns `Unknown` immediately — unless
    /// the asserted facts already clash while seeding, in which case `Unsatisfiable` is
    /// returned first (the verdict is sound: a seed clash needs no further expansion).
    pub max_nodes: usize,
    /// Maximum total number of completion-rule applications, counting each concept
    /// addition, each ⊔-disjunct trial, and each ∃-expansion — including work re-done
    /// after backtracking.
    pub max_rule_applications: usize,
}

impl Default for Budget {
    /// Generous deterministic defaults for interactive use (10 000 nodes / 200 000 rule
    /// applications). The values are arbitrary counts, not tuned constants; callers with
    /// reproducibility requirements (e.g. conformance lanes) should pin their own.
    fn default() -> Budget {
        Budget {
            max_nodes: 10_000,
            max_rule_applications: 200_000,
        }
    }
}

/// Which count-based budget was exhausted (payload of [`UnknownReason::ResourceBudget`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExhaustedBudget {
    /// [`Budget::max_nodes`] was reached.
    Nodes,
    /// [`Budget::max_rule_applications`] was reached.
    RuleApplications,
}

/// Why the checker abstained ([`Verdict::Unknown`]). Fail-closed: neither reason ever
/// degrades into a verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnknownReason {
    /// The input RDF graph could not be mapped into the ALCH structural fragment by the
    /// L1 extractor — either a genuinely out-of-fragment construct (inverses,
    /// cardinality, nominals, datatypes, `sameAs`, …) or a malformed encoding. The
    /// payload is the rendered [`ExtractError`]. The tableau never ran: it must not
    /// reason over a graph it only partially understood.
    OutOfFragment(String),
    /// A deterministic count budget was exhausted mid-search (module docs §6). The
    /// partial search says nothing about consistency either way.
    ResourceBudget(ExhaustedBudget),
}

/// The tri-state outcome of a tableau run.
///
/// For [`consistency`], `Satisfiable` means the ontology is CONSISTENT and
/// `Unsatisfiable` means INCONSISTENT (an ontology is consistent iff its constraint
/// system is satisfiable). For [`class_satisfiability`], they mean the class is
/// satisfiable / unsatisfiable w.r.t. the ontology. Both verdicts are backed by the
/// module-doc argument (§4/§5) and are made ONLY within the ALCH fragment (§1/§7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// A model exists (ontology consistent / class satisfiable). True by §5.
    Satisfiable,
    /// No model exists (ontology inconsistent / class unsatisfiable). True by §4.
    Unsatisfiable,
    /// No verdict — out-of-fragment input or budget exhaustion; see [`UnknownReason`].
    Unknown(UnknownReason),
}

impl Verdict {
    /// `true` iff the verdict is [`Verdict::Satisfiable`].
    #[must_use]
    pub fn is_satisfiable(&self) -> bool {
        matches!(self, Verdict::Satisfiable)
    }

    /// `true` iff the verdict is [`Verdict::Unsatisfiable`].
    #[must_use]
    pub fn is_unsatisfiable(&self) -> bool {
        matches!(self, Verdict::Unsatisfiable)
    }

    /// `true` iff the checker abstained ([`Verdict::Unknown`]).
    #[must_use]
    pub fn is_unknown(&self) -> bool {
        matches!(self, Verdict::Unknown(_))
    }
}

/// Decide consistency of an ALCH structural ontology under a deterministic budget.
///
/// Returns [`Verdict::Satisfiable`] (consistent) or [`Verdict::Unsatisfiable`]
/// (inconsistent) — correct by the module-doc argument (§4/§5) — or
/// [`Verdict::Unknown`] with [`UnknownReason::ResourceBudget`] when the budget is
/// exhausted first. The input [`Ontology`] is already fragment-restricted by
/// construction (L1); RDF-level inputs should go through
/// [`consistency_from_extraction`] for the fail-closed out-of-fragment path.
///
/// The empty ontology is consistent. An ontology with an empty ABox is checked over a
/// single fresh root (Direct-Semantics domains are non-empty, so e.g. `⊤ ⊑ ⊥` with no
/// individuals is still inconsistent).
#[must_use]
pub fn consistency(onto: &Ontology, budget: Budget) -> Verdict {
    let (sys, forest, seed_clash) = preprocess(onto, None);
    if seed_clash {
        // The asserted ABox facts alone are contradictory ({A(a), ¬A(a)} or ⊥(a)).
        return Verdict::Unsatisfiable;
    }
    run(&sys, forest, budget)
}

/// Decide satisfiability of a class expression w.r.t. an ALCH structural ontology
/// (TBox + RBox + ABox) under a deterministic budget.
///
/// Implemented as consistency of `O ∪ {C(a_fresh)}` with `a_fresh` occurring nowhere
/// else — an exact reduction (module docs §2). Consequently, if the ontology itself is
/// inconsistent, EVERY class is unsatisfiable w.r.t. it, including `⊤`. `class` may be
/// any ALCH class expression; it is NNF'd internally.
#[must_use]
pub fn class_satisfiability(class: &ClassExpression, onto: &Ontology, budget: Budget) -> Verdict {
    let (sys, forest, seed_clash) = preprocess(onto, Some(class));
    if seed_clash {
        return Verdict::Unsatisfiable;
    }
    run(&sys, forest, budget)
}

/// Fail-closed entry point from an L1 extraction result (module docs §1/§7).
///
/// `Err(e)` — the RDF graph contains anything the ALCH mapping could not understand —
/// yields `Unknown(OutOfFragment)` carrying the rendered error, BEFORE any tableau
/// work: the checker never reasons over a graph it only partially understood. `Ok`
/// delegates to [`consistency`].
#[must_use]
pub fn consistency_from_extraction(
    result: &Result<Ontology, ExtractError>,
    budget: Budget,
) -> Verdict {
    match result {
        Ok(onto) => consistency(onto, budget),
        Err(e) => Verdict::Unknown(UnknownReason::OutOfFragment(format!("{}", e))),
    }
}

// -------------------------------------------------------------------------------------------
// Interned concept table
// -------------------------------------------------------------------------------------------

/// Index into [`System::shapes`] — an interned NNF class expression.
type ConceptId = u32;

/// The decomposed shape of an interned NNF concept. Children are interned ids, so the
/// completion rules never re-examine expression trees (and can never mint a concept
/// outside the pre-interned closure — termination step 1).
enum Shape {
    /// A named class. (The class's dict id is not stored: the completion rules treat
    /// atoms opaquely by interned id; the expression itself remains retrievable as the
    /// interning key.)
    Atom,
    /// ⊤.
    Top,
    /// ⊥ — its presence in any label is a clash.
    Bottom,
    /// `¬A` for the named-class concept with the given id (NNF guarantees the inner
    /// expression is named).
    NegAtom(ConceptId),
    /// `C₁ ⊓ … ⊓ Cₙ`.
    And(Vec<ConceptId>),
    /// `C₁ ⊔ … ⊔ Cₙ`.
    Or(Vec<ConceptId>),
    /// `∃R.C` over the named property `R`.
    Exists(Id, ConceptId),
    /// `∀R.C` over the named property `R`.
    ForAll(Id, ConceptId),
}

/// The preprocessed, immutable constraint system one tableau run works over.
#[derive(Default)]
struct System {
    /// Interned concept shapes; `ConceptId` indexes this.
    shapes: Vec<Shape>,
    /// Expression → id (structural dedup).
    ids: FxHashMap<ClassExpression, ConceptId>,
    /// `A ↔ ¬A` pairs (both directions); a label holding both members is a clash.
    complements: FxHashMap<ConceptId, ConceptId>,
    /// Internalised GCI constraints `Uᵢ`, deduplicated, in axiom order.
    universals: Vec<ConceptId>,
    /// Strict super-roles per role (transitive closure of `rdfs:subPropertyOf`);
    /// reflexivity is handled in [`System::is_subrole`].
    super_roles: FxHashMap<Id, FxHashSet<Id>>,
    /// Roles asserted transitive (`Trans(T)`; module docs §5a). sq-zfwzq.
    #[cfg(feature = "dl_transitive")]
    transitive_roles: FxHashSet<Id>,
    /// ∀₊-propagation table (module docs §5a): the interned id of each `∀R.C` ↦ the
    /// `(T, id of ∀T.C)` pairs for every transitive `T ⊑* R`, in ascending-`T` order
    /// (deterministic scan order, §6). Built once in `preprocess` over the pre-interned
    /// `sub⁺(O)` closure; empty when the input declares no transitive role.
    #[cfg(feature = "dl_transitive")]
    forall_plus: FxHashMap<ConceptId, Vec<(Id, ConceptId)>>,
}

impl System {
    /// Intern an NNF class expression, recursively interning its subexpressions.
    fn intern(&mut self, ce: &ClassExpression) -> ConceptId {
        if let Some(&id) = self.ids.get(ce) {
            return id;
        }
        let shape = match ce {
            ClassExpression::Class(_) => Shape::Atom,
            ClassExpression::Thing => Shape::Top,
            ClassExpression::Nothing => Shape::Bottom,
            ClassExpression::ObjectIntersectionOf(members) => {
                Shape::And(members.iter().map(|m| self.intern(m)).collect())
            }
            ClassExpression::ObjectUnionOf(members) => {
                Shape::Or(members.iter().map(|m| self.intern(m)).collect())
            }
            ClassExpression::ObjectComplementOf(inner) => {
                debug_assert!(
                    matches!(inner.as_ref(), ClassExpression::Class(_)),
                    "tableau interner requires NNF input"
                );
                Shape::NegAtom(self.intern(inner))
            }
            ClassExpression::ObjectSomeValuesFrom(p, filler) => {
                Shape::Exists(p.named(), self.intern(filler))
            }
            ClassExpression::ObjectAllValuesFrom(p, filler) => {
                Shape::ForAll(p.named(), self.intern(filler))
            }
        };
        let id = self.shapes.len() as ConceptId;
        if let Shape::NegAtom(atom) = &shape {
            self.complements.insert(*atom, id);
            self.complements.insert(id, *atom);
        }
        self.shapes.push(shape);
        self.ids.insert(ce.clone(), id);
        id
    }

    /// `true` iff `sub ⊑* sup` in the reflexive-transitive role closure.
    fn is_subrole(&self, sub: Id, sup: Id) -> bool {
        sub == sup
            || self
                .super_roles
                .get(&sub)
                .is_some_and(|sups| sups.contains(&sup))
    }
}

// -------------------------------------------------------------------------------------------
// Completion forest
// -------------------------------------------------------------------------------------------

/// One forest node: a label (interned concepts, ordered by interning index for
/// deterministic iteration) and an optional tree parent (`None` = root).
#[derive(Clone)]
struct Node {
    label: BTreeSet<ConceptId>,
    parent: Option<usize>,
}

/// The completion forest of one search branch: nodes in creation order, plus ALL edges
/// (root–root ABox edges and ∃-generated tree edges) as `(source, role, target)`.
#[derive(Clone)]
struct Forest {
    nodes: Vec<Node>,
    edges: Vec<(usize, Id, usize)>,
}

impl Forest {
    /// Insert `cid` into `L(n)`; returns `true` on a CLASH (⊥ added, or the complement
    /// of `cid` already present). No-op (and no clash) if already present.
    fn add(&mut self, n: usize, cid: ConceptId, sys: &System) -> bool {
        if !self.nodes[n].label.insert(cid) {
            return false;
        }
        if matches!(sys.shapes[cid as usize], Shape::Bottom) {
            return true;
        }
        if let Some(comp) = sys.complements.get(&cid) {
            return self.nodes[n].label.contains(comp);
        }
        false
    }

    /// Ancestor subset blocking, evaluated on the CURRENT labels (module docs §2): `y`
    /// is blocked iff some non-root node `v` on the tree path root‥y (including `y`)
    /// has an ancestor `x` with `L(v) ⊆ L(x)`. Roots are never blocked.
    fn is_blocked(&self, y: usize) -> bool {
        let mut v = y;
        loop {
            let Some(parent) = self.nodes[v].parent else {
                return false; // reached the root: no more non-root nodes on the path
            };
            let mut x = Some(parent);
            while let Some(xi) = x {
                if self.nodes[v].label.is_subset(&self.nodes[xi].label) {
                    return true;
                }
                x = self.nodes[xi].parent;
            }
            v = parent;
        }
    }
}

// -------------------------------------------------------------------------------------------
// Preprocessing (module docs §2)
// -------------------------------------------------------------------------------------------

/// Internalise a GCI `sub ⊑ sup` as the NNF universal constraint `nnf(¬sub) ⊔ nnf(sup)`.
fn internalise(sub: &ClassExpression, sup: &ClassExpression) -> ClassExpression {
    ClassExpression::ObjectUnionOf(vec![nnf_complement(sub), nnf(sup)])
}

/// Build the constraint system and initial forest from a structural ontology, plus an
/// optional extra fresh root for class satisfiability. Returns `(system, forest,
/// seed_clash)` where `seed_clash` means the asserted ABox facts alone already clash.
fn preprocess(onto: &Ontology, extra: Option<&ClassExpression>) -> (System, Forest, bool) {
    let mut sys = System::default();
    let mut forest = Forest {
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut roots: FxHashMap<Id, usize> = FxHashMap::default();
    let mut universal_set: FxHashSet<ConceptId> = FxHashSet::default();
    let mut seeds: Vec<ClassExpression> = Vec::new(); // closure cross-check (§3 step 1)
    let mut seed_clash = false;

    // RBox first: the transitive role closure (independent of the concept table).
    let mut direct: FxHashMap<Id, FxHashSet<Id>> = FxHashMap::default();
    for axiom in onto.axioms() {
        if let Axiom::SubObjectPropertyOf { sub, sup } = axiom {
            direct.entry(sub.named()).or_default().insert(sup.named());
        }
        // sq-zfwzq (opt-in `dl_transitive`): record `Trans(T)` declarations for
        // the ∀₊-rule (module docs §5a).
        #[cfg(feature = "dl_transitive")]
        if let Axiom::TransitiveObjectProperty { property } = axiom {
            sys.transitive_roles.insert(property.named());
        }
    }
    for &start in direct.keys() {
        let mut reach: FxHashSet<Id> = FxHashSet::default();
        let mut work: Vec<Id> = direct[&start].iter().copied().collect();
        while let Some(next) = work.pop() {
            if reach.insert(next) {
                if let Some(sups) = direct.get(&next) {
                    work.extend(sups.iter().copied());
                }
            }
        }
        sys.super_roles.insert(start, reach);
    }

    // TBox internalisation + ABox seeding, in axiom order (deterministic).
    let mut push_universal =
        |sys: &mut System, seeds: &mut Vec<ClassExpression>, u: ClassExpression| {
            let cid = sys.intern(&u);
            seeds.push(u);
            if universal_set.insert(cid) {
                sys.universals.push(cid);
            }
        };
    fn root_of(forest: &mut Forest, roots: &mut FxHashMap<Id, usize>, ind: Id) -> usize {
        *roots.entry(ind).or_insert_with(|| {
            forest.nodes.push(Node {
                label: BTreeSet::new(),
                parent: None,
            });
            forest.nodes.len() - 1
        })
    }
    for axiom in onto.axioms() {
        match axiom {
            Axiom::SubClassOf { sub, sup } => {
                push_universal(&mut sys, &mut seeds, internalise(sub, sup));
            }
            Axiom::EquivalentClasses(left, right) => {
                push_universal(&mut sys, &mut seeds, internalise(left, right));
                push_universal(&mut sys, &mut seeds, internalise(right, left));
            }
            Axiom::DisjointClasses(left, right) => {
                // left ⊓ right ⊑ ⊥ (design record §3 desugaring).
                let both =
                    ClassExpression::ObjectIntersectionOf(vec![left.clone(), right.clone()]);
                push_universal(
                    &mut sys,
                    &mut seeds,
                    internalise(&both, &ClassExpression::Nothing),
                );
            }
            Axiom::SubObjectPropertyOf { .. } => {} // handled above
            #[cfg(feature = "dl_transitive")]
            Axiom::TransitiveObjectProperty { .. } => {} // handled above (RBox pass)
            Axiom::ObjectPropertyDomain { property, domain } => {
                // ∃R.⊤ ⊑ domain.
                let some_top = ClassExpression::some(property.named(), ClassExpression::Thing);
                push_universal(&mut sys, &mut seeds, internalise(&some_top, domain));
            }
            Axiom::ObjectPropertyRange { property, range } => {
                // ⊤ ⊑ ∀R.range.
                let all_range = ClassExpression::only(property.named(), range.clone());
                push_universal(
                    &mut sys,
                    &mut seeds,
                    internalise(&ClassExpression::Thing, &all_range),
                );
            }
            Axiom::ClassAssertion { class, individual } => {
                let n = root_of(&mut forest, &mut roots, *individual);
                let seeded = nnf(class);
                let cid = sys.intern(&seeded);
                seeds.push(seeded);
                if forest.add(n, cid, &sys) {
                    seed_clash = true;
                }
            }
            Axiom::ObjectPropertyAssertion {
                property,
                source,
                target,
            } => {
                let s = root_of(&mut forest, &mut roots, *source);
                let t = root_of(&mut forest, &mut roots, *target);
                forest.edges.push((s, property.named(), t));
            }
        }
    }

    // Class-satisfiability root, then the non-empty-domain root (module docs §2).
    if let Some(class) = extra {
        forest.nodes.push(Node {
            label: BTreeSet::new(),
            parent: None,
        });
        let n = forest.nodes.len() - 1;
        let seeded = nnf(class);
        let cid = sys.intern(&seeded);
        seeds.push(seeded);
        if forest.add(n, cid, &sys) {
            seed_clash = true;
        }
    } else if forest.nodes.is_empty() {
        forest.nodes.push(Node {
            label: BTreeSet::new(),
            parent: None,
        });
    }

    // sq-zfwzq (opt-in `dl_transitive`): pre-intern the ∀₊-closure `sub⁺(O)` and
    // build the ∀₊-propagation table (module docs §5a). For every `∀R.C` in the closure and
    // every transitive `T ⊑* R`, the ∀₊-rule may add `∀T.C`, so that concept must be part of
    // the interned universe BEFORE expansion (termination step 1 stays structural: the rules
    // move only indices). Generated `∀T.C` concepts are themselves processed (their own
    // table entries: `T' ⊑* T` transitive), to the finite fixpoint argued in §5a — at most
    // one `∀T.C` per (transitive role, ∀-filler) pair. Deterministic: the transitive roles
    // are scanned in ascending id order and the worklist order is derived from the
    // deterministic first-visit preorder of `subexpression_closure`.
    #[cfg(feature = "dl_transitive")]
    {
        let mut trans_sorted: Vec<Id> = sys.transitive_roles.iter().copied().collect();
        trans_sorted.sort_unstable();
        if !trans_sorted.is_empty() {
            let mut work: Vec<ClassExpression> = subexpression_closure(&seeds)
                .into_iter()
                .filter(|ce| matches!(ce, ClassExpression::ObjectAllValuesFrom(_, _)))
                .collect();
            let mut processed: FxHashSet<ConceptId> = FxHashSet::default();
            while let Some(ce) = work.pop() {
                let ClassExpression::ObjectAllValuesFrom(p, filler) = &ce else {
                    unreachable!("the ∀₊ worklist holds only ∀-concepts");
                };
                let parent = sys.intern(&ce); // already interned (closure member) — lookup
                if !processed.insert(parent) {
                    continue;
                }
                let r = p.named();
                let mut entries: Vec<(Id, ConceptId)> = Vec::new();
                for &t in &trans_sorted {
                    if sys.is_subrole(t, r) {
                        let all_t = ClassExpression::only(t, filler.as_ref().clone());
                        let known = sys.ids.contains_key(&all_t);
                        let tid = sys.intern(&all_t);
                        if !known {
                            // A genuinely new concept: keep the §3-step-1 cross-check exact
                            // (its own subexpressions — the filler — are already interned)
                            // and process it for its own ∀₊ entries.
                            seeds.push(all_t.clone());
                            work.push(all_t);
                        }
                        entries.push((t, tid));
                    }
                }
                if !entries.is_empty() {
                    sys.forall_plus.insert(parent, entries);
                }
            }
        }
    }

    // §3 step 1, enforced structurally: the interner now holds EXACTLY the (NNF)
    // subexpression closure of the seeds, and expansion below moves only indices.
    debug_assert!(seeds.iter().all(is_nnf), "preprocessing must emit NNF seeds");
    debug_assert_eq!(
        subexpression_closure(&seeds).len(),
        sys.shapes.len(),
        "interned table must equal the subexpression closure of the seeds"
    );

    (sys, forest, seed_clash)
}

// -------------------------------------------------------------------------------------------
// The search (module docs §2/§6)
// -------------------------------------------------------------------------------------------

/// Monotone work counters for one whole search (never decremented on backtracking).
struct Search {
    budget: Budget,
    nodes_created: usize,
    rules_applied: usize,
}

impl Search {
    fn count_rule(&mut self) -> Result<(), ExhaustedBudget> {
        if self.rules_applied >= self.budget.max_rule_applications {
            return Err(ExhaustedBudget::RuleApplications);
        }
        self.rules_applied += 1;
        Ok(())
    }

    fn count_node(&mut self) -> Result<(), ExhaustedBudget> {
        if self.nodes_created >= self.budget.max_nodes {
            return Err(ExhaustedBudget::Nodes);
        }
        self.nodes_created += 1;
        Ok(())
    }
}

/// What one saturation pass over a branch ended in.
enum Step {
    /// No rule applicable: quiescent, clash-free — Satisfiable (module docs §5).
    Quiescent,
    /// A clash: this branch is dead (module docs §4).
    Clash,
    /// A ⊔-rule needs a choice: branch over `disjuncts` at `node`.
    Choice { node: usize, disjuncts: Vec<ConceptId> },
}

/// Apply deterministic rules (GCI, ⊓, ∀) to fixpoint, then surface the first ⊔-choice,
/// else apply the first unblocked ∃ and repeat, else report quiescence. Scan orders are
/// fixed for determinism (module docs §6).
fn saturate(f: &mut Forest, sys: &System, s: &mut Search) -> Result<Step, ExhaustedBudget> {
    loop {
        // Deterministic rules to fixpoint.
        let mut changed = true;
        while changed {
            changed = false;
            // GCI-rule: every internalised constraint into every node's label.
            for n in 0..f.nodes.len() {
                for &u in &sys.universals {
                    if !f.nodes[n].label.contains(&u) {
                        s.count_rule()?;
                        if f.add(n, u, sys) {
                            return Ok(Step::Clash);
                        }
                        changed = true;
                    }
                }
            }
            // ⊓-rule.
            for n in 0..f.nodes.len() {
                let ids: Vec<ConceptId> = f.nodes[n].label.iter().copied().collect();
                for cid in ids {
                    if let Shape::And(members) = &sys.shapes[cid as usize] {
                        for &m in members {
                            if !f.nodes[n].label.contains(&m) {
                                s.count_rule()?;
                                if f.add(n, m, sys) {
                                    return Ok(Step::Clash);
                                }
                                changed = true;
                            }
                        }
                    }
                }
            }
            // ∀-rule, matched modulo the role hierarchy; fires into blocked nodes too
            // (module docs §2: the subset test must never see a stale label).
            for ei in 0..f.edges.len() {
                let (src, role, tgt) = f.edges[ei];
                let ids: Vec<ConceptId> = f.nodes[src].label.iter().copied().collect();
                for cid in ids {
                    if let Shape::ForAll(r, filler) = &sys.shapes[cid as usize] {
                        let (r, filler) = (*r, *filler);
                        if sys.is_subrole(role, r) && !f.nodes[tgt].label.contains(&filler) {
                            s.count_rule()?;
                            if f.add(tgt, filler, sys) {
                                return Ok(Step::Clash);
                            }
                            changed = true;
                        }
                        // sq-zfwzq ∀₊-rule (module docs §5a, opt-in
                        // `dl_transitive`): for `∀R.C ∈ L(src)` and a transitive `T` with
                        // `S ⊑* T ⊑* R` (the `T ⊑* R` half is pre-baked into the table
                        // entry), add `∀T.C` to `L(tgt)` — the Horrocks–Sattler propagation
                        // that carries ∀-constraints along transitive chains. Like the plain
                        // ∀-rule it fires into blocked nodes (the subset test must never see
                        // a stale label), and each addition is counted against the budget.
                        #[cfg(feature = "dl_transitive")]
                        if let Some(entries) = sys.forall_plus.get(&cid) {
                            for &(t, all_tc) in entries {
                                if sys.is_subrole(role, t)
                                    && !f.nodes[tgt].label.contains(&all_tc)
                                {
                                    s.count_rule()?;
                                    if f.add(tgt, all_tc, sys) {
                                        return Ok(Step::Clash);
                                    }
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        // ⊔-rule: first unresolved disjunction in scan order.
        for (n, node) in f.nodes.iter().enumerate() {
            for &cid in &node.label {
                if let Shape::Or(members) = &sys.shapes[cid as usize] {
                    if !members.iter().any(|m| node.label.contains(m)) {
                        return Ok(Step::Choice {
                            node: n,
                            disjuncts: members.clone(),
                        });
                    }
                }
            }
        }
        // ∃-rule: first unsatisfied existential on an UNBLOCKED node (the only rule
        // blocking gates; blocking evaluated on current labels, right now).
        let mut expanded = false;
        'nodes: for n in 0..f.nodes.len() {
            let ids: Vec<ConceptId> = f.nodes[n].label.iter().copied().collect();
            for cid in ids {
                if let Shape::Exists(r, filler) = &sys.shapes[cid as usize] {
                    let (r, filler) = (*r, *filler);
                    let satisfied = f.edges.iter().any(|&(src, erole, tgt)| {
                        src == n && sys.is_subrole(erole, r) && f.nodes[tgt].label.contains(&filler)
                    });
                    if satisfied || f.is_blocked(n) {
                        continue;
                    }
                    s.count_rule()?;
                    s.count_node()?;
                    let fresh = f.nodes.len();
                    f.nodes.push(Node {
                        label: BTreeSet::new(),
                        parent: Some(n),
                    });
                    f.edges.push((n, r, fresh));
                    if f.add(fresh, filler, sys) {
                        return Ok(Step::Clash);
                    }
                    expanded = true;
                    break 'nodes;
                }
            }
        }
        if !expanded {
            return Ok(Step::Quiescent);
        }
        // A fresh node exists: re-run the deterministic rules over it.
    }
}

/// A ⊔ choice point: the forest as it was when the choice arose, plus which disjuncts
/// remain to try.
struct ChoicePoint {
    forest: Forest,
    node: usize,
    disjuncts: Vec<ConceptId>,
    next: usize,
}

/// Chronological backtracking: take the next untried disjunct from the top-most live
/// choice point, applying it to a copy of that point's forest. `None` = all branches
/// exhausted (Unsatisfiable). Disjunct trials count as rule applications.
fn next_branch(
    stack: &mut Vec<ChoicePoint>,
    sys: &System,
    s: &mut Search,
) -> Result<Option<Forest>, ExhaustedBudget> {
    while let Some(ch) = stack.last_mut() {
        if ch.next >= ch.disjuncts.len() {
            stack.pop();
            continue;
        }
        let d = ch.disjuncts[ch.next];
        ch.next += 1;
        let node = ch.node;
        s.count_rule()?;
        let mut f = ch.forest.clone();
        if !f.add(node, d, sys) {
            return Ok(Some(f));
        }
        // This disjunct clashes immediately; the loop tries the next one.
    }
    Ok(None)
}

/// Drive the search to a verdict (module docs §2/§6).
fn run(sys: &System, mut forest: Forest, budget: Budget) -> Verdict {
    if forest.nodes.len() > budget.max_nodes {
        return Verdict::Unknown(UnknownReason::ResourceBudget(ExhaustedBudget::Nodes));
    }
    let mut s = Search {
        budget,
        nodes_created: forest.nodes.len(),
        rules_applied: 0,
    };
    let mut stack: Vec<ChoicePoint> = Vec::new();
    loop {
        let step = match saturate(&mut forest, sys, &mut s) {
            Ok(step) => step,
            Err(b) => return Verdict::Unknown(UnknownReason::ResourceBudget(b)),
        };
        match step {
            Step::Quiescent => return Verdict::Satisfiable,
            Step::Choice { node, disjuncts } => {
                stack.push(ChoicePoint {
                    forest: forest.clone(),
                    node,
                    disjuncts,
                    next: 0,
                });
            }
            Step::Clash => {}
        }
        match next_branch(&mut stack, sys, &mut s) {
            Ok(Some(f)) => forest = f,
            Ok(None) => return Verdict::Unsatisfiable,
            Err(b) => return Verdict::Unknown(UnknownReason::ResourceBudget(b)),
        }
    }
}

// -------------------------------------------------------------------------------------------
// Direct unit tests (the integration suite lives in tests/tableau.rs)
// -------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ClassExpression as CE;

    const A: Id = 1;
    const B: Id = 2;

    fn subclass(sub: CE, sup: CE) -> Axiom {
        Axiom::SubClassOf { sub, sup }
    }

    fn assert_class(class: CE, individual: Id) -> Axiom {
        Axiom::ClassAssertion { class, individual }
    }

    fn onto(axioms: Vec<Axiom>) -> Ontology {
        Ontology { axioms }
    }

    #[test]
    fn verdict_predicates() {
        assert!(Verdict::Satisfiable.is_satisfiable());
        assert!(!Verdict::Satisfiable.is_unsatisfiable());
        assert!(Verdict::Unsatisfiable.is_unsatisfiable());
        assert!(!Verdict::Unsatisfiable.is_unknown());
        let u = Verdict::Unknown(UnknownReason::ResourceBudget(ExhaustedBudget::Nodes));
        assert!(u.is_unknown());
        assert!(!u.is_satisfiable());
    }

    #[test]
    fn budget_default_is_positive_and_copy() {
        let b = Budget::default();
        assert!(b.max_nodes > 0);
        assert!(b.max_rule_applications > 0);
        let c = b; // Copy
        assert_eq!(b, c);
    }

    #[test]
    fn consistency_direct_sat_and_unsat() {
        // Empty ontology: consistent (checked over the fresh non-empty-domain root).
        assert_eq!(
            consistency(&onto(vec![]), Budget::default()),
            Verdict::Satisfiable
        );
        // a : A with A ⊑ ⊥: inconsistent.
        let o = onto(vec![
            subclass(CE::Class(A), CE::Nothing),
            assert_class(CE::Class(A), 100),
        ]);
        assert_eq!(consistency(&o, Budget::default()), Verdict::Unsatisfiable);
    }

    #[test]
    fn class_satisfiability_direct() {
        // A w.r.t. {A ⊑ ⊥}: unsatisfiable.
        let o = onto(vec![subclass(CE::Class(A), CE::Nothing)]);
        assert_eq!(
            class_satisfiability(&CE::Class(A), &o, Budget::default()),
            Verdict::Unsatisfiable
        );
        // A w.r.t. {A ⊑ B}: satisfiable.
        let o = onto(vec![subclass(CE::Class(A), CE::Class(B))]);
        assert_eq!(
            class_satisfiability(&CE::Class(A), &o, Budget::default()),
            Verdict::Satisfiable
        );
        // ⊥ is unsatisfiable w.r.t. anything (seed clash path).
        assert_eq!(
            class_satisfiability(&CE::Nothing, &onto(vec![]), Budget::default()),
            Verdict::Unsatisfiable
        );
    }

    #[test]
    fn consistency_from_extraction_direct() {
        // Extraction failure: fail-closed Unknown(OutOfFragment), never a verdict.
        let err: Result<Ontology, ExtractError> =
            Err(ExtractError::OutOfFragment("owl:inverseOf".into()));
        match consistency_from_extraction(&err, Budget::default()) {
            Verdict::Unknown(UnknownReason::OutOfFragment(msg)) => {
                assert!(
                    msg.contains("owl:inverseOf"),
                    "diagnostic must carry the cause"
                );
            }
            other => panic!("expected Unknown(OutOfFragment), got {:?}", other),
        }
        // Ok delegates to consistency().
        let ok: Result<Ontology, ExtractError> = Ok(onto(vec![]));
        assert_eq!(
            consistency_from_extraction(&ok, Budget::default()),
            Verdict::Satisfiable
        );
    }

    #[test]
    fn nested_input_exercises_closure_interner_alignment() {
        // Deeply nested NNF-relevant input: the debug_assert in preprocess() cross-checks
        // the interner against nnf::subexpression_closure on this run.
        let complex = CE::ObjectComplementOf(Box::new(CE::ObjectIntersectionOf(vec![
            CE::Class(A),
            CE::some(10, CE::ObjectUnionOf(vec![CE::Class(B), CE::Thing])),
        ])));
        let o = onto(vec![
            subclass(CE::Class(A), complex.clone()),
            Axiom::EquivalentClasses(CE::Class(B), CE::only(10, CE::Class(A))),
            assert_class(complex, 100),
        ]);
        // Verdict value is not the point here; it must simply be a verdict, not a panic.
        let v = consistency(&o, Budget::default());
        assert!(v.is_satisfiable() || v.is_unsatisfiable());
    }
}
