# OWL 2 QL: broadening the CQ-shape gate + de-experimentalising the entailment arm where sound

<!-- [FABLE-5] Design record for epic sq-pbz04.3 (program sq-pbz04 / sq-6tykl). Decomposition
     stage of the Fable collaboration tier: this record is the ONE research artifact for the
     epic; the implementation is carried by the disjoint child beads sq-pbz04.3.1–.5 listed in
     §7. Author: SPARQ agent (Claude Fable). -->

**Bead:** `sq-pbz04.3` · **Crate under design:** `sparq-reason-ql` (+ the `sparq-conformance`
QL arm) · **Status:** decomposition record — no code changes in this PR.

## 0. Verdict (summary)

The QL estate is stronger than the epic's framing assumes — the "production path"
(PerfectRef ∪ tree-witness ∪ UCQ-containment minimisation) is **implemented and
oracle-verified**, so the old QL-track beads `sq-0vcwq` (minimisation) and half of `sq-4ldc4`
(tree-witness) are **stale-landed** and are closed/superseded by this record. What genuinely
remains is exactly the epic's items (a) and (c), plus one evaluation:

1. **Broaden the fail-closed CQ-shape gate** with five shapes that each carry a written
   soundness argument (§5): UCQ input, distinguished-variable `FILTER`, constant `VALUES`,
   literal-object role atoms, and non-recursive property-path desugaring — **plus one
   deliberate narrowing**, the intensional-atom guard (§5 B6), without which the current
   rewriter silently *under-answers* schema-vocabulary queries instead of abstaining.
2. **De-experimentalise the `pr:QL` entailment arm only for the provably-sound subset**, via a
   conjunctive graduation predicate (§6). Two previously-undocumented traps make a naive
   graduation unsound: the **TBox-capture accounting hole** (§3) and the **certain-answer vs
   entailment-regime semantics divergence** (§4). Both get explicit guards. Intensional cases,
   BIND arithmetic, and variable-predicate queries stay experimental **permanently**, with a
   reason taxonomy that says so.
3. **Evaluate** (not build-by-default) the combined-approach / H-complete-ABox optimisation —
   it is a performance lever, not a soundness gap, and may end as a reasoned non-adoption
   (the EL/QL substrate precedent).

Five disjoint child beads, dependency-chained where they share files (§7). Everything stays
opt-in (`experimental` / `ql-experimental` features, off by default); the graduated floor is a
`sparq extension` row, **never** summed into the standards-conformance total.

## 1. Verified estate (read against `origin/main`, not taken on faith)

Verified by reading the actual code at the branch point (commit `8490f6d8`); the
`sparq-reason-ql` crate and the conformance QL files are identical between the drifted work
checkout and `origin/main`.

| Piece | State | Where |
| --- | --- | --- |
| Fail-closed CQ-shape gate | **implemented + always compiled** (no feature gate) | `src/cq.rs` — rejects OPTIONAL/FILTER/UNION/MINUS/paths/GRAPH/BIND/aggregation/SERVICE/VALUES/ORDER BY/Slice/sub-SELECT/LATERAL/variable-predicate/RDF-star |
| PerfectRef rewrite/reduce saturation | **implemented + oracle-verified** | `src/perfectref.rs` (behind `experimental`) |
| Tree-witness folding | **implemented** (bounded, no unbounded chase) | `src/treewitness.rs` — half of old `sq-4ldc4`, landed via the `sq-g19x0` estate |
| UCQ-containment minimisation | **implemented, fail-closed on budget** | `src/minimise.rs` — this IS old `sq-0vcwq`, landed |
| DL-Lite_R TBox extraction | **implemented, positive inclusions only** | `src/dllite.rs` — but see the accounting hole, §3 |
| Formal DL-Lite_R certain-answer floor | **graduated, pinned** (`sq-qo1a9`, PR #1316) | `sparq-conformance` `ql_dllite_suite`, a `sparq extension` scoreboard row |
| Broad `pr:QL` `sparql11/entailment` arm | **experimental / OutOfScope, never graduated** | `sparq-conformance/tests/ql_experimental_arm.rs` + `src/inference/sparql_entail.rs` (`QlArmOutcome`), opt-in `ql-experimental` |

**Stale-bead findings** (the epic asked for this verification):

- `sq-0vcwq` ("UCQ minimization by query containment") — **landed**: `minimise.rs` implements
  exactly this, wired into `rewrite_production`, with the fail-closed keep-on-undecided budget
  rule. Closed as done, pointing here.
- `sq-4ldc4` ("tree-witness rewriting over H-complete ABox + combined-approach") — **half
  landed**: tree-witness folding is in `treewitness.rs`; the H-complete-ABox / combined
  approach is **not** built (the rewrite runs over the unmodified ABox, by design). Closed as
  superseded by child bead `sq-pbz04.3.5`, which re-scopes the un-landed half as an
  evaluate-first spike.

**What the `pr:QL` suite actually contains** (verified against
`tests/w3c/rdf-tests/sparql/sparql11/entailment/`): the `pr:QL`-tagged cases mix
(i) extensional ABox CQs (`rdf04`-style `?x rdf:type :c`), (ii) literal-object role atoms
(`lang`/`plainLit`-style `?x foaf:name "name"@en` — currently **abstained** because `emit.rs`
rejects literals in atom positions, a soundly-liftable ground, §5 B2), (iii) **intensional**
schema queries (`paper-sparqldl-Q1`: `?c rdfs:subClassOf ex:Student`) that the gate currently
**admits** and the rewriter then under-answers (§5 B6), (iv) variable-predicate queries
(`paper-sparqldl-Q5`) — already correctly abstained, and (v) `bind01`–`bind08` BIND-arithmetic
cases — already correctly abstained, permanently outside a UCQ rewriting.

## 2. The soundness centre: PerfectRef applicability

The research records call the **applicability condition** the #1 unsoundness trap for QL, and
the code confirms it is the crate's load-bearing invariant: an existential-introducing
inclusion `A ⊑ ∃R` may rewrite a role atom `R(x, y)` into `A(x)` **only when `y` is unbound —
non-distinguished and non-shared** (`perfectref.rs` header + `applicable_exists_super` /
`is_bound_var`; the reduce/unify step can *re-enable* rewrites by making a variable unshared).
Firing it on a bound/shared/answer variable invents answers; suppressing legal firings loses
them. Every broadening in §5 therefore carries an explicit argument of the form: *"this shape
change does not create a new way for an existential inclusion to fire on a bound position, and
does not change which positions count as bound."* Where a shape cannot carry that argument, it
stays rejected — fail-closed is preserved as the default for everything not proven.

## 3. Second trap (found in this review): the TBox-capture accounting hole

`TBox::extract` (`dllite.rs`) counts axioms it recognises-but-skips (`TBox::skipped`), but its
match arms only cover `rdfs:subClassOf` / `subPropertyOf` / `domain` / `range` /
`owl:inverseOf`. Everything else falls into the *"not a TBox-shaping predicate"* arm and is
**silently ignored — not counted**. Concretely:

- `owl:equivalentClass` / `owl:equivalentProperty` — **QL-legal** (each is a pair of
  inclusions) but currently ignored, so the rewrite is silently *incomplete* for TBoxes that
  use them, while `skipped_axioms == 0` falsely suggests total capture.
- `owl:disjointWith`, `owl:propertyDisjointWith`, `owl:complementOf`-shaped negative axioms —
  QL-legal but **consistency-relevant**: they never contribute to positive rewriting, yet an
  inconsistent KB certain-answers *everything*, so their mere presence means the UCQ answers
  may under-approximate the certain answers. The crate documents "no consistency checking";
  the accounting must make that visible per-TBox, not just in prose.
- Anything else in the `rdfs:`/`owl:` vocabulary (e.g. `owl:FunctionalProperty` typing — not
  in QL at all) — must be *counted* as unrecognised schema vocabulary, not dropped.

Consequence: **`skipped_axioms == 0` is NOT a sufficient "TBox fully captured" signal**, and
the graduation predicate in §6 cannot be built on it as-is. Child bead `.3` fixes the
accounting (capture the equivalences; tally `consistency_relevant` and `unrecognised_schema`;
expose a decidable `fully_captured()`), and `.4` builds the predicate on top.

## 4. Third trap: certain-answer vs entailment-regime semantics divergence

The crate's contract is **certain answers of the (U)CQ** — non-distinguished variables are
existentially quantified, and an anonymous TBox-generated witness can support an answer
(`Person ⊑ ∃hasParent`, data `:a a :Person`, query `SELECT ?x WHERE { ?x :hasParent ?y }`
certain-answers `{a}` even though no `?y` binding exists in the data). SPARQL solution
mappings, by contrast, bind **every** variable to an RDF term, and the W3C entailment-regime
test oracles are written against that regime semantics — under which the same query may have
**no** solution. The two semantics coincide on the DL-Lite_R oracle floor (hand-derived
certain answers), but for `pr:QL` entailment-suite graduation the divergence is a real
unsoundness risk in *both* directions.

Conservative, fail-closed handling (until the exact spec condition is pinned): a case is
graduation-eligible only if **all body variables are distinguished** (projected) **or** the
TBox has **no existential-generating inclusions** (`exists_super` empty) — in either case no
anonymous-witness answer can arise, so the semantics provably coincide. Child bead `.4`
resolves the precise coincidence condition against the SPARQL 1.1 Entailment Regimes spec text
and may widen this guard **only** with a written argument; the default stays the conjunction
above.

## 5. Broadening analysis — each shape with its soundness argument

Verdicts: **ADMIT** (child bead implements), **GUARD** (deliberate narrowing for honesty),
**DEFER** (documented, not built now), **PERMANENT-REJECT** (stays fail-closed forever).

### B1. UCQ input (top-level `UNION` of CQ branches) — ADMIT (bead .1)

PerfectRef is *defined* over UCQs in the source paper (Calvanese, De Giacomo, Lembo,
Lenzerini, Rosati, JAR 2007): it rewrites each disjunct and unions the results. Certain
answers distribute over union in DL-Lite_R via the canonical-model property, so
`cert(q1 ∨ q2) = cert(q1) ∪ cert(q2)` — per-branch rewriting is sound *and* complete for the
union. Applicability is per-branch and unchanged. Plumbing: the gate gains a
union-of-branches classification (each branch must independently pass the CQ gate **and bind
every distinguished variable** — a branch leaving a projected variable unbound is rejected,
fail-closed, because UCQ answer-arity is uniform); `rewrite`/`rewrite_production` rewrite each
branch and concatenate before minimisation (cross-branch containment minimisation remains
sound — it only ever drops a disjunct proven contained in a retained one).

### B2. Literal constants as role-atom objects — ADMIT (bead .1)

`?x foaf:name "name"@en` currently dies in `emit.rs` (`literal in a class/role atom position
is out of DL-Lite scope`) even though the gate accepts it — so `lang`/`plainLit`-class suite
cases abstain unnecessarily. Soundness: a constant (IRI *or* literal) is never an unbound
position, so no existential inclusion is applicable to it — the applicability condition is
untouched; role inclusions `R ⊑ S` rewrite `S(x, lit)` to `R(x, lit)` carrying the constant
unchanged, which is the ordinary constant-propagation PerfectRef already performs for IRI
constants. Requires a literal-carrying constant variant in the internal `Term` (today `Const`
is an IRI string), with match arms in `perfectref.rs`/`treewitness.rs`/`minimise.rs` treating
it exactly like `Const` (never unifiable with a generated witness, distinct literals never
equal). No TBox interaction: DL-Lite_R data ranges are outside the extracted fragment and stay
skipped-counted.

### B3. `FILTER` over distinguished-only variables — ADMIT (bead .1)

For `q'(x) = q(x) ∧ F(x)` with `vars(F) ⊆ distinguished`: certain answers are ground bindings
of the answer variables, and `F` reads only those, so
`cert(q') = { a ∈ cert(q) : F(a) evaluates true }` — which is exactly what the engine computes
if the rewritten UCQ is wrapped in the *original, unmodified* `Filter(F, ·)`. The rewriter
never interprets `F`; it passes the expression through, so SPARQL evaluation semantics
(including effective-boolean-value errors → row dropped) are identical on both sides. This
also matches the entailment-regime stance that only BGP matching changes and the surrounding
algebra is standard. **Fail-closed:** a filter mentioning any non-distinguished variable is
rejected (its value on an anonymous witness is undefined — the §4 divergence in miniature).

### B4. Constant-only `VALUES` over distinguished variables — ADMIT (bead .1)

A `VALUES` block whose variables are all distinguished and whose rows are all fully bound
constants is equivalent to a disjunction of equality filters over answer variables
(SPARQL-algebra equivalence, independent of entailment), so B3's argument applies verbatim.
`UNDEF` cells or non-distinguished variables → rejected, fail-closed.

### B5. Non-recursive property-path desugaring (`/`, `^`, `|`) — ADMIT, sequenced (bead .2)

`X p1/p2 Y ≡ X p1 ?f . ?f p2 Y` (fresh non-distinguished `?f`), `X ^p Y ≡ Y p X`, and
`X p1|p2 Y ≡ { X p1 Y } UNION { X p2 Y }` are SPARQL-level query equivalences (the SPARQL 1.1
algebra translation), applied *before* the gate, after which B1/PerfectRef soundness applies
to the result. The fresh intermediate variable is existential — exactly the position
PerfectRef's applicability condition governs, and exactly the crate's certain-answer contract
(note: this makes path-desugared queries subject to the §4 regime-coincidence guard for
graduation purposes, like any query with non-distinguished variables). `+`, `*`, `?`
(zero-length-path semantics), and negated property sets stay **PERMANENT-REJECT** — they
reintroduce the recursion/reflexivity QL deliberately excludes. First implementation step is
to *verify* which forms spargebra already normalises to BGPs before writing any desugaring
(unverified today; the bead must not double-translate).

### B6. Intensional-atom guard — GUARD, a deliberate narrowing (bead .1)

The gate currently **admits** `?c rdfs:subClassOf ex:Student` (constant predicate → looks
like a role atom), and the rewriter evaluates it over asserted triples only — silently
missing TBox-entailed schema facts (reflexivity, transitive closure, entailed
subsumptions). That violates the completeness half of the certain-answer contract without
tripping fail-closed. Fix: a role atom whose predicate is **semantics-bearing schema
vocabulary** (`rdfs:subClassOf`, `rdfs:subPropertyOf`, `rdfs:domain`, `rdfs:range`, and the
`owl:` axiom vocabulary — the record's implementing bead carries the exact enumerated list)
is rejected `OutOfScope("intensional/schema-vocabulary atom")`. Annotation-ish predicates
(`rdfs:label`, `rdfs:comment`, `rdfs:seeAlso`, `rdfs:isDefinedBy`) remain admitted as plain
role atoms — no QL TBox axiom changes their extension. `rdf:type` with a *variable* object is
already rejected in `emit.rs`; the guard makes the constant-predicate schema case symmetric.
Effect on existing floors: none — the `ql_dllite_suite` cases are extensional ABox queries.
Effect on the arm: `paper-sparqldl-Q1/Q4`-class rows flip from computed-divergent to an honest
abstain.

### Explicitly out of scope (unchanged, with reasons)

`OPTIONAL`/`MINUS` (non-monotone — no certain-answer-preserving UCQ rewriting), aggregation
and `BIND` arithmetic (not CQ-answering), `SERVICE`, named-graph `GRAPH` (per-graph rewriting
semantics unsettled; the arm already reports named-graph datasets inconclusive), sub-SELECT,
RDF-star terms, variable predicates (not a DL-Lite atom). All remain **PERMANENT-REJECT** at
the gate, and the arm's reason taxonomy (§6) labels them as permanently outside sound
rewriting rather than "pending".

## 6. De-experimentalisation disposition (epic item c)

**What can graduate — and only under the full conjunctive predicate.** A `pr:QL` entailment
row moves from the experimental OutOfScope bucket to a **pinned, named-case floor** iff ALL
of:

1. the (broadened) CQ-shape gate accepts the query — including the B6 intensional guard;
2. the TBox is **totally captured**: `fully_captured()` from bead `.3` — zero skipped, zero
   unrecognised schema vocabulary;
3. **zero consistency-relevant axioms** present (no negative inclusions — until a consistency
   check exists, their presence means the UCQ may under-approximate; see §8);
4. default-graph dataset only (the existing arm precondition);
5. the §4 **regime-coincidence guard** holds (all body variables distinguished, or no
   existential-generating inclusions);
6. the rewritten UCQ's evaluation over the unmodified data is **result-equivalent to the W3C
   oracle** for that case.

Conditions 1–5 are the *a-priori* soundness argument (each removes a documented divergence
class); condition 6 is the empirical pin. The floor is a **named list of test cases** (the
`ql_dllite_suite` graduation pattern), so a regression on any graduated case is a hard test
failure, and newly-eligible cases (as B1–B5 land) ratchet the list up in later PRs.

**Honesty stance of the graduated floor.** It is a `sparq extension` scoreboard row — the
W3C entailment suite is normative, but sparq implements a *fragment* of the QL regime
(abstaining elsewhere), so the row is tallied separately and **never summed into the
standards-conformance total**, exactly like the DL-Lite_R oracle floor. No full-regime or
full-profile conformance claim is made anywhere.

**What stays experimental, with a reason taxonomy.** Every non-graduated row keeps its
`OutOfScope` reporting, upgraded with a taxonomy that distinguishes:

- `permanently-outside` — BIND arithmetic, variable predicates, intensional schema queries,
  OPTIONAL/MINUS shapes: no sound rewriting exists in this design; documented divergence.
- `pending-gate` — shapes B1–B5 not yet landed at graduation time.
- `pending-consistency` — TBoxes with consistency-relevant axioms (§8 deferral).
- `pending-coincidence` — cases failing only the §4 guard, awaiting the spec-pinned condition.

**What is NOT renamed.** The `experimental` cargo feature (crate) and `ql-experimental`
(conformance) keep their names — renaming a published feature is a breaking change and the
crate-level rewriter remains honestly experimental until the graduated floor has soaked; only
the *arm's sound subset* de-experimentalises.

## 7. Decomposition — child beads (disjoint by file, chained where files are shared)

Wave shape: `.3` first (small, unblocks two others) → `.1` ∥ `.4` (different crates) → `.2` →
`.5`. No two beads touch the same file; beads sharing a file are dependency-chained, never
parallel.

| Bead | Tier | Crate | One-line scope | File area (exclusive) |
| --- | --- | --- | --- | --- |
| `sq-pbz04.3.3` | sonnet | sparq-reason-ql | total TBox-capture accounting (§3): capture `equivalentClass`/`equivalentProperty`, tally consistency-relevant + unrecognised schema vocab, expose `fully_captured()` | `src/dllite.rs`, `tests/tbox_capture.rs` (new) |
| `sq-pbz04.3.1` | opus | sparq-reason-ql | broaden the sound fragment: B1 UCQ + B2 literal atoms + B3 FILTER + B4 VALUES + B6 intensional guard | `src/cq.rs`, `src/lib.rs`, `src/emit.rs`, `src/perfectref.rs`, `src/treewitness.rs`, `src/minimise.rs`, `tests/broadened_shapes.rs` (new), `tests/oracle.rs`, `README.md`, `skills/inference/SKILL.md` |
| `sq-pbz04.3.4` | opus | sparq-conformance | graduate the sound `pr:QL` subset behind the §6 conjunctive predicate; reason taxonomy for the rest | `src/inference/sparql_entail.rs`, `src/scoreboard.rs`, `tests/ql_experimental_arm.rs`, `tests/ql_entailment_floor.rs` (new) |
| `sq-pbz04.3.2` | sonnet | sparq-reason-ql | B5 non-recursive path desugaring (verify spargebra normalisation first); recursive forms stay rejected | `src/cq.rs`, `tests/path_desugar.rs` (new), `README.md` |
| `sq-pbz04.3.5` | opus | sparq-reason-ql | combined-approach / H-complete-ABox **evaluate-first spike** (supersedes `sq-4ldc4`): implement only if the measured UCQ sizes warrant it, else documented reasoned non-adoption | `src/combined.rs` (new, only if adopted), `examples/ql_rewrite_bench.rs`, `src/lib.rs`, `README.md` |

Dependency edges (real orderings only): `.1 ← .3` and `.4 ← .3` (the accounting lands first —
`.4` builds its predicate on it; `.1` then starts from the fresh TBox surface); `.2 ← .1`
(shares `src/cq.rs` + `README.md`); `.5 ← .1` (shares `src/lib.rs`) and `.5 ← .2` (shares
`README.md`). `.1` and `.4` run in parallel (different crates); `.4` gains a larger initial
floor if `.1` happens to land first, but does not require it (the floor ratchets upward).

Per-bead invariants and acceptance tests are carried in the bead descriptions (the fleet's
spec of record); the shared soundness invariant across all five is: **fail-closed preserved —
every construct outside the proven-sound fragment is rejected, never mis-answered — and the
pinned `ql_dllite_suite` floor never regresses.**

## 8. Deferred (documented, not built now)

- **DL-Lite_R consistency checking.** ~~Deferred~~ **LANDED (sq-p6yb7, 2026-07-12)** as the
  opt-in `ql-consistency` feature (`src/consistency.rs`): a violation query per structurally
  captured negative inclusion (`TBox::neg_incl`, from `owl:disjointWith` /
  `owl:propertyDisjointWith`), rewritten through the existing PerfectRef saturation and
  evaluated over the data — INCONSISTENT iff some violation query matches (sound at any
  capture level); definitive CONSISTENT only for a fully-captured TBox with
  `consistency_uncaptured == 0` (`owl:complementOf` is never structurally captured — it is an
  equivalence-with-complement, stronger than a negative inclusion — so it stays fail-closed
  Unknown). **Amended (sq-fj8lj follow-up):** that last parenthetical holds only for a
  NAMED-subject `A owl:complementOf B`. The **subClassOf-complement** spelling
  `A rdfs:subClassOf [ owl:complementOf B ]` is QL's
  `superClassExpression ::= ObjectComplementOf(subClassExpression)` production — i.e. exactly the
  negative inclusion `A ⊑ ¬B`, with no stronger `¬B ⊑ A` half — and IS now structurally captured
  into `TBox::neg_incl` (counted `consistency_relevant`, no longer `skipped`), which is what lets
  a graph routed to sparq-reason-dl's QL dispatch branch reach a definitive **Consistent** rather
  than only **Inconsistent**-or-Unknown. The cln(T) completeness argument is unchanged: the new
  entries are the same `NegativeInclusion::Concept(Basic, Basic)` shape and compose the same
  violation query, so no closure rule or witness shape is added (argued in
  `crates/sparq-reason-ql/src/consistency.rs`). No suite case moves: the only `owl:complementOf`
  occurrence in the pinned rdf-tests `sparql11/entailment` corpus is inside a QUERY
  (`paper-sparqldl-Q3.rq`), never in a queried TBox — checked at pin
  `f25dbc092c654d792974848e81bb519d7328f0e8` — and the DL-direct corpus was measured under
  sq-fj8lj to dispatch ZERO rows to the QL branch at all (see `DL_DIRECT_FLOOR`).
  §6 condition 3 upgraded accordingly (`ql_condition3_consistency`): a
  proven-CONSISTENT KB passes; proven-INCONSISTENT holds at the new `inconsistent-kb`
  taxonomy label (entailment-regime behaviour on an inconsistent graph is
  implementation-defined); Unknown stays `pending-consistency`. Measured at the current
  rdf-tests pin the `pending-consistency` bucket was ZERO before the upgrade, so it
  graduates no suite case today — the floor is unchanged at 15, and the check is pinned by
  the crate's own oracle suite (`tests/consistency_oracle.rs`, hand-derived verdicts).
- **Named-graph (`GRAPH`) rewriting semantics** — stays inconclusive in the arm, as today.
- **`p?` / zero-length paths** — SPARQL zero-length-path semantics binds nodes reflexively
  over graph terms; no clean certain-answer story; stays rejected with `+`/`*`.
- **Feature renaming / un-experimentalising the crate feature** — out of scope (§6).

## 9. Stale-bead disposition (executed with this record)

- `sq-0vcwq` → **closed** (landed: `minimise.rs`, fail-closed containment minimisation, wired
  into `rewrite_production`; verified against `origin/main`).
- `sq-4ldc4` → **closed as superseded** by `sq-pbz04.3.5` (tree-witness half landed in
  `treewitness.rs`; the H-complete/combined half re-scoped as an evaluate-first spike).

## 10. Combined-approach / H-complete-ABox evaluate-first spike (`sq-pbz04.3.5`) — reasoned NON-ADOPTION

[OPUS-4.8] `sq-pbz04.3.5` (supersedes `sq-4ldc4`). This is the §0-item-3 evaluation, executed
measure-first. The combined approach (Kontchakov, Lutz, Toman, Wolter, Zakharyaschev — *The
Combined Approach to Query Answering in DL-Lite*, IJCAI 2011) trades a large query **rewriting**
for a hierarchy-completed (**H-complete**) ABox plus a small filtering step. It is a **performance
lever, not a soundness gap**, and it wins **only when pure PerfectRef rewriting blows the UCQ up
while ABox saturation stays cheap**. Whether it pays off here is therefore an empirical question
about the **measured UCQ-size distribution** on the real corpus — not the literature's worst-case
claim. **Verdict: NON-ADOPTION.** The numbers are below; no combined-approach code is added.

### 10.1 What was measured (deterministic; counts, not wall-clock)

The minimised-UCQ size (`rewrite_production`: PerfectRef ∪ tree-witness ∪ containment
minimisation) is a **pure function of the (TBox, query)**, so every count is a closed form. The
reproducible harness is `crates/sparq-reason-ql/examples/ql_rewrite_bench.rs` (the
`run_corpus_profile` section — self-asserting closed forms, no fetched fixtures). Two axes:

- **Realistic corpus** — the 11 graduated DL-Lite_R oracle shapes (mirror of
  `sparq-conformance`'s `QL_DLLITE_ORACLE` C1..C11) **plus** the answered W3C `pr:QL`
  extensional-CQ shapes (single/double class atom over a shallow RDFS hierarchy, a literal-object
  role atom): **14 answered cases**.
- **Adversarial ceiling probes** — multi-atom CQs over **independent** subclass chains (the
  product shape), the only construction that forces a blow-up.

### 10.2 The measured distribution

| Corpus | Answered | min UCQ | median | max UCQ | Blow-up? |
| --- | --- | --- | --- | --- | --- |
| Realistic (oracle ∪ W3C answered) | 14 | 1 | 2 | **4** | none |

Realistic distribution (minimised UCQ size): `[1] = 4`, `[2–4] = 10`, `[5–16] = 0`, `[17+] = 0`.
**Nothing in the realistic corpus exceeds 4 disjuncts.**

The full W3C `pr:QL` entailment corpus is **31 cases**; the always-present CQ-shape gate splits it
**17 admitted / 14 held** (7 BIND-arithmetic, 3 variable-predicate, 4 intensional-schema atoms —
each `permanently-outside` sound rewriting per §5/§6). Of the 17 admitted, twelve are single-atom
and only three are multi-atom (`sparqldl-04` at 3 atoms, `sparqldl-06`/`-07` at 4) — **and those
three carry OWL-DL TBoxes (property chains, `owl:sameAs`) that fall outside DL-Lite_R and are
`skipped`**, so PerfectRef returns the identity UCQ (≈1 disjunct) despite the atom count. The
answered class/role-atom cases sit over depth-1/2 RDFS hierarchies, giving 1–3 disjuncts.

Ceiling probes (the shape that *does* blow up), closed forms `(k+1)^atoms`:

| Probe | k = 2 | k = 3 | k = 5 | k = 8 |
| --- | --- | --- | --- | --- |
| product, 2 class atoms, same var | 9 | 16 | 36 | 81 |
| product, 3 class atoms, same var | 27 | 64 | — | — |

For every product probe the pre-minimisation and minimised counts are **equal** — the product
disjuncts are mutually **incomparable**, so containment minimisation cannot collapse them. This is
precisely the case the combined approach would remove (one CQ over an H-complete ABox instead of
`(k+1)^atoms` CQs).

### 10.3 Why NON-ADOPTION is the honest verdict

1. **No blow-up on the real corpus.** The maximum measured realistic UCQ is **4 disjuncts**; the
   median is 2. A 4-way (or 36-way, or 81-way) union of BGPs is trivially evaluable by the
   in-memory engine — there is no rewriting cost for the combined approach to amortise.
2. **The two blow-up sources that the corpus *does* exhibit are already handled.** The existential
   chase is captured by **bounded tree-witness folding** (`treewitness.rs`), and redundant
   disjuncts are dropped by **UCQ-containment minimisation** (`minimise.rs`) — both landed and
   oracle-verified. The one blow-up the corpus does **not** exhibit (the incomparable product) is
   the *only* thing the combined approach would add, and it is absent.
3. **The blow-up requires a shape the corpus never produces:** a multi-atom conjunctive query
   whose atoms sit over **independent, non-trivial (depth ≥ 1) class hierarchies**. It is **not**
   the CQ-shape gate that bounds the UCQ (the gate admits multi-atom CQs) — it is the realistic
   **query and TBox shapes** (single-atom-dominated queries, shallow hierarchies, and multi-atom
   queries whose schema is outside DL-Lite_R). This distinction is the close-out lever below.
4. **Adopting it cuts against the crate's load-bearing architecture.** The crate is a **pure
   query-rewriter over the unmodified ABox**, reusing the engine's query path with **no store or
   planner changes** (README "Query-rewriter seam"). The combined approach requires **materialising
   an H-complete ABox** (data saturation) **plus** an anonymous-individual **filtering step** — a
   store-side write path and a non-standard evaluation the engine does not have. Paying that
   architectural cost for a UCQ that is already ≤ 4 disjuncts is a net regression, not a win.

This mirrors the EL/QL substrate **reasoned non-adoption** precedent (the CR1–CR5 substrate joins,
`research/owl2-rl-el-wave2-disposition.md` §3.1) and the RSP substrate non-adoption: adopt only
when a measurement — not a worst-case bound — shows the lever pays.

### 10.4 Close-out criteria — what would flip the calculus (revisit triggers)

Re-open the spike (a fresh child bead, not a reopen of this record) **iff** a future change makes
the product shape realistic AND the measured distribution shifts into the `[17+]` bucket:

- **Fragment broadening lands multi-atom CQs over deep independent hierarchies** — e.g. graduating
  a real-ontology QL benchmark whose TBox has wide/deep class hierarchies **and** whose query load
  joins several such class atoms on one variable (LUBM/NPD-style analytic CQs). The gate already
  admits the shape; only the corpus is missing.
- **A DL-Lite_R TBox with deep hierarchies enters the answered set** (e.g. `owl:equivalentClass`
  capture from `sq-pbz04.3.3` pulling in a large real vocabulary) so that a **single** rewritten
  atom's UCQ contribution itself grows past a handful.
- **Re-run `run_corpus_profile` after any such change**: if the realistic max/median climbs out of
  the `≤ 4` band and minimisation demonstrably cannot collapse the product, the combined approach
  becomes worth an evaluate-first **spike behind the `experimental` feature**, soundness-first,
  with a **differential oracle vs `rewrite_production`** (certain-answer set equality on the whole
  oracle suite) before any default-path change. Until then: no code, `rewrite_production` unchanged,
  fail-closed gate untouched.

## References

- Calvanese, De Giacomo, Lembo, Lenzerini, Rosati — *Tractable Reasoning and Efficient Query
  Answering in Description Logics: The DL-Lite Family*, JAR 39(3), 2007 (PerfectRef; defined
  over UCQs).
- W3C — *OWL 2 Profiles* §3 (OWL 2 QL); *SPARQL 1.1 Entailment Regimes* (the regime semantics
  `.4` must pin its coincidence condition against).
- Kontchakov, Lutz, Toman, Wolter, Zakharyaschev — *The Combined Approach to Query Answering in
  DL-Lite*, IJCAI 2011 (the H-complete-ABox + filtering alternative to pure rewriting; §10). Cf.
  Kontchakov, Rodríguez-Muro, Zakharyaschev — *Ontology-Based Data Access with Databases: A Short
  Course*, RW 2013 (tree-witness rewriting, the landed `treewitness.rs`).
- `research/owl2-el-ql-reasoning-spike.md` (the QL track; the applicability trap) and
  `research/reasoner-suite-on-substrate.md` §2.5 (the PerfectRef trap; sequencing).
- Repo estate: `crates/sparq-reason-ql/` (`sq-t5bne`, `sq-g19x0`), the graduated DL-Lite_R
  floor (`sq-qo1a9`, PR #1316), the experimental arm (`sq-kuvu3`).
